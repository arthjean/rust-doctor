//! A blocking call made from async code.
//!
//! An async executor runs many tasks on few threads, and a task that blocks
//! its thread stalls every task queued behind it. Clippy has no general lint
//! for it (rust-clippy#10794): the defect is a call to a synchronous API from
//! a place the executor polls, which is a question about where the call sits
//! rather than about its types.
//!
//! The list of blocking items is closed and names the ones services actually
//! reach for. The rule reads the call where it is written and does not follow
//! calls: a synchronous helper doing the same thing, called from async code,
//! is not seen, and the rule's help says so.

use ra_ap_syntax::ast;
use ra_ap_syntax::{AstNode, SyntaxKind, SyntaxNode};

use super::{Context, Detection, Detector, Item};
use crate::policy::SOURCE_BLOCKING_IN_ASYNC;
use crate::source_text::intersects_errors;

pub(in crate::source_kernel) static BLOCKING_CALL: Detector = Detector {
    definition: &SOURCE_BLOCKING_IN_ASYNC,
    node: SyntaxKind::CALL_EXPR,
    inspect: blocking_call,
};

pub(in crate::source_kernel) static BLOCKING_METHOD: Detector = Detector {
    definition: &SOURCE_BLOCKING_IN_ASYNC,
    node: SyntaxKind::METHOD_CALL_EXPR,
    inspect: blocking_method,
};

#[cfg(test)]
pub(in crate::source_kernel) static VARIANTS: super::Variants = super::Variants {
    rule: SOURCE_BLOCKING_IN_ASYNC.id,
    positive: super::Case {
        asynchronous: true,
        import: "std::process::Command",
        parameters: "$program: &str",
        setup: "",
        receiver: "{Item}::new($program)",
        chain: ".output()",
    },
    negative: super::Case {
        asynchronous: false,
        import: "std::process::Command",
        parameters: "$program: &str",
        setup: "",
        receiver: "{Item}::new($program)",
        chain: ".output()",
    },
    known_misses: &[],
};

const MESSAGE: &str = "A blocking call runs inside async code and stalls the executor thread.";

/// Functions and associated functions of `std` that block, the two methods
/// of `Command` included for their path form, `Command::output(&mut cmd)`.
static BLOCKING_FUNCTIONS: [(&[&str], &str); 12] = [
    (&["thread"], "sleep"),
    (&["fs"], "read"),
    (&["fs"], "read_to_string"),
    (&["fs"], "write"),
    (&["fs"], "copy"),
    (&["fs"], "remove_file"),
    (&["fs"], "create_dir_all"),
    (&["fs", "File"], "open"),
    (&["fs", "File"], "create"),
    (&["net", "TcpStream"], "connect"),
    (&["process", "Command"], "output"),
    (&["process", "Command"], "status"),
];

static COMMAND: Item = Item {
    krate: "std",
    segments: &["process", "Command"],
};

/// The methods of `std::process::Command` that wait for the child.
const WAITING_METHODS: [&str; 2] = ["output", "status"];

/// Calls whose closure argument runs off the executor: the two Tokio hands a
/// blocking call to, and a thread spawned for it.
const OFFLOADING_CALLS: [&str; 3] = ["block_in_place", "spawn", "spawn_blocking"];

fn blocking_call(context: &Context<'_>, node: &SyntaxNode) -> Option<Detection> {
    let call = ast::CallExpr::cast(node.clone())?;
    let ast::Expr::PathExpr(callee) = call.expr()? else {
        return None;
    };
    let resolved = context.item_path(&callee.path()?)?;
    let blocking = match resolved.split_first()? {
        (krate, rest) if krate == "std" => {
            BLOCKING_FUNCTIONS.iter().any(|(segments, member)| {
                rest.split_last()
                    .is_some_and(|(last, head)| last == member && head == *segments)
            })
        }
        (krate, rest) if krate == "reqwest" => rest.first().is_some_and(|module| module == "blocking"),
        _ => false,
    };
    if !blocking {
        return None;
    }
    report(context, call.syntax(), callee.syntax().text_range())
}

fn blocking_method(context: &Context<'_>, node: &SyntaxNode) -> Option<Detection> {
    let call = ast::MethodCallExpr::cast(node.clone())?;
    let name = call.name_ref()?;
    if !WAITING_METHODS.contains(&name.text().as_str()) {
        return None;
    }
    let ast::Expr::CallExpr(constructor) = context.chain_root(call.receiver()?)? else {
        return None;
    };
    if !context.calls(&constructor.expr()?, &COMMAND, "new") {
        return None;
    }
    report(context, call.syntax(), name.syntax().text_range())
}

fn report(
    context: &Context<'_>,
    call: &SyntaxNode,
    range: ra_ap_syntax::TextRange,
) -> Option<Detection> {
    if !in_async_code(call) || intersects_errors(call.text_range(), context.error_ranges) {
        return None;
    }
    Some(Detection {
        message: MESSAGE,
        range,
    })
}

/// Is this call polled by an executor? The nearest enclosing boundary
/// answers: an `async fn`, an `async` block or an `async` closure says yes, a
/// synchronous function says no, and a synchronous closure handed to one of
/// the offloading calls runs on another thread, so it says no as well. Any
/// other closure runs where it is called and is looked through.
fn in_async_code(call: &SyntaxNode) -> bool {
    for ancestor in call.ancestors().skip(1) {
        if let Some(function) = ast::Fn::cast(ancestor.clone()) {
            return function.async_token().is_some();
        }
        if let Some(block) = ast::BlockExpr::cast(ancestor.clone())
            && block.async_token().is_some()
        {
            return true;
        }
        if let Some(closure) = ast::ClosureExpr::cast(ancestor) {
            if closure.async_token().is_some() {
                return true;
            }
            if offloaded(&closure) {
                return false;
            }
        }
    }
    false
}

/// Is this closure an argument of `spawn_blocking`, `block_in_place` or a
/// `spawn`, whichever way the call is written?
fn offloaded(closure: &ast::ClosureExpr) -> bool {
    let Some(call) = closure.syntax().parent().and_then(|list| list.parent()) else {
        return false;
    };
    let name = if let Some(call) = ast::CallExpr::cast(call.clone()) {
        match call.expr() {
            Some(ast::Expr::PathExpr(path)) => path
                .path()
                .and_then(|path| path.segment())
                .and_then(|segment| segment.name_ref()),
            _ => None,
        }
    } else {
        ast::MethodCallExpr::cast(call).and_then(|call| call.name_ref())
    };
    name.is_some_and(|name| OFFLOADING_CALLS.contains(&name.text().as_str()))
}
