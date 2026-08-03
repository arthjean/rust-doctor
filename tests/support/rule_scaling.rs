use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::{Command, Output};

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub(crate) struct RuleScalingOracle {
    pub(crate) toolchain: ToolchainOracle,
    pub(crate) rules: Vec<RuleOracle>,
    pub(crate) historical_rules: Vec<FindingOracle>,
    pub(crate) observed_contexts: ObservedContexts,
    pub(crate) clippy_command: Vec<String>,
    pub(crate) compatibility: CompatibilityOracle,
}

impl RuleScalingOracle {
    pub(crate) fn candidate_ids(&self) -> BTreeSet<&str> {
        self.rules.iter().map(|rule| rule.id.as_str()).collect()
    }

    pub(crate) fn explicit_flags(&self) -> Vec<&str> {
        self.rules
            .iter()
            .flat_map(|rule| ["-W", rule.id.as_str()])
            .collect()
    }

    pub(crate) fn legacy_clippy_command(&self) -> Vec<String> {
        let disabled = self
            .rules
            .iter()
            .map(|rule| rule.id.clone())
            .collect::<Vec<_>>();
        clippy_command_without_rules(&self.clippy_command, &disabled)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SignalAdmission {
    OptIn,
    BaselineWarn,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ClippyDefault {
    Allow,
    Warn,
}

impl ClippyDefault {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Warn => "warn",
        }
    }

    pub(crate) const fn admission(self) -> SignalAdmission {
        match self {
            Self::Allow => SignalAdmission::OptIn,
            Self::Warn => SignalAdmission::BaselineWarn,
        }
    }
}

impl SignalAdmission {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::OptIn => "opt_in",
            Self::BaselineWarn => "baseline_warn",
        }
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct ToolchainOracle {
    pub(crate) clippy: String,
    pub(crate) rustc: String,
    pub(crate) cargo: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RuleOracle {
    pub(crate) id: String,
    pub(crate) category: String,
    pub(crate) help: String,
    pub(crate) clippy_default: ClippyDefault,
    pub(crate) message: String,
    pub(crate) positive_fixture: String,
    pub(crate) positive_span: ExpectedSpan,
    pub(crate) integration_span: ExpectedSpan,
}

impl RuleOracle {
    pub(crate) fn admission(&self) -> SignalAdmission {
        self.clippy_default.admission()
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct FindingOracle {
    pub(crate) id: String,
    pub(crate) message: String,
    pub(crate) path: String,
    pub(crate) span: Option<ExpectedSpan>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub(crate) struct ExpectedSpan {
    pub(crate) line_start: u64,
    pub(crate) column_start: u64,
    pub(crate) line_end: u64,
    pub(crate) column_end: u64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PrecisionMatrixOracle {
    pub(crate) rules: Vec<PrecisionRuleOracle>,
    pub(crate) contexts: PrecisionContexts,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PrecisionRuleOracle {
    pub(crate) id: String,
    pub(crate) positives: Vec<PrecisionPositiveOracle>,
    pub(crate) negatives: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PrecisionPositiveOracle {
    pub(crate) case: String,
    pub(crate) span: ExpectedSpan,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PrecisionContexts {
    pub(crate) build_output_candidate_diagnostics: usize,
    pub(crate) local_macro_contract: String,
    pub(crate) external_expansion_contract: String,
    pub(crate) missing_primary_span_contract: String,
    pub(crate) unicode_primary_span: ExpectedSpan,
    pub(crate) non_unix_permissions: NonUnixPermissionsOracle,
}

#[derive(Debug, Deserialize)]
pub(crate) struct NonUnixPermissionsOracle {
    pub(crate) target: String,
    pub(crate) fixture: String,
    pub(crate) candidate_diagnostics: usize,
    pub(crate) primary_span: ExpectedSpan,
}

#[derive(Debug)]
pub(crate) struct RuleScalingEvidence {
    pub(crate) catalog: RuleScalingOracle,
    pub(crate) precision: PrecisionMatrixOracle,
}

#[derive(Debug)]
pub(crate) struct PrecisionObservation {
    pub(crate) rules: Vec<PrecisionRuleObservation>,
    pub(crate) build_output_candidate_diagnostics: usize,
    pub(crate) non_unix_permissions: NonUnixPermissionsObservation,
}

impl PrecisionObservation {
    pub(crate) fn rule(&self, id: &str) -> &PrecisionRuleObservation {
        self.rules
            .iter()
            .find(|rule| rule.id == id)
            .expect("every precision rule should have an observation")
    }
}

#[derive(Debug)]
pub(crate) struct PrecisionRuleObservation {
    pub(crate) id: String,
    pub(crate) spans: Vec<ExpectedSpan>,
    pub(crate) tp: usize,
    pub(crate) fp: usize,
    pub(crate) tn: usize,
    pub(crate) r#fn: usize,
}

impl PrecisionRuleObservation {
    pub(crate) fn passed(&self) -> bool {
        self.fp == 0 && self.r#fn == 0
    }
}

#[derive(Debug)]
pub(crate) struct NonUnixPermissionsObservation {
    pub(crate) spans: Vec<ExpectedSpan>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "reason")]
pub(crate) enum CargoJsonRecord {
    #[serde(rename = "compiler-message")]
    CompilerMessage { message: CargoDiagnostic },
    #[serde(rename = "build-finished")]
    BuildFinished { success: bool },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CargoDiagnostic {
    pub(crate) code: Option<CargoDiagnosticCode>,
    pub(crate) spans: Vec<CargoDiagnosticSpan>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CargoDiagnosticCode {
    pub(crate) code: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CargoDiagnosticSpan {
    pub(crate) file_name: String,
    pub(crate) is_primary: bool,
    pub(crate) line_start: u64,
    pub(crate) column_start: u64,
    pub(crate) line_end: u64,
    pub(crate) column_end: u64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ObservedContexts {
    pub(crate) source_allow: Vec<String>,
    pub(crate) source_deny: Vec<String>,
    pub(crate) local_macro: Vec<ObservedSpan>,
    pub(crate) external_macro_under_no_deps: Vec<ObservedSpan>,
    pub(crate) build_output_candidate_codes: Vec<String>,
    pub(crate) dependency_direct_candidate_codes: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ObservedSpan {
    pub(crate) id: String,
    pub(crate) primary_line: u64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CompatibilityOracle {
    pub(crate) git_change_scope_output_hashes: BTreeMap<String, BTreeMap<String, String>>,
    pub(crate) persistent_configuration_output_hashes: BTreeMap<String, BTreeMap<String, String>>,
    pub(crate) policy_disabled_clippy_rules: BTreeMap<String, Vec<String>>,
    pub(crate) policy_clippy_pruning: ClippyPruningOracle,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct ClippyPruningOracle {
    pub(crate) inactive_rules: Vec<String>,
    pub(crate) scan_command: Option<Vec<String>>,
    pub(crate) process_count: usize,
    pub(crate) counter_test: String,
}

pub(crate) fn oracle() -> RuleScalingOracle {
    serde_json::from_str(include_str!("../fixtures/rule-scaling-kernel/oracle.json"))
        .expect("rule scaling oracle should be valid JSON")
}

pub(crate) fn evidence() -> RuleScalingEvidence {
    let catalog = oracle();
    let precision: PrecisionMatrixOracle = serde_json::from_str(include_str!(
        "../fixtures/rule-scaling-kernel/matrix/oracle.json"
    ))
    .expect("precision matrix oracle should be valid JSON");
    let precision_ids = precision
        .rules
        .iter()
        .map(|rule| rule.id.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(precision_ids.len(), precision.rules.len());
    assert_eq!(precision_ids, catalog.candidate_ids());
    RuleScalingEvidence { catalog, precision }
}

pub(crate) fn run_clippy(manifest: &Path, arguments: &[&str], target: &Path) -> Output {
    Command::new(env!("CARGO"))
        .arg("clippy")
        .arg("--manifest-path")
        .arg(manifest)
        .args(arguments)
        .current_dir(
            manifest
                .parent()
                .expect("fixture manifest should have a parent"),
        )
        .env("CARGO_NET_OFFLINE", "true")
        .env("CARGO_TARGET_DIR", target)
        .output()
        .expect("fixture Clippy process should start")
}

pub(crate) fn cargo_json_records(output: &Output) -> Result<Vec<CargoJsonRecord>, String> {
    std::str::from_utf8(&output.stdout)
        .map_err(|error| format!("Cargo JSON output should be UTF-8: {error}"))?
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            serde_json::from_str::<CargoJsonRecord>(line)
                .map_err(|error| format!("Cargo JSON record {} is invalid: {error}", index + 1))
        })
        .collect()
}

pub(crate) fn compiler_messages(output: &Output) -> Vec<CargoDiagnostic> {
    cargo_json_records(output)
        .expect("Cargo output should contain only valid JSON records")
        .into_iter()
        .filter_map(|record| match record {
            CargoJsonRecord::CompilerMessage { message } => Some(message),
            CargoJsonRecord::BuildFinished { .. } | CargoJsonRecord::Other => None,
        })
        .collect()
}

pub(crate) fn primary_span(message: &CargoDiagnostic) -> ExpectedSpan {
    let primary = message
        .spans
        .iter()
        .filter(|span| span.is_primary)
        .collect::<Vec<_>>();
    assert_eq!(primary.len(), 1);
    assert_eq!(primary[0].file_name, "src/lib.rs");
    ExpectedSpan {
        line_start: primary[0].line_start,
        column_start: primary[0].column_start,
        line_end: primary[0].line_end,
        column_end: primary[0].column_end,
    }
}

pub(crate) fn observe_precision(
    evidence: &RuleScalingEvidence,
    target_root: &Path,
) -> PrecisionObservation {
    let manifest_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let matrix_manifest =
        manifest_root.join("tests/fixtures/rule-scaling-kernel/matrix/Cargo.toml");
    let mut arguments = vec!["--lib", "--no-deps", "--message-format=json", "--"];
    arguments.extend(evidence.catalog.explicit_flags());
    let output = run_clippy(&matrix_manifest, &arguments, &target_root.join("matrix"));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let messages = compiler_messages(&output);
    let candidate_ids = evidence.catalog.candidate_ids();
    let mut observed = BTreeMap::<String, Vec<ExpectedSpan>>::new();
    for message in &messages {
        let Some(id) = message.code.as_ref().map(|code| code.code.as_str()) else {
            continue;
        };
        if candidate_ids.contains(id) {
            observed
                .entry(id.to_owned())
                .or_default()
                .push(primary_span(message));
        }
    }

    let rules = evidence
        .precision
        .rules
        .iter()
        .map(|rule| {
            let mut spans = observed.remove(&rule.id).unwrap_or_default();
            spans.sort();
            let mut unmatched = spans.clone();
            let mut tp = 0;
            for expected in rule.positives.iter().map(|case| &case.span) {
                if let Some(index) = unmatched.iter().position(|span| span == expected) {
                    unmatched.remove(index);
                    tp += 1;
                }
            }
            let fp = unmatched.len();
            PrecisionRuleObservation {
                id: rule.id.clone(),
                spans,
                tp,
                fp,
                tn: rule.negatives.len().saturating_sub(fp),
                r#fn: rule.positives.len() - tp,
            }
        })
        .collect::<Vec<_>>();
    assert!(observed.is_empty(), "unexpected candidate observations");

    let build_output_candidate_diagnostics = messages
        .iter()
        .filter(|message| {
            message
                .spans
                .iter()
                .any(|span| span.file_name == "build.rs")
                && message
                    .code
                    .as_ref()
                    .map(|code| code.code.as_str())
                    .is_some_and(|id| candidate_ids.contains(id))
        })
        .count();

    let non_unix = &evidence.precision.contexts.non_unix_permissions;
    let non_unix_manifest = manifest_root.join(&non_unix.fixture);
    let non_unix_output = run_clippy(
        &non_unix_manifest,
        &[
            "--lib",
            "--target",
            &non_unix.target,
            "--no-deps",
            "--message-format=json",
            "--",
            "-W",
            "clippy::permissions_set_readonly_false",
        ],
        &target_root.join("non-unix"),
    );
    assert!(
        non_unix_output.status.success(),
        "{}",
        String::from_utf8_lossy(&non_unix_output.stderr)
    );
    let mut non_unix_spans = compiler_messages(&non_unix_output)
        .iter()
        .filter(|message| {
            message
                .code
                .as_ref()
                .is_some_and(|code| code.code == "clippy::permissions_set_readonly_false")
        })
        .map(primary_span)
        .collect::<Vec<_>>();
    non_unix_spans.sort();

    PrecisionObservation {
        rules,
        build_output_candidate_diagnostics,
        non_unix_permissions: NonUnixPermissionsObservation {
            spans: non_unix_spans,
        },
    }
}

pub(crate) fn clippy_command_without_rules(
    command: &[String],
    disabled_rules: &[String],
) -> Vec<String> {
    let mut projected = Vec::with_capacity(command.len());
    let mut arguments = command.iter().peekable();
    while let Some(argument) = arguments.next() {
        if argument == "-W"
            && arguments
                .peek()
                .is_some_and(|rule| disabled_rules.contains(rule))
        {
            arguments.next();
        } else {
            projected.push(argument.clone());
        }
    }
    projected
}

/// Identifiants du catalogue historique EP-017, les seuls que portent les
/// rapports v7 et v6 figés. Tout ce que les tranches suivantes ont admis est
/// retiré par la projection, sans quoi chaque élargissement du catalogue
/// déplacerait un octet gelé.
pub(crate) const HISTORICAL_RULE_IDS: [&str; 7] = [
    "clippy::dbg_macro",
    "clippy::todo",
    "clippy::unimplemented",
    "rust_doctor::cargo::unbounded_registry_dependency",
    "rust_doctor::cargo::unpinned_git_dependency",
    "rust_doctor::source::disabled_tls_verification",
    "rust_doctor::source::dynamic_shell_command",
];

pub(crate) fn project_legacy_report(mut report: Value, oracle: &RuleScalingOracle) -> Value {
    let _ = oracle;
    let candidate_ids = |id: &str| !HISTORICAL_RULE_IDS.contains(&id);
    report["policy"]["rules"]
        .as_array_mut()
        .expect("policy rules should be an array")
        .retain(|rule| !rule["id"].as_str().is_some_and(candidate_ids));

    let command = report["scan"]["command"]
        .as_array_mut()
        .expect("scan command should be an array");
    let mut current = std::mem::take(command).into_iter().peekable();
    while let Some(argument) = current.next() {
        if argument == "-W"
            && current
                .peek()
                .and_then(Value::as_str)
                .is_some_and(candidate_ids)
        {
            current.next();
        } else {
            command.push(argument);
        }
    }
    report
}
