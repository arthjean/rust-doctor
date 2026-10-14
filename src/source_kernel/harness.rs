//! The native source detectors run on one unit in process, for the
//! regression seeds `tests/regression_seeds.rs` replays.
//!
//! A seed is one `.rs` file, hand-minimized from a confirmed false positive or
//! a measured miss, and asking the binary about it would compile nothing the
//! detectors read and cost a Cargo workspace per seed. This runs the registry
//! the scan runs, every detector of it, on a unit whose dependencies are the
//! crates the detectors know by name. Not a stable interface.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use ra_ap_syntax::{Edition, SourceFile};

use super::{
    DETECTORS, Enumeration, Findings, Identity, LIMITS, Reachability, Registry, SourceUnit,
    analyze_unit,
};
use crate::report::DiagnosticContext;

/// Crates a detector names an item of, each a dependency of the unit under its
/// own name.
const KNOWN_CRATES: [&str; 9] = [
    "diesel",
    "futures",
    "postgres",
    "reqwest",
    "rusqlite",
    "sea_orm",
    "sqlx",
    "tokio",
    "tokio_postgres",
];

/// One source file to run the detectors on, as the library of a crate that
/// depends on every crate a detector knows.
#[derive(Debug, Clone)]
pub struct CaseUnit {
    pub source: String,
}

/// One finding of one detector, positioned in the unit, one-based.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detected {
    pub rule: &'static str,
    pub message: String,
    pub line_start: usize,
    pub column_start: usize,
    pub line_end: usize,
    pub column_end: usize,
    pub context: Option<DiagnosticContext>,
}

/// Every finding the registered detectors report on `case`, in the order the
/// scan publishes them. A unit that does not parse is refused with the number
/// of errors, since a seed that does not parse proves nothing about a rule.
pub fn run_detectors(case: &CaseUnit) -> Result<Vec<Detected>, String> {
    let parse = SourceFile::parse(&case.source, Edition::Edition2024);
    if !parse.errors().is_empty() {
        return Err(format!("does not parse ({} errors)", parse.errors().len()));
    }
    let unit = SourceUnit {
        source: case.source.clone(),
        error_ranges: Vec::new(),
        parse,
        edition: Edition::Edition2024,
        relative_path: "src/lib.rs".to_owned(),
        reachability: BTreeSet::from([Reachability {
            package_id: "seed".to_owned(),
            package_name: "seed".to_owned(),
            target_key: "seed:0".to_owned(),
            target_name: "seed".to_owned(),
            test_gated: false,
        }]),
        traversals: BTreeSet::new(),
    };
    let enumeration = Enumeration {
        units: BTreeMap::from([(
            Identity {
                path: PathBuf::from("src/lib.rs"),
                edition: Edition::Edition2024,
            },
            unit,
        )]),
        tables: BTreeMap::from([(
            "seed".to_owned(),
            KNOWN_CRATES
                .iter()
                .map(|krate| ((*krate).to_owned(), (*krate).to_owned()))
                .collect(),
        )]),
        ..Enumeration::default()
    };
    let registry = Registry::new(DETECTORS.iter().copied());
    let mut findings = Findings::default();
    for unit in enumeration.units.values() {
        analyze_unit(unit, &enumeration, &registry, LIMITS, &mut findings);
    }
    Ok(findings
        .candidates
        .into_values()
        .map(|candidate| Detected {
            rule: candidate.definition.id,
            message: candidate.message.into_owned(),
            line_start: candidate.span.line_start,
            column_start: candidate.span.column_start,
            line_end: candidate.span.line_end,
            column_end: candidate.span.column_end,
            context: candidate.context,
        })
        .collect())
}
