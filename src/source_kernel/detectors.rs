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

use std::collections::BTreeMap;

use ra_ap_syntax::ast::{self, HasArgList, HasAttrs, LiteralKind};
use ra_ap_syntax::{AstNode, Edition, SyntaxKind, SyntaxNode, TextRange};

use super::aliases::{AliasMap, Provenance};
use super::bindings::{self, Binding, Resolver};
use super::{literal_string, unanimous};
use crate::policy::{RuleDefinition, SOURCE_DISABLED_TLS, SOURCE_DYNAMIC_SHELL};
use crate::source_text::{compact, intersects_errors};

mod blocking;
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
const CHAIN_DEPTH: usize = 64;

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
    pub(super) message: &'static str,
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
    pub(super) fn item_path(&self, path: &ast::Path) -> Option<Vec<String>> {
        let written = item_segments(path)?;
        let (first, tail) = written.split_first()?;
        let resolved: Vec<String> = match self.aliases.provenance(path.syntax(), first) {
            Provenance::Item(item) => item.iter().chain(tail).cloned().collect(),
            Provenance::Unbound => written.clone(),
            Provenance::Indeterminate => return None,
        };
        let (root, rest) = resolved.split_first()?;
        let krate = if SYSROOT.contains(&root.as_str()) {
            root.clone()
        } else {
            self.crates.get(root)?.clone()
        };
        Some(std::iter::once(krate).chain(rest.iter().cloned()).collect())
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
                Binding::Indeterminate => return None,
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

    /// `constant` for a name a format string captures, `{user}`, read where
    /// the macro sits.
    fn constant_capture(&self, site: &SyntaxNode, name: &str) -> bool {
        match self.bindings.binding(self.aliases, site, name) {
            Binding::Value(value) => self.constant(&value),
            Binding::Unbound => screaming(name),
            Binding::Indeterminate => false,
        }
    }

}

static TLS_CLIENTS: [Item; 2] = [
    Item {
        krate: "reqwest",
        segments: &["Client"],
    },
    Item {
        krate: "reqwest",
        segments: &["blocking", "Client"],
    },
];

static COMMAND: Item = Item {
    krate: "std",
    segments: &["process", "Command"],
};

const SHELLS: [&str; 4] = ["bash", "dash", "sh", "zsh"];

static DISABLED_TLS: Detector = Detector {
    definition: &SOURCE_DISABLED_TLS,
    node: SyntaxKind::METHOD_CALL_EXPR,
    inspect: disabled_tls,
};

static DYNAMIC_SHELL: Detector = Detector {
    definition: &SOURCE_DYNAMIC_SHELL,
    node: SyntaxKind::METHOD_CALL_EXPR,
    inspect: dynamic_shell,
};

/// The canonical cases of one native rule, which the variant gate in
/// `tests/variants.rs` rewrites into the forms real code takes and holds to
/// the same verdict.
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
    /// Rewrites under which the positive is known to go silent, each with the
    /// one-line reason. The gate refuses an entry that no longer misses.
    pub(super) known_misses: &'static [(&'static str, &'static str)],
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
    &SHELL_VARIANTS,
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
    known_misses: &[],
};

#[cfg(test)]
static SHELL_VARIANTS: Variants = Variants {
    rule: SOURCE_DYNAMIC_SHELL.id,
    positive: Case {
        asynchronous: false,
        import: "std::process::Command",
        parameters: "$user: &str",
        setup: "",
        receiver: "{Item}::new(\"sh\")",
        chain: ".arg(\"-c\").arg(format!(\"echo {$user}\"))",
    },
    negative: Case {
        asynchronous: false,
        import: "std::process::Command",
        parameters: "$user: &str",
        setup: "",
        receiver: "{Item}::new(\"sh\")",
        chain: ".arg(\"-c\").arg(\"echo literal\").arg($user)",
    },
    known_misses: &[],
};

pub(super) static DETECTORS: [&Detector; 6] = [
    &blocking::BLOCKING_CALL,
    &blocking::BLOCKING_METHOD,
    &DISABLED_TLS,
    &DYNAMIC_SHELL,
    &sql::SQL_CALL,
    &sql::SQL_METHOD,
];

/// Reqwest's builder disabling certificate or hostname verification.
///
/// The rule stays out of test code: verification is routinely turned off on
/// purpose against a local server with a self-signed certificate, and that is
/// the dominant reading of the pattern there. `dynamic_shell` below carries no
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
    if !TLS_CLIENTS
        .iter()
        .any(|client| context.calls(&callee, client, "builder"))
    {
        return None;
    }
    if intersects_errors(call.syntax().text_range(), context.error_ranges)
        || in_test_code(context, call.syntax())
    {
        return None;
    }
    Some(Detection {
        message,
        range: argument.syntax().text_range(),
    })
}

fn dynamic_shell(context: &Context<'_>, node: &SyntaxNode) -> Option<Detection> {
    let call = ast::MethodCallExpr::cast(node.clone())?;
    if call.name_ref()?.text() != "arg" {
        return None;
    }
    let payload = only_argument(call.arg_list()?)?;
    if !dynamic_text(context, &payload)
        || shell_invocation(context, call.receiver()?).is_none()
        || intersects_errors(call.syntax().text_range(), context.error_ranges)
    {
        return None;
    }
    Some(Detection {
        message: "A dynamic value is interpolated into a shell command string.",
        range: payload.syntax().text_range(),
    })
}

/// Is `receiver`, read through the bindings it was stored in, a
/// `Command::new(<shell>).arg("-c")`? `None` when it is not, or cannot be told.
fn shell_invocation(context: &Context<'_>, receiver: ast::Expr) -> Option<()> {
    let ast::Expr::MethodCallExpr(shell_arg) = context.value(strip_wrappers(receiver))? else {
        return None;
    };
    if shell_arg.name_ref()?.text() != "arg"
        || only_argument(shell_arg.arg_list()?)
            .and_then(literal_string)
            .as_deref()
            != Some("-c")
    {
        return None;
    }
    let ast::Expr::CallExpr(command) = context.value(strip_wrappers(shell_arg.receiver()?))? else {
        return None;
    };
    if !context.calls(&command.expr()?, &COMMAND, "new") {
        return None;
    }
    let shell = only_argument(command.arg_list()?).and_then(literal_string)?;
    SHELLS.contains(&shell.as_str()).then_some(())
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
/// value that is not constant, or a `+` concatenation with such an operand,
/// read through the bindings the expression was stored in.
pub(super) fn dynamic_text(context: &Context<'_>, expression: &ast::Expr) -> bool {
    let Some(expression) = context.value(strip_wrappers(expression.clone())) else {
        return false;
    };
    match strip_wrappers(expression) {
        ast::Expr::BinExpr(binary) => {
            binary.op_kind() == Some(ast::BinaryOp::ArithOp(ast::ArithOp::Add))
                && !context.constant(&ast::Expr::BinExpr(binary))
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
    let mut arguments = arguments.args();
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
        Some(name) => context.constant_capture(site, &name),
        None => context.constant(argument),
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
                .unwrap_or_else(|| context.constant_capture(site, field))
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
