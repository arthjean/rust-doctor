//! `why`: the findings a report holds at one line, how each is set, and what
//! silences it.
//!
//! It reads a report as JSON, the one a scan just produced or one saved with
//! `--json`, so the two paths answer from the same document. Nothing here
//! spawns a process: whether a path is declared generated is handed in.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::str::FromStr;

use serde::Serialize;
use serde_json::Value;

use crate::generated::has_generator_header;
use crate::path_glob::PathGlob;
use crate::presentation::{GroupLocation, code_frame, rule_url};
use crate::terminal_text::{sanitize, sanitize_multiline};
use crate::DiagnosticSpan;

/// How far above a finding a directive can sit and still be the one meant
/// for it: an attribute over a short item, or a comment over a line.
const DIRECTIVE_REACH: usize = 3;

/// What a generator header check reads of a file: its opening lines.
const HEADER_BYTES: u64 = 4096;

/// `path:line[:column]`, split from the right so a path may hold a colon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WhyLocation {
    pub path: String,
    pub line: usize,
    pub column: Option<usize>,
}

impl FromStr for WhyLocation {
    type Err = &'static str;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        const USAGE: &str = "expected PATH:LINE[:COLUMN] with a positive LINE";
        let positive = |field: &str| field.parse::<usize>().ok().filter(|value| *value > 0);
        let (head, last) = text.rsplit_once(':').ok_or(USAGE)?;
        let (path, line, column) = match head.rsplit_once(':') {
            Some((path, line)) if line.parse::<i64>().is_ok() => {
                (path, positive(line).ok_or(USAGE)?, Some(positive(last).ok_or(USAGE)?))
            }
            _ => (head, positive(last).ok_or(USAGE)?, None),
        };
        let path = path.strip_prefix("./").unwrap_or(path);
        if path.is_empty() {
            return Err(USAGE);
        }
        Ok(Self {
            path: path.to_owned(),
            line,
            column,
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SiteExplanation {
    pub location: WhyLocation,
    pub hits: Vec<WhyHit>,
    /// Why nothing is reported there, when nothing is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub absence: Option<Absence>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WhyHit {
    pub rule_id: String,
    pub severity: String,
    /// Where the rule's level came from, as the report's policy published it.
    pub level_source: Option<String>,
    pub message: String,
    pub help: Option<String>,
    pub url: String,
    pub path: String,
    pub span: DiagnosticSpan,
    /// The directives near the site, and what became of each.
    pub suppressions: Vec<WhySuppression>,
    /// The directive to write, when no directive names the rule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub directive: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WhySuppression {
    pub path: String,
    pub line: usize,
    pub rules: Vec<String>,
    pub status: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Absence {
    /// `[ignore] paths` covers the file.
    Ignored,
    /// `.gitattributes` or a generator header marks it.
    GeneratedOrVendored,
    NothingReported,
}

/// The diagnostics of `report` whose primary span or related location covers
/// the line, or why there is none. `declared` holds the paths the repository
/// declares generated or vendored.
pub fn explain_site(
    report: &Value,
    location: WhyLocation,
    workspace_root: &Path,
    declared: &BTreeSet<String>,
) -> SiteExplanation {
    let hits: Vec<WhyHit> = array(report, "diagnostics")
        .filter(|diagnostic| covers(diagnostic, &location))
        .filter_map(|diagnostic| hit(report, diagnostic, &location))
        .collect();
    let absence = hits.is_empty().then(|| {
        let ignored = array(report, "/policy/ignore")
            .filter_map(Value::as_str)
            .filter_map(|glob| PathGlob::parse(glob).ok())
            .any(|glob| glob.matches(&location.path));
        if ignored {
            Absence::Ignored
        } else if declared.contains(&location.path) || header_marks(workspace_root, &location.path) {
            Absence::GeneratedOrVendored
        } else {
            Absence::NothingReported
        }
    });
    SiteExplanation {
        location,
        hits,
        absence,
    }
}

fn array<'a>(report: &'a Value, pointer: &str) -> impl Iterator<Item = &'a Value> {
    let pointer = if pointer.starts_with('/') { pointer.to_owned() } else { format!("/{pointer}") };
    report
        .pointer(&pointer)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

fn span_of(value: &Value) -> Option<DiagnosticSpan> {
    serde_json::from_value(value.get("span")?.clone()).ok()
}

fn covers(diagnostic: &Value, location: &WhyLocation) -> bool {
    let at = |value: &Value| {
        value.get("path").and_then(Value::as_str) == Some(location.path.as_str())
            && span_of(value).is_some_and(|span| {
                (span.line_start..=span.line_end.max(span.line_start)).contains(&location.line)
            })
    };
    at(diagnostic) || array(diagnostic, "related").any(at)
}

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

/// A string field of the report, through the gate every renderer draws
/// workspace text through: a saved report is any file the reader names, so
/// nothing it holds may reach the terminal as an escape sequence.
fn field(value: &Value, key: &str) -> Option<String> {
    text(value, key).map(sanitize)
}

/// A directive counts as near when it sits within reach above the finding's
/// own span, or above the line asked about when a related location covers it.
fn hit(report: &Value, diagnostic: &Value, location: &WhyLocation) -> Option<WhyHit> {
    let rule_id = field(diagnostic, "code")?;
    let path = field(diagnostic, "path")?;
    let span = span_of(diagnostic)?;
    let level_source = array(report, "/policy/rules")
        .find(|rule| text(rule, "id") == Some(rule_id.as_str()))
        .and_then(|rule| field(rule, "source"));
    let anchors = [
        (path.as_str(), span.line_start, span.line_end),
        (location.path.as_str(), location.line, location.line),
    ];
    let suppressions: Vec<WhySuppression> = array(report, "suppressions")
        .filter_map(|directive| {
            let at = text(directive, "path")?;
            let line = usize::try_from(directive.get("line")?.as_u64()?).ok()?;
            let near = anchors.iter().any(|(path, start, end)| {
                *path == at && line + DIRECTIVE_REACH >= *start && line <= *end
            });
            near.then(|| WhySuppression {
                path: sanitize(at),
                line,
                rules: array(directive, "rules")
                    .filter_map(Value::as_str)
                    .map(sanitize)
                    .collect(),
                status: field(directive, "status").unwrap_or_default(),
            })
        })
        .collect();
    let named = suppressions
        .iter()
        .any(|directive| directive.rules.contains(&rule_id));
    Some(WhyHit {
        severity: field(diagnostic, "severity").unwrap_or_default(),
        level_source,
        message: text(diagnostic, "message").map(sanitize_multiline).unwrap_or_default(),
        help: text(diagnostic, "help").map(sanitize_multiline),
        url: rule_url(&rule_id),
        directive: if named { None } else { directive_for(&rule_id, &path) },
        rule_id,
        path,
        span,
        suppressions,
    })
}

/// The suppression a site of `rule_id` takes: a native rule's comment, a
/// manifest's `#` comment, or an `#[expect]` for a Clippy or rustc lint.
///
/// `None` unless the id is a lint path: `::`-separated, non-empty segments of
/// ASCII letters, digits and `_`, the charset the Markdown report trusts. A
/// saved report is any file the reader names, and a code holding `)]` would
/// otherwise close the attribute and hand the reader Rust to paste.
fn directive_for(rule_id: &str, path: &str) -> Option<String> {
    let lint_path = rule_id.split("::").all(|segment| {
        !segment.is_empty()
            && segment
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    });
    if !lint_path {
        return None;
    }
    Some(if rule_id.starts_with("rust_doctor::") {
        let marker = if path.ends_with(".rs") { "//" } else { "#" };
        format!("{marker} rust-doctor: allow({rule_id}) -- <why this site is not a defect>")
    } else {
        format!("#[expect({rule_id}, reason = \"<why this site is not a defect>\")]")
    })
}

fn header_marks(workspace_root: &Path, path: &str) -> bool {
    let Ok(file) = File::open(workspace_root.join(path)) else {
        return false;
    };
    let mut head = Vec::new();
    file.take(HEADER_BYTES).read_to_end(&mut head).is_ok()
        && has_generator_header(&String::from_utf8_lossy(&head))
}

impl SiteExplanation {
    /// The answer as `why` prints it: every hit with its code frame, read
    /// under `workspace_root` and masked like every frame the reports draw.
    pub fn text(&self, workspace_root: &Path) -> String {
        let location = &self.location;
        let path = sanitize(&location.path);
        let mut out = String::new();
        match self.absence {
            Some(Absence::Ignored) => {
                let _ = writeln!(out, "{path} is ignored by rust-doctor.toml: nothing there is reported.");
            }
            Some(Absence::GeneratedOrVendored) => {
                let _ = writeln!(out, "{path} is declared generated or vendored: nothing there is graded.");
            }
            Some(Absence::NothingReported) => {
                let _ = writeln!(out, "Nothing is reported at {path}:{}.", location.line);
            }
            None => {}
        }
        for hit in &self.hits {
            let _ = writeln!(out, "{} ({})", hit.rule_id, hit.severity);
            if let Some(source) = &hit.level_source {
                let _ = writeln!(out, "Level source: {source}");
            }
            let _ = writeln!(out, "{}", hit.message);
            if let Some(help) = &hit.help {
                let _ = writeln!(out, "Help: {help}");
            }
            let _ = writeln!(out, "Rule: {}", hit.url);
            frame(&mut out, workspace_root, hit);
            for directive in &hit.suppressions {
                let _ = writeln!(
                    out,
                    "Suppression at {}:{}: {} ({})",
                    directive.path,
                    directive.line,
                    directive.rules.join(", "),
                    directive.status
                );
            }
            if let Some(directive) = &hit.directive {
                let _ = writeln!(out, "To suppress this site: {directive}");
            }
            let _ = writeln!(out);
        }
        out
    }
}

fn frame(out: &mut String, workspace_root: &Path, hit: &WhyHit) {
    let location = GroupLocation {
        path: hit.path.clone(),
        span: hit.span.clone(),
    };
    let Ok(frame) = code_frame(workspace_root, &location) else {
        return;
    };
    let _ = writeln!(out, "{}", frame.location);
    let gutter = frame.gutter_width();
    for line in frame.lines {
        let prefix = if line.primary { ">" } else { " " };
        let _ = writeln!(out, "{prefix} {:>gutter$} | {}", line.number, line.text);
        if let Some(marker) = line.marker {
            let carets = marker.column_end.saturating_sub(marker.column_start).max(1);
            let _ = writeln!(
                out,
                "  {:gutter$} | {}{}",
                "",
                " ".repeat(marker.column_start.saturating_sub(1)),
                "^".repeat(carets)
            );
        }
    }
}
