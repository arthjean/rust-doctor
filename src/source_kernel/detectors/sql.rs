//! SQL text assembled from a runtime value.
//!
//! Every driver the rule knows binds parameters: `sqlx` through `.bind()` and
//! `query!`, `diesel` through `.bind::<T, _>()`, `rusqlite` and `postgres`
//! through a parameter list. SQL built with `format!`, `+` or `push_str` from a
//! value the program received is the one way to reach an injection through
//! them, and the one shape a reader can see without types.
//!
//! The function forms name their crate in the path, the raw-SQL escape
//! hatches of the builders included: `diesel::dsl::sql`,
//! `sea_orm::Statement::from_string` and `sqlx::QueryBuilder::new`. A method
//! form, `conn.execute(..)` or a query builder's `push(..)`, names only a
//! method many types carry, so it is judged only when its receiver resolves,
//! through the bindings it was stored in, to a constructor of a listed type;
//! anywhere else the rule abstains. `push_bind` is the builder's parameter,
//! and stays silent.

use ra_ap_syntax::ast::{self, HasArgList};
use ra_ap_syntax::{AstNode, SyntaxKind, SyntaxNode};

use super::bindings::identifier;
use super::{Context, Detection, Detector, dynamic_text, only_argument, strip_wrappers, written_dynamic};
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
    interpolates_parameters: true,
    known_misses: &[],
    semantic: &[
        super::Semantic {
            description: "a page size typed i64 interpolated into LIMIT",
            source: "async fn page(pool: &sqlx::PgPool, limit: i64) { let _ = sqlx::query(&format!(\"SELECT * FROM t LIMIT {limit}\")).fetch_all(pool).await; }",
            fires: false,
            carve_out: "a parameter typed as a numeric primitive cannot carry SQL",
        },
        super::Semantic {
            description: "a value cast to i64 before it is interpolated",
            source: "async fn page(pool: &sqlx::PgPool, n: usize) { let _ = sqlx::query(&format!(\"SELECT * FROM t LIMIT {}\", n as i64)).fetch_all(pool).await; }",
            fires: false,
            carve_out: "a cast to a numeric primitive cannot carry SQL",
        },
        super::Semantic {
            description: "a field stored in a let typed i64 before it is interpolated",
            source: "struct Page { limit: i64 }\nasync fn page(pool: &sqlx::PgPool, page: Page) { let limit: i64 = page.limit; let _ = sqlx::query(&format!(\"SELECT * FROM t LIMIT {limit}\")).fetch_all(pool).await; }",
            fires: false,
            carve_out: "a let typed as a numeric primitive cannot carry SQL",
        },
        super::Semantic {
            description: "a field stored in a let typed String before it is interpolated",
            source: "struct Filter { name: String }\nasync fn find(pool: &sqlx::PgPool, filter: Filter) { let name: String = filter.name; let _ = sqlx::query(&format!(\"SELECT * FROM t WHERE name = '{}'\", name)).fetch_all(pool).await; }",
            fires: true,
            carve_out: "",
        },
        super::Semantic {
            description: "a parameter typed through an alias of i64",
            source: "type Id = i64;\nasync fn find(pool: &sqlx::PgPool, id: Id) { let _ = sqlx::query(&format!(\"SELECT * FROM t WHERE id = {id}\")).fetch_all(pool).await; }",
            fires: true,
            carve_out: "",
        },
        super::Semantic {
            description: "a generic parameter interpolated into the query",
            source: "async fn find<T: std::fmt::Display>(pool: &sqlx::PgPool, id: T) { let _ = sqlx::query(&format!(\"SELECT * FROM t WHERE id = {id}\")).fetch_all(pool).await; }",
            fires: true,
            carve_out: "",
        },
        super::Semantic {
            description: "an i64 parameter whose type name a local alias shadows",
            source: "#[allow(non_camel_case_types)]\ntype i64 = String;\nasync fn find(pool: &sqlx::PgPool, id: i64) { let _ = sqlx::query(&format!(\"SELECT * FROM t WHERE id = '{id}'\")).fetch_all(pool).await; }",
            fires: true,
            carve_out: "",
        },
        super::Semantic {
            description: "SQL concatenated with String +",
            source: "async fn find(pool: &sqlx::PgPool, name: &str) { let sql = \"SELECT * FROM users WHERE name = '\".to_owned() + name + \"'\"; let _ = sqlx::query(&sql).fetch_all(pool).await; }",
            fires: true,
            carve_out: "",
        },
        super::Semantic {
            description: "SQL written into a String with write!",
            source: "use std::fmt::Write;\nasync fn find(pool: &sqlx::PgPool, name: &str) { let mut sql = String::new(); let _ = write!(sql, \"SELECT * FROM users WHERE name = '{name}'\"); let _ = sqlx::query(&sql).fetch_all(pool).await; }",
            fires: true,
            carve_out: "",
        },
        super::Semantic {
            description: "QueryBuilder::push with a runtime value",
            source: "use sqlx::QueryBuilder;\nfn find(name: &str) { let mut query = QueryBuilder::<sqlx::Postgres>::new(\"SELECT * FROM users WHERE \"); query.push(format!(\"name = '{name}'\")); }",
            fires: true,
            carve_out: "",
        },
        super::Semantic {
            description: "diesel::dsl::sql with a runtime value",
            source: "fn filter(name: &str) { let _ = diesel::dsl::sql::<diesel::sql_types::Bool>(&format!(\"name = '{name}'\")); }",
            fires: true,
            carve_out: "",
        },
        super::Semantic {
            description: "sea_orm::Statement::from_string with a runtime value",
            source: "fn statement(name: &str) { let _ = sea_orm::Statement::from_string(sea_orm::DatabaseBackend::Postgres, format!(\"SELECT * FROM users WHERE name = '{name}'\")); }",
            fires: true,
            carve_out: "",
        },
        super::Semantic {
            description: "a runtime value bound through QueryBuilder::push_bind",
            source: "use sqlx::QueryBuilder;\nfn find(name: &str) { let mut query = QueryBuilder::<sqlx::Postgres>::new(\"SELECT * FROM users WHERE name = \"); query.push_bind(name); }",
            fires: false,
            carve_out: "a bound parameter never becomes SQL text",
        },
        super::Semantic {
            description: "a fully qualified query under diesel's prelude glob",
            source: "use diesel::prelude::*;\nfn remove(name: &str) { let _ = diesel::sql_query(format!(\"DELETE FROM users WHERE name = '{name}'\")); }",
            fires: true,
            carve_out: "",
        },
    ],
};

const MESSAGE: &str = "SQL text is assembled from a runtime value instead of bound as a parameter.";

/// Functions taking SQL text, and the position of the argument that holds it.
static QUERY_FUNCTIONS: [(&[&str], usize); 8] = [
    (&["sqlx", "query"], 0),
    (&["sqlx", "query_as"], 0),
    (&["sqlx", "query_scalar"], 0),
    (&["sqlx", "raw_sql"], 0),
    (&["sqlx", "QueryBuilder", "new"], 0),
    (&["diesel", "sql_query"], 0),
    (&["diesel", "dsl", "sql"], 0),
    (&["sea_orm", "Statement", "from_string"], 1),
];

/// The query builder whose `push` appends raw SQL text.
const QUERY_BUILDER: [&str; 3] = ["sqlx", "QueryBuilder", "new"];

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
    let listed = QUERY_FUNCTIONS
        .iter()
        .find(|(path, _)| path.iter().copied().eq(resolved.iter().map(String::as_str)))
        .map(|(_, position)| *position);
    let position = match (listed, resolved.as_slice()) {
        (Some(position), _) => position,
        // The path form of a connection method, `Connection::execute(&conn,
        // sql, ..)`, carries the receiver first.
        (None, [krate, kind, method]) if connection_method(krate, kind, method) => 1,
        _ => return None,
    };
    let argument = call.arg_list()?.args().nth(position)?;
    report(context, call.syntax(), &argument)
}

fn sql_method(context: &Context<'_>, node: &SyntaxNode) -> Option<Detection> {
    let call = ast::MethodCallExpr::cast(node.clone())?;
    let method = call.name_ref()?.text().to_string();
    let argument = call.arg_list()?.args().next()?;
    if method == "push" {
        // `push` is `Vec::push` far more often than a builder's. An argument
        // that is neither assembled where it is read nor a binding cannot be
        // SQL text grown elsewhere, and is passed over before the receiver is
        // resolved; the search for a `push_str` waits for a builder.
        let text = transparent(argument.clone());
        if !dynamic_text(context, &text) && identifier(&text).is_none() {
            return None;
        }
        let resolved = constructor(context, &call)?;
        if !QUERY_BUILDER.iter().copied().eq(resolved.iter().map(String::as_str)) {
            return None;
        }
        return report(context, call.syntax(), &argument);
    }
    if !CONNECTIONS
        .iter()
        .any(|(_, _, methods)| methods.contains(&method.as_str()))
    {
        return None;
    }
    // The cheap test first: most calls named `execute` carry no assembled
    // text, and those never pay for resolving their receiver.
    if !assembled(context, &argument) {
        return None;
    }
    let resolved = constructor(context, &call)?;
    let [krate, kind, _] = resolved.as_slice() else {
        return None;
    };
    if !connection_method(krate, kind, &method) {
        return None;
    }
    report(context, call.syntax(), &argument)
}

/// The item that built a method call's receiver, read through the bindings
/// and the chain it was stored in: `Connection::open(path)?` names
/// `rusqlite::Connection::open`.
fn constructor(context: &Context<'_>, call: &ast::MethodCallExpr) -> Option<Vec<String>> {
    let ast::Expr::CallExpr(constructor) = context.chain_root(call.receiver()?)? else {
        return None;
    };
    let ast::Expr::PathExpr(callee) = constructor.expr()? else {
        return None;
    };
    context.item_path(&callee.path()?)
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
        message: MESSAGE.into(),
        range: argument.syntax().text_range(),
    })
}

/// Was this SQL text assembled from a runtime value? A `format!` or a `+`
/// with a non-constant operand, read through the bindings it was stored in,
/// or a binding a `push_str` or a `write!` of a runtime value grew before the
/// call.
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

/// Did a `push_str` or a `write!` of a runtime value grow the binding `text`
/// names, before `text` is read? The binding itself has to resolve: one that
/// is reassigned or shadowed is abstained on, like everywhere else.
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
    let before = |node: &ra_ap_syntax::SyntaxNode| node.text_range().end() <= read;
    let written = body
        .syntax()
        .descendants()
        .filter_map(ast::MacroCall::cast)
        .filter(|call| before(call.syntax()))
        .any(|call| written_dynamic(context, &call).is_some_and(|destination| destination == name));
    let grown = body
        .syntax()
        .descendants()
        .filter_map(ast::MethodCallExpr::cast)
        .filter(|call| before(call.syntax()))
        .filter(|call| call.name_ref().is_some_and(|method| method.text() == "push_str"))
        .filter(|call| {
            call.receiver()
                .map(strip_wrappers)
                .as_ref()
                .and_then(identifier)
                .is_some_and(|receiver| receiver == name)
        })
        .filter_map(|call| only_argument(call.arg_list()?))
        .any(|pushed| context.injectable(&pushed, text.syntax()));
    written || grown
}
