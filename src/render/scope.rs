//! The scope line that opens every report, failed ones included, the notes
//! that qualify it, and what the scan scored and read.

use std::io::Write;

use crate::{InspectReport, NotEvaluated};
use crate::git_scope::{ResolvedScope, ScopeMode};

use super::{RenderError, Style, TerminalOptions, line, short_revision};

/// What the scan was asked to judge, against which ref, and what it left out.
pub(super) fn render_scope<W: Write>(
    writer: &mut W,
    report: &InspectReport,
    options: TerminalOptions<'_>,
) -> Result<(), RenderError> {
    let Some(scope) = report.scope.as_ref() else {
        // A narrower scope that failed to resolve is named as asked, never as
        // the whole codebase the scan did not judge either.
        let description = match options.requested_scope {
            Some(mode) if mode != ScopeMode::Full => format!("Scope: {} (not resolved)", mode.as_str()),
            _ => "Scope: full codebase".to_owned(),
        };
        return line(writer, &description, options, Style::Heading);
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
            contextual,
            ..
        } => format!(
            "Scope: {judged} lines{} ({} files, {})",
            if *contextual {
                " and the functions holding them"
            } else {
                ""
            },
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

/// What the score was computed over and how long it took, what each producer
/// read, and under `--verbose` how long each phase took and whether the base
/// side came from the cache.
///
/// The two producers do not read the same targets: Clippy lints the library
/// and binary targets, while the native rules also read `tests/`. A reader who
/// is not told assumes either that test code was linted or that every finding
/// weighs, and the line used to say only `Scanned N files`.
pub(super) fn render_scanned<W: Write>(
    writer: &mut W,
    report: &InspectReport,
    options: TerminalOptions<'_>,
) -> Result<(), RenderError> {
    let scored = scored_clause(
        report.audit.source_files,
        report.audit.production_lines,
        selected_members(report).as_deref(),
    );
    line(
        writer,
        &format!(
            "{scored} in {:.1}s; {}",
            options.elapsed.as_secs_f64(),
            read_clause(report.scan.command.is_some())
        ),
        options,
        Style::Accent,
    )?;
    if !options.verbose {
        return Ok(());
    }
    if let Some(outcome) = report.base_side_cache {
        line(
            writer,
            &format!("Base side cache: {}", outcome.as_str()),
            options,
            Style::Muted,
        )?;
    }
    let Some(timings) = report.timings.as_ref() else {
        return Ok(());
    };
    let phases: Vec<String> = timings
        .phases()
        .map(|(phase, milliseconds)| format!("{phase} {milliseconds} ms"))
        .collect();
    line(
        writer,
        &format!("Phases: {}; total {} ms", phases.join(", "), timings.total),
        options,
        Style::Muted,
    )
}

/// What the score was computed over, and the members a `--package` scan
/// selected, worded once for the linear and the Markdown report.
pub(super) fn scored_clause(files: usize, lines: usize, members: Option<&[&str]>) -> String {
    let members = members
        .map(|names| {
            let noun = if names.len() == 1 { "member" } else { "members" };
            format!(" of {noun} {}", names.join(", "))
        })
        .unwrap_or_default();
    format!("Scored {files} production files ({lines} lines){members}")
}

/// What each producer read, given whether the Clippy pass ran.
pub(super) const fn read_clause(clippy_ran: bool) -> &'static str {
    if clippy_ran {
        "Clippy linted library and binary targets, native rules also read tests"
    } else {
        "Clippy did not run, native rules also read tests"
    }
}

/// The sentence naming the rules a crate version outside their validated range
/// left unevaluated, before the list of their ids.
pub(super) const VERSION_UNSUPPORTED: &str =
    "Not evaluated, the crate version is outside the range the rule was validated against";

/// The members a `--package` scan selected, absent when it scanned them all.
///
/// The audit holds one entry per scanned member and the project one per
/// workspace member, so a selection is exactly a project the audit covers only
/// part of.
fn selected_members(report: &InspectReport) -> Option<Vec<&str>> {
    let workspace = report.project.as_ref()?.packages.len();
    let scanned: Vec<&str> = report
        .audit
        .packages
        .iter()
        .map(|package| package.name.as_str())
        .collect();
    (!scanned.is_empty() && scanned.len() < workspace).then_some(scanned)
}

/// The rules the policy left on that the scan did not evaluate because of the
/// workspace's own dependencies.
///
/// A rule whose crate resolves outside the versions it was validated against is
/// a rule the reader might expect to have run, so each is named. A rule whose
/// crate no package depends on had nothing to read, and is counted only under
/// `--verbose`, since every scan of a workspace without that crate carries it.
pub(super) fn render_not_evaluated<W: Write>(
    writer: &mut W,
    report: &InspectReport,
    options: TerminalOptions<'_>,
) -> Result<(), RenderError> {
    let Some(policy) = report.policy.as_ref() else {
        return Ok(());
    };
    let with = |reason: NotEvaluated| {
        policy
            .rules
            .iter()
            .filter(move |rule| rule.not_evaluated == Some(reason))
            .map(|rule| rule.id.as_str())
    };
    let unsupported: Vec<&str> = with(NotEvaluated::CrateVersionUnsupported).collect();
    if !unsupported.is_empty() {
        line(
            writer,
            &format!("{VERSION_UNSUPPORTED}: {}", unsupported.join(", ")),
            options,
            Style::Muted,
        )?;
    }
    let absent = with(NotEvaluated::CrateAbsent).count();
    if options.verbose && absent > 0 {
        let rules = if absent == 1 { "rule was" } else { "rules were" };
        line(
            writer,
            &format!("{absent} {rules} not evaluated: no package depends on their crate."),
            options,
            Style::Muted,
        )?;
    }
    Ok(())
}
