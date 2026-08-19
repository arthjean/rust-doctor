use super::*;
use crate::report::{DiagnosticSource, Severity};

mod oracle;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_ROOT: AtomicUsize = AtomicUsize::new(0);

fn root(name: &str) -> PathBuf {
    let sequence = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/delta-kernel")
        .join(format!("{}-{name}-{sequence}", std::process::id()));
    if root.exists() {
        fs::remove_dir_all(&root).unwrap();
    }
    fs::create_dir_all(root.join("src")).unwrap();
    root
}

/// The line every oracle case starts from on both sides.
const ORIGINAL: &str = "todo!();\n";

/// A scratch tree removed when the test that made it ends.
///
/// Deleting at the end of a body leaves the tree behind on exactly the run whose
/// fixture is worth keeping out of the next one: the one whose assertion failed.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write_source(root: &Path, path: &str, source: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, source).unwrap();
}

/// Two source trees and the two diagnostic lists compared over them.
///
/// Naming the trees, writing them, listing the diagnostics, computing the delta
/// and removing the trees is the whole shape of a test here, and spelling it out
/// in each of them is what made three of them near-duplicates of one another.
struct Scenario {
    baseline_root: Scratch,
    current_root: Scratch,
    baseline: Vec<Diagnostic>,
    current: Vec<Diagnostic>,
}

impl Scenario {
    fn new(name: &str) -> Self {
        Self::at(
            root(&format!("{name}-base")),
            root(&format!("{name}-current")),
        )
    }

    /// A scenario under an existing pair of parents, which is how the oracle
    /// keeps its thirty-two cases side by side.
    fn under(baseline_parent: &Path, current_parent: &Path, name: &str) -> Self {
        let baseline_root = baseline_parent.join(name);
        let current_root = current_parent.join(name);
        fs::create_dir_all(&baseline_root).unwrap();
        fs::create_dir_all(&current_root).unwrap();
        Self::at(baseline_root, current_root)
    }

    fn at(baseline_root: PathBuf, current_root: PathBuf) -> Self {
        Self {
            baseline_root: Scratch(baseline_root),
            current_root: Scratch(current_root),
            baseline: Vec::new(),
            current: Vec::new(),
        }
    }

    fn write_baseline(&self, path: &str, source: &str) {
        write_source(&self.baseline_root.0, path, source);
    }

    fn write_current(&self, path: &str, source: &str) {
        write_source(&self.current_root.0, path, source);
    }

    fn write_both(&self, path: &str, source: &str) {
        self.write_baseline(path, source);
        self.write_current(path, source);
    }

    fn delta(&self) -> DeltaReport {
        compute(
            &self.baseline,
            &self.current,
            &self.baseline_root.0,
            &self.current_root.0,
        )
        .unwrap()
    }
}

fn diagnostic(id: &str, path: Option<&str>, span: Option<DiagnosticSpan>) -> Diagnostic {
    Diagnostic {
        context: None,
        id: id.to_owned(),
        source: DiagnosticSource::Clippy,
        code: Some("clippy::todo".to_owned()),
        base_severity: Severity::Warning,
        severity: Severity::Warning,
        category: Some("maintainability".to_owned()),
        message: "todo macro is used".to_owned(),
        help: None,
        package: Some("fixture".to_owned()),
        target: Some("fixture".to_owned()),
        path: path.map(str::to_owned),
        span,
        related: Vec::new(),
        similarity_basis_points: None,
        complexity: None,
        occurrences: 1,
    }
}

fn span(line: usize, start: usize, end: usize) -> DiagnosticSpan {
    DiagnosticSpan {
        line_start: line,
        column_start: start,
        line_end: line,
        column_end: end,
    }
}

fn test_proof(source: &str, span: &DiagnosticSpan, remaining_budget: usize) -> Option<String> {
    extract_proof(source, &LineIndex::of(source), span, remaining_budget)
}

#[test]
fn proof_normalizes_unicode_whitespace_and_rejects_invalid_or_oversized_ranges() {
    let source = "préfixe\r\n\t todo!(  \"échec\" ) ;\n";
    let proof = test_proof(
        source,
        &DiagnosticSpan {
            line_start: 2,
            column_start: 1,
            line_end: 2,
            column_end: 22,
        },
        PROOF_BYTES_BUDGET,
    )
    .unwrap();
    assert_eq!(proof, "todo!( \"échec\" ) ;");
    let lf = "préfixe\n\t todo!(  \"échec\" ) ;\n";
    assert_eq!(
        test_proof(lf, &span(2, 1, 22), PROOF_BYTES_BUDGET).unwrap(),
        proof,
        "CRLF and LF must normalize identically"
    );
    let multiline = "call(\n\tvalue,\n  other)\n";
    assert_eq!(
        test_proof(
            multiline,
            &DiagnosticSpan {
                line_start: 1,
                column_start: 1,
                line_end: 3,
                column_end: 9,
            },
            PROOF_BYTES_BUDGET,
        )
        .unwrap(),
        "call( value, other)"
    );
    assert!(test_proof(source, &span(0, 1, 2), PROOF_BYTES_BUDGET).is_none());
    assert!(test_proof(source, &span(2, 22, 1), PROOF_BYTES_BUDGET).is_none());

    let oversized = "x".repeat(PROOF_BYTES_LIMIT + 1);
    assert!(
        test_proof(
            &oversized,
            &span(1, 1, PROOF_BYTES_LIMIT.saturating_add(2)),
            PROOF_BYTES_BUDGET,
        )
        .is_none()
    );
}

/// What a delta is expected to say, as the three lists a reader checks.
#[derive(Debug, PartialEq, Eq)]
struct Verdict {
    introduced: Vec<String>,
    /// Each pairing as the current id and the baseline id it was matched with.
    pre_existing: Vec<(String, String)>,
    fixed: Vec<String>,
    cross_file_matches: usize,
}

fn verdict(
    introduced: &[&str],
    pre_existing: &[(&str, &str)],
    fixed: &[&str],
    cross_file_matches: usize,
) -> Verdict {
    Verdict {
        introduced: introduced.iter().map(|id| (*id).to_owned()).collect(),
        pre_existing: pre_existing
            .iter()
            .map(|(current, baseline)| ((*current).to_owned(), (*baseline).to_owned()))
            .collect(),
        fixed: fixed.iter().map(|id| (*id).to_owned()).collect(),
        cross_file_matches,
    }
}

/// One named comparison and the verdict it has to produce.
///
/// Writing two trees, naming two diagnostic lists and checking three vectors is
/// one shape, and the six tests that each spelled it out were near-duplicates of
/// one another and of nothing else. The shape is written once here and the cases
/// are data, which is what the thirty-two-case oracle below already does.
struct MatchingCase {
    name: &'static str,
    build: fn(&mut Scenario),
    expected: Verdict,
}

impl Scenario {
    fn verdict(&self) -> Verdict {
        let delta = self.delta();
        Verdict {
            introduced: delta.introduced,
            pre_existing: delta
                .pre_existing
                .into_iter()
                .map(|matched| (matched.current_id, matched.baseline_id))
                .collect(),
            fixed: delta
                .fixed
                .into_iter()
                .map(|diagnostic| diagnostic.id)
                .collect(),
            cross_file_matches: delta.summary.cross_file_matches,
        }
    }
}

/// What makes two diagnostics one finding, and what is deliberately left out of
/// that answer.
fn identity_cases() -> Vec<MatchingCase> {
    vec![
        MatchingCase {
            name: "severity is not part of the identity",
            build: |scenario| {
                scenario.baseline = vec![diagnostic("base", None, None)];
                scenario.current = vec![diagnostic("current", None, None)];
                scenario.current[0].severity = Severity::Error;
            },
            expected: verdict(&[], &[("current", "base")], &[], 0),
        },
        MatchingCase {
            name: "the rule is part of the identity",
            build: |scenario| {
                scenario.baseline = vec![diagnostic("base", None, None)];
                scenario.current = vec![diagnostic("current", None, None)];
                scenario.current[0].code = Some("clippy::dbg_macro".to_owned());
            },
            expected: verdict(&["current"], &[], &["base"], 0),
        },
        MatchingCase {
            name: "the message is part of the identity",
            build: |scenario| {
                scenario.baseline = vec![diagnostic("base", None, None)];
                scenario.current = vec![diagnostic("current", None, None)];
                scenario.baseline[0].message = "old message".to_owned();
            },
            expected: verdict(&["current"], &[], &["base"], 0),
        },
        MatchingCase {
            name: "changed code under an unchanged span is a different finding",
            build: |scenario| {
                scenario.write_baseline("src/changed.rs", ORIGINAL);
                scenario.write_current("src/changed.rs", "todo!(\"changed\");\n");
                scenario.baseline = vec![diagnostic(
                    "base",
                    Some("src/changed.rs"),
                    Some(span(1, 1, 8)),
                )];
                scenario.current = vec![diagnostic(
                    "current",
                    Some("src/changed.rs"),
                    Some(span(1, 1, 18)),
                )];
            },
            expected: verdict(&["current"], &[], &["base"], 0),
        },
        // `workspace_path` percent-encodes `%` and every control character of a
        // published path. Opening the published spelling literally resolved to
        // no file at all, so every finding in a file whose name carries one
        // silently lost its proof and fell back to its message, which is the
        // weakest identity the module has. Here the two excerpts differ, so a
        // proof that was read says introduced and a proof that was not says
        // pre-existing.
        MatchingCase {
            name: "an encoded path is decoded before the evidence is read",
            build: |scenario| {
                scenario.write_baseline("src/100%.rs", "todo!(\"old\");\n");
                scenario.write_current("src/100%.rs", "todo!(\"new\");\n");
                let published = "src/100%25.rs";
                scenario.baseline = vec![diagnostic("base", Some(published), Some(span(1, 1, 14)))];
                scenario.current =
                    vec![diagnostic("current", Some(published), Some(span(1, 1, 14)))];
            },
            expected: verdict(&["current"], &[], &["base"], 0),
        },
    ]
}

/// The order the passes run in, on the inputs that put two of them in
/// competition.
fn pairing_cases() -> Vec<MatchingCase> {
    vec![
        MatchingCase {
            name: "a copy does not consume the original",
            build: |scenario| {
                scenario.write_both("src/original.rs", ORIGINAL);
                scenario.write_current("src/a_copy.rs", ORIGINAL);
                scenario.baseline = vec![diagnostic(
                    "base-original",
                    Some("src/original.rs"),
                    Some(span(1, 1, 8)),
                )];
                scenario.current = vec![
                    diagnostic("current-copy", Some("src/a_copy.rs"), Some(span(1, 1, 8))),
                    diagnostic(
                        "current-original",
                        Some("src/original.rs"),
                        Some(span(1, 1, 8)),
                    ),
                ];
            },
            expected: verdict(
                &["current-copy"],
                &[("current-original", "base-original")],
                &[],
                0,
            ),
        },
        MatchingCase {
            name: "the same proof in another file is a move",
            build: |scenario| {
                scenario.write_baseline("src/old.rs", ORIGINAL);
                scenario.write_current("src/moved.rs", ORIGINAL);
                scenario.baseline =
                    vec![diagnostic("base", Some("src/old.rs"), Some(span(1, 1, 8)))];
                scenario.current = vec![diagnostic(
                    "moved",
                    Some("src/moved.rs"),
                    Some(span(1, 1, 8)),
                )];
            },
            expected: verdict(&[], &[("moved", "base")], &[], 1),
        },
        // A finding whose proof could not be read falls back to its message on
        // its own file; a finding that moved carries its proof to another file.
        // When both claim the same baseline the message match on the original
        // file wins, because the moved pass runs last. That is a decision rather
        // than a consequence, and reversing the two would report the move as
        // pre-existing and the proofless finding as introduced. It is asserted
        // so changing it is a moved expectation rather than a silent shift in
        // what `--scope baseline` calls new.
        MatchingCase {
            name: "a message match on the original file wins over a moved proof",
            build: |scenario| {
                scenario.write_both("src/old.rs", ORIGINAL);
                scenario.write_current("src/new.rs", ORIGINAL);
                scenario.baseline =
                    vec![diagnostic("base", Some("src/old.rs"), Some(span(1, 1, 8)))];
                scenario.current = vec![
                    // No span, so no proof: only its message can identify it.
                    diagnostic("unproven", Some("src/old.rs"), None),
                    // The proof the baseline carries, in another file.
                    diagnostic("moved", Some("src/new.rs"), Some(span(1, 1, 8))),
                ];
            },
            expected: verdict(&["moved"], &[("unproven", "base")], &[], 0),
        },
        MatchingCase {
            name: "two findings with no path at all pair on their message",
            build: |scenario| {
                scenario.baseline = vec![diagnostic("base", None, None)];
                scenario.current = vec![diagnostic("current", None, None)];
            },
            expected: verdict(&[], &[("current", "base")], &[], 0),
        },
        MatchingCase {
            name: "a proofless finding does not pair across paths",
            build: |scenario| {
                scenario.baseline = vec![diagnostic("base", None, None)];
                scenario.current = vec![diagnostic("moved", Some("src/missing.rs"), None)];
            },
            expected: verdict(&["moved"], &[], &["base"], 0),
        },
    ]
}

#[test]
fn every_matching_case_produces_the_verdict_it_names() {
    for (index, case) in identity_cases()
        .into_iter()
        .chain(pairing_cases())
        .enumerate()
    {
        let mut scenario = Scenario::new(&format!("case-{index:02}"));
        (case.build)(&mut scenario);
        assert_eq!(scenario.verdict(), case.expected, "case: {}", case.name);
    }
}

#[test]
fn source_file_over_the_source_kernel_bound_is_not_read() {
    let root = Scratch(root("source-limit"));
    let path = root.0.join("src/large.rs");
    File::create(&path)
        .unwrap()
        .set_len((SOURCE_FILE_BYTES_LIMIT + 1) as u64)
        .unwrap();
    let mut loader = EvidenceLoader::new(&root.0);
    let canonical_root = loader.root.clone().unwrap();

    assert!(
        loader
            .read_source(&canonical_root, "src/large.rs")
            .is_none()
    );
    assert_eq!(loader.source_bytes_read, 0);
    assert_eq!(loader.proof_bytes, 0);
}

#[test]
fn aggregate_proof_budget_disables_the_first_excess_proof() {
    let root = Scratch(root("proof-budget"));
    fs::write(root.0.join("src/lib.rs"), "x\n").unwrap();
    let mut loader = EvidenceLoader::new(&root.0);
    loader.proof_bytes = PROOF_BYTES_BUDGET;
    let source = "x\n";
    let lines = LineIndex::of(source);

    assert!(loader.proof(source, &lines, &span(1, 1, 2)).is_none());
    assert_eq!(loader.proof_bytes, PROOF_BYTES_BUDGET);
}

#[test]
fn fallback_matching_reserves_unavailable_baselines_for_available_currents() {
    let baseline = [
        diagnostic("base-unavailable", None, None),
        diagnostic("base-stable", None, None),
    ];
    let current = [
        diagnostic("current-unavailable", None, None),
        diagnostic("current-stable", None, None),
    ];
    let baseline_candidates = vec![
        Candidate {
            diagnostic: &baseline[0],
            fingerprint: None,
        },
        Candidate {
            diagnostic: &baseline[1],
            fingerprint: Some(DeltaFingerprintV1([1; 32])),
        },
    ];
    let current_candidates = vec![
        Candidate {
            diagnostic: &current[0],
            fingerprint: None,
        },
        Candidate {
            diagnostic: &current[1],
            fingerprint: Some(DeltaFingerprintV1([2; 32])),
        },
    ];

    let delta = match_candidates(&baseline_candidates, &current_candidates);

    assert_eq!(delta.pre_existing.len(), 2);
    assert!(delta.introduced.is_empty());
    assert!(delta.fixed.is_empty());
}

#[test]
fn multiset_matching_is_deterministic_and_bounded() {
    let mut scenario = Scenario::new("multiset");
    scenario.baseline = (0..32)
        .map(|index| diagnostic(&format!("base-{index:02}"), None, None))
        .collect::<Vec<_>>();
    scenario.current = (0..40)
        .map(|index| diagnostic(&format!("current-{index:02}"), None, None))
        .collect::<Vec<_>>();
    let expected = scenario.delta();
    for _ in 0..20 {
        assert_eq!(scenario.delta(), expected);
    }
    assert_eq!(expected.pre_existing.len(), 32);
    assert_eq!(expected.introduced.len(), 8);
    assert!(expected.fixed.is_empty());

    scenario.baseline = vec![diagnostic("same", None, None); DIAGNOSTIC_LIMIT + 1];
    scenario.current = Vec::new();
    let error = compute(
        &scenario.baseline,
        &scenario.current,
        &scenario.baseline_root.0,
        &scenario.current_root.0,
    )
    .unwrap_err();
    assert_eq!(error.stage, "delta");
    assert_eq!(error.code, "delta-limit-exceeded");
}

/// US-003: a structural finding is matched on the identity its own pass
/// computed, never on the message it published.
///
/// Every structural message states a number the next edit moves: the line count
/// of an oversized file, the occurrence count of a clone family, the two figures
/// of a hotspot. Matched on the message and on an excerpt of the unit, a finding
/// older than the branch reads as introduced by it and the one it replaced reads
/// as fixed, which is the opposite of what a baseline scope is asked for.
#[test]
fn a_structural_finding_survives_the_count_its_message_states() {
    let scenario = Scenario::new("structural");
    let baseline_root = &scenario.baseline_root.0;
    let current_root = &scenario.current_root.0;

    let mut before = diagnostic("family", Some("src/lib.rs"), Some(span(1, 1, 2)));
    before.source = DiagnosticSource::RustDoctor;
    before.code = Some(crate::policy::STRUCTURE_OVERSIZED_UNIT.id.to_owned());
    before.message = "src/lib.rs is 1200 lines long.".to_owned();

    let mut grown = before.clone();
    grown.message = "src/lib.rs is 1207 lines long.".to_owned();
    let delta = compute(
        std::slice::from_ref(&before),
        &[grown],
        baseline_root,
        current_root,
    )
    .unwrap();
    assert_eq!(delta.summary.introduced, 0, "{delta:?}");
    assert_eq!(delta.summary.pre_existing, 1, "{delta:?}");
    assert!(delta.fixed.is_empty(), "{delta:?}");

    // The same family reported from another file, which is what a clone family
    // does when a new member sorts ahead of the old first one. The identity does
    // not carry the path, so the family is still the same family.
    let mut moved = before.clone();
    moved.path = Some("src/other.rs".to_owned());
    let delta = compute(
        std::slice::from_ref(&before),
        &[moved],
        baseline_root,
        current_root,
    )
    .unwrap();
    assert_eq!(delta.summary.pre_existing, 1, "{delta:?}");
    assert_eq!(delta.summary.cross_file_matches, 1, "{delta:?}");

    // A per-site diagnostic keeps the matching it had: its message is part of
    // what identifies it, because nothing else does.
    let site = diagnostic("site", Some("src/lib.rs"), Some(span(1, 1, 2)));
    let mut reworded = site.clone();
    reworded.message = "another message".to_owned();
    let delta = compute(&[site], &[reworded], baseline_root, current_root).unwrap();
    assert_eq!(delta.summary.introduced, 1, "{delta:?}");
}

/// The one arithmetic the evidence rests on, at its edges.
#[test]
fn a_reported_position_resolves_to_the_byte_the_line_index_says() {
    let source = "ab\ncd\n";
    let lines = LineIndex::of(source);
    let at = |line, column| lines.offset(source, SourcePosition { line, column });

    assert_eq!(at(1, 1), Some(0));
    assert_eq!(at(1, 2), Some(1));
    // A line owns its terminator, so the newline is the last column it answers.
    assert_eq!(at(1, 3), Some(2));
    assert_eq!(at(1, 4), None);
    assert_eq!(at(2, 1), Some(3));
    // The empty line a trailing newline opens.
    assert_eq!(at(3, 1), Some(6));
    assert_eq!(at(3, 2), None);
    assert_eq!(at(4, 1), None);
    // Neither a line nor a column is ever zero.
    assert_eq!(at(0, 1), None);
    assert_eq!(at(1, 0), None);

    // A file with no trailing newline answers the position one past its last
    // character, which is what a span covering that line reports.
    let unterminated = "é!";
    let lines = LineIndex::of(unterminated);
    let at = |line, column| lines.offset(unterminated, SourcePosition { line, column });
    assert_eq!(at(1, 1), Some(0));
    assert_eq!(at(1, 2), Some(2));
    assert_eq!(at(1, 3), Some(3));
    assert_eq!(at(1, 4), None);
}

/// Every file of the module stays under the bound `oversized_unit` reports at,
/// tests included: the pass that decides what a branch introduced has to pass
/// the rule it publishes.
#[test]
fn the_delta_holds_the_size_bound_it_matches_for() {
    for own in [
        include_str!("../delta.rs"),
        include_str!("tests.rs"),
        include_str!("tests/oracle.rs"),
    ] {
        let lines = own.lines().count();
        assert!(
            lines < crate::structure::FILE_LINES,
            "a file of the delta is {lines} lines long, over the {} it publishes",
            crate::structure::FILE_LINES
        );
    }
}
