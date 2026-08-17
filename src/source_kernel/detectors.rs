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
use super::{literal_string, unanimous};
use crate::policy::{RuleDefinition, SOURCE_DISABLED_TLS, SOURCE_DYNAMIC_SHELL};
use crate::source_text::{compact, intersects_errors};

/// Dependency aliases of a unit: identifier written in the code to canonical
/// crate name.
pub(super) type CrateAliases = BTreeMap<String, String>;

/// Crates always available without a dependency declaration.
const SYSROOT: [&str; 3] = ["alloc", "core", "std"];

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
        let Some(path) = expression.path() else {
            return false;
        };
        let Some(written) = plain_segments(&path) else {
            return false;
        };
        let Some((last, qualifier)) = written.split_last() else {
            return false;
        };
        if last != member {
            return false;
        }
        let Some((first, tail)) = qualifier.split_first() else {
            return false;
        };
        let resolved: Vec<&str> = match self.aliases.provenance(expression.syntax(), first) {
            Provenance::Item(path) => path
                .iter()
                .map(String::as_str)
                .chain(tail.iter().map(String::as_str))
                .collect(),
            Provenance::Unbound => qualifier.iter().map(String::as_str).collect(),
            Provenance::Indeterminate => return false,
        };
        let Some((root, segments)) = resolved.split_first() else {
            return false;
        };
        self.resolves_crate(root, item.krate) && segments == item.segments
    }

    fn resolves_crate(&self, root: &str, krate: &str) -> bool {
        if SYSROOT.contains(&krate) {
            root == krate
        } else {
            self.crates
                .get(root)
                .is_some_and(|resolved| resolved == krate)
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

pub(super) static DETECTORS: [&Detector; 2] = [&DISABLED_TLS, &DYNAMIC_SHELL];

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
    if !literal_bool(&argument, true) {
        return None;
    }

    let mut receiver = call.receiver()?;
    while let ast::Expr::MethodCallExpr(method_call) = receiver {
        receiver = method_call.receiver()?;
    }
    let ast::Expr::CallExpr(builder) = receiver else {
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
    if !dynamic_payload(&payload, context.edition) {
        return None;
    }
    let ast::Expr::MethodCallExpr(shell_arg) = call.receiver()? else {
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
    let ast::Expr::CallExpr(command) = shell_arg.receiver()? else {
        return None;
    };
    if !context.calls(&command.expr()?, &COMMAND, "new") {
        return None;
    }
    let shell = only_argument(command.arg_list()?).and_then(literal_string)?;
    if !SHELLS.contains(&shell.as_str()) {
        return None;
    }
    if intersects_errors(call.syntax().text_range(), context.error_ranges) {
        return None;
    }
    Some(Detection {
        message: "A dynamic value is interpolated into a shell command string.",
        range: payload.syntax().text_range(),
    })
}

fn only_argument(arguments: ast::ArgList) -> Option<ast::Expr> {
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

fn under_test_attribute(node: &SyntaxNode) -> bool {
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

fn strip_wrappers(mut expression: ast::Expr) -> ast::Expr {
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

fn dynamic_payload(expression: &ast::Expr, edition: Edition) -> bool {
    let expression = strip_wrappers(expression.clone());
    match expression {
        ast::Expr::BinExpr(binary) => {
            binary.op_kind() == Some(ast::BinaryOp::ArithOp(ast::ArithOp::Add))
                && [binary.lhs(), binary.rhs()]
                    .into_iter()
                    .flatten()
                    .any(|operand| concat_operand_dynamic(&operand))
        }
        ast::Expr::MacroExpr(macro_expression) => macro_expression
            .macro_call()
            .is_some_and(|call| format_macro_dynamic(&call, edition)),
        _ => false,
    }
}

fn concat_operand_dynamic(expression: &ast::Expr) -> bool {
    let expression = strip_wrappers(expression.clone());
    match expression {
        ast::Expr::BinExpr(binary)
            if binary.op_kind() == Some(ast::BinaryOp::ArithOp(ast::ArithOp::Add)) =>
        {
            [binary.lhs(), binary.rhs()]
                .into_iter()
                .flatten()
                .any(|operand| concat_operand_dynamic(&operand))
        }
        ast::Expr::Literal(_) => false,
        _ => true,
    }
}

fn format_macro_dynamic(call: &ast::MacroCall, edition: Edition) -> bool {
    let Some(path) = call.path() else {
        return false;
    };
    if compact(path.syntax()) != "format" {
        return false;
    }
    let Some(arguments) = format_arguments(call, edition) else {
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

    let mut positional = Vec::new();
    let mut named = BTreeMap::new();
    for argument in arguments {
        match named_argument(&argument) {
            Some((name, value)) => {
                named.insert(name, is_literal(&value));
            }
            None => positional.push(is_literal(&argument)),
        }
    }
    let mut next_position = 0;
    fields.into_iter().any(|field| {
        let field = field.split(':').next().unwrap_or_default();
        if field.is_empty() {
            let literal = positional.get(next_position).copied().unwrap_or(true);
            next_position += 1;
            !literal
        } else if let Ok(index) = field.parse::<usize>() {
            !positional.get(index).copied().unwrap_or(true)
        } else {
            !named.get(field).copied().unwrap_or(false)
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

fn is_literal(expression: &ast::Expr) -> bool {
    matches!(strip_wrappers(expression.clone()), ast::Expr::Literal(_))
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
