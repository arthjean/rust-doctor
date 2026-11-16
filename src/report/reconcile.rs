use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use super::normalize::normalize_repo_finding;
use super::sanitize::HomePaths;
use super::{Diagnostic, DiagnosticSpan, Severity};
use crate::audit::{CompiledFiles, compiled_files};
use crate::execution::ExecutionResult;
use crate::policy::{CARGO_UNUSED_DEPENDENCY, REPO_SECRET_IN_BUILD_STEP, STRUCTURE_ORPHAN_MODULE_FILE};
use crate::repo_hygiene::{BuildStep, RepoFinding};
use crate::source_kernel::workspace_relative;

const EMBEDDED_HELP: &str = "Keep only a value meant to be public in a variable the build embeds. If this one is secret, read it at run time instead.";

pub(super) fn apply(diagnostics: &mut Vec<Diagnostic>, result: &ExecutionResult) -> Option<usize> {
    demote_embedded(diagnostics, result);
    with_compiled_files(diagnostics, result)
}

pub(super) fn repo_finding(
    finding: &RepoFinding,
    result: &ExecutionResult,
    workspace_root: &Path,
    home: &HomePaths,
) -> Diagnostic {
    let rewritten = embedded(finding, result);
    normalize_repo_finding(rewritten.as_ref().unwrap_or(finding), workspace_root, home)
}

fn embedded(finding: &RepoFinding, result: &ExecutionResult) -> Option<RepoFinding> {
    let step = finding.build_step.as_ref()?;
    let site = result.source.as_ref()?.embedded.get(&step.variable)?;
    let mut rewritten = finding.clone();
    rewritten.message = format!(
        "`cargo {}` compiles with `{}` in scope, and `{}` at {}:{} embeds it in the binary: whatever it holds ships to every user.",
        step.subcommand, step.variable, site.macro_name, site.path, site.line
    );
    rewritten.build_step = Some(BuildStep {
        help: EMBEDDED_HELP,
        ..step.clone()
    });
    Some(rewritten)
}

fn demote_embedded(diagnostics: &mut [Diagnostic], result: &ExecutionResult) {
    let sites: BTreeSet<(&str, Option<[usize; 4]>)> = result
        .repo
        .iter()
        .flat_map(|scan| &scan.findings)
        .filter(|finding| embedded(finding, result).is_some())
        .map(|finding| {
            let span = finding
                .span
                .map(|span| [span.line_start, span.column_start, span.line_end, span.column_end]);
            (finding.path.as_str(), span)
        })
        .collect();
    let corners = |span: &DiagnosticSpan| [span.line_start, span.column_start, span.line_end, span.column_end];
    for diagnostic in diagnostics {
        if is(diagnostic, REPO_SECRET_IN_BUILD_STEP.id)
            && diagnostic
                .path
                .as_deref()
                .is_some_and(|path| sites.contains(&(path, diagnostic.span.as_ref().map(corners))))
        {
            diagnostic.severity = Severity::Info;
        }
    }
}

fn with_compiled_files(
    diagnostics: &mut Vec<Diagnostic>,
    result: &ExecutionResult,
) -> Option<usize> {
    let scan = result.scan.finished().filter(|scan| scan.errors.is_empty())?;
    let metadata = result.metadata.as_ref()?;
    let compiled = compiled_files(metadata, scan)?;
    let workspace_root = crate::path_form::canonical(metadata.workspace_root.as_std_path()).ok()?;
    let compiled = relative(&compiled, &workspace_root);
    let every: BTreeSet<&str> = compiled.values().flatten().map(String::as_str).collect();
    diagnostics.retain(|diagnostic| {
        !(is(diagnostic, STRUCTURE_ORPHAN_MODULE_FILE.id)
            && diagnostic.path.as_deref().is_some_and(|path| every.contains(path)))
    });

    let walked = &result.source_measurement.as_ref()?.reached;
    let mut unwalked = BTreeSet::new();
    let mut unseen_packages = BTreeSet::new();
    for (package_id, files) in &compiled {
        let missed: Vec<&String> = files.iter().filter(|file| !walked.contains(*file)).collect();
        if missed.is_empty() {
            continue;
        }
        unwalked.extend(missed);
        if let Some(package) = metadata.packages.iter().find(|package| &package.id.repr == package_id) {
            unseen_packages.insert(package.name.to_string());
        }
    }
    diagnostics.retain(|diagnostic| {
        !(is(diagnostic, CARGO_UNUSED_DEPENDENCY.id)
            && diagnostic
                .package
                .as_ref()
                .is_some_and(|package| unseen_packages.contains(package)))
    });
    Some(unwalked.len())
}

fn is(diagnostic: &Diagnostic, rule: &str) -> bool {
    diagnostic.code.as_deref() == Some(rule)
}

fn relative(compiled: &CompiledFiles, workspace_root: &Path) -> BTreeMap<String, BTreeSet<String>> {
    compiled
        .by_package
        .iter()
        .map(|(package_id, files)| {
            let files = files
                .iter()
                .filter_map(|file| workspace_relative(workspace_root, file))
                .collect();
            (package_id.repr.clone(), files)
        })
        .collect()
}
