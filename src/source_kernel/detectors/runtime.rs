//! The two tokio calls that panic at run time where tokio documents they
//! panic.
//!
//! `tokio::task::block_in_place` panics on a current-thread runtime: there is
//! no other worker to hand the executor to. The runtime a function runs on is
//! written on it, `#[tokio::test]` defaulting to current-thread and
//! `#[tokio::main]` to multi-thread, so the rule reads the attribute of the
//! function holding the call and nothing else; outside such a function tokio
//! runs the closure inline and nothing panics. `block_on` called from code an
//! executor polls panics with "Cannot start a runtime from within a runtime"
//! for tokio's two forms and deadlocks for the `futures` executor, whichever
//! runtime polls it.

use ra_ap_syntax::ast::{self, HasAttrs};
use ra_ap_syntax::{AstNode, SyntaxKind, SyntaxNode};

use super::blocking::in_async_code;
use super::{Context, Detection, Detector};
use crate::policy::{SOURCE_BLOCK_IN_PLACE_CURRENT_THREAD, SOURCE_BLOCK_ON_IN_ASYNC};
use crate::source_text::{compact, intersects_errors};

pub(in crate::source_kernel) static BLOCK_IN_PLACE: Detector = Detector {
    definition: &SOURCE_BLOCK_IN_PLACE_CURRENT_THREAD,
    node: SyntaxKind::CALL_EXPR,
    inspect: block_in_place,
    crates: &["tokio"],
};

pub(in crate::source_kernel) static BLOCK_ON_CALL: Detector = Detector {
    definition: &SOURCE_BLOCK_ON_IN_ASYNC,
    node: SyntaxKind::CALL_EXPR,
    inspect: block_on_call,
    crates: &["futures", "tokio"],
};

pub(in crate::source_kernel) static BLOCK_ON_METHOD: Detector = Detector {
    definition: &SOURCE_BLOCK_ON_IN_ASYNC,
    node: SyntaxKind::METHOD_CALL_EXPR,
    inspect: block_on_method,
    crates: &["futures", "tokio"],
};

#[cfg(test)]
pub(in crate::source_kernel) static BLOCK_IN_PLACE_VARIANTS: super::Variants = super::Variants {
    rule: SOURCE_BLOCK_IN_PLACE_CURRENT_THREAD.id,
    positive: super::Case {
        attributes: "#[tokio::main(flavor = \"current_thread\")]",
        asynchronous: true,
        import: "tokio::task::block_in_place",
        parameters: "$work: fn() -> u8",
        setup: "",
        receiver: "{Item}($work)",
        chain: "",
    },
    negative: super::Case {
        attributes: "#[tokio::main(flavor = \"multi_thread\", worker_threads = 2)]",
        asynchronous: true,
        import: "tokio::task::block_in_place",
        parameters: "$work: fn() -> u8",
        setup: "",
        receiver: "{Item}($work)",
        chain: "",
    },
    interpolates_parameters: false,
    known_misses: &[(
        "through-helper",
        "the runtime is written on the function the helper is called from, which the rule does not follow",
    )],
    semantic: &[
        super::Semantic {
            description: "a default #[tokio::test], which runs on a current-thread runtime",
            source: "#[tokio::test]\nasync fn reads() { let _ = tokio::task::block_in_place(|| 1); }",
            fires: true,
            carve_out: "",
        },
        super::Semantic {
            description: "a multi-thread #[tokio::test]",
            source: "#[tokio::test(flavor = \"multi_thread\")]\nasync fn reads() { let _ = tokio::task::block_in_place(|| 1); }",
            fires: false,
            carve_out: "a multi-thread runtime hands the executor to another worker",
        },
        super::Semantic {
            description: "a default #[tokio::main], which is multi-thread",
            source: "#[tokio::main]\nasync fn main() { let _ = tokio::task::block_in_place(|| 1); }",
            fires: false,
            carve_out: "#[tokio::main] builds a multi-thread runtime unless told otherwise",
        },
        super::Semantic {
            description: "a call outside any runtime attribute",
            source: "fn compute() -> u8 { tokio::task::block_in_place(|| 1) }",
            fires: false,
            carve_out: "outside a runtime tokio runs the closure inline",
        },
    ],
};

#[cfg(test)]
pub(in crate::source_kernel) static BLOCK_ON_VARIANTS: super::Variants = super::Variants {
    rule: SOURCE_BLOCK_ON_IN_ASYNC.id,
    positive: super::Case {
        attributes: "",
        asynchronous: true,
        import: "futures::executor::block_on",
        parameters: "$value: u8",
        setup: "",
        receiver: "{Item}(async move { $value })",
        chain: "",
    },
    negative: super::Case {
        attributes: "",
        asynchronous: false,
        import: "futures::executor::block_on",
        parameters: "$value: u8",
        setup: "",
        receiver: "{Item}(async move { $value })",
        chain: "",
    },
    interpolates_parameters: false,
    known_misses: &[(
        "through-helper",
        "a synchronous helper is not polled by an executor, and the rule does not follow the async caller into it",
    )],
    semantic: &[
        super::Semantic {
            description: "a runtime built and blocked on inside an async fn",
            source: "async fn handler() { let runtime = tokio::runtime::Runtime::new().unwrap(); runtime.block_on(async {}); }",
            fires: true,
            carve_out: "",
        },
        super::Semantic {
            description: "the current handle blocked on inside an async block",
            source: "fn start() { let _ = async { tokio::runtime::Handle::current().block_on(async {}) }; }",
            fires: true,
            carve_out: "",
        },
        super::Semantic {
            description: "a runtime built and blocked on in main",
            source: "fn main() { let runtime = tokio::runtime::Builder::new_multi_thread().build().unwrap(); runtime.block_on(async {}); }",
            fires: false,
            carve_out: "a synchronous entry point is where a runtime is meant to be blocked on",
        },
        super::Semantic {
            description: "a block_on of another type inside async code",
            source: "struct Pool;\nimpl Pool { fn block_on(&self) {} }\nasync fn handler(pool: Pool) { pool.block_on(); }",
            fires: false,
            carve_out: "the receiver is no tokio runtime or handle",
        },
    ],
};

const BLOCK_IN_PLACE_MESSAGE: &str =
    "tokio::task::block_in_place panics on the current-thread runtime this function runs on.";
const BLOCK_ON_MESSAGE: &str =
    "block_on inside async code panics under tokio and deadlocks the futures executor.";

/// The calls that build or reach a tokio runtime a `block_on` can be called on.
const RUNTIME_ROOTS: [&[&str]; 4] = [
    &["tokio", "runtime", "Runtime", "new"],
    &["tokio", "runtime", "Builder", "new_current_thread"],
    &["tokio", "runtime", "Builder", "new_multi_thread"],
    &["tokio", "runtime", "Handle", "current"],
];

fn block_in_place(context: &Context<'_>, node: &SyntaxNode) -> Option<Detection> {
    let call = ast::CallExpr::cast(node.clone())?;
    let ast::Expr::PathExpr(callee) = call.expr()? else {
        return None;
    };
    let resolved = context.item_path(&callee.path()?)?;
    if resolved != ["tokio", "task", "block_in_place"] || !current_thread(node) {
        return None;
    }
    if intersects_errors(node.text_range(), context.error_ranges) {
        return None;
    }
    Some(Detection {
        message: BLOCK_IN_PLACE_MESSAGE.into(),
        range: node.text_range(),
    })
}

/// Does the function holding `node` run on a current-thread runtime, as its
/// own attribute writes it? `#[tokio::test]` is current-thread unless its
/// flavor says otherwise, `#[tokio::main]` only when its flavor says so.
pub(in crate::source_kernel) fn current_thread(node: &SyntaxNode) -> bool {
    let Some(function) = node.ancestors().find_map(ast::Fn::cast) else {
        return false;
    };
    function.attrs().any(|attribute| {
        let written = compact(attribute.syntax());
        let multi = written.contains("flavor=\"multi_thread\"");
        let current = written.contains("flavor=\"current_thread\"");
        (written.starts_with("#[tokio::test") && !multi)
            || (written.starts_with("#[tokio::main") && current)
    })
}

fn block_on_call(context: &Context<'_>, node: &SyntaxNode) -> Option<Detection> {
    let call = ast::CallExpr::cast(node.clone())?;
    let ast::Expr::PathExpr(callee) = call.expr()? else {
        return None;
    };
    let resolved = context.item_path(&callee.path()?)?;
    (resolved == ["futures", "executor", "block_on"]).then_some(())?;
    in_async(context, node)
}

fn block_on_method(context: &Context<'_>, node: &SyntaxNode) -> Option<Detection> {
    let call = ast::MethodCallExpr::cast(node.clone())?;
    (call.name_ref()?.text() == "block_on").then_some(())?;
    let ast::Expr::CallExpr(root) = context.chain_root(call.receiver()?)? else {
        return None;
    };
    let ast::Expr::PathExpr(callee) = root.expr()? else {
        return None;
    };
    let resolved = context.item_path(&callee.path()?)?;
    RUNTIME_ROOTS
        .iter()
        .any(|path| path.iter().copied().eq(resolved.iter().map(String::as_str)))
        .then_some(())?;
    in_async(context, node)
}

fn in_async(context: &Context<'_>, node: &SyntaxNode) -> Option<Detection> {
    if !in_async_code(node) || intersects_errors(node.text_range(), context.error_ranges) {
        return None;
    }
    Some(Detection {
        message: BLOCK_ON_MESSAGE.into(),
        range: node.text_range(),
    })
}
