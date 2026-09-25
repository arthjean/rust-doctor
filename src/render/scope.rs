//! The scope line that opens every report, failed ones included, and the
//! notes that qualify it.

use std::io::Write;

use crate::InspectReport;
use crate::git_scope::ResolvedScope;

use super::{RenderError, Style, TerminalOptions, line, short_revision};

/// What the scan was asked to judge, against which ref, and what it left out.
pub(super) fn render_scope<W: Write>(
    writer: &mut W,
    report: &InspectReport,
    options: TerminalOptions<'_>,
) -> Result<(), RenderError> {
    let Some(scope) = report.scope.as_ref() else {
        return line(writer, "Scope: full codebase", options, Style::Heading);
    };
    let judged = if scope.staged() { "staged" } else { "changed" };
    // The ref is named beside the commit it resolved to, so a detected default
    // branch is never a guess the reader has to take on trust.
    let base = |comparison_base: &str| match scope.base_ref() {
        Some(base_ref) => format!("base {base_ref} at {}", short_revision(comparison_base)),
        None => format!("base {}", short_revision(comparison_base)),
    };
    let description = match scope.kind() {
        ResolvedScope::Full => "Scope: full codebase".to_owned(),
        ResolvedScope::Files {
            comparison_base,
            files,
        } => format!(
            "Scope: {judged} files ({} selected, {})",
            files.len(),
            base(comparison_base)
        ),
        ResolvedScope::Lines {
            comparison_base,
            files,
            ..
        } => format!(
            "Scope: {judged} lines ({} files, {})",
            files.len(),
            base(comparison_base)
        ),
        ResolvedScope::Baseline { comparison_base } => format!(
            "Scope: {}baseline comparison ({})",
            if scope.staged() { "staged " } else { "" },
            base(comparison_base)
        ),
    };
    line(writer, &description, options, Style::Heading)?;
    if let Some(count @ 1..) = scope.untracked_unreported() {
        let files = if count == 1 { "file was" } else { "files were" };
        line(
            writer,
            &format!(
                "{count} untracked Rust {files} compiled but not reported: pass --include-untracked to judge them."
            ),
            options,
            Style::Muted,
        )?;
    }
    let rust_change = |path: &String| {
        path.ends_with(".rs") || path == "Cargo.toml" || path.ends_with("/Cargo.toml")
    };
    if scope.staged()
        && scope
            .files()
            .is_some_and(|files| !files.iter().any(rust_change))
    {
        line(
            writer,
            "No staged Rust change to judge.",
            options,
            Style::Muted,
        )?;
    }
    Ok(())
}
