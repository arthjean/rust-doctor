//! The configuration calls that switch off a web security control: a CORS
//! layer letting any origin send credentials, an auth cookie page scripts can
//! read, and a JWT decoded without its signature checked.
//!
//! Each call names the crate whose control it switches off, `tower_http`,
//! `actix_cors`, `cookie` or `jsonwebtoken`, through the path of the call or
//! of the constructor its chain starts from, read through the bindings it was
//! stored in. Booleans are read through `Context::value`, like
//! `disabled_tls_verification`, and all three rules are silent in test code,
//! where a permissive configuration is the fixture.

use ra_ap_syntax::ast::{self, HasArgList};
use ra_ap_syntax::{AstNode, SyntaxKind, SyntaxNode};

use super::{
    CHAIN_DEPTH, Context, Detection, Detector, in_test_code, literal_bool, literal_string, only_argument,
    strip_wrappers,
};
use crate::policy::{SOURCE_DISABLED_JWT_VALIDATION, SOURCE_INSECURE_AUTH_COOKIE, SOURCE_PERMISSIVE_CORS};
use crate::source_text::intersects_errors;

#[cfg(test)]
mod variants;
#[cfg(test)]
pub(in crate::source_kernel) use variants::{COOKIE_VARIANTS, CORS_VARIANTS, JWT_VARIANTS};

pub(in crate::source_kernel) static CORS_CALL: Detector = Detector {
    definition: &SOURCE_PERMISSIVE_CORS,
    node: SyntaxKind::CALL_EXPR,
    inspect: permissive_constructor,
    crates: &["actix_cors", "tower_http"],
};

pub(in crate::source_kernel) static CORS_CREDENTIALS: Detector = Detector {
    definition: &SOURCE_PERMISSIVE_CORS,
    node: SyntaxKind::METHOD_CALL_EXPR,
    inspect: credentials_from_any_origin,
    crates: &["tower_http"],
};

pub(in crate::source_kernel) static AUTH_COOKIE: Detector = Detector {
    definition: &SOURCE_INSECURE_AUTH_COOKIE,
    node: SyntaxKind::METHOD_CALL_EXPR,
    inspect: readable_auth_cookie,
    crates: &["cookie"],
};

pub(in crate::source_kernel) static JWT_METHOD: Detector = Detector {
    definition: &SOURCE_DISABLED_JWT_VALIDATION,
    node: SyntaxKind::METHOD_CALL_EXPR,
    inspect: disabled_signature_check,
    crates: &["jsonwebtoken"],
};

pub(in crate::source_kernel) static JWT_CALL: Detector = Detector {
    definition: &SOURCE_DISABLED_JWT_VALIDATION,
    node: SyntaxKind::CALL_EXPR,
    inspect: insecure_decode,
    crates: &["jsonwebtoken"],
};

/// The constructors that are a permissive CORS policy on their own.
const PERMISSIVE_CONSTRUCTORS: [&[&str]; 2] = [
    &["tower_http", "cors", "CorsLayer", "very_permissive"],
    &["actix_cors", "Cors", "permissive"],
];

/// The origin policies that accept any origin.
const ANY_ORIGIN: [&[&str]; 3] = [
    &["tower_http", "cors", "Any"],
    &["tower_http", "cors", "AllowOrigin", "any"],
    &["tower_http", "cors", "AllowOrigin", "mirror_request"],
];

/// The cookie constructors whose first argument is the cookie's name.
const COOKIE_CONSTRUCTORS: [&[&str]; 3] = [
    &["cookie", "Cookie", "build"],
    &["cookie", "Cookie", "new"],
    &["cookie", "CookieBuilder", "new"],
];

/// The words of an auth cookie's name, and of the CSRF cookie that only looks
/// like one: a bare `token` names a CSRF cookie as often as a session.
const AUTH_COOKIE_WORDS: [&str; 4] = ["access_token", "auth", "jwt", "session"];
const CSRF_WORDS: [&str; 2] = ["csrf", "xsrf"];

/// The calls that decode a JWT without checking its signature.
const INSECURE_DECODES: [&[&str]; 2] = [
    &["jsonwebtoken", "dangerous", "insecure_decode"],
    &["jsonwebtoken", "dangerous_insecure_decode"],
];

fn resolves_to(context: &Context<'_>, callee: &ast::Expr, paths: &[&[&str]]) -> bool {
    let ast::Expr::PathExpr(callee) = callee else {
        return false;
    };
    callee
        .path()
        .and_then(|path| context.item_path(&path))
        .is_some_and(|resolved| {
            paths
                .iter()
                .any(|path| path.iter().copied().eq(resolved.iter().map(String::as_str)))
        })
}

fn reported(context: &Context<'_>, node: &SyntaxNode, message: &'static str, range: &SyntaxNode) -> Option<Detection> {
    if intersects_errors(node.text_range(), context.error_ranges) || in_test_code(context, node) {
        return None;
    }
    Some(Detection {
        message: message.into(),
        range: range.text_range(),
    })
}

fn permissive_constructor(context: &Context<'_>, node: &SyntaxNode) -> Option<Detection> {
    let call = ast::CallExpr::cast(node.clone())?;
    resolves_to(context, &call.expr()?, &PERMISSIVE_CONSTRUCTORS).then_some(())?;
    reported(context, node, "The CORS policy accepts any origin, method and header, credentials included.", node)
}

/// `allow_credentials(true)` in a chain that also allows any origin, read
/// across the whole chain: the links after the call and the links before it,
/// through the bindings the layer was stored in.
fn credentials_from_any_origin(context: &Context<'_>, node: &SyntaxNode) -> Option<Detection> {
    let call = ast::MethodCallExpr::cast(node.clone())?;
    (call.name_ref()?.text() == "allow_credentials").then_some(())?;
    let argument = only_argument(call.arg_list()?)?;
    literal_bool(&context.value(argument.clone())?, true).then_some(())?;
    let mut outermost = call.syntax().clone();
    while let Some(parent) = outermost.parent().and_then(ast::MethodCallExpr::cast)
        && parent.receiver().is_some_and(|receiver| receiver.syntax() == &outermost)
    {
        outermost = parent.syntax().clone();
    }
    let mut expression = ast::Expr::cast(outermost)?;
    for _ in 0..CHAIN_DEPTH {
        let ast::Expr::MethodCallExpr(link) = context.value(strip_wrappers(expression))? else {
            return None;
        };
        if link.name_ref()?.text() == "allow_origin"
            && let Some(origin) = link.arg_list().and_then(only_argument)
            && any_origin(context, origin)
        {
            return reported(
                context,
                node,
                "The CORS layer lets any origin send credentialed requests.",
                argument.syntax(),
            );
        }
        expression = link.receiver()?;
    }
    None
}

fn any_origin(context: &Context<'_>, origin: ast::Expr) -> bool {
    match context.value(strip_wrappers(origin)) {
        Some(ast::Expr::CallExpr(call)) => call
            .expr()
            .is_some_and(|callee| resolves_to(context, &callee, &ANY_ORIGIN)),
        Some(path @ ast::Expr::PathExpr(_)) => resolves_to(context, &path, &ANY_ORIGIN),
        _ => false,
    }
}

fn readable_auth_cookie(context: &Context<'_>, node: &SyntaxNode) -> Option<Detection> {
    let call = ast::MethodCallExpr::cast(node.clone())?;
    matches!(call.name_ref()?.text().as_str(), "http_only" | "set_http_only").then_some(())?;
    let argument = only_argument(call.arg_list()?)?;
    literal_bool(&context.value(argument.clone())?, false).then_some(())?;
    let name = cookie_name(context, call.receiver()?)?;
    if !AUTH_COOKIE_WORDS.iter().any(|word| name.contains(word)) || CSRF_WORDS.iter().any(|word| name.contains(word)) {
        return None;
    }
    reported(context, node, "An auth cookie is readable by page scripts.", argument.syntax())
}

/// The literal name, lowercased, of the cookie a chain builds, read off the
/// `cookie` constructor it starts from: `Cookie::new(name, value)` or
/// `Cookie::build((name, value))`.
fn cookie_name(context: &Context<'_>, receiver: ast::Expr) -> Option<String> {
    let ast::Expr::CallExpr(constructor) = context.chain_root(receiver)? else {
        return None;
    };
    resolves_to(context, &constructor.expr()?, &COOKIE_CONSTRUCTORS).then_some(())?;
    let first = constructor.arg_list()?.args().next()?;
    let name = match context.value(strip_wrappers(first))? {
        ast::Expr::TupleExpr(pair) => literal_string(pair.fields().next()?)?,
        name => literal_string(name)?,
    };
    Some(name.to_ascii_lowercase())
}

fn disabled_signature_check(context: &Context<'_>, node: &SyntaxNode) -> Option<Detection> {
    let call = ast::MethodCallExpr::cast(node.clone())?;
    (call.name_ref()?.text() == "insecure_disable_signature_validation").then_some(())?;
    let ast::Expr::CallExpr(constructor) = context.chain_root(call.receiver()?)? else {
        return None;
    };
    resolves_to(
        context,
        &constructor.expr()?,
        &[&["jsonwebtoken", "Validation", "new"], &["jsonwebtoken", "Validation", "default"]],
    )
    .then_some(())?;
    reported(context, node, "JWT signature validation is turned off.", call.name_ref()?.syntax())
}

fn insecure_decode(context: &Context<'_>, node: &SyntaxNode) -> Option<Detection> {
    let call = ast::CallExpr::cast(node.clone())?;
    let callee = call.expr()?;
    resolves_to(context, &callee, &INSECURE_DECODES).then_some(())?;
    reported(context, node, "The JWT is decoded without checking its signature.", callee.syntax())
}
