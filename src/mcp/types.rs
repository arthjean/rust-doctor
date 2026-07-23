use crate::diagnostics::ScoreLabel;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Input schemas (schemars-derived)
// ---------------------------------------------------------------------------

#[derive(Deserialize, Serialize, JsonSchema)]
pub struct ScanInput {
    /// Absolute path to the Rust project directory (must contain a Cargo.toml).
    #[schemars(
        description = "Absolute path to the Rust project directory to analyze. Must contain a Cargo.toml file."
    )]
    pub directory: String,
    /// Canonical reporting scope. Defaults to full.
    #[serde(default)]
    #[schemars(
        description = "Reporting scope: full, files, changed, lines, staged, or baseline. Defaults to full."
    )]
    pub scope: McpScanScope,
    /// Explicit files for the files scope.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(description = "Project-relative paths required by the files scope.")]
    pub files: Vec<String>,
    /// Git base ref for changed, lines, or baseline scope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(description = "Git base ref for changed, lines, or baseline scope.")]
    pub base: Option<String>,
    /// Include untracked files in changed or lines scope.
    #[serde(default)]
    #[schemars(description = "Include Git-untracked files in changed or lines scope.")]
    pub include_untracked: bool,
    /// Deprecated changed-scope base retained for MCP client compatibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(
        description = "Deprecated alias for scope=changed plus base. Cannot be combined with scope, files, base, or include_untracked."
    )]
    pub diff: Option<String>,
    /// Run in offline mode (no network fetches). Defaults to true in MCP mode for security.
    #[serde(default = "default_mcp_offline")]
    #[schemars(
        description = "When true, cargo-audit runs with --no-fetch (no network access). Defaults to true in MCP mode for security."
    )]
    pub offline: bool,
    /// Ignore the project's rust-doctor.toml config file.
    #[serde(default)]
    #[schemars(
        description = "When true, ignores the project's rust-doctor.toml config file. Useful for untrusted projects."
    )]
    pub ignore_project_config: bool,
}

#[derive(Clone, Copy, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpScanScope {
    #[default]
    Full,
    Files,
    Changed,
    Lines,
    Staged,
    Baseline,
}

pub(super) const fn default_mcp_offline() -> bool {
    true
}

#[derive(Deserialize, Serialize, JsonSchema)]
pub struct ScoreInput {
    /// Absolute path to the Rust project directory.
    #[schemars(
        description = "Absolute path to the Rust project directory to score. Must contain a Cargo.toml file."
    )]
    pub directory: String,
    /// Run in offline mode (no network fetches). Defaults to true in MCP mode for security.
    #[serde(default = "default_mcp_offline")]
    #[schemars(
        description = "When true, cargo-audit runs with --no-fetch (no network access). Defaults to true in MCP mode."
    )]
    pub offline: bool,
    /// Ignore the project's rust-doctor.toml config file.
    #[serde(default)]
    #[schemars(
        description = "When true, ignores the project's rust-doctor.toml config file (so config cannot suppress security rules and inflate the score). Useful for untrusted projects. Aligned with the scan tool; defaults to false."
    )]
    pub ignore_project_config: bool,
}

#[derive(Deserialize, Serialize, JsonSchema)]
pub struct ExplainRuleInput {
    /// The rule ID (e.g. "unwrap-in-production", "clippy::expect_used", "blocking-in-async").
    #[schemars(
        description = "Rule identifier to explain. Accepts custom rule IDs (e.g. 'unwrap-in-production') or clippy lint names (e.g. 'clippy::expect_used'). Use list_rules to discover available IDs."
    )]
    pub rule: String,
}

#[derive(Deserialize, Serialize, JsonSchema)]
pub struct DeepAuditArgs {
    /// Absolute path to the Rust project directory.
    #[schemars(
        description = "Absolute path to the Rust project directory to audit. Must contain a Cargo.toml file."
    )]
    pub directory: String,
}

#[derive(Deserialize, Serialize, JsonSchema)]
pub struct HealthCheckArgs {
    /// Absolute path to the Rust project directory.
    #[schemars(
        description = "Absolute path to the Rust project directory to check. Must contain a Cargo.toml file."
    )]
    pub directory: String,
}

// ---------------------------------------------------------------------------
// Output schemas (schemars-derived for MCP structured output)
// ---------------------------------------------------------------------------

/// Maximum number of example locations shown per diagnostic group.
pub(super) const MAX_EXAMPLES_PER_GROUP: usize = 3;

/// A single example location for a diagnostic finding.
#[derive(Serialize, JsonSchema)]
pub struct DiagnosticExample {
    /// Source file path (relative to project root).
    pub file_path: String,
    /// Line number (1-based).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    /// Column number (1-based).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
}

/// Diagnostics grouped by rule — reduces output from thousands of individual
/// findings to ~70 compact groups that fit in an LLM context window.
#[derive(Serialize, JsonSchema)]
pub struct DiagnosticGroup {
    /// Rule identifier (e.g. "unwrap-in-production", "clippy::expect_used").
    pub rule: String,
    /// Severity: "error", "warning", or "info".
    pub severity: String,
    /// Category (e.g. "Error Handling", "Security", "Performance").
    pub category: String,
    /// Total number of occurrences across the project.
    pub count: usize,
    /// Representative diagnostic message.
    pub message: String,
    /// Actionable fix guidance.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    /// Example locations (up to 3).
    pub examples: Vec<DiagnosticExample>,
}

/// Structured output for the score tool.
#[derive(Serialize, JsonSchema)]
pub struct ScoreOutput {
    /// Health score from 0 (critical) to 100 (perfect).
    pub score: u32,
    /// Human-readable score label.
    pub score_label: ScoreLabel,
}
