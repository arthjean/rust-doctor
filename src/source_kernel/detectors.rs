//! Registry of the native detectors.
//!
//! A detector declares the rule it carries, the node kind it inspects and the
//! function that decides. The kernel walks the CST exactly once and solicits
//! every detector on the nodes it declared, so adding a detector widens no
//! analysis signature.
//!
//! No detector compares a written path against a string. A target is described
//! by a crate and segments, and the provenance of the call identifier is
//! resolved through the alias map, then through the dependency aliases of the
//! manifest.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::BTreeMap;

use ra_ap_syntax::ast::{self, HasArgList, HasAttrs, HasName, LiteralKind};
use ra_ap_syntax::{AstNode, Edition, SyntaxKind, SyntaxNode, TextRange};

use super::aliases::{AliasMap, Provenance};
use super::bindings::{self, Binding, Resolver};
use super::{literal_string, unanimous};
use crate::policy::{RuleDefinition, SOURCE_DISABLED_TLS};
use crate::source_text::{compact, intersects_errors};

mod blocking;
mod shell;
mod sql;

/// Dependency aliases of a unit: identifier written in the code to canonical
/// crate name.
pub(super) type CrateAliases = BTreeMap<String, String>;

/// Crates always available without a dependency declaration.
const SYSROOT: [&str; 3] = ["alloc", "core", "std"];

/// Longest chain of bindings followed from one expression. `let a = b;` then
/// `let b = c;` is two; a chain past this is abstained on rather than walked.
const BINDING_DEPTH: usize = 8;

/// Longest method chain a detector unwinds to reach the call that built its
/// receiver.
pub(super) const CHAIN_DEPTH: usize = 64;

/// Item targeted by a detector, described by segments rather than by a written
/// path: that is what lets the imported form and the fully qualified form be
/// recognized through the same mechanism.
struct Item {
    krate: &'static str,
    segments: &'static [&'static str],
}

pub(super) struct Detector {
    pub(super) definition: &'static RuleDefinition,
    pub(super) node: SyntaxKind,
    pub(super) inspect: fn(&Context<'_>, &SyntaxNode) -> Option<Detection>,
}

pub(super) struct Detection {
    pub(super) message: Cow<'static, str>,
    pub(super) range: TextRange,
}

/// What a detector knows about the analysed unit.
pub(super) struct Context<'a> {
    pub(super) aliases: &'a AliasMap,
    pub(super) bindings: &'a Resolver,
    pub(super) crates: &'a CrateAliases,
    pub(super) error_ranges: &'a [TextRange],
    pub(super) edition: Edition,
    /// Does the unit hold test material? Decided by the kernel from Cargo's
    /// target kind first, never by a detector reading a path.
    pub(super) test_code: bool,
    /// The last path `item_path` resolved, and its answer. Every detector of a
    /// call node resolves the same callee one after the other, so one slot
    /// turns three resolutions of it into one.
    pub(super) resolved: RefCell<Option<(TextRange, Option<Vec<String>>)>>,
}

impl Context<'_> {
    /// True when `callee` designates `item::member` once its first identifier
    /// is resolved.
    fn calls(&self, callee: &ast::Expr, item: &Item, member: &str) -> bool {
        let ast::Expr::PathExpr(expression) = callee else {
            return false;
        };
        expression
            .path()
            .and_then(|path| self.item_path(&path))
            .is_some_and(|resolved| {
                resolved.split_first().is_some_and(|(krate, rest)| {
                    krate == item.krate
                        && rest.split_last().is_some_and(|(last, segments)| {
                            last == member && segments == item.segments
                        })
                })
            })
    }

    /// The item a path names, crate first, once its first segment is resolved
    /// through the imports in scope and the crate through the manifest's
    /// dependency names. Generic arguments name no item and are left out; a
    /// relative keyword, a type anchor or an undecidable import abstain.
    ///
    /// A head only globs could have bound is read as the crate it spells when
    /// two things prove it: the head is a crate name, of the sysroot or of the
    /// manifest, and every glob in the way imports from an external crate.
    /// `use futures::prelude::*;` does not rebind `std`; a glob of the crate's
    /// own modules could, and nothing outside them says it does not.
    pub(super) fn item_path(&self, path: &ast::Path) -> Option<Vec<String>> {
        let range = path.syntax().text_range();
        if let Some((cached, answer)) = self.resolved.borrow().as_ref()
            && *cached == range
        {
            return answer.clone();
        }
        let answer = self.resolve(path);
        *self.resolved.borrow_mut() = Some((range, answer.clone()));
        answer
    }

    fn resolve(&self, path: &ast::Path) -> Option<Vec<String>> {
        let written = item_segments(path)?;
        let (first, tail) = written.split_first()?;
        let resolved: Vec<String> = match self.aliases.provenance(path.syntax(), first) {
            Provenance::Item(item) => item.iter().chain(tail).cloned().collect(),
            Provenance::Unbound => written.clone(),
            Provenance::GlobMasked(heads)
                if self.external_crate(first)
                    && heads.iter().all(|head| head.is_some_and(|head| self.external_crate(head))) =>
            {
                written.clone()
            }
            Provenance::GlobMasked(_) | Provenance::Indeterminate => return None,
        };
        let (root, rest) = resolved.split_first()?;
        let krate = if SYSROOT.contains(&root.as_str()) {
            root.clone()
        } else {
            self.crates.get(root)?.clone()
        };
        Some(std::iter::once(krate).chain(rest.iter().cloned()).collect())
    }

    /// The one `fn` item a call path names in this file, when nothing else
    /// can: a single plain segment no import and no local binding binds,
    /// matched against the `fn` items of the nearest enclosing block or module
    /// that declares one of that name, or `Self::f` against the enclosing
    /// `impl`. An item shadows a glob of its own scope and of every scope
    /// around it, but a glob of a nearer scope may bind the name first, and
    /// abstains. Two items of the name abstain, which is what a `cfg` pair
    /// looks like from here.
    pub(super) fn local_function(&self, path: &ast::Path) -> Option<ast::Fn> {
        let segment = path.segment()?;
        if segment.syntax().children().count() != 1 {
            return None;
        }
        let ast::PathSegmentKind::Name(name) = segment.kind()? else {
            return None;
        };
        let name = name.text().to_string();
        if let Some(qualifier) = path.qualifier() {
            return names_self(&qualifier)
                .then(|| self.impl_function(path.syntax(), &name))
                .flatten();
        }
        if !matches!(
            self.aliases.provenance(path.syntax(), &name),
            Provenance::Unbound | Provenance::GlobMasked(_)
        ) || self.bindings.bound_locally(path.syntax(), &name)
        {
            return None;
        }
        for scope in path.syntax().ancestors() {
            let Some(items) = item_scope(&scope) else {
                continue;
            };
            let mut found = items
                .children()
                .filter_map(ast::Fn::cast)
                .filter(|function| function.name().is_some_and(|declared| declared.text() == name));
            match (found.next(), found.next()) {
                (Some(function), None) => return Some(function),
                (Some(_), Some(_)) => return None,
                (None, _) => {}
            }
            if self.aliases.globbed(&scope) || !ast::BlockExpr::can_cast(scope.kind()) {
                return None;
            }
        }
        None
    }

    /// The one method `name` of the `impl` enclosing `site`: what `Self::f`
    /// and `self.f()` call there.
    pub(super) fn impl_function(&self, site: &SyntaxNode, name: &str) -> Option<ast::Fn> {
        let block = site.ancestors().find_map(ast::Impl::cast)?;
        let mut found = block
            .assoc_item_list()?
            .assoc_items()
            .filter_map(|item| match item {
                ast::AssocItem::Fn(function) => Some(function),
                _ => None,
            })
            .filter(|function| function.name().is_some_and(|declared| declared.text() == name));
        match (found.next(), found.next()) {
            (Some(function), None) => Some(function),
            _ => None,
        }
    }

    /// Is `name` a crate this unit can reach without a `use`: the sysroot's or
    /// a dependency's, under the name the manifest gives it?
    fn external_crate(&self, name: &str) -> bool {
        SYSROOT.contains(&name) || self.crates.contains_key(name)
    }

    /// What `expression` holds once local bindings and `const` items are
    /// followed, parentheses aside: the initializer of the `let` a bare
    /// identifier names, and so on, or the expression itself when it names no
    /// binding. `None` when a binding on the way is indeterminate, which is
    /// where a detector abstains rather than guess.
    pub(super) fn value(&self, expression: ast::Expr) -> Option<ast::Expr> {
        let mut expression = expression;
        for _ in 0..BINDING_DEPTH {
            expression = strip_parentheses(expression);
            let Some(name) = bindings::identifier(&expression) else {
                return Some(expression);
            };
            match self.bindings.binding(self.aliases, expression.syntax(), &name) {
                Binding::Value(value) => expression = value,
                Binding::Unbound => return Some(expression),
                Binding::Typed(_) | Binding::Indeterminate => return None,
            }
        }
        None
    }

    /// The expression a method chain starts from, through the bindings its
    /// receivers were stored in and the `?` and `.await` between its links:
    /// `cmd.arg("-c")` after `let mut cmd = Command::new("sh")` starts from
    /// `Command::new("sh")`, and `conn.execute(..)` after `let conn =
    /// Connection::open(path)?` from `Connection::open(path)`.
    fn chain_root(&self, expression: ast::Expr) -> Option<ast::Expr> {
        let mut expression = expression;
        for _ in 0..CHAIN_DEPTH {
            match self.value(strip_wrappers(expression))? {
                ast::Expr::MethodCallExpr(call) => expression = call.receiver()?,
                ast::Expr::TryExpr(tried) => expression = tried.expr()?,
                ast::Expr::AwaitExpr(awaited) => expression = awaited.expr()?,
                root => return Some(root),
            }
        }
        None
    }

    /// Is `expression` fixed at compile time? A literal, a `const` item, a
    /// string built from those with `+` or `.to_owned()`, or a path spelled
    /// the way Rust spells a constant, which is what a `const` or `static` of
    /// another module looks like from here.
    pub(super) fn constant(&self, expression: &ast::Expr) -> bool {
        let Some(expression) = self.value(strip_wrappers(expression.clone())) else {
            return false;
        };
        match strip_wrappers(expression) {
            ast::Expr::Literal(_) => true,
            ast::Expr::BinExpr(binary) => {
                binary.op_kind() == Some(ast::BinaryOp::ArithOp(ast::ArithOp::Add))
                    && [binary.lhs(), binary.rhs()]
                        .into_iter()
                        .all(|operand| operand.is_some_and(|operand| self.constant(&operand)))
            }
            ast::Expr::MethodCallExpr(call) => {
                call.name_ref().is_some_and(|name| {
                    matches!(name.text().as_str(), "to_owned" | "to_string" | "into")
                }) && call.arg_list().is_some_and(|arguments| arguments.args().next().is_none())
                    && call.receiver().is_some_and(|receiver| self.constant(&receiver))
            }
            ast::Expr::PathExpr(path) => path
                .path()
                .and_then(|path| plain_segments(&path))
                .and_then(|segments| segments.last().cloned())
                .is_some_and(|name| screaming(&name)),
            _ => false,
        }
    }

    /// Can `expression` carry text an attacker chose into the string it is
    /// interpolated in? Not when it is constant, and not when it is a number:
    /// a binding whose written type is a primitive, or a cast to one. `site` is
    /// where the names of those types are resolved, which is the macro call
    /// for an argument reparsed out of its token tree.
    pub(super) fn injectable(&self, expression: &ast::Expr, site: &SyntaxNode) -> bool {
        !self.constant(expression) && !self.numeric(expression, site)
    }

    /// `injectable` for a name a format string captures, `{user}`, read where
    /// the macro sits.
    fn injectable_capture(&self, site: &SyntaxNode, name: &str) -> bool {
        match self.bindings.binding(self.aliases, site, name) {
            Binding::Value(value) => {
                self.injectable(&value, site)
                    && !let_type(&value).is_some_and(|written| self.numeric_type(&written, site))
            }
            Binding::Typed(written) => !self.numeric_type(&written, site),
            Binding::Unbound => !screaming(name),
            Binding::Indeterminate => true,
        }
    }

    /// Is `expression` a number, by syntax alone: a cast to a numeric
    /// primitive, or a binding whose written type is one, followed through
    /// the `let`s it was stored in.
    fn numeric(&self, expression: &ast::Expr, site: &SyntaxNode) -> bool {
        let mut expression = strip_wrappers(expression.clone());
        for _ in 0..BINDING_DEPTH {
            if let ast::Expr::CastExpr(cast) = &expression {
                return cast.ty().is_some_and(|written| self.numeric_type(&written, site));
            }
            let Some(name) = bindings::identifier(&expression) else {
                return false;
            };
            match self.bindings.binding(self.aliases, expression.syntax(), &name) {
                Binding::Value(value) => {
                    if let Some(declared) = let_type(&value) {
                        return self.numeric_type(&declared, site);
                    }
                    expression = strip_wrappers(value);
                }
                Binding::Typed(written) => return self.numeric_type(&written, site),
                Binding::Unbound | Binding::Indeterminate => return false,
            }
        }
        false
    }

    /// Is this written type a number? A primitive spelled by its own name,
    /// which no item in scope shadows, or `std::num`'s `NonZero` types. An
    /// alias, `type Id = i64;`, or any other path is not proof, and is not
    /// followed.
    fn numeric_type(&self, written: &ast::Type, site: &SyntaxNode) -> bool {
        match written {
            ast::Type::RefType(reference) => reference
                .ty()
                .is_some_and(|inner| self.numeric_type(&inner, site)),
            ast::Type::ParenType(paren) => paren.ty().is_some_and(|inner| self.numeric_type(&inner, site)),
            ast::Type::PathType(path) => {
                let Some(path) = path.path() else {
                    return false;
                };
                if let Some(segments) = plain_segments(&path)
                    && let [name] = segments.as_slice()
                    && NUMERIC_PRIMITIVES.contains(&name.as_str())
                {
                    return self.aliases.provenance(site, name) == Provenance::Unbound;
                }
                self.item_path(&path).is_some_and(|resolved| {
                    matches!(
                        resolved.as_slice(),
                        [krate, module, kind]
                            if matches!(krate.as_str(), "core" | "std")
                                && module == "num"
                                && kind.starts_with("NonZero")
                    )
                })
            }
            _ => false,
        }
    }
}

/// The primitives whose `Display` cannot spell anything but a value of the
/// type: a number, a boolean or a single character.
const NUMERIC_PRIMITIVES: [&str; 16] = [
    "bool", "char", "f32", "f64", "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64",
    "u128", "usize",
];

/// The constructors of a reqwest client builder, each with the associated
/// function that returns one: `Client::builder()` and `ClientBuilder::new()`
/// hand back the same builder.
static TLS_BUILDERS: [(Item, &str); 4] = [
    (
        Item {
            krate: "reqwest",
            segments: &["Client"],
        },
        "builder",
    ),
    (
        Item {
            krate: "reqwest",
            segments: &["blocking", "Client"],
        },
        "builder",
    ),
    (
        Item {
            krate: "reqwest",
            segments: &["ClientBuilder"],
        },
        "new",
    ),
    (
        Item {
            krate: "reqwest",
            segments: &["blocking", "ClientBuilder"],
        },
        "new",
    ),
];

static DISABLED_TLS: Detector = Detector {
    definition: &SOURCE_DISABLED_TLS,
    node: SyntaxKind::METHOD_CALL_EXPR,
    inspect: disabled_tls,
};

/// The canonical cases of one native rule, which the variant gate in
/// `tests/variants.rs` rewrites into the forms real code takes and holds to
/// the same verdict, and the semantic cases it runs as written.
///
/// A case is written as parts rather than as a file, so each rewrite has
/// something to act on: `import` is the item the call reaches, written into
/// the code as `{Item}`; `receiver` is the expression the checked call chain
/// starts from, and `chain` the rest of it; `setup` holds the statements
/// before it and `parameters` the function's. A binding written `$name` is
/// what the renaming rewrite renames.
#[cfg(test)]
pub(super) struct Variants {
    pub(super) rule: &'static str,
    pub(super) positive: Case,
    pub(super) negative: Case,
    /// Does the positive fire because a parameter's value reaches the sink as
    /// text? Then the `numeric-parameter` rewrite, which types every parameter
    /// `i64`, has to leave it silent: a number cannot carry an injection.
    pub(super) interpolates_parameters: bool,
    /// Rewrites under which the rule is known to miss the verdict the rewrite
    /// expects, each with the one-line reason naming the story that closes
    /// it. The gate refuses an entry that no longer misses.
    pub(super) known_misses: &'static [(&'static str, &'static str)],
    /// The real forms of the defect and of its look-alikes, each a whole unit.
    pub(super) semantic: &'static [Semantic],
}

/// One form of the defect, or of something that only looks like it, written
/// as the whole unit a user would write. `fires: false` carries the reason the
/// rule stays silent in `carve_out`: either why the form is not the defect, or
/// the story that will make the rule see it. A carved case that fires is a
/// stale carve-out, and the gate refuses it.
#[cfg(test)]
pub(super) struct Semantic {
    pub(super) description: &'static str,
    pub(super) source: &'static str,
    pub(super) fires: bool,
    pub(super) carve_out: &'static str,
}

#[cfg(test)]
pub(super) struct Case {
    pub(super) asynchronous: bool,
    pub(super) import: &'static str,
    pub(super) parameters: &'static str,
    pub(super) setup: &'static str,
    pub(super) receiver: &'static str,
    pub(super) chain: &'static str,
}

#[cfg(test)]
pub(super) static VARIANTS: [&Variants; 4] = [
    &blocking::VARIANTS,
    &TLS_VARIANTS,
    &shell::VARIANTS,
    &sql::VARIANTS,
];

#[cfg(test)]
static TLS_VARIANTS: Variants = Variants {
    rule: SOURCE_DISABLED_TLS.id,
    positive: Case {
        asynchronous: false,
        import: "reqwest::Client",
        parameters: "",
        setup: "let $insecure = true;",
        receiver: "{Item}::builder()",
        chain: ".danger_accept_invalid_certs($insecure)",
    },
    negative: Case {
        asynchronous: false,
        import: "reqwest::Client",
        parameters: "",
        setup: "let $insecure = false;",
        receiver: "{Item}::builder()",
        chain: ".danger_accept_invalid_certs($insecure)",
    },
    interpolates_parameters: false,
    known_misses: &[],
    semantic: &[
        Semantic {
            description: "the builder taken from ClientBuilder::new()",
            source: "fn client() { let _ = reqwest::ClientBuilder::new().danger_accept_invalid_certs(true).build(); }",
            fires: true,
            carve_out: "",
        },
        Semantic {
            description: "the blocking client's builder disabling hostname checks",
            source: "use reqwest::blocking::Client;\nfn client() { let _ = Client::builder().danger_accept_invalid_hostnames(true).build(); }",
            fires: true,
            carve_out: "",
        },
        Semantic {
            description: "verification disabled in a test module",
            source: "#[cfg(test)]\nmod tests { fn client() { let _ = reqwest::Client::builder().danger_accept_invalid_certs(true); } }",
            fires: false,
            carve_out: "verification is disabled on purpose against a self-signed test server",
        },
    ],
};

pub(super) static DETECTORS: [&Detector; 7] = [
    &blocking::BLOCKING_CALL,
    &blocking::BLOCKING_METHOD,
    &DISABLED_TLS,
    &shell::DYNAMIC_SHELL,
    &shell::TAINTED_PROGRAM,
    &sql::SQL_CALL,
    &sql::SQL_METHOD,
];

/// Reqwest's builder disabling certificate or hostname verification.
///
/// The rule stays out of test code: verification is routinely turned off on
/// purpose against a local server with a self-signed certificate, and that is
/// the dominant reading of the pattern there. `shell::dynamic_shell` carries no
/// such exemption on purpose, because an interpolated shell command is a
/// finding wherever it is written, and a test harness is a place it runs.
fn disabled_tls(context: &Context<'_>, node: &SyntaxNode) -> Option<Detection> {
    let call = ast::MethodCallExpr::cast(node.clone())?;
    let message = match call.name_ref()?.text().as_str() {
        "tls_danger_accept_invalid_certs" | "danger_accept_invalid_certs" => {
            "Reqwest client builder disables TLS certificate verification."
        }
        "tls_danger_accept_invalid_hostnames" | "danger_accept_invalid_hostnames" => {
            "Reqwest client builder disables TLS hostname verification."
        }
        _ => return None,
    };
    let argument = only_argument(call.arg_list()?)?;
    if !literal_bool(&context.value(argument.clone())?, true) {
        return None;
    }

    let ast::Expr::CallExpr(builder) = context.chain_root(call.receiver()?)? else {
        return None;
    };
    if builder.arg_list()?.args().next().is_some() {
        return None;
    }
    let callee = builder.expr()?;
    if !TLS_BUILDERS
        .iter()
        .any(|(builder, constructor)| context.calls(&callee, builder, constructor))
    {
        return None;
    }
    if intersects_errors(call.syntax().text_range(), context.error_ranges)
        || in_test_code(context, call.syntax())
    {
        return None;
    }
    Some(Detection {
        message: message.into(),
        range: argument.syntax().text_range(),
    })
}

pub(super) fn only_argument(arguments: ast::ArgList) -> Option<ast::Expr> {
    let mut arguments = arguments.args();
    let argument = arguments.next()?;
    if arguments.next().is_some() {
        None
    } else {
        Some(argument)
    }
}

fn literal_bool(expression: &ast::Expr, expected: bool) -> bool {
    matches!(
        expression,
        ast::Expr::Literal(literal) if literal.kind() == LiteralKind::Bool(expected)
    )
}

/// The names of a path's segments, generic arguments set aside: `query_as::<_,
/// User>` names the item `query_as`. A keyword segment or a type anchor names
/// nothing on its own and abstains.
fn item_segments(path: &ast::Path) -> Option<Vec<String>> {
    path.segments()
        .map(|segment| match segment.kind() {
            Some(ast::PathSegmentKind::Name(name))
                if segment.syntax().children().all(|child| {
                    ast::NameRef::can_cast(child.kind()) || ast::GenericArgList::can_cast(child.kind())
                }) =>
            {
                Some(name.text().to_string())
            }
            _ => None,
        })
        .collect()
}

fn plain_segments(path: &ast::Path) -> Option<Vec<String>> {
    path.segments()
        .map(|segment| match segment.kind() {
            Some(ast::PathSegmentKind::Name(name)) if segment.syntax().children().count() == 1 => {
                Some(name.text().to_string())
            }
            _ => None,
        })
        .collect()
}

/// Is this node test material? The unit answers for the file, Cargo's target
/// kind first and the path convention second; the attributes answer for the
/// item, which is what covers an inline `#[cfg(test)]` module of a shipped
/// file. One question, so a detector that wants to stay quiet in tests asks it
/// once instead of composing two half-answers.
fn in_test_code(context: &Context<'_>, node: &SyntaxNode) -> bool {
    context.test_code || under_test_attribute(node)
}

pub(super) fn under_test_attribute(node: &SyntaxNode) -> bool {
    node.ancestors().any(|ancestor| {
        let has_cfg_test = ancestor
            .children()
            .filter_map(ast::Attr::cast)
            .any(|attribute| compact(attribute.syntax()) == "#[cfg(test)]");
        let is_test_function = ast::Fn::cast(ancestor.clone()).is_some_and(|function| {
            function
                .attrs()
                .any(|attribute| compact(attribute.syntax()) == "#[test]")
        });
        has_cfg_test || is_test_function
    })
}

pub(super) fn strip_wrappers(mut expression: ast::Expr) -> ast::Expr {
    loop {
        expression = match expression {
            ast::Expr::ParenExpr(paren) => match paren.expr() {
                Some(inner) => inner,
                None => return ast::Expr::ParenExpr(paren),
            },
            ast::Expr::RefExpr(reference) => match reference.expr() {
                Some(inner) => inner,
                None => return ast::Expr::RefExpr(reference),
            },
            expression => return expression,
        };
    }
}

fn strip_parentheses(mut expression: ast::Expr) -> ast::Expr {
    while let ast::Expr::ParenExpr(paren) = &expression {
        match paren.expr() {
            Some(inner) => expression = inner,
            None => break,
        }
    }
    expression
}

/// `SCREAMING_SNAKE_CASE`, the spelling Rust gives a `const` or a `static`.
fn screaming(name: &str) -> bool {
    name.chars().any(|character| character.is_ascii_uppercase())
        && name
            .chars()
            .all(|character| character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_')
}

/// Is this text assembled from a runtime value? A `format!` interpolating a
/// value that is `injectable`, or a `+` concatenation with such an operand,
/// read through the bindings the expression was stored in.
pub(super) fn dynamic_text(context: &Context<'_>, expression: &ast::Expr) -> bool {
    let Some(expression) = context.value(strip_wrappers(expression.clone())) else {
        return false;
    };
    match strip_wrappers(expression) {
        ast::Expr::BinExpr(binary) => {
            let site = binary.syntax().clone();
            binary.op_kind() == Some(ast::BinaryOp::ArithOp(ast::ArithOp::Add))
                && context.injectable(&ast::Expr::BinExpr(binary), &site)
        }
        ast::Expr::MacroExpr(macro_expression) => macro_expression
            .macro_call()
            .is_some_and(|call| format_macro_dynamic(context, &call)),
        _ => false,
    }
}

fn format_macro_dynamic(context: &Context<'_>, call: &ast::MacroCall) -> bool {
    let Some(path) = call.path() else {
        return false;
    };
    if compact(path.syntax()) != "format" {
        return false;
    }
    let Some(arguments) = format_arguments(call, context.edition) else {
        return false;
    };
    formats_dynamic(context, call, arguments.args())
}

/// The binding a `write!` or `writeln!` call writes dynamic text into:
/// `write!(sql, "... {user}")` names `sql`.
pub(super) fn written_dynamic(context: &Context<'_>, call: &ast::MacroCall) -> Option<String> {
    let path = compact(call.path()?.syntax());
    if path != "write" && path != "writeln" {
        return None;
    }
    let mut arguments = format_arguments(call, context.edition)?.args();
    let destination = bindings::identifier(&strip_wrappers(arguments.next()?))?;
    formats_dynamic(context, call, arguments).then_some(destination)
}

/// Do these format arguments, the format string first, interpolate a value
/// that is `injectable`?
fn formats_dynamic(
    context: &Context<'_>,
    call: &ast::MacroCall,
    mut arguments: impl Iterator<Item = ast::Expr>,
) -> bool {
    let Some(format_text) = arguments.next().and_then(literal_string) else {
        return false;
    };
    let fields = format_fields(&format_text);
    if fields.is_empty() {
        return false;
    }

    // The arguments were reparsed out of the macro's token tree, so they sit
    // in a tree of their own: a binding they name is resolved where the macro
    // sits, and only the shape of the argument is read from the reparse.
    let site = call.syntax();
    let constant = |argument: &ast::Expr| match bindings::identifier(&strip_wrappers(argument.clone())) {
        Some(name) => !context.injectable_capture(site, &name),
        None => !context.injectable(argument, site),
    };
    let mut positional = Vec::new();
    let mut named = BTreeMap::new();
    for argument in arguments {
        match named_argument(&argument) {
            Some((name, value)) => {
                named.insert(name, constant(&value));
            }
            None => positional.push(constant(&argument)),
        }
    }
    let mut next_position = 0;
    fields.into_iter().any(|field| {
        let field = field.split(':').next().unwrap_or_default();
        if field.is_empty() {
            let constant = positional.get(next_position).copied().unwrap_or(true);
            next_position += 1;
            !constant
        } else if let Ok(index) = field.parse::<usize>() {
            !positional.get(index).copied().unwrap_or(true)
        } else {
            !named
                .get(field)
                .copied()
                .unwrap_or_else(|| !context.injectable_capture(site, field))
        }
    })
}

/// The argument list of a `format!` invocation, parsed.
///
/// A macro's arguments are a flat token tree, not an argument list, so the
/// whole tree is handed to the parser at once: prefixing an identifier turns
/// `("echo {user}", value, width = 4)` into a call expression, and the parser
/// then splits the commas, keeps the whitespace and hands back typed
/// expressions. Reading those tokens as text instead is what used to
/// concatenate `value as usize` into the single identifier `valueasusize` and
/// cut `probe::<A, B>()` in half at the comma of its generic arguments.
fn format_arguments(call: &ast::MacroCall, edition: Edition) -> Option<ast::ArgList> {
    let inner = delimited_text(&call.token_tree()?)?;
    let ast::Expr::CallExpr(call) = parsed_expression(&format!("f({inner})"), edition)? else {
        return None;
    };
    call.arg_list()
}

/// The text between a token tree's delimiters, whichever pair was written:
/// `format!` accepts `()`, `[]` and `{}` and means the same by each.
fn delimited_text(tree: &ast::TokenTree) -> Option<String> {
    let text = tree.syntax().text().to_string();
    let mut characters = text.chars();
    let opening = characters.next()?;
    let closing = characters.next_back()?;
    matches!((opening, closing), ('(', ')') | ('[', ']') | ('{', '}'))
        .then(|| characters.as_str().to_owned())
}

/// `name = value`, which is how a named `format!` argument parses once the
/// argument list is a real one: an assignment expression whose left side is a
/// bare identifier. Recognizing the shape is what replaced splitting the text
/// on the first `=` and then guarding against `==` by hand.
fn named_argument(expression: &ast::Expr) -> Option<(String, ast::Expr)> {
    let ast::Expr::BinExpr(binary) = expression else {
        return None;
    };
    if binary.op_kind() != Some(ast::BinaryOp::Assignment { op: None }) {
        return None;
    }
    let ast::Expr::PathExpr(path) = binary.lhs()? else {
        return None;
    };
    let segments = plain_segments(&path.path()?)?;
    let [name] = segments.as_slice() else {
        return None;
    };
    Some((name.clone(), binary.rhs()?))
}

fn parsed_expression(expression: &str, edition: Edition) -> Option<ast::Expr> {
    let parse = ast::Expr::parse(expression, edition);
    if parse.errors().is_empty() {
        ast::Expr::cast(parse.syntax_node())
    } else {
        None
    }
}

fn format_fields(format: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut characters = format.char_indices().peekable();
    while let Some((_, character)) = characters.next() {
        if character != '{' {
            continue;
        }
        if characters.peek().is_some_and(|(_, next)| *next == '{') {
            characters.next();
            continue;
        }
        let mut field = String::new();
        for (_, character) in characters.by_ref() {
            if character == '}' {
                fields.push(field);
                break;
            }
            field.push(character);
        }
    }
    fields
}

/// Dependency aliases of a package: identifier usable in the code to crate
/// name. An identifier carried by two different dependencies is dropped rather
/// than arbitrated.
pub(super) fn crate_aliases(package: &cargo_metadata::Package) -> CrateAliases {
    let mut declared: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for dependency in &package.dependencies {
        let krate = dependency.name.replace('-', "_");
        let alias = dependency
            .rename
            .as_deref()
            .map_or_else(|| krate.clone(), |rename| rename.replace('-', "_"));
        declared.entry(alias).or_default().push(krate);
    }
    declared
        .into_iter()
        .filter_map(|(alias, krates)| unanimous(krates).map(|krate| (alias, krate)))
        .collect()
}

/// Aliases shared by every package that reaches a unit. An identifier whose
/// crate two packages disagree on is dropped.
pub(super) fn shared_crate_aliases<'a>(
    packages: impl Iterator<Item = Option<&'a CrateAliases>>,
) -> CrateAliases {
    let mut shared: Option<CrateAliases> = None;
    for package in packages {
        let Some(package) = package else {
            return CrateAliases::new();
        };
        shared = Some(match shared {
            None => package.clone(),
            Some(shared) => shared
                .into_iter()
                .filter(|(alias, krate)| package.get(alias) == Some(krate))
                .collect(),
        });
    }
    shared.unwrap_or_default()
}

/// The type written on the `let` that `value` initializes: `i64` for the
/// `q.limit` of `let limit: i64 = q.limit;`.
fn let_type(value: &ast::Expr) -> Option<ast::Type> {
    value.syntax().parent().and_then(ast::LetStmt::cast)?.ty()
}

/// Is this qualifier `Self` and nothing else, so that `Self::f` names a
/// method of the enclosing `impl`?
fn names_self(qualifier: &ast::Path) -> bool {
    qualifier.qualifier().is_none()
        && qualifier
            .segment()
            .is_some_and(|segment| segment.kind() == Some(ast::PathSegmentKind::SelfTypeKw))
}

/// The node whose children are the items `scope` declares, when `scope` is a
/// block, a file or an inline module.
fn item_scope(scope: &SyntaxNode) -> Option<SyntaxNode> {
    if let Some(block) = ast::BlockExpr::cast(scope.clone()) {
        block.stmt_list().map(|list| list.syntax().clone())
    } else if ast::SourceFile::can_cast(scope.kind()) {
        Some(scope.clone())
    } else {
        ast::Module::cast(scope.clone())?.item_list().map(|list| list.syntax().clone())
    }
}
