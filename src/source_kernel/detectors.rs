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
mod http;
mod runtime;
mod secrets;
mod shell;
mod sql;
mod web;

pub(super) type CrateAliases = BTreeMap<String, String>;

const SYSROOT: [&str; 3] = ["alloc", "core", "std"];

const BINDING_DEPTH: usize = 8;

pub(super) const CHAIN_DEPTH: usize = 64;

struct Item {
    krate: &'static str,
    segments: &'static [&'static str],
}

pub(super) struct Detector {
    pub(super) definition: &'static RuleDefinition,
    pub(super) node: SyntaxKind,
    pub(super) inspect: fn(&Context<'_>, &SyntaxNode) -> Option<Detection>,
    #[allow(dead_code, reason = "read by the test that holds every matched crate to a gate")]
    pub(super) crates: &'static [&'static str],
}

pub(super) struct Detection {
    pub(super) message: Cow<'static, str>,
    pub(super) range: TextRange,
}

pub(super) struct Context<'a> {
    pub(super) aliases: &'a AliasMap,
    pub(super) bindings: &'a Resolver,
    pub(super) crates: &'a CrateAliases,
    pub(super) error_ranges: &'a [TextRange],
    pub(super) edition: Edition,
    pub(super) test_code: bool,
    pub(super) resolved: RefCell<Option<(TextRange, Option<Vec<String>>)>>,
}

impl Context<'_> {
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

    fn external_crate(&self, name: &str) -> bool {
        SYSROOT.contains(&name) || self.crates.contains_key(name)
    }

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

    pub(super) fn chain_root(&self, expression: ast::Expr) -> Option<ast::Expr> {
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

    pub(super) fn injectable(&self, expression: &ast::Expr, site: &SyntaxNode) -> bool {
        !self.constant(expression) && !self.numeric(expression, site)
    }

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

const NUMERIC_PRIMITIVES: [&str; 16] = [
    "bool", "char", "f32", "f64", "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64",
    "u128", "usize",
];

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
    crates: &["reqwest"],
};

#[cfg(test)]
pub(super) struct Variants {
    pub(super) rule: &'static str,
    pub(super) positive: Case,
    pub(super) negative: Case,
    pub(super) interpolates_parameters: bool,
    pub(super) known_misses: &'static [(&'static str, &'static str)],
    pub(super) semantic: &'static [Semantic],
}

#[cfg(test)]
pub(super) struct Semantic {
    pub(super) description: &'static str,
    pub(super) source: &'static str,
    pub(super) fires: bool,
    pub(super) carve_out: &'static str,
}

#[cfg(test)]
pub(super) struct Case {
    pub(super) attributes: &'static str,
    pub(super) asynchronous: bool,
    pub(super) import: &'static str,
    pub(super) parameters: &'static str,
    pub(super) setup: &'static str,
    pub(super) receiver: &'static str,
    pub(super) chain: &'static str,
}

#[cfg(test)]
pub(super) static VARIANTS: [&Variants; 11] = [
    &blocking::VARIANTS,
    &TLS_VARIANTS,
    &shell::VARIANTS,
    &sql::VARIANTS,
    &runtime::BLOCK_IN_PLACE_VARIANTS,
    &runtime::BLOCK_ON_VARIANTS,
    &http::VARIANTS,
    &secrets::VARIANTS,
    &web::CORS_VARIANTS,
    &web::COOKIE_VARIANTS,
    &web::JWT_VARIANTS,
];

#[cfg(test)]
static TLS_VARIANTS: Variants = Variants {
    rule: SOURCE_DISABLED_TLS.id,
    positive: Case {
        attributes: "",
        asynchronous: false,
        import: "reqwest::Client",
        parameters: "",
        setup: "let $insecure = true;",
        receiver: "{Item}::builder()",
        chain: ".danger_accept_invalid_certs($insecure)",
    },
    negative: Case {
        attributes: "",
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

pub(super) static DETECTORS: [&Detector; 17] = [
    &blocking::BLOCKING_CALL,
    &blocking::BLOCKING_METHOD,
    &DISABLED_TLS,
    &shell::DYNAMIC_SHELL,
    &shell::TAINTED_PROGRAM,
    &sql::SQL_CALL,
    &sql::SQL_METHOD,
    &runtime::BLOCK_IN_PLACE,
    &runtime::BLOCK_ON_CALL,
    &runtime::BLOCK_ON_METHOD,
    &http::UNCHECKED_STATUS,
    &secrets::SECRET_FALLBACK,
    &web::CORS_CALL,
    &web::CORS_CREDENTIALS,
    &web::AUTH_COOKIE,
    &web::JWT_METHOD,
    &web::JWT_CALL,
];

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

pub(super) fn literal_bool(expression: &ast::Expr, expected: bool) -> bool {
    matches!(
        expression,
        ast::Expr::Literal(literal) if literal.kind() == LiteralKind::Bool(expected)
    )
}

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

pub(super) fn in_test_code(context: &Context<'_>, node: &SyntaxNode) -> bool {
    context.test_code || under_test_attribute(node)
}

pub(super) fn under_test_attribute(node: &SyntaxNode) -> bool {
    node.ancestors().any(|ancestor| carries_test_attribute(&ancestor))
}

pub(super) fn carries_test_attribute(node: &SyntaxNode) -> bool {
    let has_cfg_test = node
        .children()
        .filter_map(ast::Attr::cast)
        .any(|attribute| compact(attribute.syntax()) == "#[cfg(test)]");
    let is_test_function = ast::Fn::cast(node.clone()).is_some_and(|function| {
        function
            .attrs()
            .any(|attribute| compact(attribute.syntax()) == "#[test]")
    });
    has_cfg_test || is_test_function
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

fn screaming(name: &str) -> bool {
    name.chars().any(|character| character.is_ascii_uppercase())
        && name
            .chars()
            .all(|character| character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_')
}

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

pub(super) fn written_dynamic(context: &Context<'_>, call: &ast::MacroCall) -> Option<String> {
    let path = compact(call.path()?.syntax());
    if path != "write" && path != "writeln" {
        return None;
    }
    let mut arguments = format_arguments(call, context.edition)?.args();
    let destination = bindings::identifier(&strip_wrappers(arguments.next()?))?;
    formats_dynamic(context, call, arguments).then_some(destination)
}

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

fn format_arguments(call: &ast::MacroCall, edition: Edition) -> Option<ast::ArgList> {
    let inner = delimited_text(&call.token_tree()?)?;
    let ast::Expr::CallExpr(call) = parsed_expression(&format!("f({inner})"), edition)? else {
        return None;
    };
    call.arg_list()
}

fn delimited_text(tree: &ast::TokenTree) -> Option<String> {
    let text = tree.syntax().text().to_string();
    let mut characters = text.chars();
    let opening = characters.next()?;
    let closing = characters.next_back()?;
    matches!((opening, closing), ('(', ')') | ('[', ']') | ('{', '}'))
        .then(|| characters.as_str().to_owned())
}

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

fn let_type(value: &ast::Expr) -> Option<ast::Type> {
    value.syntax().parent().and_then(ast::LetStmt::cast)?.ty()
}

fn names_self(qualifier: &ast::Path) -> bool {
    qualifier.qualifier().is_none()
        && qualifier
            .segment()
            .is_some_and(|segment| segment.kind() == Some(ast::PathSegmentKind::SelfTypeKw))
}

fn item_scope(scope: &SyntaxNode) -> Option<SyntaxNode> {
    if let Some(block) = ast::BlockExpr::cast(scope.clone()) {
        block.stmt_list().map(|list| list.syntax().clone())
    } else if ast::SourceFile::can_cast(scope.kind()) {
        Some(scope.clone())
    } else {
        ast::Module::cast(scope.clone())?.item_list().map(|list| list.syntax().clone())
    }
}
