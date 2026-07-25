use crate::diagnostics::{Diagnostic as RustDiagnostic, Severity};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use tower_lsp_server::ls_types::{Diagnostic, DiagnosticSeverity, NumberOrString, Position, Range};

#[derive(Clone, Debug)]
pub(super) struct EditorFinding {
    pub(super) identity: String,
    pub(super) rule: String,
    pub(super) message: String,
    pub(super) help: Option<String>,
    pub(super) documentation_url: String,
    pub(super) range: Range,
    pub(super) severity: DiagnosticSeverity,
}

impl EditorFinding {
    pub(super) fn to_diagnostic(&self, degraded: bool) -> Diagnostic {
        Diagnostic {
            range: self.range,
            severity: Some(self.severity),
            code: Some(NumberOrString::String(self.rule.clone())),
            source: Some("rust-doctor".to_string()),
            message: self.message.clone(),
            data: Some(serde_json::json!({
                "canonical_id": self.rule,
                "identity": self.identity,
                "degraded": degraded,
            })),
            ..Diagnostic::default()
        }
    }
}

pub(super) fn analyze(
    source: &str,
    path: &Path,
    config: &crate::config::ResolvedConfig,
    capabilities: &[crate::discovery::FrameworkCapability],
    cargo_targets: &[crate::discovery::CargoTargetContext],
    cancelled: &AtomicBool,
) -> Result<Vec<EditorFinding>, syn::Error> {
    let diagnostics = crate::rules::analyze_editor_source(
        source,
        path,
        config,
        capabilities,
        cargo_targets,
        cancelled,
    )?;
    Ok(convert(source, path, diagnostics))
}

pub(super) fn convert(
    source: &str,
    path: &Path,
    diagnostics: Vec<RustDiagnostic>,
) -> Vec<EditorFinding> {
    let catalog = crate::catalog::built_in_catalog().ok();
    diagnostics
        .into_iter()
        .map(|diagnostic| {
            let descriptor = catalog.and_then(|catalog| catalog.exact(&diagnostic.rule));
            let line = diagnostic.line.unwrap_or(1).saturating_sub(1);
            let byte_column = diagnostic.column.unwrap_or(1).saturating_sub(1) as usize;
            let start = position_from_byte_column(source, line, byte_column);
            let end = one_character_end(source, start);
            let range = Range { start, end };
            let documentation_url = descriptor.map_or_else(
                || "https://rust-doctor.vercel.app/rules/external".to_string(),
                |descriptor| descriptor.documentation_url.clone(),
            );
            let identity = stable_identity(
                path,
                &diagnostic.rule,
                &diagnostic.message,
                line,
                source_line(source, line),
            );
            EditorFinding {
                identity,
                rule: diagnostic.rule,
                message: diagnostic.message,
                help: diagnostic.help,
                documentation_url,
                range,
                severity: severity(diagnostic.severity),
            }
        })
        .collect()
}

fn source_line(source: &str, line: u32) -> &str {
    source.lines().nth(line as usize).unwrap_or("")
}

fn stable_identity(path: &Path, rule: &str, message: &str, line: u32, evidence: &str) -> String {
    let mut hash = Sha256::new();
    for component in [
        path.to_string_lossy().as_bytes(),
        rule.as_bytes(),
        message.as_bytes(),
        line.to_string().as_bytes(),
        evidence.trim().as_bytes(),
    ] {
        hash.update((component.len() as u64).to_le_bytes());
        hash.update(component);
    }
    format!("{:x}", hash.finalize())
}

const fn severity(value: Severity) -> DiagnosticSeverity {
    match value {
        Severity::Error => DiagnosticSeverity::ERROR,
        Severity::Warning => DiagnosticSeverity::WARNING,
        Severity::Info => DiagnosticSeverity::INFORMATION,
    }
}

pub(super) fn position_from_byte_column(source: &str, line: u32, byte_column: usize) -> Position {
    let text = source_line(source, line);
    let mut clamped = byte_column.min(text.len());
    while clamped > 0 && !text.is_char_boundary(clamped) {
        clamped -= 1;
    }
    let utf16 = text[..clamped].encode_utf16().count();
    Position::new(line, u32::try_from(utf16).unwrap_or(u32::MAX))
}

fn one_character_end(source: &str, start: Position) -> Position {
    let text = source_line(source, start.line);
    let mut utf16 = 0_u32;
    for character in text.chars() {
        let width = u32::try_from(character.len_utf16()).unwrap_or(1);
        if utf16 >= start.character {
            return Position::new(start.line, utf16.saturating_add(width));
        }
        utf16 = utf16.saturating_add(width);
    }
    start
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn converts_utf8_byte_columns_to_utf16_positions() {
        let source = "fn main() {\n    let value = \"😀é\";\n}\n";
        let line = source.lines().nth(1).unwrap();
        let emoji_end = line.find('é').unwrap();
        let position = position_from_byte_column(source, 1, emoji_end);
        assert_eq!(position.line, 1);
        assert_eq!(position.character, 19);
    }

    #[test]
    fn identity_is_path_and_evidence_stable() {
        let first = stable_identity(Path::new("src/lib.rs"), "rule", "message", 2, "let x = 1;");
        let second = stable_identity(Path::new("src/lib.rs"), "rule", "message", 2, "let x = 1;");
        assert_eq!(first, second);
        assert_ne!(
            first,
            stable_identity(Path::new("src/main.rs"), "rule", "message", 2, "let x = 1;")
        );
    }

    #[test]
    fn analyzes_ten_thousand_line_editor_document_within_latency_budget() {
        let source = "// editor source line\n".repeat(10_000);
        let config = crate::config::resolve_config_defaults(None);
        let cancelled = AtomicBool::new(false);
        let started = Instant::now();
        let findings = analyze(
            &source,
            Path::new("src/large.rs"),
            &config,
            &[],
            &[],
            &cancelled,
        )
        .unwrap();
        assert!(findings.is_empty());
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "in-memory analysis exceeded the 500 ms post-debounce budget"
        );
    }
}
