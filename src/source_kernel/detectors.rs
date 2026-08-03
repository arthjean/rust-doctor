//! Registre des détecteurs natifs.
//!
//! Un détecteur déclare la règle qu'il porte, le type de nœud qu'il inspecte et
//! la fonction qui décide. Le noyau parcourt le CST une seule fois et sollicite
//! chaque détecteur sur les nœuds qu'il a déclarés, donc ajouter un détecteur
//! n'élargit aucune signature d'analyse.
//!
//! Aucun détecteur ne compare un chemin écrit à une chaîne. Une cible est
//! décrite par une crate et des segments, et la provenance de l'identifiant
//! d'appel est résolue par la carte d'alias puis par les alias de dépendances
//! du manifeste.

use std::collections::BTreeMap;

use ra_ap_syntax::ast::{self, HasArgList, HasAttrs, LiteralKind};
use ra_ap_syntax::{AstNode, Edition, SyntaxKind, SyntaxNode, TextRange};

use super::aliases::{AliasMap, Provenance};
use super::{intersects_errors, literal_string};
use crate::policy::{RuleDefinition, SOURCE_DISABLED_TLS, SOURCE_DYNAMIC_SHELL};

/// Alias de dépendances d'une unité: identifiant écrit dans le code vers nom
/// canonique de la crate.
pub(super) type CrateAliases = BTreeMap<String, String>;

/// Crates toujours disponibles sans déclaration de dépendance.
const SYSROOT: [&str; 3] = ["alloc", "core", "std"];

/// Item ciblé par un détecteur, décrit par segments plutôt que par un chemin
/// écrit: c'est ce qui permet de reconnaître la forme importée et la forme
/// pleinement qualifiée par le même mécanisme.
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

/// Ce qu'un détecteur connaît de l'unité analysée.
pub(super) struct Context<'a> {
    pub(super) aliases: &'a AliasMap,
    pub(super) crates: &'a CrateAliases,
    pub(super) error_ranges: &'a [TextRange],
    pub(super) edition: Edition,
    pub(super) test_path: bool,
}

impl Context<'_> {
    /// Vrai quand `callee` désigne `item::member` une fois son premier
    /// identifiant résolu.
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

fn disabled_tls(context: &Context<'_>, node: &SyntaxNode) -> Option<Detection> {
    if context.test_path {
        return None;
    }
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
        || excluded_test_context(&call)
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

fn compact(node: &SyntaxNode) -> String {
    node.descendants_with_tokens()
        .filter_map(|element| element.into_token())
        .filter(|token| !token.kind().is_trivia())
        .map(|token| token.text().to_string())
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

fn excluded_test_context(call: &ast::MethodCallExpr) -> bool {
    call.syntax().ancestors().any(|ancestor| {
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
    let Some(token_tree) = call.token_tree() else {
        return false;
    };
    let arguments = token_tree_arguments(token_tree.syntax());
    let Some((format_literal, values)) = arguments.split_first() else {
        return false;
    };
    let Some(format_expression) = parsed_expression(format_literal, edition) else {
        return false;
    };
    let Some(format_text) = literal_string(format_expression) else {
        return false;
    };
    let fields = format_fields(&format_text);
    if fields.is_empty() {
        return false;
    }

    let mut positional = Vec::new();
    let mut named = BTreeMap::new();
    for value in values {
        if let Some((name, expression)) = named_format_argument(value) {
            named.insert(name, expression_is_literal(expression, edition));
        } else {
            positional.push(expression_is_literal(value, edition));
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

fn token_tree_arguments(tree: &SyntaxNode) -> Vec<String> {
    let mut arguments = vec![String::new()];
    for element in tree.children_with_tokens() {
        if element
            .as_token()
            .is_some_and(|token| matches!(token.text(), "(" | ")" | "[" | "]" | "{" | "}"))
        {
            continue;
        }
        if element.as_token().is_some_and(|token| token.text() == ",") {
            arguments.push(String::new());
            continue;
        }
        if element.kind().is_trivia() {
            continue;
        }
        if let Some(argument) = arguments.last_mut() {
            argument.push_str(&element.to_string());
        }
    }
    arguments
        .into_iter()
        .filter(|argument| !argument.is_empty())
        .collect()
}

fn named_format_argument(argument: &str) -> Option<(&str, &str)> {
    let (name, expression) = argument.split_once('=')?;
    if !name.is_empty()
        && !expression.starts_with('=')
        && name
            .chars()
            .all(|character| character == '_' || character.is_alphanumeric())
    {
        Some((name, expression))
    } else {
        None
    }
}

fn expression_is_literal(expression: &str, edition: Edition) -> bool {
    parsed_expression(expression, edition)
        .is_some_and(|expression| matches!(strip_wrappers(expression), ast::Expr::Literal(_)))
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

/// Alias de dépendances d'un paquet: identifiant utilisable dans le code vers
/// nom de crate. Un identifiant porté par deux dépendances différentes est
/// retiré plutôt qu'arbitré.
pub(super) fn crate_aliases(package: &cargo_metadata::Package) -> CrateAliases {
    let mut aliases = CrateAliases::new();
    let mut conflicting = Vec::new();
    for dependency in &package.dependencies {
        let krate = dependency.name.replace('-', "_");
        let alias = dependency
            .rename
            .as_deref()
            .map_or_else(|| krate.clone(), |rename| rename.replace('-', "_"));
        if let Some(existing) = aliases.get(&alias)
            && existing != &krate
        {
            conflicting.push(alias);
            continue;
        }
        aliases.insert(alias, krate);
    }
    for alias in conflicting {
        aliases.remove(&alias);
    }
    aliases
}

/// Alias partagés par tous les paquets qui atteignent une unité. Un identifiant
/// dont deux paquets ne s'accordent pas sur la crate est retiré.
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
