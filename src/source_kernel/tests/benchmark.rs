//! What resolving local bindings costs the detector pass, on this repository.
//!
//! Measured the way `src/structure/benchmark.rs` measures the structural pass:
//! the walk is taken once, the pass alone is timed, the best sample is kept,
//! and the clock is compared to what the development machine recorded. The
//! workload is this repository rather than a generated one, because the
//! question is what the pass costs on real code, where a binding the detectors
//! ask about is the exception and not the rule.

use std::time::{Duration, Instant};

use cargo_metadata::MetadataCommand;

use crate::policy::PolicyPlan;
use crate::source_kernel::{enumerate, inspect};
use crate::test_clock::machine_allowance;

/// Passes per sample: one optimized pass over this repository is a few
/// milliseconds, too close to a scheduler slice to time alone. An
/// unoptimized pass is thirteen times longer and needs no repetition.
const PASSES: u32 = if cfg!(debug_assertions) { 2 } else { 20 };

/// What one pass cost on the development machine on 2026-09-26, in
/// microseconds: the median of seven samples before binding resolution
/// existed (commit 9b8c0ab, a worktree of which was measured with this very
/// test), and of three with it (US-028). The story allows the second a tenth
/// more than the first; it measured 4.6 % optimized and 1.8 % unoptimized,
/// against a run-to-run spread of about 4 % on the same machine.
const BEFORE_MICROSECONDS: u128 = if cfg!(debug_assertions) { 360_500 } else { 27_100 };
const AFTER_MICROSECONDS: u128 = if cfg!(debug_assertions) { 367_000 } else { 28_300 };

const _: () = assert!(AFTER_MICROSECONDS * 10 <= BEFORE_MICROSECONDS * 11);

#[test]
fn the_detector_pass_holds_its_recorded_cost_on_this_repository() {
    let metadata = MetadataCommand::new()
        .manifest_path(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
        .no_deps()
        .other_options(["--offline".to_owned()])
        .exec()
        .expect("this repository's metadata should load");
    let enumeration = enumerate(&metadata);
    let plan = PolicyPlan::default();

    let mut best = Duration::MAX;
    for _ in 0..if cfg!(debug_assertions) { 3 } else { 7 } {
        let started = Instant::now();
        for _ in 0..PASSES {
            let scan = inspect(&enumeration, &plan);
            assert!(scan.counters.nodes_visited > 0);
        }
        best = best.min(started.elapsed() / PASSES);
    }
    eprintln!("detector pass on this repository: {} us", best.as_micros());

    // The recorded pair above is the story's proof, taken on an idle
    // machine. What runs here is a tripwire against a regression: an
    // optimized run is held to a quarter over the record, and an unoptimized
    // one, which `cargo test` runs beside the whole suite, to three times it,
    // the load of that suite having measured at twice the idle figure.
    let recorded = AFTER_MICROSECONDS * u128::from(machine_allowance());
    let bound = if cfg!(debug_assertions) { recorded * 3 } else { recorded * 5 / 4 };
    assert!(
        best.as_micros() <= bound,
        "the detector pass took {best:?}, over its {bound} us bound (recorded {recorded} us)"
    );
}
