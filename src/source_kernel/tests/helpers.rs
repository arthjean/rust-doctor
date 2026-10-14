//! US-017: a blocking call one synchronous helper away from async code.
//!
//! The helper is followed only when the call names exactly one `fn` of the
//! file, and the finding sits on the call in async code with a message naming
//! the helper and what it reaches.

use super::{REGISTERED, synthetic_enumeration};
use crate::source_kernel::{Findings, LIMITS, Registry, analyze_unit};

/// Every finding of the blocking rule in `source`, as (line, message).
fn blocking(source: &str) -> Vec<(usize, String)> {
    let enumeration = synthetic_enumeration(source);
    let registry = Registry::new(
        REGISTERED
            .iter()
            .copied()
            .filter(|detector| detector.definition.id == crate::policy::SOURCE_BLOCKING_IN_ASYNC.id),
    );
    let mut findings = Findings::default();
    for unit in enumeration.units.values() {
        analyze_unit(unit, &enumeration, &registry, LIMITS, &mut findings);
    }
    findings
        .candidates
        .into_values()
        .map(|candidate| (candidate.span.line_start, candidate.message.into_owned()))
        .collect()
}

#[test]
fn a_helper_is_reported_at_the_async_call_naming_what_it_reaches() {
    let found = blocking(
        "fn load_config() -> String {\n    std::fs::read_to_string(\"app.toml\").unwrap_or_default()\n}\n\
         async fn handler() {\n    let _ = load_config();\n}\n",
    );
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].0, 5);
    assert!(found[0].1.contains("`load_config`"), "{}", found[0].1);
    assert!(found[0].1.contains("`std::fs::read_to_string`"), "{}", found[0].1);
}

#[test]
fn helpers_are_followed_four_deep_through_self_and_methods() {
    let chain = "fn a() { b(); }\nfn b() { c(); }\nfn c() { d(); }\nfn d() { std::thread::sleep(std::time::Duration::ZERO); }\n";
    assert_eq!(blocking(&format!("{chain}async fn run() {{ a(); }}\n")).len(), 1);
    // A fifth helper is past the bound.
    assert!(blocking(&format!("{chain}fn e() {{ a(); }}\nasync fn run() {{ e(); }}\n")).is_empty());

    let methods = "struct Store;\nimpl Store {\n    fn read(&self) -> Vec<u8> { std::fs::read(\"db\").unwrap_or_default() }\n    \
                   fn load() -> Vec<u8> { std::fs::read(\"db\").unwrap_or_default() }\n    \
                   async fn get(&self) { let _ = self.read(); let _ = Self::load(); }\n}\n";
    assert_eq!(blocking(methods).len(), 2);
}

#[test]
fn an_ambiguous_recursive_async_or_shadowed_helper_is_abstained_on() {
    let body = "std::fs::read(\"x\").unwrap_or_default()";
    // Two `fn` items of the name, which a `cfg` pair produces.
    assert!(blocking(&format!(
        "#[cfg(unix)]\nfn load() -> Vec<u8> {{ {body} }}\n#[cfg(not(unix))]\nfn load() -> Vec<u8> {{ {body} }}\nasync fn run() {{ load(); }}\n"
    ))
    .is_empty());
    // Mutual recursion terminates without a finding.
    assert!(blocking("fn ping() { pong(); }\nfn pong() { ping(); }\nasync fn run() { ping(); }\n").is_empty());
    // An async helper is async code of its own, reported where it blocks.
    assert_eq!(
        blocking(&format!("async fn load() -> Vec<u8> {{ {body} }}\nasync fn run() {{ load().await; }}\n")).len(),
        1
    );
    // A local binding of the name is not the `fn`.
    assert!(blocking(&format!(
        "fn load() -> Vec<u8> {{ {body} }}\nasync fn run() {{ let load = || Vec::<u8>::new(); load(); }}\n"
    ))
    .is_empty());
    // A glob of a nearer scope may bind the name before the item does; one of
    // the item's own scope may not.
    assert!(blocking(&format!(
        "fn load() -> Vec<u8> {{ {body} }}\nasync fn run() {{ use http_client::*; load(); }}\n"
    ))
    .is_empty());
    assert_eq!(
        blocking(&format!("use http_client::*;\nfn load() -> Vec<u8> {{ {body} }}\nasync fn run() {{ load(); }}\n")).len(),
        1
    );
    // An imported name is not a local `fn`.
    assert!(blocking(&format!(
        "use other::load;\nfn load_local() -> Vec<u8> {{ {body} }}\nasync fn run() {{ load(); }}\n"
    ))
    .is_empty());
    // Work a helper hands off the executor, or only builds as a future, does
    // not run when it does: the future's own read is reported inside it, on
    // line 2, and the calls on line 3 are not.
    let found = blocking(&format!(
        "fn offload() {{ let _ = tokio::task::spawn_blocking(|| {body}); }}\nfn later() {{ let _ = async {{ {body} }}; }}\nasync fn run() {{ offload(); later(); }}\n"
    ));
    assert_eq!(found.iter().map(|(line, _)| *line).collect::<Vec<_>>(), [2], "{found:?}");
}
