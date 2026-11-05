use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use cargo_metadata::MetadataCommand;

use super::*;
use crate::source_kernel::enumerate;
use crate::test_clock::machine_allowance;

const FILES: usize = 1_000;
const FUNCTIONS_PER_FILE: usize = 10;

const BUDGET: Duration = Duration::from_millis(2_000);

const DEBUG_ALLOWANCE: u32 = 14;

const RECORDED_MILLISECONDS: u128 = if cfg!(debug_assertions) { 21_100 } else { 1_650 };

const SHARE_PERCENT: u128 = 15;

const MEMORY_LIMIT_BYTES: usize = 200 * 1024 * 1024;

const OPERATORS: [&str; 6] = ["+", "-", "*", "|", "&", "^"];
const LITERALS: [&str; 6] = ["1", "2.5", "\"text\"", "true", "'c'", "7u64"];

const fn allowance() -> u32 {
    if cfg!(debug_assertions) {
        DEBUG_ALLOWANCE
    } else {
        1
    }
}

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
