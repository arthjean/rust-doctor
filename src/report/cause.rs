use std::collections::BTreeMap;
use std::path::Path;

use super::{Diagnostic, DiagnosticSpan};
use crate::policy::{CauseSite, find};

const MAX_SOURCE_BYTES: u64 = 4 * 1024 * 1024;

pub(super) fn assign(diagnostics: &mut [Diagnostic], workspace_root: &Path) {
    let mut sources = BTreeMap::<String, Option<String>>::new();
    for diagnostic in diagnostics {
        let Some(key) = diagnostic
            .code
            .as_deref()
            .and_then(find)
            .and_then(|definition| definition.cause)
        else {
            continue;
        };
        let spanned = if key.reads_the_span() {
            diagnostic
                .path
                .as_deref()
                .zip(diagnostic.span.as_ref())
                .and_then(|(path, span)| {
                    let source = sources
                        .entry(path.to_owned())
                        .or_insert_with(|| read_source(workspace_root, path));
                    source.as_deref().and_then(|source| spanned_text(source, span))
                })
        } else {
            None
        };
        diagnostic.cause = key.of(CauseSite {
            message: &diagnostic.message,
            spanned: spanned.as_deref(),
        });
    }
}

fn read_source(workspace_root: &Path, path: &str) -> Option<String> {
    let file = workspace_root.join(crate::workspace_path::decode_normalized_relative(path)?);
    let metadata = std::fs::metadata(&file).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_SOURCE_BYTES {
        return None;
    }
    std::fs::read_to_string(file).ok()
}

pub(super) fn spanned_text(source: &str, span: &DiagnosticSpan) -> Option<String> {
    if span.line_start != span.line_end || span.column_start == 0 || span.column_end <= span.column_start {
        return None;
    }
    let line = source.lines().nth(span.line_start.checked_sub(1)?)?;
    let length = line.chars().count();
    if span.column_end.saturating_sub(1) > length {
        return None;
    }
    let text: String = line
        .chars()
        .skip(span.column_start - 1)
        .take(span.column_end - span.column_start)
        .collect();
    (!text.trim().is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(line: usize, start: usize, end: usize) -> DiagnosticSpan {
        DiagnosticSpan {
            line_start: line,
            column_start: start,
            line_end: line,
            column_end: end,
        }
    }

    fn by_value(path: &str, line: usize) -> Diagnostic {
        Diagnostic {
            id: format!("id-{path}-{line}"),
            source: super::super::DiagnosticSource::Clippy,
            code: Some("clippy::large_types_passed_by_value".to_owned()),
            base_severity: super::super::Severity::Warning,
            severity: super::super::Severity::Warning,
            category: Some("performance".to_owned()),
            message: "this argument (512 byte) is passed by value".to_owned(),
            help: None,
            package: None,
            target: None,
            context: None,
            unscored: None,
            path: Some(path.to_owned()),
            span: Some(span(line, 9, 12)),
            related: Vec::new(),
            similarity_basis_points: None,
            cause: None,
            complexity: None,
            suggestion: None,
            occurrences: 1,
        }
    }

    #[test]
    fn a_span_whose_text_cannot_be_read_leaves_the_finding_its_own_cause() {
        let root = std::env::temp_dir().join(format!("rust-doctor-cause-{}", std::process::id()));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/lib.rs"), "fn a(v: Big) {}\nfn b(v: Big) {}\n").unwrap();
        let mut diagnostics = vec![
            by_value("src/lib.rs", 1),
            by_value("src/lib.rs", 2),
            by_value("src/lib.rs", 9),
            by_value("src/missing.rs", 1),
        ];
        let before = diagnostics.clone();
        assign(&mut diagnostics, &root);
        let causes: Vec<Option<&str>> = diagnostics.iter().map(|diagnostic| diagnostic.cause.as_deref()).collect();
        assert_eq!(causes, [Some("Big (512 byte)"), Some("Big (512 byte)"), None, None]);
        for (assigned, original) in diagnostics.iter().zip(&before) {
            assert_eq!(
                (&assigned.id, &assigned.message, &assigned.span),
                (&original.id, &original.message, &original.span)
            );
        }
        let counted = crate::presentation::RuleCauses::of(&diagnostics);
        assert_eq!((counted.findings, counted.causes), (4, 3));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_text_under_a_span_is_read_by_character_or_not_at_all() {
        let source = "fn a() {}\nfn draw(colors: crate::theme::UiColors) {}\nfn é(x: Big) {}\n";
        assert_eq!(
            spanned_text(source, &span(2, 17, 39)).as_deref(),
            Some("crate::theme::UiColors")
        );
        assert_eq!(spanned_text(source, &span(3, 9, 12)).as_deref(), Some("Big"));
        assert_eq!(spanned_text(source, &span(9, 1, 3)), None);
        assert_eq!(spanned_text(source, &span(2, 17, 400)), None);
        assert_eq!(spanned_text(source, &span(2, 0, 4)), None);
        assert_eq!(spanned_text(source, &span(2, 5, 5)), None);
        assert_eq!(
            spanned_text(
                source,
                &DiagnosticSpan {
                    line_start: 1,
                    column_start: 1,
                    line_end: 2,
                    column_end: 3,
                }
            ),
            None
        );
    }
}
