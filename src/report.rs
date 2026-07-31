use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::env;
use std::ffi::OsStr;
use std::fmt;
use std::path::{Component, Path, PathBuf};

use cargo_metadata::Metadata;
use serde::Serialize;
use serde_json::Value;

use crate::cargo_health;
use crate::execution::{
    CapturedDiagnostic, CapturedMessage, CapturedSpan, CompilerMessageData, ExecutionResult,
    InternalError, ScanExecution,
};
use crate::rules;

pub const SCHEMA_VERSION: u8 = 3;

#[derive(Debug, Clone)]
pub struct InspectRequest {
    pub path: PathBuf,
}

impl InspectRequest {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

impl Default for InspectRequest {
    fn default() -> Self {
        Self::new(".")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InspectReport {
    pub schema_version: u8,
    pub status: Status,
    pub complete: bool,
    pub project: Option<ProjectReport>,
    pub toolchain: ToolchainReport,
    pub scan: ScanReport,
    pub diagnostics: Vec<Diagnostic>,
    pub errors: Vec<ReportError>,
    pub summary: Summary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Complete,
    Incomplete,
    Failed,
}

impl Status {
    pub const fn exit_code(self) -> u8 {
        match self {
            Self::Complete => 0,
            Self::Incomplete => 1,
            Self::Failed => 2,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Incomplete => "incomplete",
            Self::Failed => "failed",
        }
    }
}

impl fmt::Display for Status {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProjectReport {
    pub workspace_root: String,
    pub manifest_path: String,
    pub packages: Vec<PackageReport>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PackageReport {
    pub name: String,
    pub manifest_path: Option<String>,
    pub targets: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolchainReport {
    pub rustc: Option<String>,
    pub cargo: Option<String>,
    pub clippy: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScanReport {
    pub command: Option<Vec<String>>,
    pub exit_code: Option<i32>,
    pub build_finished: Option<bool>,
    pub noise_lines: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Diagnostic {
    pub id: String,
    pub source: DiagnosticSource,
    pub code: Option<String>,
    pub severity: Severity,
    pub category: Option<String>,
    pub message: String,
    pub help: Option<String>,
    pub package: Option<String>,
    pub target: Option<String>,
    pub path: Option<String>,
    pub span: Option<DiagnosticSpan>,
    pub occurrences: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DiagnosticSource {
    Rustc,
    Clippy,
    #[serde(rename = "rust-doctor")]
    RustDoctor,
}

impl DiagnosticSource {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Rustc => "rustc",
            Self::Clippy => "clippy",
            Self::RustDoctor => "rust-doctor",
        }
    }
}

impl fmt::Display for DiagnosticSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
    Info,
    Unknown,
}

impl Severity {
    const fn rank(self) -> u8 {
        match self {
            Self::Error => 0,
            Self::Warning => 1,
            Self::Info => 2,
            Self::Unknown => 3,
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Info => "info",
            Self::Unknown => "unknown",
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiagnosticSpan {
    pub line_start: usize,
    pub column_start: usize,
    pub line_end: usize,
    pub column_end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReportError {
    pub stage: String,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Summary {
    pub errors: usize,
    pub warnings: usize,
    pub info: usize,
    pub unknown: usize,
    pub total: usize,
}

#[derive(Debug, Default)]
struct HomePaths {
    lexical: Option<String>,
    canonical: Option<String>,
}

impl HomePaths {
    fn from_path(path: Option<PathBuf>) -> Self {
        let canonical = path
            .as_ref()
            .and_then(|path| path.canonicalize().ok())
            .map(|path| path.to_string_lossy().into_owned());
        Self {
            lexical: path.map(|path| path.to_string_lossy().into_owned()),
            canonical,
        }
    }
}

pub(crate) fn from_execution(result: ExecutionResult) -> InspectReport {
    let workspace_root = result
        .metadata
        .as_ref()
        .map(|metadata| metadata.workspace_root.as_std_path());
    let home = home_paths();
    let status = classify(&result);
    let diagnostics = match (status, result.scan.as_ref()) {
        (Status::Failed, _) | (_, None) => Vec::new(),
        (_, Some(scan)) => normalize_diagnostics(
            &scan.messages,
            workspace_root,
            result.metadata.as_ref(),
            &home,
        ),
    };
    let summary = summarize(&diagnostics);
    let project = project_report(result.manifest_path.as_deref(), result.metadata.as_ref());
    let scan = scan_report(result.scan.as_ref());
    let errors = report_errors(&result, workspace_root, &home);

    InspectReport {
        schema_version: SCHEMA_VERSION,
        status,
        complete: status == Status::Complete,
        project,
        toolchain: ToolchainReport {
            rustc: result
                .toolchain
                .rustc
                .as_deref()
                .map(|value| sanitize_text(value, workspace_root, &home)),
            cargo: result
                .toolchain
                .cargo
                .as_deref()
                .map(|value| sanitize_text(value, workspace_root, &home)),
            clippy: result
                .toolchain
                .clippy
                .as_deref()
                .map(|value| sanitize_text(value, workspace_root, &home)),
        },
        scan,
        diagnostics,
        errors,
        summary,
    }
}

fn classify(result: &ExecutionResult) -> Status {
    if result.error.is_some() {
        return Status::Failed;
    }

    let Some(scan) = result.scan.as_ref() else {
        return Status::Failed;
    };
    if scan.exit_success == Some(true)
        && scan.build_finished == Some(true)
        && scan.malformed_messages == 0
        && scan.errors.is_empty()
    {
        Status::Complete
    } else {
        Status::Incomplete
    }
}

fn project_report(
    manifest_path: Option<&Path>,
    metadata: Option<&Metadata>,
) -> Option<ProjectReport> {
    let metadata = metadata?;
    let workspace_root = metadata.workspace_root.as_std_path();
    let manifest_path = manifest_path
        .and_then(|path| normalize_path(workspace_root, path))
        .unwrap_or_else(|| "Cargo.toml".to_owned());
    let mut packages: Vec<_> = metadata
        .packages
        .iter()
        .filter(|package| metadata.workspace_members.contains(&package.id))
        .map(|package| {
            let manifest_path = normalize_path(workspace_root, package.manifest_path.as_std_path());
            let mut targets: Vec<_> = package
                .targets
                .iter()
                .map(|target| target.name.to_string())
                .collect();
            targets.sort();
            PackageReport {
                name: package.name.to_string(),
                manifest_path,
                targets,
            }
        })
        .collect();
    packages.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| compare_optional_paths(&left.manifest_path, &right.manifest_path))
    });

    Some(ProjectReport {
        workspace_root: ".".to_owned(),
        manifest_path,
        packages,
    })
}

fn scan_report(scan: Option<&ScanExecution>) -> ScanReport {
    match scan {
        Some(scan) => ScanReport {
            command: Some(scan.command.clone()),
            exit_code: scan.exit_code,
            build_finished: scan.build_finished,
            noise_lines: Some(scan.noise_lines),
        },
        None => ScanReport {
            command: None,
            exit_code: None,
            build_finished: None,
            noise_lines: None,
        },
    }
}

fn report_errors(
    result: &ExecutionResult,
    workspace_root: Option<&Path>,
    home: &HomePaths,
) -> Vec<ReportError> {
    let mut errors = Vec::new();
    if let Some(error) = result.error.as_ref() {
        errors.push(normalize_error(error, workspace_root, home));
    }
    if let Some(scan) = result.scan.as_ref() {
        errors.extend(
            scan.errors
                .iter()
                .map(|error| normalize_error(error, workspace_root, home)),
        );
        match scan.exit_code {
            Some(code) if code != 0 || scan.exit_success != Some(true) => {
                errors.push(ReportError {
                    stage: "execution".to_owned(),
                    code: "clippy-exit".to_owned(),
                    message: format!("Clippy exited with status {code}"),
                });
            }
            None => errors.push(ReportError {
                stage: "execution".to_owned(),
                code: "clippy-exit".to_owned(),
                message: "Clippy terminated without an exit code".to_owned(),
            }),
            Some(_) => {}
        }
        match scan.build_finished {
            Some(false) => errors.push(ReportError {
                stage: "execution".to_owned(),
                code: "build-failed".to_owned(),
                message: "Cargo reported build-finished.success: false".to_owned(),
            }),
            None => errors.push(ReportError {
                stage: "execution".to_owned(),
                code: "build-finished-missing".to_owned(),
                message: "Cargo did not emit build-finished".to_owned(),
            }),
            Some(true) => {}
        }
        if scan.malformed_messages > 0 {
            errors.push(ReportError {
                stage: "parsing".to_owned(),
                code: "malformed-message".to_owned(),
                message: "malformed Cargo message".to_owned(),
            });
        }
    }
    errors.sort_by(|left, right| {
        (&left.stage, &left.code, &left.message).cmp(&(&right.stage, &right.code, &right.message))
    });
    errors.dedup_by(|left, right| left == right);
    errors
}

fn normalize_error(
    error: &InternalError,
    workspace_root: Option<&Path>,
    home: &HomePaths,
) -> ReportError {
    ReportError {
        stage: error.stage.to_owned(),
        code: error.code.to_owned(),
        message: sanitize_text(&error.message, workspace_root, home),
    }
}

fn normalize_diagnostics(
    messages: &[CapturedMessage],
    workspace_root: Option<&Path>,
    metadata: Option<&Metadata>,
    home: &HomePaths,
) -> Vec<Diagnostic> {
    let Some(workspace_root) = workspace_root else {
        return Vec::new();
    };
    let mut diagnostics = BTreeMap::<String, Diagnostic>::new();

    if let Some(metadata) = metadata {
        for candidate in cargo_health::inspect(metadata) {
            let diagnostic = normalize_cargo_health_candidate(candidate, workspace_root, home);
            merge_diagnostic(&mut diagnostics, diagnostic);
        }
    }

    for message in messages {
        let message = match message {
            CapturedMessage::Compiler(message) => message,
            CapturedMessage::Known(message) => {
                let _ = message;
                continue;
            }
            CapturedMessage::Unknown(value) => {
                let _ = value;
                continue;
            }
        };
        let diagnostic = normalize_diagnostic(message, workspace_root, metadata, home);
        merge_diagnostic(&mut diagnostics, diagnostic);
    }

    let mut diagnostics: Vec<_> = diagnostics.into_values().collect();
    diagnostics.sort_by(compare_diagnostics);
    diagnostics
}

fn normalize_cargo_health_candidate(
    candidate: cargo_health::Candidate,
    workspace_root: &Path,
    home: &HomePaths,
) -> Diagnostic {
    let source = DiagnosticSource::RustDoctor;
    let code = Some(candidate.code.to_owned());
    let message = sanitize_text(&candidate.message, Some(workspace_root), home);
    let path = candidate
        .manifest_path
        .as_deref()
        .and_then(|path| normalize_relative_path(Path::new(path)));
    let id = fingerprint(
        source,
        code.as_deref(),
        path.as_deref(),
        None,
        candidate.severity,
        &message,
    );

    Diagnostic {
        id,
        source,
        code,
        severity: candidate.severity,
        category: Some(candidate.category.to_owned()),
        message,
        help: Some(candidate.help.to_owned()),
        package: Some(normalize_text(&candidate.package)),
        target: None,
        path,
        span: None,
        occurrences: 1,
    }
}

fn merge_diagnostic(diagnostics: &mut BTreeMap<String, Diagnostic>, diagnostic: Diagnostic) {
    match diagnostics.get_mut(&diagnostic.id) {
        Some(existing) => {
            existing.occurrences += 1;
            merge_optional_context(&mut existing.package, diagnostic.package);
            merge_optional_context(&mut existing.target, diagnostic.target);
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
        .and_then(|code| rules::find(&code.code));
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
        severity,
        category: rule.map(|rule| rule.category.to_owned()),
        message,
        help: rule.map(|rule| rule.help.to_owned()),
        package,
        target,
        path,
        span,
        occurrences: 1,
    }
}

fn merge_optional_context(existing: &mut Option<String>, incoming: Option<String>) {
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
        normalize_path(workspace_root, Path::new(&span.file_name)),
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

fn compare_diagnostics(left: &Diagnostic, right: &Diagnostic) -> Ordering {
    compare_optional_paths(&left.path, &right.path)
        .then_with(|| span_start(left.span.as_ref()).cmp(&span_start(right.span.as_ref())))
        .then_with(|| left.severity.rank().cmp(&right.severity.rank()))
        .then_with(|| left.code.cmp(&right.code))
        .then_with(|| left.id.cmp(&right.id))
}

fn compare_optional_paths(left: &Option<String>, right: &Option<String>) -> Ordering {
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

fn fingerprint(
    source: DiagnosticSource,
    code: Option<&str>,
    path: Option<&str>,
    span: Option<&DiagnosticSpan>,
    severity: Severity,
    message: &str,
) -> String {
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
    let tuple = format!(
        "[{},{},{},{},{},{}]",
        json_string(source.as_str()),
        json_optional_string(code),
        json_optional_string(path),
        span,
        json_string(severity.as_str()),
        json_string(message),
    );
    blake3::hash(tuple.as_bytes()).to_hex().to_string()
}

fn json_optional_string(value: Option<&str>) -> String {
    value.map_or_else(|| "null".to_owned(), json_string)
}

fn json_string(value: &str) -> String {
    Value::String(value.to_owned()).to_string()
}

fn summarize(diagnostics: &[Diagnostic]) -> Summary {
    let mut summary = Summary::default();
    for diagnostic in diagnostics {
        match diagnostic.severity {
            Severity::Error => summary.errors += 1,
            Severity::Warning => summary.warnings += 1,
            Severity::Info => summary.info += 1,
            Severity::Unknown => summary.unknown += 1,
        }
    }
    summary.total = diagnostics.len();
    summary
}

fn normalize_path(workspace_root: &Path, path: &Path) -> Option<String> {
    let workspace_root = lexical_normalize(workspace_root)?;
    let physical_candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace_root.join(path)
    };
    let candidate = lexical_normalize(&physical_candidate)?;
    let relative = candidate.strip_prefix(&workspace_root).ok()?;
    let canonical_workspace = workspace_root.canonicalize().ok()?;
    let existing_ancestor = existing_ancestor(&physical_candidate)?;
    let canonical_ancestor = existing_ancestor.canonicalize().ok()?;
    if !canonical_ancestor.starts_with(&canonical_workspace) {
        return None;
    }
    normalize_relative_path(relative)
}

fn normalize_relative_path(relative: &Path) -> Option<String> {
    if relative.is_absolute() {
        return None;
    }
    let components: Option<Vec<_>> = relative
        .components()
        .map(|component| match component {
            Component::Normal(value) => safe_path_component(value),
            Component::CurDir => Some(".".to_owned()),
            _ => None,
        })
        .collect();
    let normalized = components?.join("/");
    if normalized.split('/').any(|component| component == "..") {
        None
    } else if normalized.is_empty() {
        Some(".".to_owned())
    } else {
        Some(normalized)
    }
}

fn existing_ancestor(path: &Path) -> Option<&Path> {
    existing_ancestor_with(path, |candidate| candidate.symlink_metadata().map(|_| ()))
}

fn existing_ancestor_with(
    path: &Path,
    mut probe: impl FnMut(&Path) -> std::io::Result<()>,
) -> Option<&Path> {
    for ancestor in path.ancestors() {
        match probe(ancestor) {
            Ok(()) => return Some(ancestor),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return None,
        }
    }
    None
}

fn safe_path_component(value: &OsStr) -> Option<String> {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";

    let value = value.to_str()?;
    let mut encoded = String::with_capacity(value.len());
    for character in value.chars() {
        if character == '%' || character.is_control() {
            let mut buffer = [0; 4];
            for byte in character.encode_utf8(&mut buffer).bytes() {
                encoded.push('%');
                encoded.push(char::from(HEX[usize::from(byte >> 4)]));
                encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
            }
        } else {
            encoded.push(character);
        }
    }
    Some(encoded)
}

fn lexical_normalize(path: &Path) -> Option<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return None;
                }
            }
            Component::Normal(value) => normalized.push(value),
        }
    }
    Some(normalized)
}

fn sanitize_text(value: &str, workspace_root: Option<&Path>, home: &HomePaths) -> String {
    let mut value = normalize_text(value);
    if let Some(workspace_root) = workspace_root.and_then(Path::to_str)
        && !workspace_root.is_empty()
    {
        value = value.replace(workspace_root, ".");
    }
    let mut home_forms: Vec<_> = [home.lexical.as_deref(), home.canonical.as_deref()]
        .into_iter()
        .flatten()
        .filter(|path| !path.is_empty())
        .collect();
    home_forms.sort_by_key(|path| std::cmp::Reverse(path.len()));
    home_forms.dedup();
    for home in home_forms {
        value = value.replace(home, "<home>");
    }
    value
}

fn normalize_text(value: &str) -> String {
    let line_endings = value.replace("\r\n", "\n").replace('\r', "\n");
    let without_ansi = strip_ansi(&line_endings);
    without_ansi
        .split('\n')
        .map(|line| line.trim_end_matches([' ', '\t']))
        .collect::<Vec<_>>()
        .join("\n")
}

fn strip_ansi(value: &str) -> String {
    let mut characters = value.chars().peekable();
    let mut output = String::with_capacity(value.len());
    while let Some(character) = characters.next() {
        match character {
            '\u{001b}' => consume_escape(&mut characters),
            '\u{009b}' => consume_csi(&mut characters),
            '\u{009d}' => consume_control_string(&mut characters, true),
            '\u{0090}' | '\u{0098}' | '\u{009e}' | '\u{009f}' => {
                consume_control_string(&mut characters, false);
            }
            '\n' | '\t' => output.push(character),
            character if character.is_control() => {}
            _ => output.push(character),
        }
    }
    output
}

fn consume_escape(characters: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    let Some(introducer) = characters.next() else {
        return;
    };
    match introducer {
        '[' => consume_csi(characters),
        ']' => consume_control_string(characters, true),
        'P' | 'X' | '^' | '_' => consume_control_string(characters, false),
        '\u{20}'..='\u{2f}' => {
            while characters
                .next_if(|character| ('\u{20}'..='\u{2f}').contains(character))
                .is_some()
            {}
            let _ = characters.next_if(|character| ('\u{30}'..='\u{7e}').contains(character));
        }
        _ => {}
    }
}

fn consume_csi(characters: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    for character in characters.by_ref() {
        if matches!(character, '\u{0018}' | '\u{001a}')
            || ('\u{40}'..='\u{7e}').contains(&character)
        {
            break;
        }
    }
}

fn consume_control_string(
    characters: &mut std::iter::Peekable<std::str::Chars<'_>>,
    bell_terminates: bool,
) {
    while let Some(character) = characters.next() {
        if matches!(character, '\u{0018}' | '\u{001a}') {
            break;
        }
        if character == '\u{009c}' || (bell_terminates && character == '\u{0007}') {
            break;
        }
        if character == '\u{001b}' && characters.next_if(|character| *character == '\\').is_some() {
            break;
        }
    }
}

fn home_paths() -> HomePaths {
    HomePaths::from_path(env::var_os("HOME").map(PathBuf::from))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    #[cfg(unix)]
    use std::ffi::OsString;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::ffi::OsStringExt;
    #[cfg(unix)]
    use std::os::unix::fs::symlink;

    use super::*;
    use crate::execution::{CapturedDiagnosticCode, CapturedTarget, ToolchainProvenance};

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/projects")
            .join(name)
    }

    fn compiler_message(
        code: Option<&str>,
        level: &str,
        message: &str,
        path: &str,
        line: usize,
    ) -> CapturedMessage {
        CapturedMessage::Compiler(CompilerMessageData {
            package_id: "opaque-package-id".to_owned(),
            target: CapturedTarget {
                name: "example".to_owned(),
            },
            message: CapturedDiagnostic {
                message: message.to_owned(),
                code: code.map(|code| CapturedDiagnosticCode {
                    code: code.to_owned(),
                }),
                level: level.to_owned(),
                spans: vec![CapturedSpan {
                    file_name: path.to_owned(),
                    line_start: line,
                    line_end: line,
                    column_start: 2,
                    column_end: 4,
                    is_primary: true,
                }],
            },
        })
    }

    fn compiler_message_for_target(target: &str) -> CapturedMessage {
        let mut message =
            compiler_message(Some("clippy::lint"), "warning", "same", "src/lib.rs", 2);
        if let CapturedMessage::Compiler(message) = &mut message {
            message.target.name = target.to_owned();
        }
        message
    }

    fn clone_compiler_message(message: &CapturedMessage) -> CapturedMessage {
        match message {
            CapturedMessage::Compiler(message) => CapturedMessage::Compiler(CompilerMessageData {
                package_id: message.package_id.clone(),
                target: CapturedTarget {
                    name: message.target.name.clone(),
                },
                message: CapturedDiagnostic {
                    message: message.message.message.clone(),
                    code: message
                        .message
                        .code
                        .as_ref()
                        .map(|code| CapturedDiagnosticCode {
                            code: code.code.clone(),
                        }),
                    level: message.message.level.clone(),
                    spans: message
                        .message
                        .spans
                        .iter()
                        .map(|span| CapturedSpan {
                            file_name: span.file_name.clone(),
                            line_start: span.line_start,
                            line_end: span.line_end,
                            column_start: span.column_start,
                            column_end: span.column_end,
                            is_primary: span.is_primary,
                        })
                        .collect(),
                },
            }),
            _ => unreachable!(),
        }
    }

    fn next_permutation(values: &mut [usize]) -> bool {
        let Some(pivot) = (0..values.len().saturating_sub(1))
            .rev()
            .find(|&index| values[index] < values[index + 1])
        else {
            return false;
        };
        let successor = (pivot + 1..values.len())
            .rev()
            .find(|&index| values[pivot] < values[index])
            .unwrap_or(pivot);
        values.swap(pivot, successor);
        values[pivot + 1..].reverse();
        true
    }

    fn report_with_diagnostics(diagnostics: Vec<Diagnostic>) -> InspectReport {
        InspectReport {
            schema_version: SCHEMA_VERSION,
            status: Status::Complete,
            complete: true,
            project: None,
            toolchain: ToolchainReport {
                rustc: None,
                cargo: None,
                clippy: None,
            },
            scan: ScanReport {
                command: None,
                exit_code: Some(0),
                build_finished: Some(true),
                noise_lines: Some(0),
            },
            summary: summarize(&diagnostics),
            diagnostics,
            errors: Vec::new(),
        }
    }

    #[test]
    fn normalizes_text_paths_severity_and_deduplicates() {
        let workspace = fixture("clean").canonicalize().unwrap();
        let source = workspace.join("src/lib.rs");
        let raw_message = format!(
            "\u{1b}[31mmessage {} /home/person  \r\nnext\t\r",
            workspace.display()
        );
        let home = HomePaths {
            lexical: Some("/home/person".to_owned()),
            canonical: None,
        };
        let messages = vec![
            compiler_message(
                Some("clippy::needless_return"),
                "warning",
                &raw_message,
                source.to_str().unwrap(),
                3,
            ),
            compiler_message(
                Some("clippy::needless_return"),
                "warning",
                &raw_message,
                source.to_str().unwrap(),
                3,
            ),
        ];
        let diagnostics = normalize_diagnostics(&messages, Some(&workspace), None, &home);

        assert_eq!(diagnostics.len(), 1);
        let diagnostic = &diagnostics[0];
        assert_eq!(diagnostic.source, DiagnosticSource::Clippy);
        assert_eq!(diagnostic.severity, Severity::Warning);
        assert_eq!(diagnostic.category, None);
        assert_eq!(diagnostic.message, "message . <home>\nnext\n");
        assert_eq!(diagnostic.help, None);
        assert_eq!(diagnostic.path.as_deref(), Some("src/lib.rs"));
        assert_eq!(diagnostic.occurrences, 2);
        assert_eq!(diagnostic.id.len(), 64);
        let tuple = serde_json::to_vec(&(
            diagnostic.source.as_str(),
            diagnostic.code.as_deref(),
            diagnostic.path.as_deref(),
            diagnostic.span.as_ref(),
            diagnostic.severity.as_str(),
            diagnostic.message.as_str(),
        ))
        .unwrap();
        assert_eq!(diagnostic.id, blake3::hash(&tuple).to_hex().to_string());
    }

    #[test]
    fn external_paths_are_null_and_future_severity_is_unknown() {
        let messages = vec![compiler_message(
            None,
            "future-level",
            "future",
            "/outside/project/lib.rs",
            1,
        )];
        let workspace = fixture("clean").canonicalize().unwrap();
        let diagnostics =
            normalize_diagnostics(&messages, Some(&workspace), None, &HomePaths::default());

        assert_eq!(diagnostics[0].path, None);
        assert_eq!(diagnostics[0].severity, Severity::Unknown);
        assert_eq!(diagnostics[0].category, None);
        assert_eq!(diagnostics[0].help, None);
        assert!(diagnostics[0].span.is_some());
    }

    #[test]
    fn exact_curated_codes_gain_metadata_without_restamping_severity() {
        let workspace = fixture("clean").canonicalize().unwrap();
        let messages = [
            compiler_message(
                Some("clippy::todo"),
                "error",
                "toolchain-owned message",
                "src/lib.rs",
                2,
            ),
            compiler_message(
                Some("clippy::todo_suffix"),
                "warning",
                "similar code",
                "src/lib.rs",
                3,
            ),
        ];
        let diagnostics =
            normalize_diagnostics(&messages, Some(&workspace), None, &HomePaths::default());

        assert_eq!(diagnostics[0].severity, Severity::Error);
        assert_eq!(diagnostics[0].category.as_deref(), Some("correctness"));
        assert_eq!(
            diagnostics[0].help.as_deref(),
            Some(
                "Replace todo! with the intended implementation or remove the reachable placeholder."
            )
        );
        assert_eq!(diagnostics[1].category, None);
        assert_eq!(diagnostics[1].help, None);
    }

    #[test]
    fn code_normalization_cannot_turn_a_non_exact_code_into_a_curated_match() {
        let workspace = fixture("clean").canonicalize().unwrap();
        let diagnostics = normalize_diagnostics(
            &[compiler_message(
                Some("\u{1b}[31mclippy::todo\u{1b}[0m"),
                "warning",
                "similar code",
                "src/lib.rs",
                2,
            )],
            Some(&workspace),
            None,
            &HomePaths::default(),
        );

        assert_eq!(diagnostics[0].code.as_deref(), Some("clippy::todo"));
        assert_eq!(diagnostics[0].category, None);
        assert_eq!(diagnostics[0].help, None);
    }

    #[test]
    fn missing_code_or_primary_span_does_not_invent_structured_fields() {
        let workspace = fixture("clean").canonicalize().unwrap();
        let mut without_span =
            compiler_message(Some("clippy::todo"), "warning", "todo", "src/lib.rs", 2);
        if let CapturedMessage::Compiler(message) = &mut without_span {
            message.message.spans.clear();
        }
        let diagnostics = normalize_diagnostics(
            &[
                compiler_message(None, "warning", "todo", "src/lib.rs", 1),
                without_span,
            ],
            Some(&workspace),
            None,
            &HomePaths::default(),
        );

        assert_eq!(diagnostics[0].code, None);
        assert_eq!(diagnostics[0].category, None);
        assert_eq!(diagnostics[0].help, None);
        assert!(diagnostics[1].path.is_none());
        assert!(diagnostics[1].span.is_none());
        assert_eq!(diagnostics[1].category.as_deref(), Some("correctness"));
    }

    #[test]
    fn editorial_metadata_is_not_part_of_the_v1_fingerprint_tuple() {
        let identity = (
            DiagnosticSource::Clippy,
            Some("clippy::todo"),
            Some("src/lib.rs"),
            Some(DiagnosticSpan {
                line_start: 2,
                column_start: 3,
                line_end: 2,
                column_end: 10,
            }),
            Severity::Warning,
            "toolchain-owned message",
        );
        let first = fingerprint(
            identity.0,
            identity.1,
            identity.2,
            identity.3.as_ref(),
            identity.4,
            identity.5,
        );
        let second = fingerprint(
            identity.0,
            identity.1,
            identity.2,
            identity.3.as_ref(),
            identity.4,
            identity.5,
        );
        let reports = [
            ("correctness", "first editorial help", first),
            ("maintainability", "second editorial help", second),
        ];

        assert_ne!(reports[0].0, reports[1].0);
        assert_ne!(reports[0].1, reports[1].1);
        assert_eq!(reports[0].2, reports[1].2);
    }

    #[test]
    fn multiple_primary_spans_use_the_documented_canonical_order() {
        let mut message = match compiler_message(None, "error", "boom", "src/z.rs", 8) {
            CapturedMessage::Compiler(message) => message,
            _ => unreachable!(),
        };
        message.message.spans.push(CapturedSpan {
            file_name: "src/a.rs".to_owned(),
            line_start: 2,
            line_end: 2,
            column_start: 1,
            column_end: 3,
            is_primary: true,
        });
        let workspace = fixture("clean").canonicalize().unwrap();
        let diagnostics = normalize_diagnostics(
            &[CapturedMessage::Compiler(message)],
            Some(&workspace),
            None,
            &HomePaths::default(),
        );

        assert_eq!(diagnostics[0].path.as_deref(), Some("src/a.rs"));
        assert_eq!(
            diagnostics[0].span.as_ref().map(|span| span.line_start),
            Some(2)
        );
    }

    #[test]
    fn duplicate_context_conflicts_are_arrival_order_independent() {
        let workspace = fixture("clean").canonicalize().unwrap();
        let home = HomePaths::default();
        let first = normalize_diagnostics(
            &[
                compiler_message_for_target("target-a"),
                compiler_message_for_target("target-b"),
            ],
            Some(&workspace),
            None,
            &home,
        );
        let reversed = normalize_diagnostics(
            &[
                compiler_message_for_target("target-b"),
                compiler_message_for_target("target-a"),
            ],
            Some(&workspace),
            None,
            &home,
        );

        assert_eq!(first, reversed);
        assert_eq!(first[0].target, None);
        assert_eq!(first[0].occurrences, 2);
    }

    #[test]
    fn malformed_messages_make_a_started_scan_incomplete() {
        let result = ExecutionResult {
            manifest_path: None,
            metadata: None,
            toolchain: ToolchainProvenance::default(),
            scan: Some(ScanExecution {
                command: vec!["cargo".to_owned(), "clippy".to_owned()],
                exit_code: Some(0),
                exit_success: Some(true),
                build_finished: Some(true),
                noise_lines: 0,
                malformed_messages: 1,
                messages: Vec::new(),
                errors: Vec::new(),
            }),
            error: None,
        };
        let report = from_execution(result);

        assert_eq!(report.status, Status::Incomplete);
        assert_eq!(
            report
                .errors
                .iter()
                .map(|error| (
                    error.stage.as_str(),
                    error.code.as_str(),
                    error.message.as_str()
                ))
                .collect::<Vec<_>>(),
            [("parsing", "malformed-message", "malformed Cargo message")]
        );
    }

    #[test]
    fn incomplete_scan_reports_each_distinct_normative_cause_once() {
        let duplicate = InternalError {
            stage: "execution",
            code: "build-failed",
            message: "Cargo reported build-finished.success: false".to_owned(),
        };
        let result = ExecutionResult {
            manifest_path: None,
            metadata: None,
            toolchain: ToolchainProvenance::default(),
            scan: Some(ScanExecution {
                command: vec!["cargo".to_owned(), "clippy".to_owned()],
                exit_code: Some(101),
                exit_success: Some(false),
                build_finished: Some(false),
                noise_lines: 0,
                malformed_messages: 2,
                messages: Vec::new(),
                errors: vec![duplicate],
            }),
            error: None,
        };
        let report = from_execution(result);
        let errors: Vec<_> = report
            .errors
            .iter()
            .map(|error| {
                (
                    error.stage.as_str(),
                    error.code.as_str(),
                    error.message.as_str(),
                )
            })
            .collect();

        assert_eq!(report.status, Status::Incomplete);
        assert_eq!(
            errors,
            [
                (
                    "execution",
                    "build-failed",
                    "Cargo reported build-finished.success: false"
                ),
                ("execution", "clippy-exit", "Clippy exited with status 101"),
                ("parsing", "malformed-message", "malformed Cargo message"),
            ]
        );
    }

    #[test]
    fn missing_exit_and_build_finished_have_explicit_causes() {
        let result = ExecutionResult {
            manifest_path: None,
            metadata: None,
            toolchain: ToolchainProvenance::default(),
            scan: Some(ScanExecution {
                command: vec!["cargo".to_owned(), "clippy".to_owned()],
                exit_code: None,
                exit_success: None,
                build_finished: None,
                noise_lines: 0,
                malformed_messages: 0,
                messages: Vec::new(),
                errors: Vec::new(),
            }),
            error: None,
        };
        let report = from_execution(result);

        assert_eq!(report.status, Status::Incomplete);
        assert_eq!(report.errors.len(), 2);
        assert!(report.errors.iter().any(|error| {
            error.code == "clippy-exit" && error.message == "Clippy terminated without an exit code"
        }));
        assert!(report.errors.iter().any(|error| {
            error.code == "build-finished-missing"
                && error.message == "Cargo did not emit build-finished"
        }));
    }

    #[test]
    fn twenty_message_permutations_render_identically() {
        let base = [
            compiler_message(Some("E0001"), "error", "alpha", "src/e.rs", 7),
            compiler_message(Some("clippy::lint"), "warning", "beta", "src/b.rs", 3),
            compiler_message(None, "help", "gamma", "src/c.rs", 4),
            compiler_message(None, "future", "delta", "/external/d.rs", 1),
            compiler_message(None, "note", "epsilon", "src/a.rs", 2),
        ];
        let workspace = fixture("clean").canonicalize().unwrap();
        let mut expected = None;
        let mut order = [0, 1, 2, 3, 4];
        let mut seen = BTreeSet::new();

        for permutation in 0..20 {
            assert!(seen.insert(order));
            let messages: Vec<_> = order
                .iter()
                .map(|&index| clone_compiler_message(&base[index]))
                .collect();
            let diagnostics =
                normalize_diagnostics(&messages, Some(&workspace), None, &HomePaths::default());
            let mut rendered = Vec::new();
            crate::render::render_json(&report_with_diagnostics(diagnostics), &mut rendered)
                .unwrap();
            match expected.as_ref() {
                Some(expected) => assert_eq!(&rendered, expected),
                None => expected = Some(rendered),
            }
            if permutation < 19 {
                assert!(next_permutation(&mut order));
            }
        }
        assert_eq!(seen.len(), 20);
    }

    #[test]
    fn sanitizes_workspace_and_both_home_forms_from_errors() {
        let home = HomePaths {
            lexical: Some("/linked/home".to_owned()),
            canonical: Some("/real/home".to_owned()),
        };
        let sanitized = sanitize_text(
            "\u{1b}[31m/work/project failed in /linked/home and /real/home \r\n",
            Some(Path::new("/work/project")),
            &home,
        );

        assert_eq!(sanitized, ". failed in <home> and <home>\n");
    }

    #[test]
    fn lexical_home_is_redacted_when_canonicalization_fails() {
        let home =
            HomePaths::from_path(Some(PathBuf::from("/definitely/missing/rust-doctor-home")));

        assert!(home.canonical.is_none());
        assert_eq!(
            sanitize_text(
                "failed in /definitely/missing/rust-doctor-home/project",
                None,
                &home
            ),
            "failed in <home>/project"
        );
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_lexical_home_display_is_redacted() {
        let path = PathBuf::from(OsString::from_vec(
            b"/definitely/missing/rust-doctor-\xff-home".to_vec(),
        ));
        let displayed = path.to_string_lossy().into_owned();
        let home = HomePaths::from_path(Some(path));

        assert_eq!(home.lexical.as_deref(), Some(displayed.as_str()));
        assert_eq!(
            sanitize_text(&format!("failed in {displayed}/project"), None, &home),
            "failed in <home>/project"
        );
    }

    #[test]
    fn strips_ecma_48_control_sequences_and_payloads() {
        let value = concat!(
            "a\u{001b}[31mb\u{001b}[0mc",
            "\u{001b}]osc\u{0007}d",
            "\u{001b}Pdcs\u{001b}\\e",
            "\u{001b}Xsos\u{001b}\\f",
            "\u{001b}^pm\u{001b}\\g",
            "\u{001b}_apc\u{001b}\\h",
            "\u{001b}(Bi",
            "\u{009b}31mj",
            "\u{009d}osc\u{009c}k",
        );

        assert_eq!(normalize_text(value), "abcdefghijk");
    }

    #[test]
    fn ecma_48_can_and_sub_cancel_sequences_without_consuming_following_text() {
        let value = concat!(
            "a\u{001b}[31\u{0018}b",
            "\u{001b}Pdiscarded\u{001a}c",
            "\u{009b}32\u{0018}d",
            "\u{009d}discarded\u{001a}e",
        );

        assert_eq!(normalize_text(value), "abcde");
    }

    #[test]
    fn control_characters_in_internal_paths_are_encoded_before_rendering() {
        let workspace = fixture("clean").canonicalize().unwrap();
        let diagnostics = normalize_diagnostics(
            &[compiler_message(
                Some("clippy::lint"),
                "warning",
                "message",
                "src/100%\u{001b}[31mline\n.rs",
                1,
            )],
            Some(&workspace),
            None,
            &HomePaths::default(),
        );

        assert_eq!(
            diagnostics[0].path.as_deref(),
            Some("src/100%25%1B[31mline%0A.rs")
        );
        let report = report_with_diagnostics(diagnostics);
        let mut terminal = Vec::new();
        crate::render::render_terminal(&report, &mut terminal).unwrap();
        let terminal = String::from_utf8(terminal).unwrap();
        assert!(!terminal.contains('\u{001b}'));
        assert!(terminal.contains("src/100%25%1B[31mline%0A.rs:1:2"));

        let mut json = Vec::new();
        crate::render::render_json(&report, &mut json).unwrap();
        let json: Value = serde_json::from_slice(&json).unwrap();
        assert_eq!(
            json["diagnostics"][0]["path"],
            "src/100%25%1B[31mline%0A.rs"
        );
    }

    #[test]
    fn physical_containment_stops_on_non_not_found_errors() {
        let mut probes = 0;
        let ancestor = existing_ancestor_with(Path::new("one/two/three.rs"), |_| {
            probes += 1;
            match probes {
                1 => Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "missing leaf",
                )),
                2 => Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "unreadable parent",
                )),
                _ => Ok(()),
            }
        });

        assert!(ancestor.is_none());
        assert_eq!(probes, 2);
    }

    #[cfg(unix)]
    #[test]
    fn paths_crossing_symlinks_outside_the_workspace_are_null() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!("report-paths-{}", std::process::id()));
        if root.exists() {
            fs::remove_dir_all(&root).unwrap();
        }
        let workspace = root.join("workspace");
        let outside = root.join("outside");
        fs::create_dir_all(workspace.join("src")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("external.rs"), "pub fn external() {}\n").unwrap();
        symlink(&outside, workspace.join("linked")).unwrap();
        symlink(
            outside.join("external.rs"),
            workspace.join("direct-link.rs"),
        )
        .unwrap();

        assert_eq!(
            normalize_path(&workspace, &workspace.join("src/future.rs")).as_deref(),
            Some("src/future.rs")
        );
        assert_eq!(
            normalize_path(&workspace, &workspace.join("linked/external.rs")),
            None
        );
        assert_eq!(
            normalize_path(&workspace, &workspace.join("direct-link.rs")),
            None
        );
        assert_eq!(
            normalize_path(&workspace, &workspace.join("linked/future.rs")),
            None
        );
        assert_eq!(
            normalize_path(&workspace, &workspace.join("linked/../outside/external.rs")),
            None
        );

        fs::remove_dir_all(root).unwrap();
    }
}
