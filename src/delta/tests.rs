use super::*;
use crate::report::{DiagnosticSource, Severity};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_ROOT: AtomicUsize = AtomicUsize::new(0);

fn root(name: &str) -> PathBuf {
    let sequence = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/delta-kernel")
        .join(format!("{name}-{sequence}"));
    if root.exists() {
        fs::remove_dir_all(&root).unwrap();
    }
    fs::create_dir_all(root.join("src")).unwrap();
    root
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
    let requested = [
        SourcePosition {
            line: span.line_start,
            column: span.column_start,
        },
        SourcePosition {
            line: span.line_end,
            column: span.column_end,
        },
    ];
    let offsets = resolve_positions(source, &requested);
    extract_proof(source, span, &offsets, remaining_budget)
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
        EVIDENCE_BYTES_LIMIT,
    )
    .unwrap();
    assert_eq!(proof, "todo!( \"échec\" ) ;");
    let lf = "préfixe\n\t todo!(  \"échec\" ) ;\n";
    assert_eq!(
        test_proof(lf, &span(2, 1, 22), EVIDENCE_BYTES_LIMIT).unwrap(),
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
            EVIDENCE_BYTES_LIMIT,
        )
        .unwrap(),
        "call( value, other)"
    );
    assert!(test_proof(source, &span(0, 1, 2), EVIDENCE_BYTES_LIMIT).is_none());
    assert!(test_proof(source, &span(2, 22, 1), EVIDENCE_BYTES_LIMIT).is_none());

    let oversized = "x".repeat(PROOF_BYTES_LIMIT + 1);
    assert!(
        test_proof(
            &oversized,
            &span(1, 1, PROOF_BYTES_LIMIT.saturating_add(2)),
            EVIDENCE_BYTES_LIMIT,
        )
        .is_none()
    );
}

#[test]
fn same_file_matching_precedes_cross_file_and_preserves_copies() {
    let baseline_root = root("copy-base");
    let current_root = root("copy-current");
    fs::write(baseline_root.join("src/original.rs"), "todo!();\n").unwrap();
    fs::write(current_root.join("src/original.rs"), "todo!();\n").unwrap();
    fs::write(current_root.join("src/a_copy.rs"), "todo!();\n").unwrap();
    let baseline = vec![diagnostic(
        "base-original",
        Some("src/original.rs"),
        Some(span(1, 1, 8)),
    )];
    let current = vec![
        diagnostic("current-copy", Some("src/a_copy.rs"), Some(span(1, 1, 8))),
        diagnostic(
            "current-original",
            Some("src/original.rs"),
            Some(span(1, 1, 8)),
        ),
    ];

    let delta = compute(&baseline, &current, &baseline_root, &current_root).unwrap();

    assert_eq!(delta.introduced, ["current-copy"]);
    assert_eq!(delta.pre_existing.len(), 1);
    assert_eq!(delta.pre_existing[0].current_id, "current-original");
    assert_eq!(delta.pre_existing[0].baseline_id, "base-original");
    assert_eq!(delta.summary.cross_file_matches, 0);
    assert!(delta.fixed.is_empty());
    fs::remove_dir_all(baseline_root).unwrap();
    fs::remove_dir_all(current_root).unwrap();
}

#[test]
fn move_matches_cross_file_but_changed_proof_is_fixed_and_introduced() {
    let baseline_root = root("move-base");
    let current_root = root("move-current");
    fs::write(baseline_root.join("src/old.rs"), "todo!();\n").unwrap();
    fs::write(current_root.join("src/moved.rs"), "todo!();\n").unwrap();
    fs::write(current_root.join("src/changed.rs"), "todo!(\"changed\");\n").unwrap();
    let baseline = vec![diagnostic("base", Some("src/old.rs"), Some(span(1, 1, 8)))];
    let moved = vec![diagnostic(
        "moved",
        Some("src/moved.rs"),
        Some(span(1, 1, 8)),
    )];
    let delta = compute(&baseline, &moved, &baseline_root, &current_root).unwrap();
    assert!(delta.introduced.is_empty());
    assert_eq!(delta.pre_existing.len(), 1);
    assert_eq!(delta.summary.cross_file_matches, 1);

    let changed = vec![diagnostic(
        "changed",
        Some("src/changed.rs"),
        Some(span(1, 1, 18)),
    )];
    let delta = compute(&baseline, &changed, &baseline_root, &current_root).unwrap();
    assert_eq!(delta.introduced, ["changed"]);
    assert!(delta.pre_existing.is_empty());
    assert_eq!(delta.fixed[0].id, "base");
    fs::remove_dir_all(baseline_root).unwrap();
    fs::remove_dir_all(current_root).unwrap();
}

#[test]
fn unavailable_evidence_uses_only_same_path_source_code_and_message() {
    let baseline_root = root("fallback-base");
    let current_root = root("fallback-current");
    let baseline = vec![diagnostic("base", None, None)];
    let current = vec![diagnostic("current", None, None)];
    let delta = compute(&baseline, &current, &baseline_root, &current_root).unwrap();
    assert_eq!(delta.pre_existing.len(), 1);

    let moved = vec![diagnostic("moved", Some("src/missing.rs"), None)];
    let delta = compute(&baseline, &moved, &baseline_root, &current_root).unwrap();
    assert_eq!(delta.introduced, ["moved"]);
    assert_eq!(delta.fixed[0].id, "base");
    fs::remove_dir_all(baseline_root).unwrap();
    fs::remove_dir_all(current_root).unwrap();
}

#[test]
fn source_file_over_the_source_kernel_bound_is_not_read() {
    let root = root("source-limit");
    let path = root.join("src/large.rs");
    File::create(&path)
        .unwrap()
        .set_len((SOURCE_FILE_BYTES_LIMIT + 1) as u64)
        .unwrap();
    let mut loader = EvidenceLoader::new(&root);
    let canonical_root = loader.root.clone().unwrap();

    assert!(
        loader
            .read_source(&canonical_root, "src/large.rs")
            .is_none()
    );
    assert_eq!(loader.source_bytes_read, 0);
    assert_eq!(loader.proof_bytes, 0);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn aggregate_proof_budget_disables_the_first_excess_proof() {
    let root = root("proof-budget");
    fs::write(root.join("src/lib.rs"), "x\n").unwrap();
    let mut loader = EvidenceLoader::new(&root);
    loader.proof_bytes = EVIDENCE_BYTES_LIMIT;
    let source = "x\n";
    let requested = [
        SourcePosition { line: 1, column: 1 },
        SourcePosition { line: 1, column: 2 },
    ];
    let offsets = resolve_positions(source, &requested);

    assert!(loader.proof(source, &span(1, 1, 2), &offsets).is_none());
    assert_eq!(loader.proof_bytes, EVIDENCE_BYTES_LIMIT);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn severity_is_excluded_but_code_and_message_are_included() {
    let baseline_root = root("domain-base");
    let current_root = root("domain-current");
    let mut baseline = diagnostic("base", None, None);
    let mut current = diagnostic("current", None, None);
    current.severity = Severity::Error;
    let delta = compute(
        std::slice::from_ref(&baseline),
        std::slice::from_ref(&current),
        &baseline_root,
        &current_root,
    )
    .unwrap();
    assert_eq!(delta.pre_existing.len(), 1);

    current.code = Some("clippy::dbg_macro".to_owned());
    let delta = compute(
        std::slice::from_ref(&baseline),
        std::slice::from_ref(&current),
        &baseline_root,
        &current_root,
    )
    .unwrap();
    assert_eq!(delta.introduced, ["current"]);
    current.code = baseline.code.clone();
    baseline.message = "old message".to_owned();
    let delta = compute(
        std::slice::from_ref(&baseline),
        std::slice::from_ref(&current),
        &baseline_root,
        &current_root,
    )
    .unwrap();
    assert_eq!(delta.introduced, ["current"]);
    fs::remove_dir_all(baseline_root).unwrap();
    fs::remove_dir_all(current_root).unwrap();
}

#[test]
fn fallback_matching_reserves_unavailable_baselines_for_available_currents() {
    let baseline = vec![
        diagnostic("base-unavailable", None, None),
        diagnostic("base-stable", None, None),
    ];
    let current = vec![
        diagnostic("current-unavailable", None, None),
        diagnostic("current-stable", None, None),
    ];
    let baseline_candidates = vec![
        Candidate {
            path: None,
            fallback: fallback_key(&baseline[0]),
            fingerprint: None,
        },
        Candidate {
            path: None,
            fallback: fallback_key(&baseline[1]),
            fingerprint: Some(DeltaFingerprintV1([1; 32])),
        },
    ];
    let current_candidates = vec![
        Candidate {
            path: None,
            fallback: fallback_key(&current[0]),
            fingerprint: None,
        },
        Candidate {
            path: None,
            fallback: fallback_key(&current[1]),
            fingerprint: Some(DeltaFingerprintV1([2; 32])),
        },
    ];

    let delta = match_candidates(
        &baseline,
        &current,
        &baseline_candidates,
        &current_candidates,
    );

    assert_eq!(delta.pre_existing.len(), 2);
    assert!(delta.introduced.is_empty());
    assert!(delta.fixed.is_empty());
}

#[test]
fn multiset_matching_is_deterministic_and_bounded() {
    let baseline_root = root("multiset-base");
    let current_root = root("multiset-current");
    let baseline = (0..32)
        .map(|index| diagnostic(&format!("base-{index:02}"), None, None))
        .collect::<Vec<_>>();
    let current = (0..40)
        .map(|index| diagnostic(&format!("current-{index:02}"), None, None))
        .collect::<Vec<_>>();
    let expected = compute(&baseline, &current, &baseline_root, &current_root).unwrap();
    for _ in 0..20 {
        assert_eq!(
            compute(&baseline, &current, &baseline_root, &current_root).unwrap(),
            expected
        );
    }
    assert_eq!(expected.pre_existing.len(), 32);
    assert_eq!(expected.introduced.len(), 8);
    assert!(expected.fixed.is_empty());

    let over_limit = vec![diagnostic("same", None, None); DIAGNOSTIC_LIMIT + 1];
    let error = compute(&over_limit, &[], &baseline_root, &current_root).unwrap_err();
    assert_eq!(error.code, "baseline-limit-exceeded");
    fs::remove_dir_all(baseline_root).unwrap();
    fs::remove_dir_all(current_root).unwrap();
}

fn write_source(root: &Path, path: &str, source: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, source).unwrap();
}

fn line_span(line: usize, source: &str) -> DiagnosticSpan {
    let content = source.trim_end_matches('\n').trim_end_matches('\r');
    span(line, 1, content.chars().count() + 1)
}

fn oracle_case(
    index: usize,
    baseline_parent: &Path,
    current_parent: &Path,
) -> (String, DeltaReport) {
    let baseline_root = baseline_parent.join(format!("{index:02}"));
    let current_root = current_parent.join(format!("{index:02}"));
    fs::create_dir_all(&baseline_root).unwrap();
    fs::create_dir_all(&current_root).unwrap();

    let original = "todo!();\n";
    write_source(&baseline_root, "src/original.rs", original);
    write_source(&current_root, "src/original.rs", original);
    let mut baseline = vec![diagnostic(
        &format!("base-{index:02}"),
        Some("src/original.rs"),
        Some(span(1, 1, 8)),
    )];
    let mut current = vec![diagnostic(
        &format!("current-{index:02}"),
        Some("src/original.rs"),
        Some(span(1, 1, 8)),
    )];

    let name = match index {
        0 => {
            let evidence = "let café = todo!();\n";
            write_source(&baseline_root, "src/original.rs", evidence);
            write_source(
                &current_root,
                "src/original.rs",
                &format!("// shifted\n{evidence}"),
            );
            baseline[0].span = Some(line_span(1, evidence));
            current[0].span = Some(line_span(2, evidence));
            "unicode".to_owned()
        }
        1 => {
            let baseline_source = "\t todo!(  \"échec\" ) ;\r\n";
            let current_source = "todo!( \"échec\" ) ;\n";
            write_source(&baseline_root, "src/original.rs", baseline_source);
            write_source(&current_root, "src/original.rs", current_source);
            baseline[0].span = Some(line_span(1, baseline_source));
            current[0].span = Some(line_span(1, current_source));
            "crlf-lf".to_owned()
        }
        2 => {
            let baseline_source = "todo!(\t\"tabs\" );\n";
            let current_source = "todo!( \"tabs\" );\n";
            write_source(&baseline_root, "src/original.rs", baseline_source);
            write_source(&current_root, "src/original.rs", current_source);
            baseline[0].span = Some(line_span(1, baseline_source));
            current[0].span = Some(line_span(1, current_source));
            "tabs".to_owned()
        }
        3 => {
            let baseline_source = "call(\n\tvalue,\nother)\n";
            let current_source = "call( \n value,\t\nother)\n";
            write_source(&baseline_root, "src/original.rs", baseline_source);
            write_source(&current_root, "src/original.rs", current_source);
            baseline[0].span = Some(DiagnosticSpan {
                line_start: 1,
                column_start: 1,
                line_end: 3,
                column_end: 7,
            });
            current[0].span = baseline[0].span.clone();
            "multiline-span".to_owned()
        }
        4 => {
            let source = "todo!();\ntodo!();\n";
            write_source(&baseline_root, "src/original.rs", source);
            write_source(&current_root, "src/original.rs", source);
            baseline.push(diagnostic(
                "base-duplicate",
                Some("src/original.rs"),
                Some(span(2, 1, 8)),
            ));
            current.push(diagnostic(
                "current-duplicate",
                Some("src/original.rs"),
                Some(span(2, 1, 8)),
            ));
            "duplicates".to_owned()
        }
        5 => {
            write_source(&current_root, "src/copy.rs", original);
            current.insert(
                0,
                diagnostic("current-copy", Some("src/copy.rs"), Some(span(1, 1, 8))),
            );
            "copies".to_owned()
        }
        6 => {
            baseline[0].code = None;
            current[0].code = None;
            "nullable-code".to_owned()
        }
        7 => {
            current[0].severity = Severity::Error;
            "severity-excluded".to_owned()
        }
        8..=15 => {
            let moved_path = format!("src/moved-{:02}.rs", index - 8);
            write_source(&current_root, &moved_path, " todo!();\n");
            current[0].path = Some(moved_path);
            current[0].span = Some(span(1, 2, 9));
            format!("cross-file-stable-{:02}", index - 8)
        }
        16 => {
            baseline[0].path = None;
            baseline[0].span = None;
            current[0].path = None;
            current[0].span = None;
            "pathless-fallback".to_owned()
        }
        17 => {
            baseline[0].path = Some("src/missing.rs".to_owned());
            current[0].path = Some("src/missing.rs".to_owned());
            "missing-source-fallback".to_owned()
        }
        18 => {
            let path = "src/current-only.rs";
            write_source(&current_root, path, original);
            baseline[0].path = Some(path.to_owned());
            current[0].path = Some(path.to_owned());
            "baseline-unavailable-fallback".to_owned()
        }
        19 => {
            let path = "src/baseline-only.rs";
            write_source(&baseline_root, path, original);
            baseline[0].path = Some(path.to_owned());
            current[0].path = Some(path.to_owned());
            "current-unavailable-fallback".to_owned()
        }
        20..=23 => {
            let prefix = if index == 23 {
                "shared-prefix-shared-prefix-shared-prefix"
            } else {
                "evidence"
            };
            let baseline_source = format!("{prefix}-old\n");
            let current_source = format!("{prefix}-new\n");
            write_source(&baseline_root, "src/original.rs", &baseline_source);
            write_source(&current_root, "src/original.rs", &current_source);
            baseline[0].span = Some(line_span(1, &baseline_source));
            current[0].span = Some(line_span(1, &current_source));
            format!("changed-evidence-{:02}", index - 20)
        }
        24 => {
            baseline[0].message = "old message".to_owned();
            current[0].message = "new message".to_owned();
            "changed-message-00".to_owned()
        }
        25 => {
            current[0].code = Some("clippy::dbg_macro".to_owned());
            "changed-code-00".to_owned()
        }
        26 => {
            current[0].source = DiagnosticSource::Rustc;
            "changed-source-00".to_owned()
        }
        27 => {
            baseline[0].message = "shared prefix old".to_owned();
            current[0].message = "shared prefix new".to_owned();
            "changed-message-prefix-00".to_owned()
        }
        28..=29 => {
            baseline.clear();
            format!("introduced-only-{:02}", index - 28)
        }
        30..=31 => {
            current.clear();
            format!("fixed-only-{:02}", index - 30)
        }
        _ => unreachable!(),
    };

    let delta = compute(&baseline, &current, &baseline_root, &current_root).unwrap();
    (name, delta)
}

fn oracle_document(baseline_root: &Path, current_root: &Path) -> Value {
    let cases = (0..32)
        .map(|index| {
            let (name, delta) = oracle_case(index, baseline_root, current_root);
            json!({
                "name": name,
                "introduced": delta.summary.introduced,
                "pre_existing": delta.summary.pre_existing,
                "fixed": delta.summary.fixed,
                "cross_file_matches": delta.summary.cross_file_matches,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "schema_version": 1,
        "fingerprint_version": FINGERPRINT_VERSION,
        "runs": 20,
        "features": [
            "unicode",
            "crlf-lf",
            "tabs",
            "multiline-span",
            "pathless",
            "duplicates",
            "copies",
            "prefix-collisions",
        ],
        "cases": cases,
    })
}

#[test]
fn adversarial_oracle_runs_the_complete_kernel_for_32_cases_and_20_identical_runs() {
    let baseline_root = root("oracle-base");
    let current_root = root("oracle-current");
    let actual = oracle_document(&baseline_root, &current_root);
    for _ in 1..20 {
        assert_eq!(
            oracle_document(&baseline_root, &current_root),
            actual,
            "the complete evidence and matching kernel must be deterministic",
        );
    }

    let fixture_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/baseline/delta-oracle.json");
    if std::env::var_os("RUST_DOCTOR_UPDATE_DELTA_ORACLE").is_some() {
        fs::write(
            &fixture_path,
            format!("{}\n", serde_json::to_string_pretty(&actual).unwrap()),
        )
        .unwrap();
    }
    let expected: Value = serde_json::from_str(&fs::read_to_string(fixture_path).unwrap()).unwrap();
    fs::remove_dir_all(baseline_root).unwrap();
    fs::remove_dir_all(current_root).unwrap();
    assert_eq!(actual, expected);
}
