//! One diagnostic per finding, whichever producer raised it.
//!
//! The five producers publish five shapes and the report publishes one. Each
//! `normalize_*` states what identity its producer's findings are paired on,
//! since that is the only thing that separates them: a span-bearing
//! diagnostic keeps its fingerprint while its line stays put, a manifest-level
//! one keeps it whatever moves above the key, and a structural one keeps it
//! whatever its message counts.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::path::Path;

use cargo_metadata::Metadata;
use serde_json::Value;

use super::sanitize::{HomePaths, normalize_text, sanitize_text};
use super::{
    Diagnostic, DiagnosticContext, DiagnosticSource, DiagnosticSpan,
    RelatedLocation, Severity,
};
use crate::cargo_health;
use crate::execution::{CapturedDiagnostic, CapturedMessage, CapturedSpan, CompilerMessageData};
use crate::policy::{self, PolicyPlan, RuleLevel};
use crate::source_kernel;
use crate::structure;
use crate::workspace_path;

/// Cargo's own stream, merged.
///
/// A catalogued rule the plan switched off is dropped here rather than by the
/// producer: Clippy is asked for every catalogued lint under `-W`, and only
/// the plan knows the reader turned one back off afterwards. An uncatalogued
/// warning is kept, and costs the score its authoritative flag downstream.
pub(super) fn merge_compiler_messages(
    merged: &mut BTreeMap<String, Diagnostic>,
    messages: &[CapturedMessage],
    workspace_root: &Path,
    metadata: Option<&Metadata>,
    home: &HomePaths,
    plan: &PolicyPlan,
) {
    for message in messages {
        let CapturedMessage::Compiler(message) = message else {
            continue;
        };
        if message
            .message
            .code
            .as_ref()
            .is_some_and(|code| policy::find(&code.code).is_some() && !plan.is_active(&code.code))
        {
            continue;
        }
        merge_diagnostic(
            merged,
            normalize_diagnostic(message, workspace_root, metadata, home),
        );
    }
}

/// The normalization of Cargo's stream alone, for the tests that exercise
/// identity, merging and ordering without assembling a whole execution.
#[cfg(test)]
pub(super) fn normalize_diagnostics(
    messages: &[CapturedMessage],
    workspace_root: Option<&Path>,
    metadata: Option<&Metadata>,
    home: &HomePaths,
) -> Vec<Diagnostic> {
    let Some(workspace_root) = workspace_root else {
        return Vec::new();
    };
    let plan = PolicyPlan::default();
    let mut merged = BTreeMap::<String, Diagnostic>::new();
    if let Some(metadata) = metadata {
        for candidate in &cargo_health::inspect(metadata, &plan).candidates {
            merge_diagnostic(
                &mut merged,
                normalize_cargo_health_candidate(candidate, workspace_root, home),
            );
        }
    }
    merge_compiler_messages(
        &mut merged,
        messages,
        workspace_root,
        metadata,
        home,
        &plan,
    );
    let mut diagnostics: Vec<_> = merged.into_values().collect();
    diagnostics.sort_by(compare_diagnostics);
    diagnostics
}

/// A dependency-pack candidate, on the manifest-level path: the identity
/// deliberately ignores the span, so a finding keeps its baseline fingerprint
/// when lines are inserted above the key that raised it.
pub(super) fn normalize_cargo_health_candidate(
    candidate: &cargo_health::Candidate,
    workspace_root: &Path,
    home: &HomePaths,
) -> Diagnostic {
    let definition = candidate.definition;
    let source = DiagnosticSource::RustDoctor;
    let code = Some(definition.id.to_owned());
    let message = sanitize_text(&candidate.message, Some(workspace_root), home);
    let path = candidate
        .manifest_path
        .as_deref()
        .and_then(|path| workspace_path::normalize_relative(Path::new(path)));
    let id = fingerprint(
        source,
        code.as_deref(),
        path.as_deref(),
        None,
        canonical_severity(definition.default_level),
        &message,
    );

    Diagnostic {
        id,
        source,
        // Native diagnostic: no Cargo target carries it.
        context: None,
        code,
        base_severity: canonical_severity(definition.default_level),
        severity: canonical_severity(definition.default_level),
        category: Some(definition.category.to_owned()),
        message,
        help: Some(definition.help.to_owned()),
        package: Some(normalize_text(&candidate.package)),
        target: None,
        path,
        span: candidate.span.map(|span| DiagnosticSpan {
            line_start: span.line_start,
            column_start: span.column_start,
            line_end: span.line_end,
            column_end: span.column_end,
        }),
        related: Vec::new(),
        similarity_basis_points: None,
        complexity: None,
        occurrences: 1,
    }
}

/// A native detector's candidate, whose identity is its span and its message,
/// exactly like a Clippy diagnostic's.
pub(super) fn normalize_source_candidate(
    candidate: &source_kernel::Candidate,
    workspace_root: &Path,
    home: &HomePaths,
) -> Diagnostic {
    let definition = candidate.definition;
    let source = DiagnosticSource::RustDoctor;
    let code = Some(definition.id.to_owned());
    let path = workspace_path::normalize_relative(Path::new(&candidate.path));
    let span = Some(DiagnosticSpan {
        line_start: candidate.span.line_start,
        column_start: candidate.span.column_start,
        line_end: candidate.span.line_end,
        column_end: candidate.span.column_end,
    });
    let message = sanitize_text(candidate.message, Some(workspace_root), home);
    let id = fingerprint(
        source,
        code.as_deref(),
        path.as_deref(),
        span.as_ref(),
        canonical_severity(definition.default_level),
        &message,
    );
    Diagnostic {
        id,
        source,
        // Native diagnostic: no Cargo target carries it.
        context: None,
        code,
        base_severity: canonical_severity(definition.default_level),
        severity: canonical_severity(definition.default_level),
        category: Some(definition.category.to_owned()),
        message,
        help: Some(definition.help.to_owned()),
        package: candidate.package.as_deref().map(normalize_text),
        target: candidate.target.as_deref().map(normalize_text),
        path,
        span,
        related: Vec::new(),
        similarity_basis_points: None,
        complexity: None,
        occurrences: 1,
    }
}

/// A structural finding, on the path the native detectors already take. What
/// differs is its identity: `fingerprint_of` reads the structural hash the pass
/// computed instead of the span and the message, so inserting fifty lines above
/// a finding renames nothing.
pub(super) fn normalize_structure_finding(
    finding: &structure::StructureFinding,
    workspace_root: &Path,
    home: &HomePaths,
) -> Diagnostic {
    let definition = finding.definition;
    let source = DiagnosticSource::RustDoctor;
    let code = Some(definition.id.to_owned());
    let path = workspace_path::normalize_relative(Path::new(&finding.path));
    let span = Some(DiagnosticSpan {
        line_start: finding.span.line_start,
        column_start: finding.span.column_start,
        line_end: finding.span.line_end,
        column_end: finding.span.column_end,
    });
    let message = sanitize_text(&finding.message, Some(workspace_root), home);
    // The identity of a structural finding is its family, and a family is not a
    // file: `duplicate_function_body` gathers members across the workspace, and
    // the path published beside it is only the first of them in sorted order.
    // Feeding that path into the identity would retire the finding and publish a
    // new one the day a member sorts ahead of it, while the normalized key of a
    // per-file family already carries its path.
    let id = fingerprint_of(
        source,
        code.as_deref(),
        None,
        canonical_severity(definition.default_level),
        &FingerprintBody::Structure(&finding.structure),
    );
    Diagnostic {
        id,
        source,
        context: finding.context,
        code,
        base_severity: canonical_severity(definition.default_level),
        severity: canonical_severity(definition.default_level),
        category: Some(definition.category.to_owned()),
        message,
        help: Some(definition.help.to_owned()),
        package: finding.package.as_deref().map(normalize_text),
        target: finding.target.as_deref().map(normalize_text),
        path,
        span,
        related: finding
            .related
            .iter()
            .filter_map(|location| {
                Some(RelatedLocation {
                    path: workspace_path::normalize_relative(Path::new(&location.path))?,
                    span: DiagnosticSpan {
                        line_start: location.span.line_start,
                        column_start: location.span.column_start,
                        line_end: location.span.line_end,
                        column_end: location.span.column_end,
                    },
                })
            })
            .collect(),
        similarity_basis_points: finding.similarity,
        complexity: finding.complexity,
        occurrences: finding.occurrences,
    }
}

/// A repository finding, on the manifest-level path: its identity ignores the
/// span exactly as the dependency pack's does, so a credential finding keeps
/// its baseline fingerprint when lines are inserted above it.
pub(super) fn normalize_repo_finding(
    finding: &crate::repo_hygiene::RepoFinding,
    workspace_root: &Path,
    home: &HomePaths,
) -> Diagnostic {
    let definition = finding.definition;
    let source = DiagnosticSource::RustDoctor;
    let code = Some(definition.id.to_owned());
    // The pass publishes an already normalized path, so normalizing again
    // would escape a `%` twice and hand the consumer a name that decodes back
    // to a file that does not exist.
    let path = Some(finding.path.clone());
    let message = sanitize_text(&finding.message, Some(workspace_root), home);
    let id = fingerprint(
        source,
        code.as_deref(),
        path.as_deref(),
        None,
        canonical_severity(definition.default_level),
        &message,
    );
    Diagnostic {
        id,
        source,
        context: finding.context,
        code,
        base_severity: canonical_severity(definition.default_level),
        severity: canonical_severity(definition.default_level),
        category: Some(definition.category.to_owned()),
        message,
        help: Some(definition.help.to_owned()),
        // No Cargo package owns a repository-level finding.
        package: None,
        target: None,
        path,
        span: finding.span.map(|span| DiagnosticSpan {
            line_start: span.line_start,
            column_start: span.column_start,
            line_end: span.line_end,
            column_end: span.column_end,
        }),
        related: Vec::new(),
        similarity_basis_points: None,
        complexity: None,
        occurrences: 1,
    }
}

pub(super) fn merge_diagnostic(diagnostics: &mut BTreeMap<String, Diagnostic>, diagnostic: Diagnostic) {
    match diagnostics.get_mut(&diagnostic.id) {
        Some(existing) => {
            existing.occurrences += diagnostic.occurrences;
            merge_optional_context(&mut existing.package, diagnostic.package);
            merge_optional_context(&mut existing.target, diagnostic.target);
            merge_optional_context(&mut existing.context, diagnostic.context);
        }
        None => {
            diagnostics.insert(diagnostic.id.clone(), diagnostic);
        }
    }
}

fn normalize_diagnostic(
    captured: &CompilerMessageData,
    workspace_root: &Path,
    metadata: Option<&Metadata>,
    home: &HomePaths,
) -> Diagnostic {
    let rule = captured
        .message
        .code
        .as_ref()
        .and_then(|code| policy::find(&code.code));
    let code = captured
        .message
        .code
        .as_ref()
        .map(|code| normalize_text(&code.code));
    let source = match code.as_deref() {
        Some(code) if code.starts_with("clippy::") => DiagnosticSource::Clippy,
        _ => DiagnosticSource::Rustc,
    };
    let severity = severity(&captured.message.level);
    let message = sanitize_text(&captured.message.message, Some(workspace_root), home);
    let (path, span) = select_primary_span(&captured.message, workspace_root);
    let package = metadata.and_then(|metadata| {
        metadata
            .packages
            .iter()
            .find(|package| package.id.repr == captured.package_id)
            .map(|package| package.name.to_string())
    });
    let target = Some(normalize_text(&captured.target.name));
    let context = DiagnosticContext::from_target_kinds(&captured.target.kind);
    let id = fingerprint(
        source,
        code.as_deref(),
        path.as_deref(),
        span.as_ref(),
        severity,
        &message,
    );

    Diagnostic {
        id,
        source,
        code,
        base_severity: severity,
        severity,
        context,
        category: rule.map(|rule| rule.category.to_owned()),
        message,
        help: rule.map(|rule| rule.help.to_owned()),
        package,
        target,
        path,
        span,
        related: Vec::new(),
        similarity_basis_points: None,
        complexity: None,
        occurrences: 1,
    }
}

/// Two occurrences of the same diagnostic that disagree on an optional field
/// leave it empty: publishing either of the two values would assert a
/// provenance the occurrences contradict.
fn merge_optional_context<T: PartialEq>(existing: &mut Option<T>, incoming: Option<T>) {
    if existing.as_ref() != incoming.as_ref() {
        *existing = None;
    }
}

fn severity(level: &str) -> Severity {
    match level {
        "error" | "failure-note" | "error: internal compiler error" => Severity::Error,
        "warning" => Severity::Warning,
        "note" | "help" => Severity::Info,
        _ => Severity::Unknown,
    }
}

pub(super) fn canonical_severity(level: RuleLevel) -> Severity {
    match level {
        RuleLevel::Warn => Severity::Warning,
        RuleLevel::Error => Severity::Error,
        RuleLevel::Off => Severity::Unknown,
    }
}

fn select_primary_span(
    diagnostic: &CapturedDiagnostic,
    workspace_root: &Path,
) -> (Option<String>, Option<DiagnosticSpan>) {
    let mut spans: Vec<_> = diagnostic
        .spans
        .iter()
        .filter(|span| span.is_primary)
        .map(|span| normalized_span(span, workspace_root))
        .collect();
    spans.sort_by(compare_spans);
    spans.into_iter().next().unwrap_or((None, None))
}

fn normalized_span(
    span: &CapturedSpan,
    workspace_root: &Path,
) -> (Option<String>, Option<DiagnosticSpan>) {
    (
        workspace_path::normalize(workspace_root, Path::new(&span.file_name)),
        Some(DiagnosticSpan {
            line_start: span.line_start,
            column_start: span.column_start,
            line_end: span.line_end,
            column_end: span.column_end,
        }),
    )
}

fn compare_spans(
    left: &(Option<String>, Option<DiagnosticSpan>),
    right: &(Option<String>, Option<DiagnosticSpan>),
) -> Ordering {
    compare_optional_paths(&left.0, &right.0)
        .then_with(|| span_coordinates(left.1.as_ref()).cmp(&span_coordinates(right.1.as_ref())))
}

pub(super) fn compare_diagnostics(left: &Diagnostic, right: &Diagnostic) -> Ordering {
    compare_optional_paths(&left.path, &right.path)
        .then_with(|| span_start(left.span.as_ref()).cmp(&span_start(right.span.as_ref())))
        .then_with(|| left.severity.rank().cmp(&right.severity.rank()))
        .then_with(|| left.code.cmp(&right.code))
        .then_with(|| left.id.cmp(&right.id))
}

pub(super) fn compare_optional_paths(left: &Option<String>, right: &Option<String>) -> Ordering {
    match (left, right) {
        (Some(left), Some(right)) => left.cmp(right),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn span_start(span: Option<&DiagnosticSpan>) -> (usize, usize) {
    span.map_or((usize::MAX, usize::MAX), |span| {
        (span.line_start, span.column_start)
    })
}

fn span_coordinates(span: Option<&DiagnosticSpan>) -> (usize, usize, usize, usize) {
    span.map_or((usize::MAX, usize::MAX, usize::MAX, usize::MAX), |span| {
        (
            span.line_start,
            span.column_start,
            span.line_end,
            span.column_end,
        )
    })
}

/// What identifies a finding beyond its rule and its file.
///
/// A per-site diagnostic is identified by where it is and what it says. A
/// structural finding cannot be: its position moves whenever anything above it
/// moves, and it spans several sites at once, so it is identified by the
/// normalized content the pass hashed. Both go through one definition, so there
/// is one fingerprint to freeze in an oracle rather than two.
enum FingerprintBody<'a> {
    Position {
        span: Option<&'a DiagnosticSpan>,
        message: &'a str,
    },
    Structure(&'a str),
}

impl FingerprintBody<'_> {
    /// Fields the tuple carries after the path, severity included, so the
    /// historical order `span, base_severity, message` stays byte for byte
    /// what it was.
    fn render(&self, base_severity: Severity) -> String {
        let severity = json_string(base_severity.as_str());
        match self {
            Self::Position { span, message } => {
                let span = span.map_or_else(
                    || "null".to_owned(),
                    |span| {
                        format!(
                            concat!(
                                "{{\"line_start\":{},\"column_start\":{},",
                                "\"line_end\":{},\"column_end\":{}}}"
                            ),
                            span.line_start, span.column_start, span.line_end, span.column_end
                        )
                    },
                );
                format!("{span},{severity},{}", json_string(message))
            }
            Self::Structure(hash) => format!("{},{severity}", json_string(hash)),
        }
    }
}

pub(super) fn fingerprint(
    source: DiagnosticSource,
    code: Option<&str>,
    path: Option<&str>,
    span: Option<&DiagnosticSpan>,
    base_severity: Severity,
    message: &str,
) -> String {
    fingerprint_of(
        source,
        code,
        path,
        base_severity,
        &FingerprintBody::Position { span, message },
    )
}

fn fingerprint_of(
    source: DiagnosticSource,
    code: Option<&str>,
    path: Option<&str>,
    base_severity: Severity,
    body: &FingerprintBody<'_>,
) -> String {
    let tuple = format!(
        "[{},{},{},{}]",
        json_string(source.as_str()),
        json_optional_string(code),
        json_optional_string(path),
        body.render(base_severity),
    );
    blake3::hash(tuple.as_bytes()).to_hex().to_string()
}

fn json_optional_string(value: Option<&str>) -> String {
    value.map_or_else(|| "null".to_owned(), json_string)
}

fn json_string(value: &str) -> String {
    Value::String(value.to_owned()).to_string()
}
