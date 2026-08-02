use std::collections::{BTreeMap, BTreeSet};

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
    pub(crate) clippy_default: String,
    pub(crate) message: String,
    pub(crate) positive_fixture: String,
    pub(crate) positive_span: ExpectedSpan,
    pub(crate) integration_span: ExpectedSpan,
}

#[derive(Debug, Deserialize)]
pub(crate) struct FindingOracle {
    pub(crate) id: String,
    pub(crate) message: String,
    pub(crate) path: String,
    pub(crate) span: Option<ExpectedSpan>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct ExpectedSpan {
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

pub(crate) fn project_legacy_report(mut report: Value, oracle: &RuleScalingOracle) -> Value {
    let candidate_ids = oracle.candidate_ids();
    report["policy"]["rules"]
        .as_array_mut()
        .expect("policy rules should be an array")
        .retain(|rule| {
            !rule["id"]
                .as_str()
                .is_some_and(|id| candidate_ids.contains(id))
        });

    let command = report["scan"]["command"]
        .as_array_mut()
        .expect("scan command should be an array");
    let mut current = std::mem::take(command).into_iter().peekable();
    while let Some(argument) = current.next() {
        if argument == "-W"
            && current
                .peek()
                .and_then(Value::as_str)
                .is_some_and(|id| candidate_ids.contains(id))
        {
            current.next();
        } else {
            command.push(argument);
        }
    }
    report
}
