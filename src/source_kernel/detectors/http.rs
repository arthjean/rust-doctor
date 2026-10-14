//! A reqwest response body read before its status is checked.
//!
//! reqwest resolves a request to a `Response` whatever the status, so a 4xx or
//! 5xx error payload reaches `.json()` and is parsed as data. The rule follows
//! the chain the body read sits on, through `?`, `.await` and the bindings it
//! was stored in, down to a reqwest constructor: `reqwest::get`, or a client
//! from `Client::new` or `Client::builder` whose request was sent. A link that
//! checks the status, `error_for_status` or `error_for_status_ref`, silences
//! it. A response stored in a `let` silences it when the body reads `status`,
//! `is_success` or `error_for_status` on that binding before the body, or
//! hands the binding to another function, which may check it. A client whose
//! origin the file does not show, a parameter or a field, abstains.

use ra_ap_syntax::ast::{self, HasArgList};
use ra_ap_syntax::{AstNode, SyntaxKind, SyntaxNode};

use super::bindings::{Binding, identifier};
use super::{CHAIN_DEPTH, Context, Detection, Detector, in_test_code, strip_wrappers};
use crate::policy::SOURCE_UNCHECKED_HTTP_STATUS;
use crate::source_text::intersects_errors;

pub(in crate::source_kernel) static UNCHECKED_STATUS: Detector = Detector {
    definition: &SOURCE_UNCHECKED_HTTP_STATUS,
    node: SyntaxKind::METHOD_CALL_EXPR,
    inspect: unchecked_status,
    crates: &["reqwest"],
};

/// The `Response` methods that consume the body.
const BODY_READS: [&str; 5] = ["bytes", "bytes_stream", "chunk", "json", "text"];

/// The links that turn an error status into an error.
const STATUS_CHECKS: [&str; 2] = ["error_for_status", "error_for_status_ref"];

/// What a binding holding the response may be asked before its body is read.
const STATUS_READS: [&str; 4] = ["error_for_status", "error_for_status_ref", "is_success", "status"];

/// The reqwest calls a chain may start from, and whether the call already
/// yields a response (`get`) or a client whose request still has to be sent.
const ROOTS: [(&[&str], bool); 6] = [
    (&["reqwest", "get"], true),
    (&["reqwest", "blocking", "get"], true),
    (&["reqwest", "Client", "new"], false),
    (&["reqwest", "Client", "builder"], false),
    (&["reqwest", "blocking", "Client", "new"], false),
    (&["reqwest", "blocking", "Client", "builder"], false),
];

#[cfg(test)]
pub(in crate::source_kernel) static VARIANTS: super::Variants = super::Variants {
    rule: SOURCE_UNCHECKED_HTTP_STATUS.id,
    positive: super::Case {
        attributes: "",
        asynchronous: true,
        import: "reqwest::Client",
        parameters: "$url: &str",
        setup: "",
        receiver: "{Item}::new().get($url).send().await?",
        chain: ".text()",
    },
    negative: super::Case {
        attributes: "",
        asynchronous: true,
        import: "reqwest::Client",
        parameters: "$url: &str",
        setup: "",
        receiver: "{Item}::new().get($url).send().await?",
        chain: ".error_for_status()?.text()",
    },
    interpolates_parameters: false,
    known_misses: &[],
    semantic: &[
        super::Semantic {
            description: "a body read from reqwest::get",
            source: "async fn fetch(url: &str) -> Result<u8, reqwest::Error> { reqwest::get(url).await?.json::<u8>().await }",
            fires: true,
            carve_out: "",
        },
        super::Semantic {
            description: "a client stored in a let, its typed body read",
            source: "async fn fetch(url: &str) -> Result<u8, reqwest::Error> { let client = reqwest::Client::new(); client.get(url).send().await?.json::<u8>().await }",
            fires: true,
            carve_out: "",
        },
        super::Semantic {
            description: "a response whose status is read before its body",
            source: "async fn fetch(url: &str) -> Option<String> { let response = reqwest::Client::new().get(url).send().await.ok()?; if !response.status().is_success() { return None; } response.text().await.ok() }",
            fires: false,
            carve_out: "the status is read on the binding before the body",
        },
        super::Semantic {
            description: "a response handed to another function first",
            source: "fn log(_: &reqwest::Response) {}\nasync fn fetch(url: &str) -> Option<String> { let response = reqwest::Client::new().get(url).send().await.ok()?; log(&response); response.text().await.ok() }",
            fires: false,
            carve_out: "the function the response is handed to may check its status",
        },
        super::Semantic {
            description: "a JSON request body set before sending",
            source: "async fn post(url: &str) { let _ = reqwest::Client::new().post(url).json(&1).send().await; }",
            fires: false,
            carve_out: "json with an argument sets the request body, and reads nothing",
        },
        super::Semantic {
            description: "a body read in a test",
            source: "#[test]\nfn reads() { let _ = reqwest::blocking::get(\"http://localhost\").unwrap().text(); }",
            fires: false,
            carve_out: "a test asserts on the payload it reads, whatever the status",
        },
    ],
};

fn unchecked_status(context: &Context<'_>, node: &SyntaxNode) -> Option<Detection> {
    let call = ast::MethodCallExpr::cast(node.clone())?;
    let name = call.name_ref()?;
    if !BODY_READS.contains(&name.text().as_str()) || call.arg_list()?.args().next().is_some() {
        return None;
    }
    let (root, sent) = response_root(context, call.receiver()?, node)?;
    (reqwest_response(context, &root, sent)
        && !intersects_errors(node.text_range(), context.error_ranges)
        && !in_test_code(context, node))
    .then_some(())?;
    Some(Detection {
        message: "The response body is read without checking the HTTP status.".into(),
        range: name.syntax().text_range(),
    })
}

/// The call the response's chain starts from, and whether a link of it sent
/// the request, followed through `?`, `.await` and the bindings the response
/// was stored in. `None` when a link checks the status, or a binding abstains.
fn response_root(context: &Context<'_>, receiver: ast::Expr, read: &SyntaxNode) -> Option<(ast::CallExpr, bool)> {
    let mut expression = receiver;
    let mut sent = false;
    for _ in 0..CHAIN_DEPTH {
        expression = match strip_wrappers(expression) {
            ast::Expr::MethodCallExpr(link) => {
                sent |= unchecked_link(&link)?;
                link.receiver()?
            }
            ast::Expr::TryExpr(tried) => tried.expr()?,
            ast::Expr::AwaitExpr(awaited) => awaited.expr()?,
            ast::Expr::CallExpr(root) => return Some((root, sent)),
            bound => unchecked_binding(context, &bound, read)?,
        };
    }
    None
}

/// `None` for a link that checks the status, else whether it sends the
/// request.
fn unchecked_link(link: &ast::MethodCallExpr) -> Option<bool> {
    let name = link.name_ref()?;
    (!STATUS_CHECKS.contains(&name.text().as_str())).then(|| name.text() == "send")
}

/// Does this call start a chain that yields a reqwest response: `get`, or a
/// client whose request was sent?
fn reqwest_response(context: &Context<'_>, root: &ast::CallExpr, sent: bool) -> bool {
    let Some(ast::Expr::PathExpr(callee)) = root.expr() else {
        return false;
    };
    let Some(resolved) = callee.path().and_then(|path| context.item_path(&path)) else {
        return false;
    };
    ROOTS.iter().any(|(path, response)| {
        (*response || sent) && path.iter().copied().eq(resolved.iter().map(String::as_str))
    })
}

/// The value a response binding was initialized to, unless the body checks
/// its status first, hands it over, or the binding cannot be read.
fn unchecked_binding(context: &Context<'_>, bound: &ast::Expr, read: &SyntaxNode) -> Option<ast::Expr> {
    let binding = identifier(bound)?;
    if checked_or_handed_over(&binding, bound, read) {
        return None;
    }
    match context.bindings.binding(context.aliases, bound.syntax(), &binding) {
        Binding::Value(value) => Some(value),
        Binding::Typed(_) | Binding::Indeterminate | Binding::Unbound => None,
    }
}

/// Does the body around `read` read the status of `binding` before `read`,
/// or hand `binding` to a function? A use of the name anywhere in the body
/// counts, shadowing aside, which only ever makes the rule abstain.
fn checked_or_handed_over(binding: &str, used: &ast::Expr, read: &SyntaxNode) -> bool {
    let Some(body) = used
        .syntax()
        .ancestors()
        .find(|ancestor| ast::Fn::can_cast(ancestor.kind()) || ast::ClosureExpr::can_cast(ancestor.kind()))
    else {
        return false;
    };
    body.descendants().filter_map(ast::PathExpr::cast).any(|path| {
        if identifier(&ast::Expr::PathExpr(path.clone())).as_deref() != Some(binding) {
            return false;
        }
        let mut site = path.syntax().clone();
        while let Some(parent) = site.parent()
            && ast::RefExpr::can_cast(parent.kind())
        {
            site = parent;
        }
        let Some(parent) = site.parent() else {
            return false;
        };
        if ast::ArgList::can_cast(parent.kind()) {
            return true;
        }
        ast::MethodCallExpr::cast(parent).is_some_and(|method| {
            method.receiver().is_some_and(|receiver| receiver.syntax() == &site)
                && method
                    .name_ref()
                    .is_some_and(|name| STATUS_READS.contains(&name.text().as_str()))
                && site.text_range().start() < read.text_range().start()
        })
    })
}
