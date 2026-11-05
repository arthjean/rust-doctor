use std::time::{Duration, Instant};

use cargo_metadata::MetadataCommand;

use crate::policy::PolicyPlan;
use crate::source_kernel::{enumerate, inspect};
use crate::test_clock::machine_allowance;

const PASSES: u32 = if cfg!(debug_assertions) { 2 } else { 20 };

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

    let recorded = AFTER_MICROSECONDS * u128::from(machine_allowance());
    let bound = if cfg!(debug_assertions) { recorded * 3 } else { recorded * 5 / 4 };
    assert!(
        best.as_micros() <= bound,
        "the detector pass took {best:?}, over its {bound} us bound (recorded {recorded} us)"
    );
}
