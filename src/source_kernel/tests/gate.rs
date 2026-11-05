use crate::report::DiagnosticContext;
use crate::source_kernel::walk::{enumerate, enumerate_with_limits};
use crate::source_kernel::{Enumeration, LIMITS, Limit, Limits};

use super::metadata;

fn context_of(enumeration: &Enumeration, relative_path: &str) -> Option<DiagnosticContext> {
    let unit = enumeration
        .units()
        .find(|unit| unit.relative_path() == relative_path);
    assert!(unit.is_some(), "the walk never reached \"{relative_path}\"");
    unit.and_then(|unit| unit.context(enumeration.contexts()))
}

#[test]
fn a_gated_declaration_marks_the_file_it_names_and_everything_it_declares() {
    let enumeration = enumerate(&metadata("test-gate"));

    assert_eq!(
        context_of(&enumeration, "src/tests/mod.rs"),
        Some(DiagnosticContext::Tests)
    );
    assert_eq!(
        context_of(&enumeration, "src/tests/helpers.rs"),
        Some(DiagnosticContext::Tests)
    );
    assert_eq!(
        context_of(&enumeration, "src/feature/tests.rs"),
        Some(DiagnosticContext::Tests)
    );
    assert_eq!(
        context_of(&enumeration, "src/feature/tests/nested.rs"),
        Some(DiagnosticContext::Tests)
    );
}

#[test]
fn only_the_bare_test_predicate_and_an_all_that_carries_it_are_gates() {
    let enumeration = enumerate(&metadata("test-gate"));

    assert_eq!(
        context_of(&enumeration, "src/feature/strict.rs"),
        Some(DiagnosticContext::Tests),
        "every arm of an `all` has to hold, so `all(test, ...)` is a gate"
    );
    for shipped in [
        "src/feature/production.rs",
        "src/feature/util.rs",
        "src/feature/loose.rs",
    ] {
        assert_eq!(
            context_of(&enumeration, shipped),
            None,
            "\"{shipped}\" is shipped code and has to keep weighing on the score"
        );
    }
}

#[test]
fn a_file_reached_gated_and_ungated_abstains() {
    let enumeration = enumerate(&metadata("test-gate"));
    let shared = enumeration
        .units()
        .find(|unit| unit.relative_path() == "src/shared.rs")
        .expect("the walk reaches src/shared.rs");

    let gates: Vec<bool> = shared
        .reachability
        .iter()
        .map(|reach| reach.test_gated)
        .collect();
    assert_eq!(gates, vec![false, true]);
    assert_eq!(context_of(&enumeration, "src/shared.rs"), None);
}

#[test]
fn a_target_cargo_already_names_keeps_its_own_context() {
    let enumeration = enumerate(&metadata("test-gate"));

    assert_eq!(
        context_of(&enumeration, "benches/reach.rs"),
        Some(DiagnosticContext::Benchmark)
    );
    assert_eq!(
        context_of(&enumeration, "benches/tests.rs"),
        Some(DiagnosticContext::Benchmark)
    );
}

#[test]
fn a_bench_reach_and_a_gated_reach_abstain_rather_than_arbitrate() {
    let enumeration = enumerate(&metadata("test-gate"));

    assert_eq!(context_of(&enumeration, "benches/dual.rs"), None);
}

#[test]
fn a_refused_depth_names_the_bound_rather_than_a_context() {
    let enumeration = enumerate_with_limits(
        &metadata("test-gate"),
        Limits {
            module_depth: 1,
            ..LIMITS
        },
    );

    assert!(
        enumeration.errors.iter().any(|error| {
            error.code == "limit-exceeded" && error.message.contains(Limit::ModuleDepth.name())
        }),
        "the walk names the bound it stopped at"
    );
    assert!(
        enumeration
            .units()
            .all(|unit| unit.relative_path() != "src/tests/helpers.rs"),
        "a file the walk never reached carries no context at all"
    );
}
