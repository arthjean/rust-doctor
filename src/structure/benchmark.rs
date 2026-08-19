//! US-009: what the pass is allowed to cost on a workspace of a thousand files.
//!
//! The benchmark is generated rather than committed: ten thousand functions
//! are a megabyte of source that would say nothing to a reader of this
//! repository, and generating them keeps the shape of the workload written
//! down instead of frozen.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use cargo_metadata::MetadataCommand;

use super::*;
use crate::source_kernel::enumerate;
// The machine allowance touches the two clocks below only. The counter
// assertions beside them, which are what prove the scoring is nominated
// rather than pairwise, hold on any machine and are never relaxed.
use crate::test_clock::machine_allowance;

const FILES: usize = 1_000;
const FUNCTIONS_PER_FILE: usize = 10;

/// Absolute bound the NFR sets on this workload, for the binary that ships.
const BUDGET: Duration = Duration::from_millis(2_000);

/// A test binary is unoptimized, and the pass runs about thirteen times
/// slower there: 21.1 s against 1.63 s, measured on 2026-08-08 on the same
/// workspace. The NFR is written for what ships, so an unoptimized run is
/// allowed a multiple of every bound the pass appears in, rather than held
/// to one the profile makes unreachable.
///
/// The share below needs it most: its numerator is unoptimized Rust while
/// its denominator is a Clippy subprocess running at full speed whatever
/// the profile, so a debug run reads the share thirteen times too high.
const DEBUG_ALLOWANCE: u32 = 14;

/// What the pass measured here on 2026-08-08, in milliseconds, on the
/// development machine. The assertion below allows a quarter more than
/// this: a change costing more than that is a regression to explain, not a
/// number to raise quietly.
const RECORDED_MILLISECONDS: u128 = if cfg!(debug_assertions) { 21_100 } else { 1_650 };

/// Share of a whole scan the structural pass may take.
const SHARE_PERCENT: u128 = 15;

/// Peak the pass may hold for the functions it kept.
const MEMORY_LIMIT_BYTES: usize = 200 * 1024 * 1024;

const OPERATORS: [&str; 6] = ["+", "-", "*", "|", "&", "^"];
const LITERALS: [&str; 6] = ["1", "2.5", "\"text\"", "true", "'c'", "7u64"];

/// Multiple of every bound this profile is allowed.
const fn allowance() -> u32 {
    if cfg!(debug_assertions) {
        DEBUG_ALLOWANCE
    } else {
        1
    }
}

/// One statement of a generated body.
///
/// The canonical form keeps control flow, operators and literal types, so
/// these are what the draw varies. Eight kinds over six operators and six
/// literal types is a space wide enough that two bodies of a dozen
/// statements rarely land close, which is the property the workload needs:
/// real code is mostly shapes that are all different, with a few families
/// inside it.
fn statement(kind: u64, operator: &str, literal: &str, binding: usize) -> String {
    match kind % 8 {
        0 => format!("    for value in values {{ total = total {operator} *value; }}\n"),
        1 => format!(
            "    if total > limit {{ total = total {operator} limit; }} else {{ total = total {operator} 1; }}\n"
        ),
        2 => format!("    while total < limit {{ total = total {operator} 2; }}\n"),
        3 => format!(
            "    match total {{ 0 => total = total {operator} 3, 1 => total = limit, _ => total = total {operator} limit }};\n"
        ),
        4 => format!(
            "    let hold{binding} = values.iter().copied().filter(|v| *v {operator} 1 > limit).count() as u32; total = total {operator} hold{binding};\n"
        ),
        5 => format!(
            "    if let Some(v) = values.first() {{ let hold{binding} = *v {operator} limit; total = total {operator} hold{binding}; }}\n"
        ),
        6 => format!(
            "    let hold{binding} = ({literal}, total {operator} limit); total = total {operator} hold{binding}.1;\n"
        ),
        _ => format!(
            "    let hold{binding} = |x: u32| x {operator} limit; total = hold{binding}(total);\n"
        ),
    }
}

/// One function, drawn from a seeded sequence.
///
/// What this workload has to present is ten thousand *shapes*, not ten
/// thousand functions: a canonical form is what the pass compares, so a
/// generator producing five forms ten thousand times collapses to five
/// before anything expensive runs, and the scoring loop the budget is
/// written against never sees the workload the assertion claims. It must
/// also keep those shapes apart: a workspace where every function is a
/// near duplicate of every other has a quadratic answer, not a quadratic
/// pass, and measures nothing either. Every hundredth function repeats the
/// shape of the one before it, which plants the families the pass must
/// still find among shapes that are otherwise all different.
fn source(position: usize) -> String {
    let seed = if position % 100 == 99 {
        position.saturating_sub(1)
    } else {
        position
    };
    let mut state = (seed as u64).wrapping_add(1).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    let mut draw = move || {
        state ^= state >> 30;
        state = state.wrapping_mul(0xbf58_476d_1ce4_e5b9);
        state ^= state >> 27;
        state
    };
    let arity = 1 + draw() as usize % 3;
    let statements = 5 + draw() as usize % 11;
    let mut body = format!(
        "    let mut total = 0;\n    let limit = {};\n",
        (0..arity)
            .map(|index| format!("limit{index}"))
            .collect::<Vec<_>>()
            .join(" + ")
    );
    for binding in 0..statements {
        let drawn = draw();
        body.push_str(&statement(
            drawn,
            OPERATORS[(drawn as usize >> 3) % OPERATORS.len()],
            LITERALS[(drawn as usize >> 6) % LITERALS.len()],
            binding,
        ));
    }
    let parameters = (0..arity)
        .map(|index| format!("limit{index}: u32"))
        .fold("values: &[u32]".to_owned(), |left, right| {
            format!("{left}, {right}")
        });
    format!("pub fn shape_{position}({parameters}) -> u32 {{\n{body}    total\n}}\n")
}

/// The generated workload lives outside this repository. Under `target/` it
/// inherits whatever that path is on the machine, and where `target` is a
/// symlink to a build directory elsewhere the units resolve outside the
/// workspace root cargo reports: the walk then reads nothing and the
/// benchmark measures an empty pass.
fn workspace() -> PathBuf {
    let root = std::env::temp_dir()
        .canonicalize()
        .expect("a canonical temporary directory")
        .join("rust-doctor-structure-benchmark")
        .join(std::process::id().to_string());
    let sources = root.join("src");
    fs::create_dir_all(&sources).expect("the benchmark workspace should be writable");
    let mut declarations = String::new();
    for file in 0..FILES {
        let mut unit = String::new();
        for index in 0..FUNCTIONS_PER_FILE {
            unit.push_str(&source(file * FUNCTIONS_PER_FILE + index));
            unit.push('\n');
        }
        fs::write(sources.join(format!("m{file}.rs")), unit).expect("a module should write");
        declarations.push_str(&format!("pub mod m{file};\n"));
    }
    fs::write(sources.join("lib.rs"), declarations).expect("the root should write");
    fs::write(
        root.join("Cargo.toml"),
        concat!(
            "[package]\n",
            "name = \"structure-benchmark\"\n",
            "version = \"0.1.0\"\n",
            "edition = \"2024\"\n",
            "publish = false\n\n",
            "[lib]\n",
            "path = \"src/lib.rs\"\n",
        ),
    )
    .expect("the manifest should write");
    root
}

#[test]
fn the_pass_holds_its_budget_on_a_thousand_files() {
    let root = workspace();
    let metadata = MetadataCommand::new()
        .manifest_path(root.join("Cargo.toml"))
        .no_deps()
        .other_options(["--offline".to_owned()])
        .exec()
        .expect("the benchmark metadata should load");

    let enumeration = enumerate(&metadata);

    // The best of three, where three are affordable: the suite runs its
    // tests in parallel, so a single sample measures the scheduler as much
    // as it measures the pass. An unoptimized run costs twenty seconds a
    // sample and is already allowed an order of magnitude, which is wider
    // than the noise the repetition removes.
    //
    // The shipped stop-and-report budget is scaled with everything else an
    // unoptimized profile is allowed here: it is a bound on what a user
    // waits for, and holding a test binary to it would stop the pass
    // mid-workload and measure the stop instead of the pass.
    let mut pass = Duration::MAX;
    let mut scan = StructureScan::default();
    for _ in 0..if cfg!(debug_assertions) { 1 } else { 3 } {
        let started = Instant::now();
        scan = analyze_within(
            &metadata,
            &enumeration,
            &PolicyPlan::default(),
            &StructureSettings::default(),
            TIME_BUDGET * allowance() * machine_allowance(),
        );
        pass = pass.min(started.elapsed());
    }

    assert!(scan.errors.is_empty(), "{:?}", scan.errors);
    assert_eq!(
        scan.counters.functions,
        FILES * FUNCTIONS_PER_FILE,
        "the benchmark did not present the workload it claims"
    );
    // The shapes are the workload. A generator whose functions collapse
    // into a handful of canonical forms leaves the scoring loop with
    // nothing to do and reports a budget it never tested.
    assert!(
        scan.counters.shapes * 100 >= scan.counters.functions * 95,
        "the benchmark presented {} functions in only {} shapes",
        scan.counters.functions,
        scan.counters.shapes
    );
    assert!(
        !scan.findings.is_empty(),
        "the benchmark planted clone families the pass did not find"
    );
    // US-007, and the NFR behind it: the scoring must be nominated, not
    // pairwise. This is the assertion that says so, and it holds on any
    // machine, unlike the two clocks below.
    let pairwise = scan.counters.shapes * scan.counters.shapes / 2;
    assert!(
        scan.counters.comparisons * 20 <= pairwise,
        "the near-duplicate pass scored {} pairs of {} shapes, against {pairwise} for a pairwise scan",
        scan.counters.comparisons,
        scan.counters.shapes
    );

    let budget = BUDGET * allowance() * machine_allowance();
    assert!(
        pass <= budget,
        "the structural pass took {pass:?}, over its {budget:?} budget"
    );
    let recorded = RECORDED_MILLISECONDS * u128::from(machine_allowance());
    assert!(
        pass.as_millis() * 4 <= recorded * 5,
        "the structural pass took {pass:?}, more than a quarter over the recorded {recorded} ms"
    );
    assert!(
        scan.counters.retained_bytes <= MEMORY_LIMIT_BYTES,
        "the pass held {} bytes for {} functions",
        scan.counters.retained_bytes,
        scan.counters.functions
    );

    // The share is measured against a whole scan rather than against the
    // walk it shares with the source kernel, because the number the NFR
    // bounds is what the user waits for. The scan below compiles the same
    // thousand files through Clippy, which is where that wait comes from.
    //
    // Only what ships is measured here. An unoptimized numerator over a
    // Clippy denominator that runs at full speed whatever the profile
    // reads the share an order of magnitude too high, and the shipped
    // stop-and-report budget cuts a debug pass short on this workload,
    // which is the pass behaving as FR-10 asks and not a measurement. So
    // `cargo test --release` is what proves this bound.
    if !cfg!(debug_assertions) {
        let started = Instant::now();
        let report = crate::inspect(crate::InspectRequest::new(&root));
        let whole = started.elapsed();
        assert_eq!(report.status, crate::Status::Complete, "{:?}", report.errors);
        assert!(
            pass.as_millis() * 100 <= SHARE_PERCENT * whole.as_millis(),
            "the structural pass took {pass:?} of a {whole:?} scan, over its {SHARE_PERCENT}% share"
        );
    }

    let _ = fs::remove_dir_all(&root);
}
