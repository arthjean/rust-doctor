//! A blocking call made from async code.
//!
//! An async executor runs many tasks on few threads, and a task that blocks
//! its thread stalls every task queued behind it. Clippy has no general lint
//! for it (rust-clippy#10794): the defect is a call to a synchronous API from
//! a place the executor polls, which is a question about where the call sits
//! rather than about its types.
//!
//! The list of blocking items is closed and names the ones services actually
//! reach for. The rule reads the call where it is written, and follows one
//! kind of call further: a synchronous `fn` of the same file that the call
//! names unambiguously, the way services hide `std::fs` behind a
//! `load_config()`. That helper's body is searched, and the helpers it calls
//! in turn, `HELPER_DEPTH` deep, and the finding sits on the call in async code
//! and names what the helper reaches.

use std::borrow::Cow;

use ra_ap_syntax::ast::{self, HasName};
use ra_ap_syntax::{AstNode, SyntaxKind, SyntaxNode, TextRange, WalkEvent};

use super::{Context, Detection, Detector, Item, strip_wrappers};
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
    interpolates_parameters: false,
    known_misses: &[
    ],
    semantic: &[
        super::Semantic {
            description: "an async move block handed to tokio::spawn",
            source: "fn start() { tokio::spawn(async move { std::thread::sleep(std::time::Duration::from_secs(1)); }); }",
            fires: true,
            carve_out: "",
        },
        super::Semantic {
            description: "a fully qualified call under an external prelude glob",
            source: "use futures::prelude::*;\nasync fn tick() { std::thread::sleep(std::time::Duration::from_secs(1)); }",
            fires: true,
            carve_out: "",
        },
        super::Semantic {
            description: "a synchronous helper of the same module called from async code",
            source: "fn load() -> String { std::fs::read_to_string(\"config.toml\").unwrap_or_default() }\nasync fn handler() { let _ = load(); }",
            fires: true,
            carve_out: "",
        },
        super::Semantic {
            description: "a blocking read handed to spawn_blocking",
            source: "async fn handler() { let _ = tokio::task::spawn_blocking(|| std::fs::read(\"data\")).await; }",
            fires: false,
            carve_out: "the closure runs on the blocking pool, not on the executor",
        },
        super::Semantic {
            description: "a fully qualified call under the crate's own prelude glob",
            source: "use crate::prelude::*;\nasync fn tick() { std::thread::sleep(std::time::Duration::from_secs(1)); }",
            fires: false,
            carve_out: "a glob of the crate's own module may bind `std` itself, and nothing proves it does not",
        },
    ],
};

const MESSAGE: &str = "A blocking call runs inside async code and stalls the executor thread.";

/// Helpers followed from one call in async code, the called one included.
const HELPER_DEPTH: usize = 4;

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
    let path = callee.path()?;
    let range = callee.syntax().text_range();
    if blocking_item(context, &path).is_some() {
        return report(context, call.syntax(), range, MESSAGE.into());
    }
    // A helper is named by one segment, or by `Self::` and one more.
    if path.qualifier().and_then(|qualifier| qualifier.qualifier()).is_some() || !in_async_code(call.syntax()) {
        return None;
    }
    let helper = context.local_function(&path)?;
    through_helper(context, call.syntax(), range, &helper)
}

fn blocking_method(context: &Context<'_>, node: &SyntaxNode) -> Option<Detection> {
    let call = ast::MethodCallExpr::cast(node.clone())?;
    let name = call.name_ref()?;
    let range = name.syntax().text_range();
    if waiting_method(context, &call).is_some() {
        return report(context, call.syntax(), range, MESSAGE.into());
    }
    if !on_self(&call) || !in_async_code(call.syntax()) {
        return None;
    }
    let helper = self_method(context, &call)?;
    through_helper(context, call.syntax(), range, &helper)
}

/// The blocking item a call path names, written the way the message prints
/// it: `std::fs::read`.
fn blocking_item(context: &Context<'_>, path: &ast::Path) -> Option<String> {
    let resolved = context.item_path(path)?;
    let blocking = match resolved.split_first()? {
        (krate, rest) if krate == "std" => BLOCKING_FUNCTIONS.iter().any(|(segments, member)| {
            rest.split_last()
                .is_some_and(|(last, head)| last == member && head == *segments)
        }),
        (krate, rest) if krate == "reqwest" => rest.first().is_some_and(|module| module == "blocking"),
        _ => false,
    };
    blocking.then(|| resolved.join("::"))
}

/// `cmd.output()` or `cmd.status()` on a `std::process::Command`, named the
/// way the message prints it.
fn waiting_method(context: &Context<'_>, call: &ast::MethodCallExpr) -> Option<String> {
    let name = call.name_ref()?;
    if !WAITING_METHODS.contains(&name.text().as_str()) {
        return None;
    }
    let ast::Expr::CallExpr(constructor) = context.chain_root(call.receiver()?)? else {
        return None;
    };
    context
        .calls(&constructor.expr()?, &COMMAND, "new")
        .then(|| {
            std::iter::once(COMMAND.krate)
                .chain(COMMAND.segments.iter().copied())
                .chain([name.text().as_str()])
                .collect::<Vec<_>>()
                .join("::")
        })
}

/// What `self.f()` calls: the one method `f` of the enclosing `impl`.
fn self_method(context: &Context<'_>, call: &ast::MethodCallExpr) -> Option<ast::Fn> {
    if !on_self(call) {
        return None;
    }
    context.impl_function(call.syntax(), call.name_ref()?.text().as_str())
}

/// Is this method called on `self`?
fn on_self(call: &ast::MethodCallExpr) -> bool {
    let Some(ast::Expr::PathExpr(receiver)) = call.receiver().map(strip_wrappers) else {
        return false;
    };
    receiver.path().is_some_and(|receiver| {
        receiver.qualifier().is_none()
            && receiver
                .segment()
                .and_then(|segment| segment.kind())
                .is_some_and(|kind| kind == ast::PathSegmentKind::SelfKw)
    })
}

/// The finding for a call in async code to a synchronous helper whose body,
/// or a helper's it calls in turn, blocks.
fn through_helper(
    context: &Context<'_>,
    call: &SyntaxNode,
    range: TextRange,
    helper: &ast::Fn,
) -> Option<Detection> {
    if helper.async_token().is_some() {
        return None;
    }
    let name = helper.name()?.text().to_string();
    let mut visited = Vec::new();
    let blocking = hidden_blocking(context, helper, HELPER_DEPTH, &mut visited)?;
    report(
        context,
        call,
        range,
        Cow::Owned(format!(
            "A blocking call runs inside async code through the synchronous helper `{name}`, which reaches `{blocking}`, and stalls the executor thread."
        )),
    )
}

/// The first blocking call `helper` makes, directly or through the
/// synchronous helpers it calls, `depth` helpers deep. A helper already on
/// the way is not entered twice, so recursion ends without a finding. Nested
/// `fn` items, `async` blocks and closures, and closures handed off the
/// executor are skipped: none of them runs when the helper does.
fn hidden_blocking(
    context: &Context<'_>,
    helper: &ast::Fn,
    depth: usize,
    visited: &mut Vec<TextRange>,
) -> Option<String> {
    let range = helper.syntax().text_range();
    if depth == 0 || visited.contains(&range) || helper.async_token().is_some() {
        return None;
    }
    visited.push(range);
    let body = helper.body()?;
    let mut nested = Vec::new();
    let mut walk = body.syntax().preorder();
    while let Some(event) = walk.next() {
        let WalkEvent::Enter(node) = event else {
            continue;
        };
        if skipped(&node) {
            walk.skip_subtree();
            continue;
        }
        if let Some(call) = ast::CallExpr::cast(node.clone())
            && let Some(ast::Expr::PathExpr(callee)) = call.expr()
            && let Some(path) = callee.path()
        {
            if let Some(blocking) = blocking_item(context, &path) {
                return Some(blocking);
            }
            nested.extend(context.local_function(&path));
        } else if let Some(call) = ast::MethodCallExpr::cast(node) {
            if let Some(blocking) = waiting_method(context, &call) {
                return Some(blocking);
            }
            nested.extend(self_method(context, &call));
        }
    }
    nested
        .iter()
        .find_map(|next| hidden_blocking(context, next, depth - 1, visited))
}

/// Code inside a helper's body that does not run when the helper does.
fn skipped(node: &SyntaxNode) -> bool {
    ast::Fn::can_cast(node.kind())
        || ast::BlockExpr::cast(node.clone()).is_some_and(|block| block.async_token().is_some())
        || ast::ClosureExpr::cast(node.clone())
            .is_some_and(|closure| closure.async_token().is_some() || offloaded(&closure))
}

fn report(
    context: &Context<'_>,
    call: &SyntaxNode,
    range: TextRange,
    message: Cow<'static, str>,
) -> Option<Detection> {
    if !in_async_code(call) || intersects_errors(call.text_range(), context.error_ranges) {
        return None;
    }
    Some(Detection { message, range })
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
