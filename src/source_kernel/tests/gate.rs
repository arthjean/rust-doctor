//! What a `#[cfg(test)]` module declaration says about the file it names.
//!
//! An out-of-line test module is no Cargo target and sits wherever its
//! declaration points, so Cargo's target kind cannot see it and a path
//! convention only guesses at it. The gate the declaration carries is the one
//! authority, and the whole subtree below a gated declaration is compiled under
//! `cfg(test)` with it. The `test-gate` fixture holds one file per form the
//! grammar has to tell apart, so the grammar is asserted where it is read,
//! through the walk, rather than against a parser in isolation.

use crate::report::DiagnosticContext;
use crate::source_kernel::walk::{enumerate, enumerate_with_limits};
use crate::source_kernel::{Enumeration, LIMITS, Limit, Limits};

use super::metadata;

/// Context the walk publishes for one file of the fixture, by its
/// workspace-relative path.
fn context_of(enumeration: &Enumeration, relative_path: &str) -> Option<DiagnosticContext> {
    let unit = enumeration
        .units()
        .find(|unit| unit.relative_path() == relative_path);
    assert!(unit.is_some(), "the walk never reached \"{relative_path}\"");
    unit.and_then(|unit| unit.context(enumeration.contexts()))
}

/// `#[cfg(test)] mod tests;` resolving to `src/tests/mod.rs` gates that file,
/// and the declaration `src/tests/mod.rs` makes in turn inherits the gate: the
/// module is compiled under `cfg(test)`, and so is everything below it.
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
    // The other spelling of the same declaration, and one more level below it.
    assert_eq!(
        context_of(&enumeration, "src/feature/tests.rs"),
        Some(DiagnosticContext::Tests)
    );
    assert_eq!(
        context_of(&enumeration, "src/feature/tests/nested.rs"),
        Some(DiagnosticContext::Tests)
    );
}

/// The grammar, one file per form. `test` is a gate as the bare predicate and
/// inside an `all(...)`, where every arm has to hold. Nothing else is: a
/// negation is the opposite claim, an `any(...)` leaves the module compiled
/// outside a test build whenever another arm holds, and a feature is a string
/// however it is named. The last form, a key spelled `test = "..."`, is asserted
/// beside the reader in `walk`: a `cfg` the compiler has no value for warns, and
/// the fixture would publish that warning uncatalogued.
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

/// `src/shared.rs` is declared ungated by the crate root and gated from
/// `src/tests/mod.rs` through a `#[path]` attribute. The gate travels with the
/// file the declaration resolved to, so the two traversals reach the same unit
/// and disagree, and a unit whose reachers disagree abstains. Silencing shipped
/// code is the expensive mistake, so the abstention keeps the file weighing.
#[test]
fn a_file_reached_gated_and_ungated_abstains() {
    let enumeration = enumerate(&metadata("test-gate"));
    let shared = enumeration
        .units()
        .find(|unit| unit.relative_path() == "src/shared.rs")
        .expect("the walk reaches src/shared.rs");

    // The disagreement is what abstains, so it is what the assertion names: the
    // gate is carried in the set the unit accumulates, and a second reach adds
    // an entry rather than overwriting the first. Stamped on the unit, the
    // ungated reach would have erased the gated one, or the other way round,
    // and one of the two answers would have been silently lost.
    let gates: Vec<bool> = shared
        .reachability
        .iter()
        .map(|reach| reach.test_gated)
        .collect();
    assert_eq!(gates, vec![false, true]);
    assert_eq!(context_of(&enumeration, "src/shared.rs"), None);
}

/// Cargo's target kind answers before the declaration does. `benches/reach.rs`
/// is a bench target that gates its own module, and the file it names is bench
/// material, not test material: a context the manifest declares is not
/// something a declaration inside the file can restate.
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

/// `benches/dual.rs` is declared by the bench target and, through a `#[path]`
/// attribute, by a file sitting under a `#[cfg(test)]` gate. One reach calls it
/// bench material and the other test material, and two different non-production
/// contexts are no more unanimous than a non-production one and a shipped one.
#[test]
fn a_bench_reach_and_a_gated_reach_abstain_rather_than_arbitrate() {
    let enumeration = enumerate(&metadata("test-gate"));

    assert_eq!(context_of(&enumeration, "benches/dual.rs"), None);
}

/// A walk that stops at its module depth reports that bound and nothing else.
/// The files below it never became units, so no context was ever asked of them:
/// a refusal is a budget on work, never a classification.
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
