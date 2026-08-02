#![cfg(unix)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

#[path = "support/rule_scaling_evaluation.rs"]
mod model;
mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicUsize;

use support::rule_scaling::{
    CargoJsonRecord, PrecisionMatrixOracle, PrecisionObservation, RuleScalingEvidence,
    RuleScalingOracle, SignalAdmission, cargo_json_records, evidence, observe_precision,
};

static NEXT_TARGET: AtomicUsize = AtomicUsize::new(0);
const GENERATED_AT: &str = "2026-08-02T14:06:33+02:00";
const REPOSITORIES: [(&str, &str); 5] = [
    ("anyhow", "18c2598afa0f996f56217ef128aa3a20ea1e9512"),
    ("thiserror", "72ae716e6d6a7f7fdabdc394018c745b4d39ca45"),
    ("serde_json", "efa66e3a1d61459ab2d325f92ebe3acbd6ca18b1"),
    ("log", "6e1735597bb21c5d979a077395df85e1d633e077"),
    ("hexyl", "abc20a380c8c2d9d76c1976222725d3211cef809"),
];

fn artifact_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tasks/rust-doctor-rule-scaling-kernel-evaluation.json")
}

fn artifact() -> model::EvaluationArtifact {
    serde_json::from_str(include_str!(
        "../tasks/rust-doctor-rule-scaling-kernel-evaluation.json"
    ))
    .expect("rule scaling evaluation should match its typed schema")
}

fn hash_bytes(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn command_version(program: &str) -> String {
    let output = Command::new(program).arg("--version").output().unwrap();
    assert!(output.status.success(), "{program}");
    String::from_utf8(output.stdout)
        .unwrap()
        .trim_end()
        .to_owned()
}

fn candidate_counts<'a>(
    oracle: &RuleScalingOracle,
    codes: impl Iterator<Item = &'a str>,
) -> BTreeMap<String, usize> {
    let mut counts = oracle
        .rules
        .iter()
        .map(|rule| (rule.id.clone(), 0))
        .collect::<BTreeMap<_, _>>();
    for code in codes {
        if let Some(count) = counts.get_mut(code) {
            *count += 1;
        }
    }
    counts
}

fn legacy_scan(
    repository: &Path,
    target: &Path,
    oracle: &RuleScalingOracle,
) -> model::LegacyScanEvidence {
    let command = oracle.legacy_clippy_command();
    let output = Command::new(env!("CARGO"))
        .args(command.iter().skip(1))
        .current_dir(repository)
        .env("CARGO_NET_OFFLINE", "true")
        .env("CARGO_TARGET_DIR", target)
        .env("RUSTUP_TOOLCHAIN", "1.97.1")
        .output()
        .unwrap();
    let records = cargo_json_records(&output).expect("legacy Cargo output should be valid JSON");
    let candidate_ids = oracle.candidate_ids();
    let findings = records
        .iter()
        .filter_map(|record| match record {
            CargoJsonRecord::CompilerMessage { message } => {
                message.code.as_ref().map(|code| code.code.as_str())
            }
            CargoJsonRecord::BuildFinished { .. } | CargoJsonRecord::Other => None,
        })
        .filter(|code| candidate_ids.contains(code))
        .collect::<Vec<_>>();
    let build_success = records.iter().rev().find_map(|record| match record {
        CargoJsonRecord::BuildFinished { success } => Some(*success),
        CargoJsonRecord::CompilerMessage { .. } | CargoJsonRecord::Other => None,
    });
    assert!(output.status.success());
    assert_eq!(build_success, Some(true));
    assert!(
        findings.is_empty(),
        "legacy corpus findings require manual review before certification: {findings:?}"
    );
    model::LegacyScanEvidence {
        command,
        status: model::ScanStatus::Complete,
        exit_code: output
            .status
            .code()
            .expect("Cargo should return an exit code"),
        counts: candidate_counts(oracle, findings.into_iter()),
        structured_id_hash: hash_bytes(b""),
    }
}

fn expanded_scan(
    repository: &Path,
    target: &Path,
    oracle: &RuleScalingOracle,
) -> model::ExpandedScanEvidence {
    let output = Command::new(env!("CARGO_BIN_EXE_rust-doctor"))
        .arg("inspect")
        .arg("--json")
        .arg(repository)
        .env("CARGO_NET_OFFLINE", "true")
        .env("CARGO_TARGET_DIR", target)
        .env("RUSTUP_TOOLCHAIN", "1.97.1")
        .output()
        .unwrap();
    let report: model::ExpandedReportObservation = serde_json::from_slice(&output.stdout).unwrap();
    let command = report
        .scan
        .command
        .expect("a complete scan should expose its command");
    assert_eq!(command, oracle.clippy_command);
    let candidate_ids = oracle.candidate_ids();
    let findings = report
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic
                .code
                .as_deref()
                .is_some_and(|code| candidate_ids.contains(code))
        })
        .collect::<Vec<_>>();
    assert_eq!(report.status, model::ObservedReportStatus::Complete);
    assert!(
        findings.is_empty(),
        "expanded corpus findings require a manual verdict before certification"
    );
    let counts = candidate_counts(
        oracle,
        findings
            .iter()
            .filter_map(|finding| finding.code.as_deref()),
    );
    let mut ids = findings
        .iter()
        .map(|finding| finding.id.as_str())
        .collect::<Vec<_>>();
    ids.sort_unstable();
    model::ExpandedScanEvidence {
        command,
        status: model::ScanStatus::Complete,
        exit_code: output
            .status
            .code()
            .expect("rust-doctor should return an exit code"),
        counts,
        finding_id_hash: hash_bytes(ids.join("\n").as_bytes()),
        findings: Vec::new(),
        manual_verdicts: Vec::new(),
        false_positives: 0,
        ambiguous: 0,
    }
}

fn matrix_evidence(
    oracle: &PrecisionMatrixOracle,
    observation: &PrecisionObservation,
) -> model::MatrixEvidence {
    let rules = observation
        .rules
        .iter()
        .map(|rule| {
            let spans = rule
                .spans
                .iter()
                .map(|span| serde_json::to_string(&model::SpanEvidence::from(span)).unwrap())
                .collect::<Vec<_>>()
                .join("\n");
            model::MatrixRuleEvidence {
                id: rule.id.clone(),
                tp: rule.tp,
                fp: rule.fp,
                tn: rule.tn,
                false_negatives: rule.r#fn,
                positive_span_hash: hash_bytes(spans.as_bytes()),
                verdict: if rule.passed() {
                    model::EvidenceVerdict::Pass
                } else {
                    model::EvidenceVerdict::Blocked
                },
            }
        })
        .collect::<Vec<_>>();
    let totals = model::MatrixTotals {
        tp: observation.rules.iter().map(|rule| rule.tp).sum(),
        fp: observation.rules.iter().map(|rule| rule.fp).sum(),
        tn: observation.rules.iter().map(|rule| rule.tn).sum(),
        false_negatives: observation.rules.iter().map(|rule| rule.r#fn).sum(),
    };
    let non_unix = &oracle.contexts.non_unix_permissions;
    let observed_non_unix = &observation.non_unix_permissions.spans;
    assert_eq!(observed_non_unix.len(), 1);
    model::MatrixEvidence {
        admission_basis: model::AdmissionBasis::PerRule,
        global_rate_used_for_admission: false,
        rules,
        contexts: model::MatrixContextEvidence {
            build_output_candidate_diagnostics: observation.build_output_candidate_diagnostics,
            local_macro_contract: oracle.contexts.local_macro_contract.clone(),
            external_expansion_contract: oracle.contexts.external_expansion_contract.clone(),
            missing_primary_span_contract: oracle.contexts.missing_primary_span_contract.clone(),
            unicode_primary_span: model::SpanEvidence::from(&oracle.contexts.unicode_primary_span),
            non_unix_permissions: model::NonUnixContextEvidence {
                target: non_unix.target.clone(),
                fixture: non_unix.fixture.clone(),
                candidate_diagnostics: observed_non_unix.len(),
                primary_span: model::SpanEvidence::from(&observed_non_unix[0]),
            },
        },
        totals,
    }
}

fn matrix_passes(oracle: &PrecisionMatrixOracle, matrix: &model::MatrixEvidence) -> bool {
    let expected_rule_ids = oracle
        .rules
        .iter()
        .map(|rule| rule.id.as_str())
        .collect::<BTreeSet<_>>();
    let matrix_rule_ids = matrix
        .rules
        .iter()
        .map(|rule| rule.id.as_str())
        .collect::<BTreeSet<_>>();
    let expected_tp = oracle
        .rules
        .iter()
        .map(|rule| rule.positives.len())
        .sum::<usize>();
    let expected_tn = oracle
        .rules
        .iter()
        .map(|rule| rule.negatives.len())
        .sum::<usize>();
    matrix_rule_ids == expected_rule_ids
        && matrix.rules.len() == oracle.rules.len()
        && matrix.totals.tp == expected_tp
        && matrix.totals.tn == expected_tn
        && matrix.totals.fp == 0
        && matrix.totals.false_negatives == 0
        && oracle.rules.iter().all(|expected| {
            matrix.rules.iter().any(|observed| {
                observed.id == expected.id
                    && observed.tp == expected.positives.len()
                    && observed.tn == expected.negatives.len()
                    && observed.fp == 0
                    && observed.false_negatives == 0
                    && observed.verdict == model::EvidenceVerdict::Pass
            })
        })
}

fn repository_passes(repository: &model::RepositoryEvaluation) -> bool {
    let finding_ids = repository
        .expanded
        .findings
        .iter()
        .map(|finding| finding.id.as_str())
        .collect::<BTreeSet<_>>();
    let verdict_ids = repository
        .expanded
        .manual_verdicts
        .iter()
        .map(|verdict| verdict.finding_id.as_str())
        .collect::<BTreeSet<_>>();
    repository.legacy.status == model::ScanStatus::Complete
        && repository.expanded.status == model::ScanStatus::Complete
        && repository.legacy.exit_code == 0
        && repository.expanded.exit_code == 0
        && repository.trusted
        && repository.build_code_warning_acknowledged
        && repository.network == model::NetworkMode::Offline
        && repository.repository_state.before == repository.repository_state.after
        && repository.expanded.false_positives == 0
        && repository.expanded.ambiguous == 0
        && finding_ids.len() == repository.expanded.findings.len()
        && verdict_ids.len() == repository.expanded.manual_verdicts.len()
        && finding_ids == verdict_ids
        && repository
            .expanded
            .manual_verdicts
            .iter()
            .all(|verdict| verdict.verdict == model::FindingVerdict::TruePositive)
}

fn repositories_pass(repositories: &[model::RepositoryEvaluation]) -> bool {
    let repository_commits = repositories
        .iter()
        .map(|repository| (repository.name.as_str(), repository.commit.as_str()))
        .collect::<BTreeMap<_, _>>();
    repository_commits == REPOSITORIES.into_iter().collect()
        && repositories.len() == REPOSITORIES.len()
        && repositories.iter().all(repository_passes)
}

fn rule_results_pass(oracle: &PrecisionMatrixOracle, rule_results: &[model::RuleResult]) -> bool {
    let expected_rule_ids = oracle
        .rules
        .iter()
        .map(|rule| rule.id.as_str())
        .collect::<BTreeSet<_>>();
    let rule_result_ids = rule_results
        .iter()
        .map(|rule| rule.id.as_str())
        .collect::<BTreeSet<_>>();
    rule_result_ids == expected_rule_ids
        && rule_results.len() == oracle.rules.len()
        && rule_results.iter().all(|rule| {
            rule.false_positives == 0
                && rule.ambiguous == 0
                && rule.verdict == model::EvidenceVerdict::Pass
        })
}

fn aggregate_verdict(
    oracle: &PrecisionMatrixOracle,
    matrix: &model::MatrixEvidence,
    repositories: &[model::RepositoryEvaluation],
    rule_results: &[model::RuleResult],
) -> model::EvidenceVerdict {
    if matrix_passes(oracle, matrix)
        && repositories_pass(repositories)
        && rule_results_pass(oracle, rule_results)
    {
        model::EvidenceVerdict::Pass
    } else {
        model::EvidenceVerdict::Blocked
    }
}

#[test]
fn aggregate_verdict_blocks_each_evidence_family_independently() {
    let expected = evidence();
    let mut blocked_matrix = artifact();
    blocked_matrix.matrix.rules[0].verdict = model::EvidenceVerdict::Blocked;
    assert_eq!(
        aggregate_verdict(
            &expected.precision,
            &blocked_matrix.matrix,
            &blocked_matrix.repositories,
            &blocked_matrix.rule_results
        ),
        model::EvidenceVerdict::Blocked
    );

    let mut missing_matrix_rows = artifact();
    missing_matrix_rows.matrix.rules.clear();
    assert_eq!(
        aggregate_verdict(
            &expected.precision,
            &missing_matrix_rows.matrix,
            &missing_matrix_rows.repositories,
            &missing_matrix_rows.rule_results
        ),
        model::EvidenceVerdict::Blocked
    );

    let mut mutated_repository = artifact();
    mutated_repository.repositories[0]
        .repository_state
        .after
        .head = "different".to_owned();
    assert_eq!(
        aggregate_verdict(
            &expected.precision,
            &mutated_repository.matrix,
            &mutated_repository.repositories,
            &mutated_repository.rule_results
        ),
        model::EvidenceVerdict::Blocked
    );

    let mut blocked_rule = artifact();
    blocked_rule.rule_results[0].ambiguous = 1;
    assert_eq!(
        aggregate_verdict(
            &expected.precision,
            &blocked_rule.matrix,
            &blocked_rule.repositories,
            &blocked_rule.rule_results
        ),
        model::EvidenceVerdict::Blocked
    );
}

fn reconstruct(
    corpus_root: &Path,
    target_root: &Path,
    evidence: &RuleScalingEvidence,
) -> model::EvaluationArtifact {
    let oracle = &evidence.catalog;
    let precision = observe_precision(evidence, &target_root.join("precision"));
    let mut repositories = Vec::new();
    for (name, commit) in REPOSITORIES {
        let repository = corpus_root.join(name);
        let resolved =
            String::from_utf8(support::git_output(&repository, &["rev-parse", "HEAD"]).stdout)
                .unwrap()
                .trim_end()
                .to_owned();
        assert_eq!(resolved, commit, "{name}");
        let before = support::git_repository_state(&repository);
        assert_eq!(before.status_hash, hash_bytes(b""), "{name} is dirty");
        let legacy = legacy_scan(
            &repository,
            &target_root.join(format!("{name}-legacy")),
            oracle,
        );
        let expanded = expanded_scan(
            &repository,
            &target_root.join(format!("{name}-expanded")),
            oracle,
        );
        let after = support::git_repository_state(&repository);
        assert_eq!(after, before, "{name} was mutated");

        let opt_in_findings = oracle
            .rules
            .iter()
            .filter(|rule| rule.admission() == SignalAdmission::OptIn)
            .map(|rule| (rule.id.clone(), expanded.counts[rule.id.as_str()]))
            .collect();
        let baseline_warn_contracts = oracle
            .rules
            .iter()
            .filter(|rule| rule.admission() == SignalAdmission::BaselineWarn)
            .map(|rule| {
                let id = rule.id.as_str();
                (
                    rule.id.clone(),
                    model::BaselineWarnContract {
                        legacy_findings: legacy.counts[id],
                        expanded_findings: expanded.counts[id],
                        metadata_admitted: !rule.category.is_empty() && !rule.help.is_empty(),
                    },
                )
            })
            .collect();
        repositories.push(model::RepositoryEvaluation {
            name: name.to_owned(),
            commit: commit.to_owned(),
            trusted: true,
            build_code_warning_acknowledged: true,
            network: model::NetworkMode::Offline,
            legacy,
            expanded,
            signal_classification: model::SignalClassification {
                opt_in_findings,
                baseline_warn_contracts,
            },
            repository_state: model::RepositoryStateEvidence { before, after },
        });
    }

    let rule_results = oracle
        .rules
        .iter()
        .map(|rule| {
            let id = rule.id.as_str();
            let legacy_findings = repositories
                .iter()
                .map(|repository| repository.legacy.counts[id])
                .sum();
            let expanded_findings = repositories
                .iter()
                .map(|repository| repository.expanded.counts[id])
                .sum();
            model::RuleResult {
                id: rule.id.clone(),
                clippy_default: rule.clippy_default,
                admission: rule.admission(),
                legacy_findings,
                expanded_findings,
                true_positives: 0,
                false_positives: 0,
                ambiguous: 0,
                verdict: if expanded_findings == 0 {
                    model::EvidenceVerdict::Pass
                } else {
                    model::EvidenceVerdict::Blocked
                },
            }
        })
        .collect::<Vec<_>>();
    let repository_count = repositories.len();
    let expanded_findings = rule_results.iter().map(|rule| rule.expanded_findings).sum();
    let matrix = matrix_evidence(&evidence.precision, &precision);
    let verdict = aggregate_verdict(&evidence.precision, &matrix, &repositories, &rule_results);
    model::EvaluationArtifact {
        artifact: "rust-doctor-rule-scaling-kernel-evaluation".to_owned(),
        schema_version: 1,
        generated_at: GENERATED_AT.to_owned(),
        epic: "EP-018".to_owned(),
        network_in_automated_tests: false,
        scan_network_mode: model::NetworkMode::Offline,
        trust_boundary: model::TrustBoundary {
            repositories: "five explicitly approved local repositories at exact commits".to_owned(),
            cargo_build_code_acknowledged: true,
            substitution_allowed: false,
        },
        toolchain: model::ToolchainEvidence {
            rustc: command_version("rustc"),
            cargo: command_version(env!("CARGO")),
            clippy: command_version("clippy-driver"),
        },
        commands: model::CommandEvidence {
            legacy: oracle.legacy_clippy_command(),
            expanded: oracle.clippy_command.clone(),
        },
        matrix,
        repositories,
        rule_results,
        totals: model::EvaluationTotals {
            repositories: repository_count,
            legacy_scans: repository_count,
            expanded_scans: repository_count,
            expanded_findings,
            manual_verdicts: 0,
            false_positives: 0,
            ambiguous: 0,
            repositories_unchanged: repository_count,
        },
        reconstruction: model::ReconstructionEvidence {
            test: "rule_scaling_evaluation_is_reconstructed_from_trusted_observations".to_owned(),
            network: model::NetworkMode::Offline,
            automated_default: "artifact-validation-only".to_owned(),
        },
        verdict,
    }
}

#[test]
fn pinned_rule_scaling_evaluation_is_complete_private_and_static() {
    let evidence = evidence();
    let oracle = &evidence.catalog;
    let artifact = artifact();
    assert_eq!(artifact.epic, "EP-018");
    assert!(!artifact.network_in_automated_tests);
    assert_eq!(artifact.scan_network_mode, model::NetworkMode::Offline);
    assert_eq!(artifact.commands.legacy, oracle.legacy_clippy_command());
    assert_eq!(artifact.commands.expanded, oracle.clippy_command);
    assert_eq!(
        artifact.matrix.admission_basis,
        model::AdmissionBasis::PerRule
    );
    assert!(!artifact.matrix.global_rate_used_for_admission);
    assert_eq!(artifact.matrix.totals.tp, 20);
    assert_eq!(artifact.matrix.totals.fp, 0);
    assert_eq!(artifact.matrix.totals.tn, 40);
    assert_eq!(artifact.matrix.totals.false_negatives, 0);

    let contexts = &evidence.precision.contexts;
    assert_eq!(
        artifact.matrix.contexts.build_output_candidate_diagnostics,
        contexts.build_output_candidate_diagnostics
    );
    assert_eq!(
        artifact.matrix.contexts.local_macro_contract,
        contexts.local_macro_contract
    );
    assert_eq!(
        artifact.matrix.contexts.external_expansion_contract,
        contexts.external_expansion_contract
    );
    assert_eq!(
        artifact.matrix.contexts.missing_primary_span_contract,
        contexts.missing_primary_span_contract
    );
    assert_eq!(
        artifact.matrix.contexts.unicode_primary_span,
        model::SpanEvidence::from(&contexts.unicode_primary_span)
    );
    assert_eq!(
        artifact.matrix.contexts.non_unix_permissions.target,
        contexts.non_unix_permissions.target
    );
    assert_eq!(
        artifact.matrix.contexts.non_unix_permissions.fixture,
        contexts.non_unix_permissions.fixture
    );
    assert_eq!(
        artifact
            .matrix
            .contexts
            .non_unix_permissions
            .candidate_diagnostics,
        contexts.non_unix_permissions.candidate_diagnostics
    );
    assert_eq!(
        artifact.matrix.contexts.non_unix_permissions.primary_span,
        model::SpanEvidence::from(&contexts.non_unix_permissions.primary_span)
    );

    assert_eq!(artifact.repositories.len(), 5);
    let commits = artifact
        .repositories
        .iter()
        .map(|repository| (repository.name.as_str(), repository.commit.as_str()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(commits, REPOSITORIES.into_iter().collect());
    for repository in &artifact.repositories {
        assert!(repository.trusted);
        assert!(repository.build_code_warning_acknowledged);
        assert_eq!(repository.network, model::NetworkMode::Offline);
        assert_eq!(repository.legacy.status, model::ScanStatus::Complete);
        assert_eq!(repository.expanded.status, model::ScanStatus::Complete);
        assert!(repository.expanded.findings.is_empty());
        assert!(repository.expanded.manual_verdicts.is_empty());
        assert_eq!(repository.expanded.false_positives, 0);
        assert_eq!(repository.expanded.ambiguous, 0);
        assert_eq!(
            repository.repository_state.before,
            repository.repository_state.after
        );
        for rule in &oracle.rules {
            let id = rule.id.as_str();
            assert_eq!(repository.legacy.counts[id], 0);
            assert_eq!(repository.expanded.counts[id], 0);
            if rule.admission() == SignalAdmission::BaselineWarn {
                assert!(
                    repository.signal_classification.baseline_warn_contracts[id].metadata_admitted
                );
            }
        }
    }
    assert_eq!(artifact.rule_results.len(), 5);
    assert!(artifact.rule_results.iter().all(|rule| {
        rule.false_positives == 0
            && rule.ambiguous == 0
            && rule.verdict == model::EvidenceVerdict::Pass
    }));
    assert_eq!(artifact.totals.repositories_unchanged, 5);
    assert_eq!(
        artifact.verdict,
        aggregate_verdict(
            &evidence.precision,
            &artifact.matrix,
            &artifact.repositories,
            &artifact.rule_results
        )
    );
    assert_eq!(artifact.verdict, model::EvidenceVerdict::Pass);

    let serialized = serde_json::to_string(&artifact).unwrap();
    for forbidden in [
        "http://",
        "https://",
        "file://",
        "/home/",
        "/tmp/",
        "credential=",
        "source=",
        "\u{1b}",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "artifact leaked {forbidden:?}"
        );
    }
}

#[test]
#[ignore = "requires the trusted pinned corpus and executes Cargo build code offline"]
fn rule_scaling_evaluation_is_reconstructed_from_trusted_observations() {
    let corpus_root = env::var_os("RUST_DOCTOR_RULE_SCALING_CORPUS_ROOT")
        .map(PathBuf::from)
        .expect("RUST_DOCTOR_RULE_SCALING_CORPUS_ROOT must name the trusted pinned corpus");
    let target_root = support::temporary_target("rule-scaling-evaluation", &NEXT_TARGET);
    let _ = fs::remove_dir_all(&target_root);
    fs::create_dir_all(&target_root).unwrap();
    let evaluation = reconstruct(&corpus_root, &target_root, &evidence());
    if env::var_os("RUST_DOCTOR_UPDATE_RULE_SCALING_EVALUATION").is_some() {
        fs::write(
            artifact_path(),
            format!("{}\n", serde_json::to_string_pretty(&evaluation).unwrap()),
        )
        .unwrap();
    } else {
        assert_eq!(evaluation, artifact());
    }
    fs::remove_dir_all(target_root).unwrap();
}
