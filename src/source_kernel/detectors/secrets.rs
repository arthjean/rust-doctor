//! A secret-named environment variable that falls back to a value.
//!
//! `env::var("JWT_SECRET").unwrap_or("dev-key".into())` makes two defects at
//! one call: the fallback is a secret committed in the source, and a service
//! deployed without the variable runs on it instead of refusing to start.
//! `unwrap_or_default()` is the second defect alone, an empty secret. The
//! name and value vocabularies live in `vocabulary`, the one place a false
//! positive is tuned. The rule is silent in test code, where a fixed secret is
//! the fixture.

use ra_ap_syntax::ast::{self, HasArgList};
use ra_ap_syntax::{AstNode, SyntaxKind, SyntaxNode};

use super::{Context, Detection, Detector, in_test_code, literal_string, only_argument, strip_wrappers};
use crate::policy::SOURCE_SECRET_ENV_FALLBACK;
use crate::source_text::intersects_errors;

mod vocabulary;

pub(in crate::source_kernel) static SECRET_FALLBACK: Detector = Detector {
    definition: &SOURCE_SECRET_ENV_FALLBACK,
    node: SyntaxKind::METHOD_CALL_EXPR,
    inspect: secret_fallback,
    crates: &[],
};

#[cfg(test)]
pub(in crate::source_kernel) static VARIANTS: super::Variants = super::Variants {
    rule: SOURCE_SECRET_ENV_FALLBACK.id,
    positive: super::Case {
        attributes: "",
        asynchronous: false,
        import: "std::env::var",
        parameters: "",
        setup: "",
        receiver: "{Item}(\"JWT_SECRET\")",
        chain: ".unwrap_or(\"dev-signing-key\".to_owned())",
    },
    negative: super::Case {
        attributes: "",
        asynchronous: false,
        import: "std::env::var",
        parameters: "",
        setup: "",
        receiver: "{Item}(\"DATABASE_URL\")",
        chain: ".unwrap_or(\"postgres://localhost/dev\".to_owned())",
    },
    interpolates_parameters: false,
    known_misses: &[],
    semantic: &[
        super::Semantic {
            description: "an empty default for a token",
            source: "fn token() -> String { std::env::var(\"GITHUB_TOKEN\").unwrap_or_default() }",
            fires: true,
            carve_out: "",
        },
        super::Semantic {
            description: "a closure fallback after .ok()",
            source: "use std::env;\nfn key() -> String { env::var(\"STRIPE_API_KEY\").ok().unwrap_or_else(|| String::from(\"sk_live_fallback\")) }",
            fires: true,
            carve_out: "",
        },
        super::Semantic {
            description: "a placeholder fallback",
            source: "fn secret() -> String { std::env::var(\"APP_SECRET\").unwrap_or(\"changeme-now\".into()) }",
            fires: false,
            carve_out: "a placeholder is no secret, and fails loudly where it is used",
        },
        super::Semantic {
            description: "a header name read from a secret-sounding variable",
            source: "fn header() -> String { std::env::var(\"API_KEY_HEADER\").unwrap_or(\"x-api-key-header\".into()) }",
            fires: false,
            carve_out: "a name ending in _HEADER names where the secret goes, not the secret",
        },
        super::Semantic {
            description: "a fallback in a test module",
            source: "#[cfg(test)]\nmod tests { fn secret() -> String { std::env::var(\"JWT_SECRET\").unwrap_or(\"test-signing-key\".into()) } }",
            fires: false,
            carve_out: "a fixed secret is the fixture of a test",
        },
    ],
};

const LITERAL_MESSAGE: &str =
    "A secret environment variable falls back to a literal, which ships in the binary and runs the service without the secret.";
const EMPTY_MESSAGE: &str =
    "A secret environment variable falls back to an empty value, so the service runs without the secret.";

fn secret_fallback(context: &Context<'_>, node: &SyntaxNode) -> Option<Detection> {
    let call = ast::MethodCallExpr::cast(node.clone())?;
    let message = fallback_message(context, &call)?;
    let mut receiver = strip_wrappers(call.receiver()?);
    if let ast::Expr::MethodCallExpr(ok) = &receiver
        && ok.name_ref()?.text() == "ok"
        && ok.arg_list()?.args().next().is_none()
    {
        receiver = ok.receiver()?;
    }
    (reads_secret_variable(context, receiver)?
        && !intersects_errors(node.text_range(), context.error_ranges)
        && !in_test_code(context, node))
    .then_some(())?;
    Some(Detection {
        message: message.into(),
        range: call.name_ref()?.syntax().text_range(),
    })
}

/// The message a fallback earns: an empty default, or a literal that can be
/// a real secret.
fn fallback_message(context: &Context<'_>, call: &ast::MethodCallExpr) -> Option<&'static str> {
    let fallback = match call.name_ref()?.text().as_str() {
        "unwrap_or_default" => {
            return call.arg_list()?.args().next().is_none().then_some(EMPTY_MESSAGE);
        }
        "unwrap_or" => only_argument(call.arg_list()?)?,
        "unwrap_or_else" => {
            let ast::Expr::ClosureExpr(closure) = only_argument(call.arg_list()?)? else {
                return None;
            };
            tail(closure.body()?)?
        }
        _ => return None,
    };
    vocabulary::real_secret(&fallback_text(context, fallback)?).then_some(LITERAL_MESSAGE)
}

/// Is `receiver` `std::env::var` or `var_os` called on a secret's name?
fn reads_secret_variable(context: &Context<'_>, receiver: ast::Expr) -> Option<bool> {
    let ast::Expr::CallExpr(read) = context.value(strip_wrappers(receiver))? else {
        return None;
    };
    let ast::Expr::PathExpr(callee) = read.expr()? else {
        return None;
    };
    let resolved = context.item_path(&callee.path()?)?;
    let reads_env = matches!(resolved.as_slice(), [krate, module, function]
        if krate == "std" && module == "env" && (function == "var" || function == "var_os"));
    let name = literal_string(context.value(only_argument(read.arg_list()?)?)?)?;
    Some(reads_env && vocabulary::secret_name(&name))
}

/// The expression a closure body returns: itself, or a block's tail.
fn tail(body: ast::Expr) -> Option<ast::Expr> {
    match body {
        ast::Expr::BlockExpr(block) => block.stmt_list()?.tail_expr(),
        expression => Some(expression),
    }
}

/// The text of a fallback written as a literal, converted or not:
/// `"x"`, `"x".into()`, `"x".to_owned()`, `"x".to_string()`, `String::from("x")`.
fn fallback_text(context: &Context<'_>, expression: ast::Expr) -> Option<String> {
    match context.value(strip_wrappers(expression))? {
        ast::Expr::MethodCallExpr(conversion) => {
            let converts = matches!(
                conversion.name_ref()?.text().as_str(),
                "into" | "to_owned" | "to_string"
            );
            (converts && conversion.arg_list()?.args().next().is_none()).then_some(())?;
            literal_string(strip_wrappers(conversion.receiver()?))
        }
        ast::Expr::CallExpr(conversion) => {
            let ast::Expr::PathExpr(callee) = conversion.expr()? else {
                return None;
            };
            let segments: Vec<String> = callee
                .path()?
                .segments()
                .filter_map(|segment| segment.name_ref().map(|name| name.text().to_string()))
                .collect();
            matches!(segments.as_slice(), [.., ty, from] if (ty == "String" || ty == "OsString") && from == "from")
                .then_some(())?;
            literal_string(only_argument(conversion.arg_list()?)?)
        }
        literal => literal_string(literal),
    }
}
