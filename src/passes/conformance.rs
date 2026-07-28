//! Versioned conformance matrix for every analyzer that can carry authority.
//!
//! Parser drift is the failure mode this module exists to catch. Each row pins
//! an adapter's identity, the tool versions it was measured against, the parser
//! contract version it reads, where its fixtures came from, and whether it may
//! speak authoritatively at all. The suite at the bottom replays recorded
//! documents — supported, malformed, mixed, truncated, and unknown — so a
//! change to any parser has to update the matrix before it can ship
//! (US-011, NFR-020).
#![expect(
    clippy::redundant_pub_crate,
    reason = "the conformance contract is consumed by sibling pass, catalog, and trust modules"
)]

use crate::passes::adapter::EvidenceSource;

/// How an observed tool version relates to the approved support matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VersionSupport {
    /// Measured against a recorded conformance fixture.
    Supported,
    /// Parses, but outside the measured matrix: observable, never authoritative.
    BestEffort,
    /// The version could not be read at all.
    Unsupported,
}

impl VersionSupport {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Supported => "supported",
            Self::BestEffort => "best-effort",
            Self::Unsupported => "unsupported",
        }
    }
}

/// Degraded execution shapes every adapter must survive without panicking and
/// without reporting an empty successful result.
///
/// This is the specification the conformance suite enforces; production code
/// produces the receipts, it does not consult the table.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DegradedInput {
    MissingExecutable,
    MalformedDocument,
    MixedText,
    TruncatedOutput,
    Timeout,
    NonZeroExit,
    UnknownOutputVersion,
}

#[cfg(test)]
impl DegradedInput {
    /// Every degraded shape the conformance suite exercises.
    pub(crate) const ALL: &'static [Self] = &[
        Self::MissingExecutable,
        Self::MalformedDocument,
        Self::MixedText,
        Self::TruncatedOutput,
        Self::Timeout,
        Self::NonZeroExit,
        Self::UnknownOutputVersion,
    ];

    /// The receipt this input must produce. `Clean` is deliberately absent:
    /// no degraded input may normalize to a successful empty result.
    pub(crate) const fn expected_receipt(self) -> ExpectedReceipt {
        match self {
            Self::MissingExecutable => ExpectedReceipt::Skipped,
            Self::MalformedDocument | Self::TruncatedOutput | Self::NonZeroExit => {
                ExpectedReceipt::Failed
            }
            Self::Timeout => ExpectedReceipt::TimedOut,
            Self::MixedText | Self::UnknownOutputVersion => ExpectedReceipt::PartialClassification,
        }
    }
}

/// Receipt class an adapter owes for a degraded input.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExpectedReceipt {
    /// Optional evidence was never collected. Completeness degrades, the Core
    /// Score does not move.
    Skipped,
    /// The adapter ran but could not produce trustworthy evidence.
    Failed,
    /// The adapter exceeded its deadline.
    TimedOut,
    /// The adapter kept the evidence it could read and classified the rest.
    PartialClassification,
}

#[cfg(test)]
impl ExpectedReceipt {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Skipped => "skipped",
            Self::Failed => "failed",
            Self::TimedOut => "timed-out",
            Self::PartialClassification => "partial",
        }
    }
}

/// One conformance row.
///
/// `tool`, `evidence_source`, and `fixture_provenance` are the declarative
/// half of the contract: the conformance suite asserts them, production code
/// only consults the version and parser-contract half.
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "matrix documentation required by US-011 AC-1 and asserted by the conformance suite"
    )
)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct AdapterConformance {
    /// Stable adapter identity used by receipts and this matrix.
    pub(crate) adapter: &'static str,
    /// Executable or library the adapter reads.
    pub(crate) tool: &'static str,
    /// Output shape version this adapter parses.
    pub(crate) parser_contract_version: &'static str,
    pub(crate) evidence_source: EvidenceSource,
    /// `major.minor` prefixes measured by a recorded fixture.
    pub(crate) supported_versions: &'static [&'static str],
    /// Where the recorded fixtures came from.
    pub(crate) fixture_provenance: &'static str,
    /// Whether this adapter may contribute authority at all. An audit-only
    /// adapter observes; it never makes a dimension authoritative.
    pub(crate) authority_capable: bool,
}

impl AdapterConformance {
    /// Classify a reported tool version against the measured matrix.
    pub(crate) fn version_support(&self, version: &str) -> VersionSupport {
        let Some(observed) = major_minor(version) else {
            return VersionSupport::Unsupported;
        };
        if self
            .supported_versions
            .iter()
            .any(|supported| *supported == observed)
        {
            VersionSupport::Supported
        } else {
            VersionSupport::BestEffort
        }
    }

    /// A version outside the approved matrix cannot become authoritative, even
    /// when its output happens to parse (US-011 AC-5).
    pub(crate) fn authoritative(&self, version: &str) -> bool {
        self.authority_capable && self.version_support(version) == VersionSupport::Supported
    }
}

/// First `major.minor` pair found in a version banner such as
/// `cargo-audit-audit 0.21.2`.
fn major_minor(version: &str) -> Option<String> {
    version.split_whitespace().find_map(|token| {
        let token = token.trim_start_matches('v');
        let mut parts = token.split('.');
        let major = parts.next()?;
        let minor = parts.next()?;
        (!major.is_empty()
            && major.bytes().all(|byte| byte.is_ascii_digit())
            && !minor.is_empty()
            && minor.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| format!("{major}.{minor}"))
    })
}

/// The approved matrix. Adding an analyzer to authority requires adding it
/// here with recorded fixtures first.
pub(crate) const MATRIX: &[AdapterConformance] = &[
    AdapterConformance {
        adapter: "cargo-metadata",
        tool: "cargo metadata",
        parser_contract_version: "cargo-metadata-format-1",
        evidence_source: EvidenceSource::StructuredJson,
        supported_versions: &["1.0"],
        fixture_provenance: "authored from `cargo metadata --format-version 1` on MSRV 1.97",
        authority_capable: true,
    },
    AdapterConformance {
        adapter: "rustc",
        tool: "cargo --message-format=json",
        parser_contract_version: "cargo-jsonlines-v1",
        evidence_source: EvidenceSource::StructuredJsonLines,
        supported_versions: &["1.97", "1.98"],
        fixture_provenance: "captured from `cargo check --message-format=json` on MSRV 1.97",
        authority_capable: true,
    },
    AdapterConformance {
        adapter: "clippy",
        tool: "cargo clippy --message-format=json",
        parser_contract_version: "cargo-jsonlines-v1",
        evidence_source: EvidenceSource::StructuredJsonLines,
        supported_versions: &["1.97", "1.98"],
        fixture_provenance: "captured from `cargo clippy --message-format=json` on MSRV 1.97",
        authority_capable: true,
    },
    AdapterConformance {
        adapter: "cargo-audit",
        tool: "cargo audit --json",
        parser_contract_version: "audit-json-v1",
        evidence_source: EvidenceSource::StructuredJson,
        supported_versions: &["0.20", "0.21"],
        fixture_provenance: "authored from the cargo-audit 0.21 JSON report shape",
        authority_capable: true,
    },
    AdapterConformance {
        adapter: "cargo-deny",
        tool: "cargo deny --format json check",
        parser_contract_version: "deny-jsonlines-v2",
        evidence_source: EvidenceSource::StructuredJsonLines,
        supported_versions: &["0.18", "0.19"],
        fixture_provenance: "cargo-deny 0.18.3 and 0.19.8 captured JSON Lines on stderr from clean and findings fixtures",
        authority_capable: true,
    },
    AdapterConformance {
        adapter: "cargo-geiger",
        tool: "cargo geiger",
        parser_contract_version: "geiger-tree-v1",
        evidence_source: EvidenceSource::TextFixtureBacked,
        supported_versions: &["0.11", "0.12"],
        fixture_provenance: "authored from the cargo-geiger 0.12 ASCII tree",
        // Unsafe exposure is an observation, never a proven defect, so geiger
        // is excluded from authority by contract rather than by availability.
        authority_capable: false,
    },
    AdapterConformance {
        adapter: "cargo-shear",
        tool: "cargo shear --format json",
        parser_contract_version: "shear-json-v1",
        evidence_source: EvidenceSource::StructuredJson,
        supported_versions: &["1.5", "1.6"],
        fixture_provenance: "authored from the cargo-shear 1.6 JSON findings shape",
        authority_capable: true,
    },
    AdapterConformance {
        adapter: "cargo-semver-checks",
        tool: "cargo semver-checks",
        parser_contract_version: "semver-checks-text-v1",
        evidence_source: EvidenceSource::TextFixtureBacked,
        supported_versions: &["0.41", "0.42"],
        fixture_provenance: "authored from the cargo-semver-checks 0.42 report text",
        authority_capable: true,
    },
];

/// Matrix row for one adapter identity.
pub(crate) fn entry(adapter: &str) -> Option<&'static AdapterConformance> {
    MATRIX.iter().find(|row| row.adapter == adapter)
}

/// Parser contract version currently declared by an adapter's own code.
///
/// Trust validation compares this against the matrix: a compiler-proven rule
/// whose parser drifted away from its recorded conformance loses authority even
/// when corpus precision is untouched (US-012 AC-4).
pub(crate) fn conformant(adapter: &str, declared_parser_contract: &str) -> bool {
    entry(adapter).is_some_and(|row| row.parser_contract_version == declared_parser_contract)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::PassError;
    use crate::passes::quality::{semver_checks, shear};
    use crate::passes::security::{audit, deny, geiger};
    use crate::passes::static_analysis::clippy;
    use crate::process::ProcessOutput;
    use sha2::{Digest, Sha256};

    const AUDIT_REPORT: &str = include_str!("../../evaluation/conformance/cargo-audit/report.json");
    const DENY_018_REPORT: &str =
        include_str!("../../evaluation/conformance/cargo-deny/v0.18-findings.jsonl");
    const DENY_018_CLEAN: &str =
        include_str!("../../evaluation/conformance/cargo-deny/v0.18-clean.jsonl");
    const DENY_019_REPORT: &str =
        include_str!("../../evaluation/conformance/cargo-deny/v0.19-findings.jsonl");
    const DENY_019_CLEAN: &str =
        include_str!("../../evaluation/conformance/cargo-deny/v0.19-clean.jsonl");
    const DENY_MATRIX: &str = include_str!("../../evaluation/conformance/cargo-deny/matrix.json");
    const DENY_ADVISORY_SUPPORT: &str =
        include_str!("../../evaluation/conformance/cargo-deny/advisory-db-snapshot/support.toml");
    const DENY_RSA_ADVISORY: &str = include_str!(
        "../../evaluation/conformance/cargo-deny/advisory-db-snapshot/crates/rsa/RUSTSEC-2023-0071.md"
    );
    const GEIGER_REPORT: &str =
        include_str!("../../evaluation/conformance/cargo-geiger/report.txt");
    const SHEAR_REPORT: &str = include_str!("../../evaluation/conformance/cargo-shear/report.json");
    const SEMVER_REPORT: &str =
        include_str!("../../evaluation/conformance/cargo-semver-checks/report.txt");
    const CARGO_STREAM: &str = include_str!("../../evaluation/conformance/clippy/stream.jsonl");
    const CARGO_STREAM_DEGRADED: &str =
        include_str!("../../evaluation/conformance/clippy/degraded-stream.jsonl");
    const METADATA_ADDITIVE: &str =
        include_str!("../../evaluation/conformance/cargo-metadata/additive-fields.json");

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

    // ------------------------------------------------------------------
    // Matrix completeness
    // ------------------------------------------------------------------

    #[test]
    fn every_row_declares_a_complete_conformance_contract() {
        assert!(!MATRIX.is_empty());
        let mut identities = std::collections::BTreeSet::new();
        for row in MATRIX {
            assert!(
                identities.insert(row.adapter),
                "duplicate conformance row for {}",
                row.adapter
            );
            assert!(!row.tool.is_empty(), "{} has no tool", row.adapter);
            assert!(
                !row.parser_contract_version.is_empty(),
                "{} has no parser contract version",
                row.adapter
            );
            assert!(
                !row.supported_versions.is_empty(),
                "{} declares no supported version",
                row.adapter
            );
            assert!(
                !row.fixture_provenance.is_empty(),
                "{} records no fixture provenance",
                row.adapter
            );
        }
    }

    /// The protected test of US-011 AC-6: a parser contract bump that does not
    /// reach the matrix fails CI instead of silently reinterpreting fixtures.
    #[test]
    fn matrix_parser_contracts_match_the_adapters_they_describe() {
        for (adapter, declared) in [
            ("cargo-audit", audit::CONTRACT.parser_contract_version),
            ("cargo-deny", deny::CONTRACT.parser_contract_version),
            ("cargo-geiger", geiger::CONTRACT.parser_contract_version),
            ("cargo-shear", shear::CONTRACT.parser_contract_version),
            (
                "cargo-semver-checks",
                semver_checks::CONTRACT.parser_contract_version,
            ),
            ("clippy", clippy::PARSER_CONTRACT_VERSION),
            ("rustc", clippy::PARSER_CONTRACT_VERSION),
        ] {
            assert!(
                conformant(adapter, declared),
                "{adapter} parses '{declared}' but the matrix records '{}'",
                entry(adapter).map_or("<missing row>", |row| row.parser_contract_version),
            );
        }
    }

    #[test]
    fn matrix_evidence_sources_match_the_adapters_they_describe() {
        for (adapter, declared) in [
            ("cargo-audit", audit::CONTRACT.evidence_source),
            ("cargo-deny", deny::CONTRACT.evidence_source),
            ("cargo-geiger", geiger::CONTRACT.evidence_source),
            ("cargo-shear", shear::CONTRACT.evidence_source),
            (
                "cargo-semver-checks",
                semver_checks::CONTRACT.evidence_source,
            ),
        ] {
            let row = entry(adapter).expect("adapter has a conformance row");
            assert_eq!(row.evidence_source, declared, "{adapter} evidence source");
        }
    }

    // ------------------------------------------------------------------
    // Version support
    // ------------------------------------------------------------------

    #[test]
    fn a_version_outside_the_matrix_is_best_effort_and_never_authoritative() {
        let row = entry("cargo-audit").expect("row");
        assert_eq!(
            row.version_support("cargo-audit-audit 0.21.2"),
            VersionSupport::Supported
        );
        assert!(row.authoritative("cargo-audit-audit 0.21.2"));

        assert_eq!(
            row.version_support("cargo-audit-audit 9.0.0"),
            VersionSupport::BestEffort
        );
        assert!(!row.authoritative("cargo-audit-audit 9.0.0"));

        assert_eq!(row.version_support("unknown"), VersionSupport::Unsupported);
        assert!(!row.authoritative("unknown"));
    }

    #[test]
    fn an_audit_only_adapter_cannot_become_authoritative_on_any_version() {
        let row = entry("cargo-geiger").expect("row");
        assert_eq!(
            row.version_support("cargo-geiger 0.12.0"),
            VersionSupport::Supported
        );
        assert!(!row.authoritative("cargo-geiger 0.12.0"));
    }

    // ------------------------------------------------------------------
    // Cargo metadata
    // ------------------------------------------------------------------

    #[test]
    fn cargo_metadata_requests_an_explicit_supported_format_version() {
        let command = cargo_metadata::MetadataCommand::new().cargo_command();
        let args: Vec<String> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        let position = args
            .iter()
            .position(|arg| arg == "--format-version")
            .expect("cargo metadata must request an explicit format version");
        assert_eq!(args.get(position + 1).map(String::as_str), Some("1"));
        let row = entry("cargo-metadata").expect("row");
        assert_eq!(row.version_support("1.0"), VersionSupport::Supported);
    }

    #[test]
    fn cargo_metadata_tolerates_additive_fields_without_losing_identity() {
        let metadata = cargo_metadata::MetadataCommand::parse(METADATA_ADDITIVE)
            .expect("additive unknown fields must not break parsing");
        let package = metadata
            .packages
            .first()
            .expect("package identity survives additive fields");
        assert_eq!(package.name.as_str(), "conformance-fixture");
        assert_eq!(package.version.to_string(), "0.1.0");
        let target = package.targets.first().expect("target identity survives");
        assert_eq!(target.name, "conformance-fixture");
        assert!(target.kind.iter().any(|kind| kind.to_string() == "lib"));
    }

    // ------------------------------------------------------------------
    // Supported fixtures normalize to the approved shape
    // ------------------------------------------------------------------

    #[test]
    fn a_supported_audit_document_normalizes_to_the_approved_shape() {
        let diagnostics =
            audit::parse_audit_report(AUDIT_REPORT, "cargo-audit 0.21.2 (parser audit-json-v1)")
                .expect("the supported fixture parses");
        let advisory = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.rule == "RUSTSEC-2023-0071")
            .expect("advisory identity is the rule identity");
        assert_eq!(advisory.file_path.to_string_lossy(), "Cargo.lock");
        assert_eq!(advisory.severity, crate::diagnostics::Severity::Error);
        assert_eq!(
            advisory.category,
            crate::diagnostics::Category::Dependencies
        );
        assert!(
            advisory
                .help
                .as_ref()
                .is_some_and(|help| help.contains("audit-json-v1")),
            "provenance survives normalization"
        );
    }

    #[test]
    fn a_supported_deny_document_keeps_advisory_identity_and_provenance() {
        for (report, provenance) in [
            (DENY_018_REPORT, "cargo-deny 0.18.3 (deny-jsonlines-v2)"),
            (DENY_019_REPORT, "cargo-deny 0.19.8 (deny-jsonlines-v2)"),
        ] {
            let diagnostics = deny::parse_deny_output(report, provenance, provenance, 15)
                .expect("qualified fixture parses");
            assert!(
                !diagnostics.is_empty(),
                "the supported fixture yields diagnostics"
            );
            assert!(
                diagnostics.iter().any(|diagnostic| {
                    crate::passes::adapter::advisory_identity(&diagnostic.message).as_deref()
                        == Some("RUSTSEC-2023-0071")
                }),
                "the advisory root cause survives normalization"
            );
            for diagnostic in &diagnostics {
                assert_eq!(diagnostic.file_path.to_string_lossy(), "Cargo.toml");
            }
        }
    }

    #[test]
    fn cargo_deny_capture_manifest_matches_the_conformance_row() {
        let manifest: serde_json::Value =
            serde_json::from_str(DENY_MATRIX).expect("capture manifest parses");
        let versions: Vec<_> = manifest["captures"]
            .as_array()
            .expect("captures are an array")
            .iter()
            .filter_map(|capture| capture["version"].as_str())
            .map(|version| version.rsplit_once('.').map_or(version, |(minor, _)| minor))
            .collect();
        let row = entry("cargo-deny").expect("cargo-deny conformance row");
        assert_eq!(versions, row.supported_versions);
        for capture in manifest["captures"].as_array().expect("captures") {
            assert_eq!(capture["offline_argument"], "--disable-fetch");
            assert_eq!(
                capture["capture_command"],
                "cargo deny --format json check --disable-fetch"
            );
            assert_eq!(
                capture["qualified_exit_codes"],
                serde_json::json!([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15])
            );
            assert_eq!(capture["capture_status"], "verified_real_binary");
            assert_eq!(capture["clean_exit_code"], 0);
            assert_eq!(capture["finding_exit_code"], 15);
            assert_eq!(capture["observed_channels"], serde_json::json!(["stderr"]));
            let version = capture["version"].as_str().expect("capture version");
            let (clean, findings) = if version.starts_with("0.18.") {
                (DENY_018_CLEAN, DENY_018_REPORT)
            } else {
                (DENY_019_CLEAN, DENY_019_REPORT)
            };
            assert_eq!(
                capture["fixture_sha256"]["clean"].as_str(),
                Some(sha256(clean.as_bytes()).as_str())
            );
            assert_eq!(
                capture["fixture_sha256"]["findings"].as_str(),
                Some(sha256(findings.as_bytes()).as_str())
            );
            assert!(
                capture["capture_binary_sha256"]
                    .as_str()
                    .is_some_and(is_sha256)
            );
            assert!(
                capture["advisory_database_commit"]
                    .as_str()
                    .is_some_and(|commit| {
                        commit.len() == 40 && commit.bytes().all(|byte| byte.is_ascii_hexdigit())
                    })
            );
            assert_eq!(
                capture["advisory_snapshot_tree_sha256"].as_str(),
                Some(advisory_snapshot_sha256().as_str())
            );
        }
    }

    fn sha256(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    fn is_sha256(value: &str) -> bool {
        value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
    }

    fn advisory_snapshot_sha256() -> String {
        let mut hasher = Sha256::new();
        hasher.update(b"support.toml\0");
        hasher.update(DENY_ADVISORY_SUPPORT.as_bytes());
        hasher.update(b"\0crates/rsa/RUSTSEC-2023-0071.md\0");
        hasher.update(DENY_RSA_ADVISORY.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    #[test]
    fn a_supported_geiger_report_is_unsafe_exposure_and_not_a_vulnerability() {
        let diagnostics = geiger::parse_geiger_ascii(GEIGER_REPORT, "cargo-geiger 0.12.0");
        assert!(!diagnostics.is_empty());
        for diagnostic in &diagnostics {
            assert_eq!(diagnostic.rule, "unsafe-dependency");
            let trust = crate::catalog::RuleTrust::resolve(
                &diagnostic.rule,
                crate::catalog::AnalyzerKind::External,
            );
            assert!(
                !trust.score_eligible,
                "unsafe exposure never scores as a confirmed defect"
            );
        }
    }

    #[test]
    fn a_supported_shear_document_records_its_package_scope() {
        let diagnostics = shear::parse_shear_output(SHEAR_REPORT, "cargo-shear 1.6.0")
            .expect("the supported fixture parses");
        let finding = diagnostics.first().expect("one unused dependency");
        assert_eq!(finding.rule, "unused-dependency");
        assert_eq!(finding.file_path.to_string_lossy(), "crates/api/Cargo.toml");
    }

    #[test]
    fn a_supported_semver_report_normalizes_to_a_semver_violation() {
        let diagnostics =
            semver_checks::parse_semver_output(SEMVER_REPORT, "cargo-semver-checks 0.42.0");
        assert!(!diagnostics.is_empty());
        for diagnostic in &diagnostics {
            assert_eq!(diagnostic.rule, "semver-violation");
        }
    }

    #[test]
    fn a_supported_cargo_stream_takes_identity_from_structured_fields() {
        let outcome = clippy::parse_message_stream(CARGO_STREAM.lines().map(ToString::to_string));
        assert!(outcome.build_succeeded);
        assert_eq!(outcome.foreign.non_json_lines(), 0);
        let diagnostic = outcome
            .diagnostics
            .first()
            .expect("the compiler message normalizes");
        // Identity comes from `code.code`, not from the rendered text.
        assert_eq!(diagnostic.rule, "clippy::unwrap_used");
        assert_eq!(diagnostic.line, Some(4));
        assert_eq!(diagnostic.file_path.to_string_lossy(), "src/lib.rs");
    }

    // ------------------------------------------------------------------
    // Degraded inputs
    // ------------------------------------------------------------------

    #[test]
    fn the_degraded_receipt_table_never_maps_to_clean_success() {
        for input in DegradedInput::ALL {
            let receipt = input.expected_receipt();
            assert!(
                matches!(
                    receipt,
                    ExpectedReceipt::Skipped
                        | ExpectedReceipt::Failed
                        | ExpectedReceipt::TimedOut
                        | ExpectedReceipt::PartialClassification
                ),
                "{input:?} has no receipt"
            );
        }
        assert_eq!(
            DegradedInput::MissingExecutable.expected_receipt().as_str(),
            "skipped"
        );
        assert_eq!(
            DegradedInput::Timeout.expected_receipt().as_str(),
            "timed-out"
        );
        assert_eq!(
            DegradedInput::MixedText.expected_receipt().as_str(),
            "partial"
        );
        assert_eq!(
            DegradedInput::MalformedDocument.expected_receipt().as_str(),
            "failed"
        );
    }

    #[test]
    fn every_adapter_produces_the_expected_receipt_for_each_degraded_run() {
        for contract in [
            audit::CONTRACT,
            deny::CONTRACT,
            geiger::CONTRACT,
            shear::CONTRACT,
            semver_checks::CONTRACT,
        ] {
            assert!(matches!(
                contract.skipped("install it"),
                PassError::Skipped { .. }
            ));

            let mut timed_out = output(None);
            timed_out.timed_out = true;
            assert!(matches!(
                contract.require_complete_run(&timed_out, &[0, 1]),
                Err(PassError::TimedOut { .. })
            ));

            let mut truncated = output(Some(0));
            truncated.truncated = true;
            assert!(matches!(
                contract.require_complete_run(&truncated, &[0, 1]),
                Err(PassError::Failed { .. })
            ));

            assert!(matches!(
                contract.require_complete_run(&output(Some(101)), &[0, 1]),
                Err(PassError::Failed { .. })
            ));
            assert!(matches!(
                contract.require_complete_run(&output(None), &[0, 1]),
                Err(PassError::Failed { .. })
            ));
        }
    }

    #[test]
    fn a_malformed_structured_document_fails_instead_of_reporting_no_findings() {
        let malformed = "{ this is not json";
        assert!(matches!(
            audit::parse_audit_report(malformed, "cargo-audit 0.21.2"),
            Err(PassError::Failed { .. })
        ));
        assert!(audit::parse_audit_report("   ", "cargo-audit 0.21.2").is_err());
        assert!(shear::parse_shear_output(malformed, "cargo-shear 1.6.0").is_err());
    }

    #[test]
    fn mixed_text_and_unknown_variants_stay_bounded_and_never_panic() {
        let outcome =
            clippy::parse_message_stream(CARGO_STREAM_DEGRADED.lines().map(ToString::to_string));
        // Valid compiler messages survive alongside build-script noise.
        assert!(
            outcome
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.rule == "clippy::unwrap_used"),
            "valid messages are retained"
        );
        assert!(
            outcome.foreign.non_json_lines() > 0,
            "non-JSON lines are classified, not dropped silently"
        );
        assert!(
            outcome.foreign.unknown_reason_count() > 0,
            "an unmodelled cargo reason is classified rather than discarded"
        );
        assert!(
            outcome.foreign.receipt().is_some(),
            "degraded stream yields a receipt"
        );
    }

    #[test]
    fn a_truncated_stream_never_fabricates_a_successful_build() {
        // The stream stops mid-document: no `build-finished` arrives, and the
        // parser must not invent one.
        let truncated = "{\"reason\":\"compiler-artifact\",\"package_id\":\"x\"";
        let outcome = clippy::parse_message_stream(std::iter::once(truncated.to_string()));
        assert!(outcome.diagnostics.is_empty());
        assert_eq!(outcome.foreign.non_json_lines(), 1);
        assert!(outcome.foreign.receipt().is_some());
    }

    #[test]
    fn unknown_variant_fixtures_produce_no_panic_and_no_fabricated_authority() {
        // NFR-020: every unknown-variant shape is survivable.
        for line in [
            "",
            "   ",
            "{}",
            "{\"reason\":\"future-cargo-event\"}",
            "{\"reason\":\"compiler-message\",\"message\":{}}",
            "not json at all",
            "[1, 2, 3]",
        ] {
            let outcome = clippy::parse_message_stream(std::iter::once(line.to_string()));
            assert!(
                outcome.diagnostics.iter().all(|diagnostic| {
                    !crate::catalog::RuleTrust::resolve(
                        &diagnostic.rule,
                        crate::catalog::AnalyzerKind::Clippy,
                    )
                    .score_eligible
                }),
                "an unknown variant must not become score-eligible"
            );
        }
    }
}
