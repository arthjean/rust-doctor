use crate::catalog::{RuleCatalog, RuleDescriptor, built_in_catalog};
use crate::config::{self, ResolvedConfig};
use crate::diagnostics::{
    CanonicalDiagnostic, CheckStatus, DiagnosticLocation, ReportV1, ScanMode,
};
use crate::{diff, discovery, scan, suppression};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

#[derive(Debug)]
pub struct WhyRequest {
    pub directory: PathBuf,
    pub location: String,
    pub rule: Option<String>,
    pub offline: bool,
    pub max_duration: Option<Duration>,
    pub no_project_config: bool,
}

#[derive(Debug, Serialize)]
pub struct WhyReport {
    pub location: String,
    pub findings: Vec<WhyFinding>,
    pub decisions: Vec<WhyDecision>,
    pub unavailable_evidence: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct WhyFinding {
    pub diagnostic: CanonicalDiagnostic,
    pub matched_evidence: String,
    pub severity_resolution: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inline_suppression: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct WhyDecision {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule: Option<String>,
    pub explanation: String,
}

#[derive(Debug, thiserror::Error)]
pub enum WhyError {
    #[error("invalid source location '{input}': {reason}")]
    InvalidLocation { input: String, reason: String },
    #[error("invalid source path '{}': {reason}", path.display())]
    InvalidPath { path: PathBuf, reason: String },
    #[error("source path '{}' is outside project root '{}'", path.display(), root.display())]
    OutsideRoot { path: PathBuf, root: PathBuf },
    #[error("project bootstrap failed: {0}")]
    Bootstrap(#[from] crate::error::BootstrapError),
    #[error("project discovery failed: {0}")]
    Discovery(#[from] crate::error::DiscoveryError),
    #[error("configuration failed: {0}")]
    Config(#[from] crate::error::ConfigError),
    #[error("scope resolution failed: {0}")]
    Scope(#[from] crate::error::DiffError),
    #[error("scan failed: {0}")]
    Scan(#[from] crate::error::ScanError),
    #[error("rule catalog is unavailable: {0}")]
    Catalog(String),
    #[error("unknown rule '{rule}'. Nearest rules: {suggestions}")]
    UnknownRule { rule: String, suggestions: String },
}

#[derive(Debug)]
struct ParsedLocation {
    path: PathBuf,
    line: u32,
    column: Option<u32>,
}

#[expect(
    clippy::too_many_lines,
    reason = "the why workflow keeps validation, scoped analysis, and explanation assembly in one transaction"
)]
pub fn execute(request: &WhyRequest) -> Result<WhyReport, WhyError> {
    let parsed = parse_location(&request.location)?;
    let requested_root =
        request
            .directory
            .canonicalize()
            .map_err(|error| WhyError::InvalidPath {
                path: request.directory.clone(),
                reason: error.to_string(),
            })?;
    let source = canonical_source(&requested_root, &parsed)?;
    validate_source_position(&source, &parsed)?;
    if !source.starts_with(&requested_root) {
        return Err(WhyError::OutsideRoot {
            path: source,
            root: requested_root,
        });
    }

    let (_, project, discovered_file_config) =
        discovery::bootstrap_project(&requested_root, request.offline)?;
    let project_root = project
        .root_dir
        .canonicalize()
        .map_err(|error| WhyError::InvalidPath {
            path: project.root_dir.clone(),
            reason: error.to_string(),
        })?;
    if !source.starts_with(&project_root) {
        return Err(WhyError::OutsideRoot {
            path: source,
            root: project_root,
        });
    }
    let relative = source
        .strip_prefix(&project_root)
        .map_err(|_| WhyError::OutsideRoot {
            path: source.clone(),
            root: project_root.clone(),
        })?
        .to_path_buf();

    let file_config = if request.no_project_config {
        None
    } else {
        discovered_file_config
    };
    let resolved = config::resolve_config_defaults(file_config.as_ref());
    let catalog = built_in_catalog().map_err(|error| WhyError::Catalog(error.to_string()))?;
    let analysis_config = explanation_config(&resolved, catalog);
    let selected_descriptor = request
        .rule
        .as_deref()
        .map(|rule| resolve_requested_rule(catalog, rule))
        .transpose()?;
    let scope = diff::resolve_scope(
        &project.root_dir,
        &diff::ScopeRequest {
            reporting_scope: diff::ReportingScope::Files,
            base: None,
            files: vec![relative.clone()],
            include_untracked: false,
        },
        &resolved.ignore_files,
    )?;
    let normalized_relative = normalize_path(&relative);
    if let Some(scope_decision) = scope_exclusion_decision(&scope, &relative) {
        let mut decisions = policy_decisions(
            catalog.descriptors(),
            selected_descriptor.as_ref(),
            &resolved,
            &relative,
            &project,
        );
        decisions.push(scope_decision);
        add_test_context_decisions(&mut decisions, &relative);
        normalize_decisions(&mut decisions);
        return Ok(WhyReport {
            location: format_location(&normalized_relative, parsed.line, parsed.column),
            findings: Vec::new(),
            decisions,
            unavailable_evidence: Vec::new(),
        });
    }
    let control =
        crate::process::ScanControl::new(Arc::new(AtomicBool::new(false)), request.max_duration);
    let result = scan::scan_project_scoped(
        &project,
        &analysis_config,
        request.offline,
        &[],
        true,
        &scope,
        &control,
    )?;
    let report = ReportV1::from_scan(&result, &project, &analysis_config, ScanMode::Files);
    let mut findings = Vec::new();
    let mut finding_decisions = Vec::new();
    for mut diagnostic in report.diagnostics {
        let requested_rule_mismatch = request.rule.as_deref().is_some_and(|rule| {
            diagnostic.rule != rule && !rule_matches(catalog, rule, &diagnostic.rule)
        });
        if !intersects(
            &diagnostic,
            &normalized_relative,
            parsed.line,
            parsed.column,
        ) || requested_rule_mismatch
        {
            continue;
        }
        let descriptor =
            catalog.resolve(&diagnostic.rule, &diagnostic.category, diagnostic.severity);
        let severity_resolution =
            resolved.rule_policy_trace(descriptor.as_descriptor(), Some(&relative));
        let effective_policy = resolved.rule_policy(descriptor.as_descriptor(), Some(&relative));
        if let Some(severity) = effective_policy.severity {
            diagnostic.severity = severity;
        } else {
            finding_decisions.push(WhyDecision {
                kind: "config_suppression".to_string(),
                rule: Some(diagnostic.rule.clone()),
                explanation: format!(
                    "the analyzer matched this location, but effective policy resolves to off: {}",
                    severity_resolution.join(" -> ")
                ),
            });
        }
        let inline_suppression = result
            .diagnostics
            .iter()
            .find(|legacy| {
                legacy.rule == diagnostic.rule
                    && legacy.line == Some(parsed.line)
                    && project_relative(&project.root_dir, &legacy.file_path) == normalized_relative
            })
            .and_then(|legacy| suppression::inline_suppression_reason(legacy, &project.root_dir));
        let matched_evidence = evidence_text(&diagnostic);
        findings.push(WhyFinding {
            diagnostic,
            matched_evidence,
            severity_resolution,
            inline_suppression,
        });
    }
    findings.sort_by(|left, right| {
        left.diagnostic
            .rule
            .cmp(&right.diagnostic.rule)
            .then(left.diagnostic.site_id.cmp(&right.diagnostic.site_id))
    });

    let mut decisions = policy_decisions(
        catalog.descriptors(),
        selected_descriptor.as_ref(),
        &resolved,
        &relative,
        &project,
    );
    decisions.append(&mut finding_decisions);
    decisions.push(WhyDecision {
        kind: "scope".to_string(),
        rule: None,
        explanation: format!("analysis reporting was restricted to {normalized_relative}"),
    });
    add_test_context_decisions(&mut decisions, &relative);
    normalize_decisions(&mut decisions);

    let unavailable_evidence = unavailable_required_evidence(&result.execution.checks);

    Ok(WhyReport {
        location: format_location(&normalized_relative, parsed.line, parsed.column),
        findings,
        decisions,
        unavailable_evidence,
    })
}

fn explanation_config(resolved: &ResolvedConfig, catalog: &RuleCatalog) -> ResolvedConfig {
    let mut analysis = resolved.clone();
    analysis.lint = true;
    analysis.adapter_policy.compiler_lint = true;
    analysis.adapter_policy.custom_ast = true;
    analysis.respect_inline_disables = false;
    analysis.ignore_rules.clear();
    analysis.enable_rules.clear();
    analysis.rules_config.clear();
    analysis.category_config.clear();
    analysis.tag_config.clear();
    analysis.path_overrides.clear();
    for descriptor in catalog.descriptors() {
        analysis.rules_config.insert(
            descriptor.canonical_id.clone(),
            config::RuleConfig {
                severity: Some(config::RuleLevel::from(descriptor.default_severity)),
                ..config::RuleConfig::default()
            },
        );
    }
    analysis
}

fn scope_exclusion_decision(scope: &diff::ScopePlan, path: &Path) -> Option<WhyDecision> {
    if !scope.paths.contains(path) {
        return Some(WhyDecision {
            kind: "scope_exclusion".to_string(),
            rule: None,
            explanation: format!(
                "{} is excluded by the effective ignore-file policy",
                normalize_path(path)
            ),
        });
    }
    if !scope.rust_files.contains(path) {
        return Some(WhyDecision {
            kind: "scope_exclusion".to_string(),
            rule: None,
            explanation: format!(
                "{} is not a Rust source file and no Rust analyzer was started",
                normalize_path(path)
            ),
        });
    }
    None
}

fn normalize_decisions(decisions: &mut Vec<WhyDecision>) {
    decisions.sort_by(|left, right| {
        left.kind
            .cmp(&right.kind)
            .then(left.rule.cmp(&right.rule))
            .then(left.explanation.cmp(&right.explanation))
    });
    decisions.dedup_by(|left, right| {
        left.kind == right.kind && left.rule == right.rule && left.explanation == right.explanation
    });
}

pub fn render_terminal(report: &WhyReport) {
    println!("Why {}", report.location);
    if report.findings.is_empty() {
        if report.unavailable_evidence.is_empty() {
            println!("No intersecting finding was emitted for this location.");
        } else {
            println!("No conclusion: required evidence is unavailable for this location.");
        }
    }
    for finding in &report.findings {
        println!(
            "{} {}: {}",
            finding.diagnostic.severity, finding.diagnostic.rule, finding.diagnostic.message
        );
        println!("  evidence: {}", finding.matched_evidence);
        println!("  confidence: {}", finding.diagnostic.confidence);
        println!("  severity: {}", finding.severity_resolution.join(" -> "));
        if let Some(reason) = &finding.inline_suppression {
            println!("  suppression: {reason}");
        }
        if let Some(help) = &finding.diagnostic.help {
            println!("  fix: {help}");
        } else if !finding.diagnostic.fixes.is_empty() {
            println!(
                "  fix: {} structured edit(s) available",
                finding.diagnostic.fixes.len()
            );
        }
    }
    for decision in &report.decisions {
        println!(
            "{}{}: {}",
            decision.kind,
            decision
                .rule
                .as_ref()
                .map_or_else(String::new, |rule| format!(" [{rule}]")),
            decision.explanation
        );
    }
    for unavailable in &report.unavailable_evidence {
        println!("unavailable evidence: {unavailable}");
    }
}

fn parse_location(input: &str) -> Result<ParsedLocation, WhyError> {
    let (before_last, last) = input
        .rsplit_once(':')
        .ok_or_else(|| WhyError::InvalidLocation {
            input: input.to_string(),
            reason: "expected FILE:LINE or FILE:LINE:COLUMN".to_string(),
        })?;
    let last_number = parse_position(input, last)?;
    let (path, line, column) = if let Some((path, possible_line)) = before_last.rsplit_once(':') {
        possible_line
            .parse::<u32>()
            .map_or((before_last, last_number, None), |line| {
                (path, line, Some(last_number))
            })
    } else {
        (before_last, last_number, None)
    };
    if path.is_empty() {
        return Err(WhyError::InvalidLocation {
            input: input.to_string(),
            reason: "file path is empty".to_string(),
        });
    }
    if line == 0 || column == Some(0) {
        return Err(WhyError::InvalidLocation {
            input: input.to_string(),
            reason: "line and column are one-based".to_string(),
        });
    }
    Ok(ParsedLocation {
        path: PathBuf::from(path),
        line,
        column,
    })
}

fn parse_position(input: &str, value: &str) -> Result<u32, WhyError> {
    value.parse::<u32>().map_err(|_| WhyError::InvalidLocation {
        input: input.to_string(),
        reason: format!("'{value}' is not a positive source position"),
    })
}

fn canonical_source(root: &Path, location: &ParsedLocation) -> Result<PathBuf, WhyError> {
    let candidate = if location.path.is_absolute() {
        location.path.clone()
    } else {
        root.join(&location.path)
    };
    let canonical = candidate
        .canonicalize()
        .map_err(|error| WhyError::InvalidPath {
            path: candidate.clone(),
            reason: error.to_string(),
        })?;
    if !canonical.is_file() {
        return Err(WhyError::InvalidPath {
            path: canonical,
            reason: "path is not a readable file".to_string(),
        });
    }
    Ok(canonical)
}

fn validate_source_position(path: &Path, location: &ParsedLocation) -> Result<(), WhyError> {
    let content = std::fs::read_to_string(path).map_err(|error| WhyError::InvalidPath {
        path: path.to_path_buf(),
        reason: error.to_string(),
    })?;
    let line = content
        .lines()
        .nth(location.line.saturating_sub(1) as usize)
        .ok_or_else(|| WhyError::InvalidLocation {
            input: format_location(&normalize_path(path), location.line, location.column),
            reason: "line is outside the file".to_string(),
        })?;
    if let Some(column) = location.column
        && column as usize > line.chars().count().saturating_add(1)
    {
        return Err(WhyError::InvalidLocation {
            input: format_location(&normalize_path(path), location.line, location.column),
            reason: "column is outside the line".to_string(),
        });
    }
    Ok(())
}

fn resolve_requested_rule(catalog: &RuleCatalog, rule: &str) -> Result<RuleDescriptor, WhyError> {
    if let Some(descriptor) = catalog.exact(rule) {
        return Ok(descriptor.clone());
    }
    if let Some((category, severity)) = dynamic_rule_defaults(rule) {
        return Ok(catalog
            .resolve(rule, &category, severity)
            .as_descriptor()
            .clone());
    }
    Err(WhyError::UnknownRule {
        rule: rule.to_string(),
        suggestions: nearest_rules(catalog.descriptors(), rule).join(", "),
    })
}

fn dynamic_rule_defaults(
    rule: &str,
) -> Option<(crate::diagnostics::Category, crate::diagnostics::Severity)> {
    use crate::diagnostics::{Category, Severity};
    if rule
        .strip_prefix("clippy::")
        .is_some_and(valid_dynamic_code)
    {
        Some((Category::Style, Severity::Warning))
    } else if valid_rustsec_id(rule) {
        Some((Category::Security, Severity::Error))
    } else if rule.strip_prefix("deny::").is_some_and(valid_dynamic_code) {
        Some((Category::Dependencies, Severity::Warning))
    } else if rule
        .strip_prefix('E')
        .is_some_and(|code| code.len() == 4 && code.bytes().all(|byte| byte.is_ascii_digit()))
    {
        Some((Category::Correctness, Severity::Error))
    } else {
        None
    }
}

fn valid_dynamic_code(code: &str) -> bool {
    !code.is_empty()
        && code.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        })
}

fn valid_rustsec_id(rule: &str) -> bool {
    let Some(value) = rule.strip_prefix("RUSTSEC-") else {
        return false;
    };
    let mut parts = value.split('-');
    matches!(
        (parts.next(), parts.next(), parts.next()),
        (Some(year), Some(sequence), None)
            if year.len() == 4
                && sequence.len() == 4
                && year.bytes().all(|byte| byte.is_ascii_digit())
                && sequence.bytes().all(|byte| byte.is_ascii_digit())
    )
}

fn nearest_rules(descriptors: &[RuleDescriptor], needle: &str) -> Vec<String> {
    let mut candidates: Vec<_> = descriptors
        .iter()
        .map(|descriptor| {
            (
                edit_distance(&descriptor.canonical_id, needle),
                descriptor.canonical_id.clone(),
            )
        })
        .collect();
    candidates.sort();
    candidates
        .into_iter()
        .take(5)
        .map(|(_, value)| value)
        .collect()
}

fn edit_distance(left: &str, right: &str) -> usize {
    let mut row: Vec<usize> = (0..=right.chars().count()).collect();
    for (left_index, left_char) in left.chars().enumerate() {
        let mut diagonal = left_index;
        row[0] = left_index + 1;
        for (right_index, right_char) in right.chars().enumerate() {
            let above = row[right_index + 1];
            let cost = usize::from(left_char != right_char);
            row[right_index + 1] = (row[right_index] + 1).min(above + 1).min(diagonal + cost);
            diagonal = above;
        }
    }
    row.last().copied().unwrap_or(0)
}

fn policy_decisions(
    descriptors: &[RuleDescriptor],
    selected: Option<&RuleDescriptor>,
    resolved: &ResolvedConfig,
    path: &Path,
    project: &discovery::ProjectInfo,
) -> Vec<WhyDecision> {
    let package = package_for_path(project, path);
    let frameworks: Vec<String> = package
        .map_or(project.frameworks.as_slice(), |member| {
            member.frameworks.as_slice()
        })
        .iter()
        .map(ToString::to_string)
        .map(|value| value.to_ascii_lowercase())
        .collect();
    let mut decisions = Vec::new();
    for descriptor in descriptors {
        if selected.is_some_and(|selected| selected.canonical_id != descriptor.canonical_id) {
            continue;
        }
        let trace = resolved.rule_policy_trace(descriptor, Some(path));
        let policy = resolved.rule_policy(descriptor, Some(path));
        if policy.severity.is_none() && (selected.is_some() || trace.len() > 1) {
            decisions.push(WhyDecision {
                kind: "config_suppression".to_string(),
                rule: Some(descriptor.canonical_id.clone()),
                explanation: trace.join(" -> "),
            });
        }
        if !descriptor.applicable_frameworks.is_empty()
            && !descriptor
                .applicable_frameworks
                .iter()
                .any(|framework| frameworks.contains(&framework.to_ascii_lowercase()))
        {
            decisions.push(WhyDecision {
                kind: "framework_gate".to_string(),
                rule: Some(descriptor.canonical_id.clone()),
                explanation: format!(
                    "requires one of [{}], detected [{}]",
                    descriptor.applicable_frameworks.join(", "),
                    frameworks.join(", ")
                ),
            });
        }
        if selected.is_some() {
            decisions.extend(framework_context_decisions(
                descriptor,
                package.map_or(project.framework_capabilities.as_slice(), |member| {
                    member.framework_capabilities.as_slice()
                }),
            ));
        }
    }
    decisions
}

fn framework_context_decisions(
    descriptor: &RuleDescriptor,
    capabilities: &[discovery::FrameworkCapability],
) -> Vec<WhyDecision> {
    let mut decisions = Vec::new();
    for requirement in &descriptor.framework_requirements {
        for capability in capabilities
            .iter()
            .filter(|capability| capability.framework.to_string() == requirement.framework)
        {
            decisions.push(WhyDecision {
                kind: if capability.active {
                    "framework_context"
                } else {
                    "framework_gate"
                }
                .to_string(),
                rule: Some(descriptor.canonical_id.clone()),
                explanation: format!(
                    "{} version={} features=[{}] targets=[{}] analyzed_target={}{}",
                    requirement.framework,
                    capability.version.as_deref().unwrap_or("unresolved"),
                    capability.enabled_features.join(", "),
                    capability.target_contexts.join(", "),
                    capability
                        .analyzed_target
                        .as_deref()
                        .unwrap_or("unresolved"),
                    capability
                        .gate_reason
                        .as_ref()
                        .map_or_else(String::new, |reason| format!(" gate={reason}"))
                ),
            });
        }
    }
    if let Some(rule) = crate::rules::all_custom_rules()
        .into_iter()
        .find(|rule| rule.name() == descriptor.canonical_id)
        && !rule.applicable_frameworks().is_empty()
    {
        let gate = crate::rules::framework_packs::capability_decision(rule.as_ref(), capabilities);
        decisions.push(WhyDecision {
            kind: if gate.is_ok() {
                "framework_rule_context"
            } else {
                "framework_rule_gate"
            }
            .to_string(),
            rule: Some(descriptor.canonical_id.clone()),
            explanation: gate.map_or_else(
                |reason| format!("rule gate inactive: {reason}"),
                |()| "rule version, feature, and target gate is active".to_string(),
            ),
        });
    }
    decisions
}

fn package_for_path<'a>(
    project: &'a discovery::ProjectInfo,
    path: &Path,
) -> Option<&'a discovery::WorkspaceMember> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        project.root_dir.join(path)
    };
    project
        .workspace_members
        .iter()
        .filter(|member| absolute.starts_with(&member.root_dir))
        .max_by_key(|member| member.root_dir.components().count())
}

fn unavailable_required_evidence(checks: &[crate::diagnostics::CheckState]) -> Vec<String> {
    let mut unavailable: Vec<_> = checks
        .iter()
        .filter(|check| check.required && check.status != CheckStatus::Completed)
        .map(|check| {
            check.reason.as_ref().map_or_else(
                || format!("{}: {:?}", check.name, check.status),
                |reason| format!("{}: {:?} ({reason})", check.name, check.status),
            )
        })
        .collect();
    unavailable.sort();
    unavailable.dedup();
    unavailable
}

fn add_test_context_decisions(decisions: &mut Vec<WhyDecision>, path: &Path) {
    let normalized = normalize_path(path);
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    if normalized.starts_with("tests/")
        || normalized.contains("/tests/")
        || file_name.starts_with("test_")
        || file_name.ends_with("_test.rs")
    {
        decisions.push(WhyDecision {
            kind: "test_context_policy".to_string(),
            rule: None,
            explanation:
                "test-context rules and Clippy restriction lints may be suppressed before reporting"
                    .to_string(),
        });
    }
}

fn intersects(
    diagnostic: &CanonicalDiagnostic,
    path: &str,
    line: u32,
    column: Option<u32>,
) -> bool {
    let DiagnosticLocation::Source {
        path: diagnostic_path,
        range,
    } = &diagnostic.location
    else {
        return false;
    };
    if diagnostic_path != path || line < range.start.line || line > range.end.line {
        return false;
    }
    column.is_none_or(|column| {
        (line > range.start.line || column >= range.start.column)
            && (line < range.end.line || column <= range.end.column)
    })
}

fn rule_matches(catalog: &RuleCatalog, requested: &str, actual: &str) -> bool {
    catalog
        .exact(requested)
        .is_some_and(|descriptor| descriptor.canonical_id == actual)
}

fn evidence_text(diagnostic: &CanonicalDiagnostic) -> String {
    match &diagnostic.location {
        DiagnosticLocation::Source { path, range } => format!(
            "{}:{}:{} through {}:{}",
            path, range.start.line, range.start.column, range.end.line, range.end.column
        ),
        DiagnosticLocation::Project => "project-level evidence".to_string(),
    }
}

fn project_relative(root: &Path, path: &Path) -> String {
    let relative = if path.is_absolute() {
        path.strip_prefix(root).unwrap_or(path)
    } else {
        path
    };
    normalize_path(relative)
}

fn normalize_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn format_location(path: &str, line: u32, column: Option<u32>) -> String {
    column.map_or_else(
        || format!("{path}:{line}"),
        |column| format!("{path}:{line}:{column}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_paths_with_colons_from_the_right() {
        let location = parse_location("C:\\project\\src\\lib.rs:12:7").unwrap();
        assert_eq!(location.path, PathBuf::from("C:\\project\\src\\lib.rs"));
        assert_eq!(location.line, 12);
        assert_eq!(location.column, Some(7));
    }

    #[test]
    fn rejects_zero_and_missing_lines() {
        assert!(parse_location("src/lib.rs:0").is_err());
        assert!(parse_location("src/lib.rs").is_err());
    }

    #[test]
    fn edit_distance_drives_deterministic_suggestions() {
        assert_eq!(edit_distance("unwrap", "unwarp"), 2);
    }

    #[test]
    fn dynamic_namespaces_are_accepted_but_arbitrary_unknown_rules_are_not() {
        let catalog = built_in_catalog().unwrap();
        assert_eq!(
            resolve_requested_rule(catalog, "clippy::future_lint")
                .unwrap()
                .canonical_id,
            "clippy::future_lint"
        );
        assert!(resolve_requested_rule(catalog, "not-a-real-rule").is_err());
    }

    #[test]
    fn optional_unavailable_checks_do_not_make_why_inconclusive() {
        let checks = vec![
            crate::diagnostics::CheckState {
                name: "custom rules".to_string(),
                required: true,
                status: CheckStatus::Completed,
                reason: None,
            },
            crate::diagnostics::CheckState {
                name: "cargo audit".to_string(),
                required: false,
                status: CheckStatus::Skipped,
                reason: Some("not installed".to_string()),
            },
        ];
        assert!(unavailable_required_evidence(&checks).is_empty());
    }
}
