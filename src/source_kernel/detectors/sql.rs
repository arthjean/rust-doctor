//! SQL text assembled from a runtime value.
//!
//! Every driver the rule knows binds parameters: `sqlx` through `.bind()` and
//! `query!`, `diesel` through `.bind::<T, _>()`, `rusqlite` and `postgres`
//! through a parameter list. SQL built with `format!`, `+` or `push_str` from a
//! value the program received is the one way to reach an injection through
//! them, and the one shape a reader can see without types.
//!
//! The function forms name their crate in the path. A method form,
//! `conn.execute(..)`, names only a method many types carry, so it is judged
//! only when its receiver resolves, through the bindings it was stored in, to
//! a constructor of a listed connection type; anywhere else the rule abstains.

use ra_ap_syntax::ast::{self, HasArgList};
use ra_ap_syntax::{AstNode, SyntaxKind, SyntaxNode};

use super::bindings::identifier;
use super::{Context, Detection, Detector, dynamic_text, only_argument, strip_wrappers};
use crate::policy::SOURCE_SQL_FROM_FORMAT;
use crate::source_text::intersects_errors;

pub(in crate::source_kernel) static SQL_CALL: Detector = Detector {
    definition: &SOURCE_SQL_FROM_FORMAT,
    node: SyntaxKind::CALL_EXPR,
    inspect: sql_call,
};

pub(in crate::source_kernel) static SQL_METHOD: Detector = Detector {
    definition: &SOURCE_SQL_FROM_FORMAT,
    node: SyntaxKind::METHOD_CALL_EXPR,
    inspect: sql_method,
};

#[cfg(test)]
pub(in crate::source_kernel) static VARIANTS: super::Variants = super::Variants {
    rule: SOURCE_SQL_FROM_FORMAT.id,
    positive: super::Case {
        asynchronous: false,
        import: "rusqlite::Connection",
        parameters: "$name: &str",
        setup: "",
        receiver: "{Item}::open_in_memory().unwrap()",
        chain: ".execute(&format!(\"DELETE FROM users WHERE name = '{$name}'\"), ())",
    },
    negative: super::Case {
        asynchronous: false,
        import: "rusqlite::Connection",
        parameters: "$name: &str",
        setup: "",
        receiver: "{Item}::open_in_memory().unwrap()",
        chain: ".execute(\"DELETE FROM users WHERE name = ?1\", [$name])",
    },
    known_misses: &[],
};

const MESSAGE: &str = "SQL text is assembled from a runtime value instead of bound as a parameter.";

/// Functions taking the SQL text as their first argument.
static QUERY_FUNCTIONS: [(&str, &str); 5] = [
    ("sqlx", "query"),
    ("sqlx", "query_as"),
    ("sqlx", "query_scalar"),
    ("sqlx", "raw_sql"),
    ("diesel", "sql_query"),
];

/// Connection types and the methods of theirs taking the SQL text first.
static CONNECTIONS: [(&str, &str, &[&str]); 3] = [
    ("rusqlite", "Connection", &["execute", "prepare", "query_row"]),
    ("postgres", "Client", &["batch_execute", "execute", "query"]),
    ("tokio_postgres", "Client", &["batch_execute", "execute", "query"]),
];

/// Methods that hand back the text they were called on, so the SQL argument is
/// read through them: `sqlx::query(sql.as_str())` binds what `sql` holds.
const TRANSPARENT_METHODS: [&str; 3] = ["as_ref", "as_str", "clone"];

fn sql_call(context: &Context<'_>, node: &SyntaxNode) -> Option<Detection> {
    let call = ast::CallExpr::cast(node.clone())?;
    let ast::Expr::PathExpr(callee) = call.expr()? else {
        return None;
    };
    let resolved = context.item_path(&callee.path()?)?;
    let position = match resolved.as_slice() {
        [krate, function] if QUERY_FUNCTIONS.contains(&(krate.as_str(), function.as_str())) => 0,
        // The path form of a connection method, `Connection::execute(&conn,
        // sql, ..)`, carries the receiver first.
        [krate, kind, method] if connection_method(krate, kind, method) => 1,
        _ => return None,
    };
    let argument = call.arg_list()?.args().nth(position)?;
    report(context, call.syntax(), &argument)
}

fn sql_method(context: &Context<'_>, node: &SyntaxNode) -> Option<Detection> {
    let call = ast::MethodCallExpr::cast(node.clone())?;
    let method = call.name_ref()?.text().to_string();
    if !CONNECTIONS
        .iter()
        .any(|(_, _, methods)| methods.contains(&method.as_str()))
    {
        return None;
    }
    let argument = call.arg_list()?.args().next()?;
    // The cheap test first: most calls named `execute` carry no assembled
    // text, and those never pay for resolving their receiver.
    if !assembled(context, &argument) {
        return None;
    }
    let ast::Expr::CallExpr(constructor) = context.chain_root(call.receiver()?)? else {
        return None;
    };
    let ast::Expr::PathExpr(callee) = constructor.expr()? else {
        return None;
    };
    let resolved = context.item_path(&callee.path()?)?;
    let [krate, kind, _] = resolved.as_slice() else {
        return None;
    };
    if !connection_method(krate, kind, &method) {
        return None;
    }
    report(context, call.syntax(), &argument)
}

fn connection_method(krate: &str, kind: &str, method: &str) -> bool {
    CONNECTIONS.iter().any(|(listed_crate, listed_kind, methods)| {
        *listed_crate == krate && *listed_kind == kind && methods.contains(&method)
    })
}

fn report(context: &Context<'_>, call: &SyntaxNode, argument: &ast::Expr) -> Option<Detection> {
    if !assembled(context, argument) || intersects_errors(call.text_range(), context.error_ranges) {
        return None;
    }
    Some(Detection {
        message: MESSAGE,
        range: argument.syntax().text_range(),
    })
}

/// Was this SQL text assembled from a runtime value? A `format!` or a `+`
/// with a non-constant operand, read through the bindings it was stored in,
/// or a binding a `push_str` of a non-constant value grew before the call.
fn assembled(context: &Context<'_>, argument: &ast::Expr) -> bool {
    let text = transparent(argument.clone());
    dynamic_text(context, &text) || pushed(context, &text)
}

fn transparent(mut expression: ast::Expr) -> ast::Expr {
    loop {
        expression = strip_wrappers(expression);
        let ast::Expr::MethodCallExpr(call) = &expression else {
            return expression;
        };
        let passes_through = call
            .name_ref()
            .is_some_and(|name| TRANSPARENT_METHODS.contains(&name.text().as_str()))
            && call.arg_list().is_some_and(|list| list.args().next().is_none());
        match call.receiver() {
            Some(receiver) if passes_through => expression = receiver,
            _ => return expression,
        }
    }
}

/// Did a `push_str` of a non-constant value grow the binding `text` names,
/// before `text` is read? The binding itself has to resolve: one that is
/// reassigned or shadowed is abstained on, like everywhere else.
fn pushed(context: &Context<'_>, text: &ast::Expr) -> bool {
    let Some(name) = identifier(text) else {
        return false;
    };
    if context.value(text.clone()).is_none() {
        return false;
    }
    let Some(body) = text.syntax().ancestors().find_map(ast::Fn::cast) else {
        return false;
    };
    let read = text.syntax().text_range().start();
    body.syntax()
        .descendants()
        .filter_map(ast::MethodCallExpr::cast)
        .filter(|call| call.syntax().text_range().end() <= read)
        .filter(|call| call.name_ref().is_some_and(|method| method.text() == "push_str"))
        .filter(|call| {
            call.receiver()
                .map(strip_wrappers)
                .as_ref()
                .and_then(identifier)
                .is_some_and(|receiver| receiver == name)
        })
        .filter_map(|call| only_argument(call.arg_list()?))
        .any(|pushed| !context.constant(&pushed))
}
