//! Shared normalization contract for optional external analyzers.
//!
//! Every adapter answers the same three questions before it reports anything:
//! did the tool actually run, was its output complete, and what does its
//! evidence *mean*. A missing tool, a timeout, a non-zero exit, a truncated
//! document, or an unparseable payload all produce an explicit skipped or
//! failed receipt — never an empty successful result (US-009 AC-5).
#![expect(
    clippy::redundant_pub_crate,
    reason = "the adapter contract is consumed by sibling pass modules through this private crate module"
)]

use crate::diagnostics::{Category, Diagnostic, Severity};
use crate::error::PassError;
use crate::process::ProcessOutput;
use std::path::{Path, PathBuf};

/// Identity and parser contract of one external adapter.
///
/// The parser contract version is bumped whenever the shape this adapter reads
/// changes, so a conformance fixture recorded under an older contract is not
/// silently reinterpreted.
#[derive(Debug, Clone, Copy)]
pub(crate) struct AdapterContract {
    /// Pass name used in receipts.
    pub(crate) pass: &'static str,
    /// Cargo subcommand, e.g. `audit`.
    pub(crate) subcommand: &'static str,
    /// Version of the output shape this adapter parses.
    pub(crate) parser_contract_version: &'static str,
    /// Whether the adapter consumes a structured document or parses text.
    pub(crate) evidence_source: EvidenceSource,
}

/// How an adapter obtains its evidence. Structured output is always preferred;
/// text parsing is confined to tools without a stable machine format and is
/// protected by versioned fixtures (US-009 AC-4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EvidenceSource {
    StructuredJson,
    StructuredJsonLines,
    TextFixtureBacked,
}

impl EvidenceSource {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::StructuredJson => "structured-json",
            Self::StructuredJsonLines => "structured-json-lines",
            Self::TextFixtureBacked => "text-fixture-backed",
        }
    }
}

impl AdapterContract {
    /// The receipt for a tool that is not installed.
    pub(crate) fn skipped(self, install_hint: &str) -> PassError {
        PassError::Skipped {
            pass: self.pass.to_string(),
            reason: format!("{} is not installed — {install_hint}", self.tool_name()),
        }
    }

    pub(crate) fn failed(self, message: impl std::fmt::Display) -> PassError {
        PassError::Failed {
            pass: self.pass.to_string(),
            message: sanitize(&message.to_string()),
        }
    }

    pub(crate) fn tool_name(self) -> String {
        format!("cargo-{}", self.subcommand)
    }

    /// Installed tool version, or `unknown` when it cannot be read. Recorded
    /// next to the parser contract so a version outside the supported matrix is
    /// visible rather than assumed compatible.
    pub(crate) fn tool_version(self) -> String {
        crate::process::cargo_subcommand_version(self.subcommand)
            .unwrap_or_else(|| "unknown".to_string())
    }

    /// Reject every execution outcome that cannot yield complete evidence.
    ///
    /// `success_codes` lists the exit codes that mean "the tool ran to
    /// completion"; findings themselves usually make a tool exit non-zero, so
    /// each adapter states its own set.
    pub(crate) fn require_complete_run(
        self,
        output: &ProcessOutput,
        success_codes: &[i32],
    ) -> Result<(), PassError> {
        if output.timed_out {
            return Err(PassError::TimedOut {
                pass: self.pass.to_string(),
                reason: format!("{} exceeded its analysis deadline", self.tool_name()),
            });
        }
        if output.cancelled {
            return Err(PassError::Cancelled {
                pass: self.pass.to_string(),
                reason: format!("{} was cancelled before completing", self.tool_name()),
            });
        }
        if output.truncated {
            return Err(self.failed(format!(
                "{} output exceeded the capture limit; the document is incomplete",
                self.tool_name()
            )));
        }
        match output.exit_code {
            Some(code) if success_codes.contains(&code) => Ok(()),
            Some(code) => {
                Err(self.failed(format!("{} exited with status {code}", self.tool_name())))
            }
            None => Err(self.failed(format!(
                "{} terminated without an exit status",
                self.tool_name()
            ))),
        }
    }

    /// A provenance suffix every adapter appends to its findings so the
    /// originating tool, parser contract, evidence source, and conformance
    /// state survive normalization (US-009 AC-3, US-011 AC-5).
    pub(crate) fn provenance(self, tool_version: &str) -> String {
        format!(
            "{} {tool_version} (parser {}, {}, {}, {})",
            self.tool_name(),
            self.parser_contract_version,
            self.evidence_source.as_str(),
            self.version_support(tool_version).as_str(),
            if self.authoritative(tool_version) {
                "authoritative"
            } else {
                "non-authoritative"
            }
        )
    }

    /// Whether this tool version may carry authority: it must be inside the
    /// approved matrix and belong to an authority-capable adapter.
    pub(crate) fn authoritative(self, tool_version: &str) -> bool {
        crate::passes::conformance::entry(&self.tool_name())
            .is_some_and(|row| row.authoritative(tool_version))
    }

    /// Where this tool version sits in the approved conformance matrix. A
    /// version outside it is observable but not authoritative.
    pub(crate) fn version_support(
        self,
        tool_version: &str,
    ) -> crate::passes::conformance::VersionSupport {
        crate::passes::conformance::entry(&self.tool_name()).map_or(
            crate::passes::conformance::VersionSupport::Unsupported,
            |row| row.version_support(tool_version),
        )
    }
}

/// Strip absolute filesystem paths from adapter text.
///
/// External tools happily print `/home/<user>/...` into their messages. Error
/// reporting must not carry that through (US-009 AC-8, NFR-017).
pub(crate) fn sanitize(message: &str) -> String {
    let mut output = String::with_capacity(message.len());
    for token in message.split_inclusive(char::is_whitespace) {
        let trimmed = token.trim_end();
        let separator = &token[trimmed.len()..];
        if trimmed.starts_with('/') || trimmed.starts_with("\\\\") {
            let name = Path::new(trimmed).file_name().map_or_else(
                || "<path>".to_string(),
                |name| name.to_string_lossy().into(),
            );
            output.push_str("<path>/");
            output.push_str(&name);
        } else {
            output.push_str(trimmed);
        }
        output.push_str(separator);
    }
    output
}

/// Cap tool text kept in a diagnostic so an unbounded message cannot reach the
/// report (NFR-012).
pub(crate) fn bounded_text(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_string();
    }
    let mut truncated: String = value.chars().take(limit).collect();
    truncated.push('\u{2026}');
    truncated
}

/// Build a project-level diagnostic owned by one adapter.
pub(crate) fn project_diagnostic(
    rule: &str,
    category: Category,
    severity: Severity,
    message: &str,
    help: Option<&str>,
    manifest: &str,
) -> Diagnostic {
    Diagnostic {
        file_path: PathBuf::from(manifest),
        rule: rule.to_string(),
        category,
        severity,
        message: bounded_text(&sanitize(message), 600),
        help: help.map(|value| bounded_text(&sanitize(value), 600)),
        line: None,
        column: None,
        fix: None,
    }
}

/// Stable RustSec advisory identifier found anywhere in a message.
///
/// cargo-audit reports the advisory as its rule ID; cargo-deny reports the same
/// advisory inside prose. Recovering the identifier from either lets the scan
/// deduplicate one root cause across two analyzers while both keep their
/// provenance (US-009 AC-1).
pub(crate) fn advisory_identity(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let start = value.find("RUSTSEC-")?;
    // RUSTSEC-YYYY-NNNN
    let candidate = &bytes[start..];
    if candidate.len() < 17 {
        return None;
    }
    let tail = &candidate[8..17];
    let shaped = tail.iter().enumerate().all(|(index, byte)| match index {
        4 => *byte == b'-',
        _ => byte.is_ascii_digit(),
    });
    shaped.then(|| value[start..start + 17].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONTRACT: AdapterContract = AdapterContract {
        pass: "dependencies (cargo-audit)",
        subcommand: "audit",
        parser_contract_version: "audit-json-v1",
        evidence_source: EvidenceSource::StructuredJson,
    };

    fn output(exit_code: Option<i32>) -> ProcessOutput {
        ProcessOutput {
            stdout: String::new(),
            stderr: String::new(),
            timed_out: false,
            cancelled: false,
            exit_code,
            truncated: false,
        }
    }

    #[test]
    fn a_timeout_is_a_receipt_and_never_a_clean_result() {
        let mut result = output(None);
        result.timed_out = true;
        assert!(matches!(
            CONTRACT.require_complete_run(&result, &[0]),
            Err(PassError::TimedOut { .. })
        ));
    }

    #[test]
    fn cancellation_truncation_and_bad_exits_all_fail_explicitly() {
        let mut cancelled = output(Some(0));
        cancelled.cancelled = true;
        assert!(matches!(
            CONTRACT.require_complete_run(&cancelled, &[0]),
            Err(PassError::Cancelled { .. })
        ));

        let mut truncated = output(Some(0));
        truncated.truncated = true;
        assert!(matches!(
            CONTRACT.require_complete_run(&truncated, &[0]),
            Err(PassError::Failed { .. })
        ));

        assert!(matches!(
            CONTRACT.require_complete_run(&output(Some(101)), &[0, 1]),
            Err(PassError::Failed { .. })
        ));
        assert!(matches!(
            CONTRACT.require_complete_run(&output(None), &[0]),
            Err(PassError::Failed { .. })
        ));
        assert!(
            CONTRACT
                .require_complete_run(&output(Some(1)), &[0, 1])
                .is_ok()
        );
    }

    #[test]
    fn absolute_paths_are_stripped_from_adapter_text() {
        let sanitized = sanitize("failed reading /home/arthur/secret-project/Cargo.lock now");
        assert!(!sanitized.contains("arthur"));
        assert!(!sanitized.contains("secret-project"));
        assert!(sanitized.contains("<path>/Cargo.lock"));
        assert!(sanitized.starts_with("failed reading "));
        assert!(sanitized.ends_with(" now"));
    }

    #[test]
    fn captured_tool_text_is_bounded() {
        let long = "x".repeat(1000);
        let bounded = bounded_text(&long, 10);
        assert_eq!(bounded.chars().count(), 11);
        assert!(bounded.ends_with('\u{2026}'));
        assert_eq!(bounded_text("short", 10), "short");
    }

    #[test]
    fn advisory_identity_is_recovered_from_either_analyzer() {
        assert_eq!(
            advisory_identity("RUSTSEC-2023-0071").as_deref(),
            Some("RUSTSEC-2023-0071")
        );
        assert_eq!(
            advisory_identity("crate `rsa` has a vulnerability: RUSTSEC-2023-0071 Marvin Attack")
                .as_deref(),
            Some("RUSTSEC-2023-0071")
        );
        assert!(advisory_identity("no advisory here").is_none());
        assert!(advisory_identity("RUSTSEC-20xx-0071").is_none());
        assert!(advisory_identity("RUSTSEC-2023").is_none());
    }

    #[test]
    fn provenance_records_tool_parser_and_evidence_source() {
        let provenance = CONTRACT.provenance("cargo-audit 0.21.0");
        assert!(provenance.contains("cargo-audit 0.21.0"));
        assert!(provenance.contains("audit-json-v1"));
        assert!(provenance.contains("structured-json"));
    }
}
