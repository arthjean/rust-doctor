use clap::error::ErrorKind;
use clap::{ArgAction, Args, CommandFactory, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

/// Exit-code reference shown after `--help` (clap `after_help`).
///
/// Mirrors the exit constants in `run.rs` and the README so CI authors can
/// distinguish setup, scan, quality-gate, and incomplete-analysis failures.
const EXIT_CODES_HELP: &str = "\
Examples:
  rust-doctor . --scope changed --base main --blocking warning
  rust-doctor rules list --category security
  rust-doctor why src/lib.rs:42
  rust-doctor install --yes --hook pre-commit

Output contract:
  Terminal diagnostics use stderr; the score box uses stdout.
  --score writes one integer to stdout.
  --json, --json-compact, and --sarif write machine output to stdout.
  --json-out writes JSON atomically to PATH instead of stdout.
  --score, --sarif, --json, and --json-compact are mutually exclusive.
  --json-out may pair with --json or --json-compact, but not --score or --sarif.
  --share prints one stateless public URL after the terminal report.
  --color and --no-color affect terminal output only and conflict when explicit.

Exit codes:
  0  Success: scan completed and all quality gates passed
  1  Setup error: installer, MCP server, or --install-deps failed
  2  Scan error: project discovery, analysis, or output failed
  3  Quality gate failed: score or --blocking threshold reached
  4  Required analysis incomplete when --require-complete is active";

/// Diagnose your Rust project's health with a single command.
///
/// rust-doctor scans Rust codebases for security, performance, correctness,
/// architecture, and dependency issues, producing a 0-100 health score
/// with actionable diagnostics.
#[derive(Parser, Debug)]
#[command(
    version,
    about,
    long_about = None,
    args_conflicts_with_subcommands = true,
    after_help = EXIT_CODES_HELP,
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Directory to scan (defaults to current directory)
    #[arg(default_value = ".")]
    pub directory: PathBuf,

    /// Show detailed file:line information per diagnostic
    #[arg(long, short = 'v')]
    pub verbose: bool,

    /// Restrict reported findings to these categories
    #[arg(long, value_enum, value_delimiter = ',', value_name = "CATEGORY")]
    pub category: Vec<ScanCategory>,

    /// Whether warning diagnostics are shown in terminal output
    #[arg(long, value_enum, default_value_t = WarningVisibility::Show)]
    pub warnings: WarningVisibility,

    /// Maximum workspace package scans running concurrently
    #[arg(long, short = 'j', value_name = "COUNT", value_parser = parse_positive_usize)]
    pub jobs: Option<usize>,

    /// Print only the bare integer score (for CI piping)
    #[arg(long, conflicts_with_all = ["json", "json_compact", "json_out", "sarif"])]
    pub score: bool,

    /// Output full scan results as JSON
    #[arg(long, conflicts_with_all = ["score", "sarif", "json_compact"])]
    pub json: bool,

    /// Output Report V1 as compact JSON
    #[arg(long, conflicts_with_all = ["score", "sarif", "json"])]
    pub json_compact: bool,

    /// Write Report V1 atomically to a file instead of stdout
    #[arg(long, value_name = "PATH", conflicts_with_all = ["score", "sarif"])]
    pub json_out: Option<PathBuf>,

    /// Output results in SARIF 2.1.0 format (for GitHub Code Scanning, GitLab SAST)
    #[arg(long, conflicts_with_all = ["score", "json", "json_compact", "json_out"])]
    pub sarif: bool,

    /// Print a stateless public summary URL without uploading report data
    #[arg(long, conflicts_with_all = ["score", "json", "json_compact", "json_out", "sarif"])]
    pub share: bool,

    /// Scan only changed files vs a base branch
    #[arg(long, num_args = 0..=1, default_missing_value = "auto", value_name = "BASE")]
    pub diff: Option<String>,

    /// Reporting scope for the scan
    #[arg(long, value_enum, default_value_t = Scope::Full)]
    pub scope: Scope,

    /// Explicit files for --scope files (repeat or comma-separate)
    #[arg(long, value_delimiter = ',', value_name = "PATHS")]
    pub files: Vec<PathBuf>,

    /// Git base ref for changed, lines, or baseline scope
    #[arg(long, value_name = "REF")]
    pub base: Option<String>,

    /// Include Git-untracked, non-ignored files in changed or lines scope
    #[arg(long)]
    pub include_untracked: bool,

    /// Analyze the exact Git index snapshot
    #[arg(long, conflicts_with = "baseline")]
    pub staged: bool,

    /// Compare head findings with the resolved merge-base
    #[arg(long, conflicts_with = "staged")]
    pub baseline: bool,

    /// One wall-clock budget for the complete scan, in seconds
    #[arg(long, value_name = "SECONDS", value_parser = parse_positive_u64)]
    pub max_duration: Option<u64>,

    /// Fail the quality gate when required analysis is incomplete
    #[arg(long)]
    pub require_complete: bool,

    /// Fail the quality gate (exit code 3) when this severity is reached
    #[arg(long, value_enum, conflicts_with = "fail_on")]
    pub blocking: Option<FailOn>,

    /// Deprecated alias for --blocking
    #[arg(long, value_enum, conflicts_with = "blocking")]
    pub fail_on: Option<FailOn>,

    /// Report findings hidden by rust-doctor inline suppression comments
    #[arg(long)]
    pub no_respect_inline_disables: bool,

    /// Apply machine-applicable fixes from custom rules (modifies source files)
    #[arg(long)]
    pub fix: bool,

    /// Show a prioritized remediation plan after scanning
    #[arg(long)]
    pub plan: bool,

    /// Check and install missing external tools (cargo-deny, cargo-audit, etc.)
    #[arg(long)]
    pub install_deps: bool,

    /// Skip network-dependent checks (cargo-audit advisory DB fetch, etc.)
    #[arg(long)]
    pub offline: bool,

    /// Disable opt-in observability even when local consent is stored
    #[arg(long)]
    pub no_telemetry: bool,

    /// Run as an MCP (Model Context Protocol) stdio server for AI tool integration
    #[arg(long, conflicts_with_all = ["score", "json", "json_compact", "json_out", "sarif", "lsp"])]
    pub mcp: bool,

    /// Run the feature-gated language server over stdio
    #[arg(long, conflicts_with_all = ["score", "json", "json_compact", "json_out", "sarif", "mcp"])]
    pub lsp: bool,

    /// Ignore the project's rust-doctor.toml config file
    #[arg(long)]
    pub no_project_config: bool,

    /// Color policy for terminal output
    #[arg(long, value_enum, default_value_t = ColorMode::Auto, conflicts_with = "no_color")]
    pub color: ColorMode,

    /// Disable terminal color output
    #[arg(long, conflicts_with = "color")]
    pub no_color: bool,

    /// Scan only specific workspace members (comma-separated)
    #[arg(long, value_delimiter = ',', value_name = "NAMES", value_parser = parse_non_empty)]
    pub project: Vec<String>,
}

impl Cli {
    pub const fn wants_json(&self) -> bool {
        self.json || self.json_compact || self.json_out.is_some()
    }

    /// Validate value-dependent conflicts before project discovery starts.
    ///
    /// Clap handles structural conflicts while parsing. These checks cover
    /// relationships involving enum values, which derive attributes cannot
    /// express without starting application work first.
    pub fn validate_contract(&self) -> Result<(), clap::Error> {
        if self.diff.is_some()
            && (self.scope != Scope::Full
                || self.staged
                || self.baseline
                || self.base.is_some()
                || self.include_untracked
                || !self.files.is_empty())
        {
            return Err(usage_error(
                "--diff cannot be combined with --scope, --base, --staged, --baseline, --include-untracked, or --files",
            ));
        }
        if (self.staged || self.baseline) && self.scope != Scope::Full {
            return Err(usage_error(
                "--staged and --baseline cannot be combined with --scope",
            ));
        }
        if self.base.is_some()
            && !self.baseline
            && !matches!(self.scope, Scope::Changed | Scope::Lines)
        {
            return Err(usage_error(
                "--base requires --scope changed, --scope lines, or --baseline",
            ));
        }
        if self.include_untracked && !matches!(self.scope, Scope::Changed | Scope::Lines) {
            return Err(usage_error(
                "--include-untracked requires --scope changed or --scope lines",
            ));
        }
        if !self.files.is_empty() && self.scope != Scope::Files {
            return Err(usage_error("--files requires --scope files"));
        }
        if self.scope == Scope::Files && self.files.is_empty() {
            return Err(usage_error(
                "--scope files requires at least one --files path",
            ));
        }
        Ok(())
    }
}

fn usage_error(message: &str) -> clap::Error {
    Cli::command().error(ErrorKind::ArgumentConflict, message)
}

/// Reject empty project name segments (e.g. `--project ,api` or `--project core,`)
fn parse_non_empty(s: &str) -> Result<String, String> {
    if s.is_empty() {
        Err("project name cannot be empty".to_string())
    } else {
        Ok(s.to_string())
    }
}

fn parse_positive_u64(value: &str) -> Result<u64, String> {
    let parsed = value
        .parse::<u64>()
        .map_err(|_| "duration must be a positive integer".to_string())?;
    if parsed == 0 {
        Err("duration must be greater than zero".to_string())
    } else {
        Ok(parsed)
    }
}

fn parse_positive_usize(value: &str) -> Result<usize, String> {
    let parsed = value
        .parse::<usize>()
        .map_err(|_| "parallelism must be a positive integer".to_string())?;
    if parsed == 0 {
        Err("parallelism must be greater than zero".to_string())
    } else {
        Ok(parsed)
    }
}

/// File-reporting scope. Staged and baseline are separate flags because they
/// select an alternate source snapshot in addition to a reporting scope.
#[derive(ValueEnum, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    #[default]
    Full,
    Files,
    Changed,
    Lines,
}

/// Diagnostic category accepted by scan and rule-list filters.
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
#[value(rename_all = "kebab-case")]
pub enum ScanCategory {
    ErrorHandling,
    Performance,
    Security,
    Correctness,
    Architecture,
    Dependencies,
    Async,
    Framework,
    Cargo,
    Style,
}

/// Terminal warning rendering policy.
#[derive(ValueEnum, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[value(rename_all = "lowercase")]
pub enum WarningVisibility {
    #[default]
    Show,
    Hide,
}

/// Terminal color rendering policy.
#[derive(ValueEnum, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[value(rename_all = "lowercase")]
pub enum ColorMode {
    #[default]
    Auto,
    Always,
    Never,
}

/// When to exit with a non-zero status code
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailOn {
    /// Exit 3 if any errors found
    Error,
    /// Exit 3 if any errors or warnings found
    Warning,
    /// Exit 3 if any errors, warnings, or info findings found
    Info,
    /// Always exit 0 (unless rust-doctor itself crashes)
    None,
}

/// Blocking level supported by generated staged-scan hooks.
#[derive(ValueEnum, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[value(rename_all = "lowercase")]
pub enum HookBlocking {
    Error,
    #[default]
    Warning,
    None,
}

impl std::fmt::Display for FailOn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Error => write!(f, "error"),
            Self::Warning => write!(f, "warning"),
            Self::Info => write!(f, "info"),
            Self::None => write!(f, "none"),
        }
    }
}

/// Subcommands. With no subcommand, rust-doctor scans the selected directory.
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Install Rust Doctor into supported coding agents
    #[command(alias = "setup")]
    Install(InstallArgs),
    /// Remove only Rust Doctor-managed agent files and marked hook blocks
    Uninstall(UninstallArgs),
    /// Discover, explain, and configure rules
    Rules(RulesArgs),
    /// Explain findings and suppression decisions at one source location
    Why(WhyArgs),
    /// Install, configure, or upgrade managed CI scaffolding
    Ci(CiArgs),
    /// Manage privacy-safe observability consent
    Telemetry(TelemetryArgs),
    /// Show Rust Doctor and local Rust toolchain information
    Version,
}

#[derive(Args, Debug)]
pub struct TelemetryArgs {
    #[command(subcommand)]
    pub command: TelemetryCommand,
}

#[derive(Subcommand, Debug)]
pub enum TelemetryCommand {
    /// Store explicit consent for one HTTPS endpoint
    Enable(TelemetryEnableArgs),
    /// Revoke consent and remove the local endpoint
    Disable,
    /// Show effective consent and override state
    Status,
}

#[derive(Args, Debug)]
pub struct TelemetryEnableArgs {
    /// HTTPS collector endpoint; loopback HTTP is accepted for local development
    #[arg(long)]
    pub endpoint: String,
    /// Accept the displayed aggregate event contract without prompting
    #[arg(long)]
    pub yes: bool,
}

#[derive(Args, Debug)]
pub struct CiArgs {
    #[command(subcommand)]
    pub command: CiCommand,
}

#[derive(Subcommand, Debug)]
pub enum CiCommand {
    /// Install a namespaced CI workflow block
    Install(CiInstallArgs),
    /// Change only Rust Doctor-owned workflow settings
    Config(CiConfigArgs),
    /// Upgrade the managed action major and required permissions
    Upgrade(CiUpgradeArgs),
}

#[derive(Args, Debug)]
pub struct CiInstallArgs {
    #[arg(default_value = ".")]
    pub directory: PathBuf,
    #[arg(long, value_enum, default_value_t = CiProvider::Github)]
    pub provider: CiProvider,
    #[arg(long, value_enum, default_value_t = CiScope::Baseline)]
    pub scope: CiScope,
    #[arg(long, value_enum, default_value_t = FailOn::Warning)]
    pub blocking: FailOn,
    #[arg(long, default_value_t = true, action = ArgAction::Set)]
    pub comment: bool,
    #[arg(long, default_value_t = false, action = ArgAction::Set)]
    pub review_comments: bool,
    #[arg(long, default_value_t = true, action = ArgAction::Set)]
    pub commit_status: bool,
    #[arg(long, default_value_t = true, action = ArgAction::Set)]
    pub sarif: bool,
    #[arg(long, default_value = "v1", value_parser = parse_action_major)]
    pub version: String,
    #[arg(long)]
    pub dry_run: bool,
    /// Create a branch and pull request after local validation succeeds
    #[arg(long, conflicts_with = "dry_run")]
    pub pr: bool,
    /// Add a non-closing `Refs #N` line to generated pull-request text
    #[arg(long, requires = "pr")]
    pub issue: Option<u64>,
}

#[derive(Args, Debug)]
pub struct CiConfigArgs {
    #[arg(default_value = ".")]
    pub directory: PathBuf,
    #[arg(long, value_enum, default_value_t = CiProvider::Github)]
    pub provider: CiProvider,
    #[arg(long, value_enum)]
    pub scope: Option<CiScope>,
    #[arg(long, value_enum)]
    pub blocking: Option<FailOn>,
    #[arg(long, action = ArgAction::Set)]
    pub comment: Option<bool>,
    #[arg(long, action = ArgAction::Set)]
    pub review_comments: Option<bool>,
    #[arg(long, action = ArgAction::Set)]
    pub commit_status: Option<bool>,
    #[arg(long, action = ArgAction::Set)]
    pub sarif: Option<bool>,
    #[arg(long, value_parser = parse_action_major)]
    pub version: Option<String>,
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Args, Debug)]
pub struct CiUpgradeArgs {
    #[arg(default_value = ".")]
    pub directory: PathBuf,
    #[arg(long, value_enum, default_value_t = CiProvider::Github)]
    pub provider: CiProvider,
    #[arg(long, default_value = "v1", value_parser = parse_action_major)]
    pub version: String,
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(ValueEnum, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[value(rename_all = "lowercase")]
pub enum CiProvider {
    #[default]
    Github,
    Gitlab,
}

impl std::fmt::Display for CiProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Github => "github",
            Self::Gitlab => "gitlab",
        })
    }
}

#[derive(
    ValueEnum, Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize,
)]
#[value(rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum CiScope {
    Full,
    Changed,
    #[default]
    Baseline,
    Staged,
}

impl std::fmt::Display for CiScope {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Full => "full",
            Self::Changed => "changed",
            Self::Baseline => "baseline",
            Self::Staged => "staged",
        })
    }
}

fn parse_action_major(value: &str) -> Result<String, String> {
    let Some(digits) = value.strip_prefix('v') else {
        return Err("action version must be a major such as v1".to_string());
    };
    if digits.is_empty() || !digits.chars().all(|character| character.is_ascii_digit()) {
        return Err("action version must be a major such as v1".to_string());
    }
    Ok(value.to_string())
}

#[derive(Args, Debug)]
pub struct RulesArgs {
    #[command(subcommand)]
    pub command: RulesCommand,
}

#[derive(Subcommand, Debug)]
pub enum RulesCommand {
    /// List canonical rules and their effective policy
    List(RuleListArgs),
    /// Explain one canonical or external rule
    Explain(RuleExplainArgs),
    /// Set a rule level and optional supported threshold
    Set(RuleSetArgs),
    /// Enable a rule at its catalog default level
    Enable(RuleToggleArgs),
    /// Disable a rule
    Disable(RuleToggleArgs),
    /// Set a category-wide level
    Category(CategoryMutationArgs),
    /// Disable every rule carrying a tag
    IgnoreTag(TagMutationArgs),
    /// Remove a tag-wide override
    UnignoreTag(TagMutationArgs),
}

#[derive(Args, Debug)]
pub struct RuleListArgs {
    /// Project directory whose effective configuration should be resolved
    #[arg(default_value = ".")]
    pub directory: PathBuf,
    #[arg(long, value_enum)]
    pub category: Option<ScanCategory>,
    #[arg(long)]
    pub tag: Option<String>,
    #[arg(long)]
    pub framework: Option<String>,
    #[arg(long, value_enum)]
    pub analyzer: Option<AnalyzerFilter>,
    #[arg(long)]
    pub configured_only: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct RuleExplainArgs {
    pub rule: String,
    #[arg(default_value = ".")]
    pub directory: PathBuf,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct RuleSetArgs {
    pub rule: String,
    #[arg(value_enum)]
    pub level: RuleLevelArg,
    #[arg(long)]
    pub threshold: Option<u32>,
    #[arg(long, default_value = ".")]
    pub directory: PathBuf,
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Args, Debug)]
pub struct RuleToggleArgs {
    pub rule: String,
    #[arg(long, default_value = ".")]
    pub directory: PathBuf,
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Args, Debug)]
pub struct CategoryMutationArgs {
    #[arg(value_enum)]
    pub category: ScanCategory,
    #[arg(value_enum)]
    pub level: RuleLevelArg,
    #[arg(long, default_value = ".")]
    pub directory: PathBuf,
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Args, Debug)]
pub struct TagMutationArgs {
    pub tag: String,
    #[arg(long, default_value = ".")]
    pub directory: PathBuf,
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
#[value(rename_all = "kebab-case")]
pub enum AnalyzerFilter {
    SynAst,
    Clippy,
    Dependency,
    Project,
    External,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
#[value(rename_all = "lowercase")]
pub enum RuleLevelArg {
    Off,
    Info,
    Warning,
    Error,
}

#[derive(Args, Debug)]
pub struct WhyArgs {
    /// Source location in FILE:LINE or FILE:LINE:COLUMN form
    pub location: String,
    #[arg(long, default_value = ".")]
    pub directory: PathBuf,
    #[arg(long)]
    pub rule: Option<String>,
    #[arg(long)]
    pub json: bool,
    #[arg(long)]
    pub offline: bool,
    #[arg(long, value_name = "SECONDS", value_parser = parse_positive_u64)]
    pub max_duration: Option<u64>,
    #[arg(long)]
    pub no_project_config: bool,
}

#[derive(Args, Debug)]
pub struct InstallArgs {
    #[arg(default_value = ".")]
    pub directory: PathBuf,
    #[arg(long, value_enum, value_delimiter = ',')]
    pub agent: Vec<AgentId>,
    /// Accept the detected plan without prompting
    #[arg(long)]
    pub yes: bool,
    /// Print every proposed change without writing
    #[arg(long)]
    pub dry_run: bool,
    /// Also configure the existing MCP server entry
    #[arg(long)]
    pub mcp: bool,
    /// Do not install the Rust Doctor skill
    #[arg(long)]
    pub no_skill: bool,
    /// Install a namespaced staged-scan hook
    #[arg(long, value_enum)]
    pub hook: Option<HookKind>,
    /// Blocking level embedded in generated hooks
    #[arg(long, value_enum, default_value_t = HookBlocking::Warning)]
    pub blocking: HookBlocking,
}

#[derive(Args, Debug)]
pub struct UninstallArgs {
    #[arg(default_value = ".")]
    pub directory: PathBuf,
    #[arg(long, value_enum, value_delimiter = ',')]
    pub agent: Vec<AgentId>,
    #[arg(long)]
    pub yes: bool,
    #[arg(long)]
    pub dry_run: bool,
    /// Remove only skill files when any component selector is present
    #[arg(long)]
    pub skill: bool,
    /// Remove only MCP entries when any component selector is present
    #[arg(long)]
    pub mcp: bool,
    /// Remove only the marked hook block when any selector is present
    #[arg(long)]
    pub hook: bool,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
#[value(rename_all = "kebab-case")]
pub enum AgentId {
    ClaudeCode,
    Cursor,
    Codex,
    OpenCode,
    Windsurf,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
#[value(rename_all = "kebab-case")]
pub enum HookKind {
    PreCommit,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, Parser};

    #[test]
    fn test_default_directory() {
        let cli = Cli::try_parse_from(["rust-doctor"]).unwrap();
        assert_eq!(cli.directory, PathBuf::from("."));
    }

    #[test]
    fn test_help_documents_exit_codes() {
        // US-010: `--help` must carry the exit-code reference so CI authors can
        // tell a quality-gate failure (3) from a crash (2) or setup error (1).
        let help = Cli::command().render_long_help().to_string();
        assert!(
            help.contains("Exit codes:"),
            "help is missing the exit-code section:\n{help}"
        );
        assert!(help.contains("Quality gate failed"));
        assert!(help.contains("Examples:"));
        for code in ["  0 ", "  1 ", "  2 ", "  3 ", "  4 "] {
            assert!(
                help.contains(code),
                "help is missing exit code line `{code}`"
            );
        }
    }

    #[test]
    fn test_custom_directory() {
        let cli = Cli::try_parse_from(["rust-doctor", "/some/path"]).unwrap();
        assert_eq!(cli.directory, PathBuf::from("/some/path"));
    }

    #[test]
    fn test_score_flag() {
        let cli = Cli::try_parse_from(["rust-doctor", "--score"]).unwrap();
        assert!(cli.score);
    }

    #[test]
    fn test_json_flag() {
        let cli = Cli::try_parse_from(["rust-doctor", "--json"]).unwrap();
        assert!(cli.json);
    }

    #[test]
    fn test_json_contract_flags() {
        let compact = Cli::try_parse_from(["rust-doctor", "--json-compact"]).unwrap();
        assert!(compact.wants_json());

        let file = Cli::try_parse_from(["rust-doctor", "--json-out", "report.json"]).unwrap();
        assert_eq!(file.json_out, Some(PathBuf::from("report.json")));
        assert!(file.wants_json());

        assert!(Cli::try_parse_from(["rust-doctor", "--json", "--json-compact"]).is_err());
    }

    #[test]
    fn test_score_and_json_conflict() {
        let result = Cli::try_parse_from(["rust-doctor", "--score", "--json"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_verbose_flag() {
        let cli = Cli::try_parse_from(["rust-doctor", "--verbose"]).unwrap();
        assert!(cli.verbose);
    }

    #[test]
    fn test_offline_flag() {
        let cli = Cli::try_parse_from(["rust-doctor", "--offline"]).unwrap();
        assert!(cli.offline);
    }

    #[test]
    fn test_lsp_and_mcp_are_exclusive() {
        let cli = Cli::try_parse_from(["rust-doctor", "--lsp"]).unwrap();
        assert!(cli.lsp);
        assert!(Cli::try_parse_from(["rust-doctor", "--lsp", "--mcp"]).is_err());
    }

    #[test]
    fn test_fail_on_default() {
        let cli = Cli::try_parse_from(["rust-doctor"]).unwrap();
        assert_eq!(cli.fail_on, Option::None);
        assert_eq!(cli.blocking, Option::None);
    }

    #[test]
    fn test_fail_on_error() {
        let cli = Cli::try_parse_from(["rust-doctor", "--fail-on", "error"]).unwrap();
        assert_eq!(cli.fail_on, Some(FailOn::Error));
    }

    #[test]
    fn test_fail_on_warning() {
        let cli = Cli::try_parse_from(["rust-doctor", "--fail-on", "warning"]).unwrap();
        assert_eq!(cli.fail_on, Some(FailOn::Warning));
    }

    #[test]
    fn test_fail_on_none() {
        let cli = Cli::try_parse_from(["rust-doctor", "--fail-on", "none"]).unwrap();
        assert_eq!(cli.fail_on, Some(FailOn::None));
    }

    #[test]
    fn test_fail_on_invalid() {
        let result = Cli::try_parse_from(["rust-doctor", "--fail-on", "critical"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_diff_without_value() {
        let cli = Cli::try_parse_from(["rust-doctor", "--diff"]).unwrap();
        assert_eq!(cli.diff, Some("auto".to_string()));
    }

    #[test]
    fn test_diff_with_value() {
        let cli = Cli::try_parse_from(["rust-doctor", "--diff", "main"]).unwrap();
        assert_eq!(cli.diff, Some("main".to_string()));
    }

    #[test]
    fn test_diff_absent() {
        let cli = Cli::try_parse_from(["rust-doctor"]).unwrap();
        assert_eq!(cli.diff, Option::None);
    }

    #[test]
    fn test_typed_scope_flags() {
        let changed = Cli::try_parse_from([
            "rust-doctor",
            "--scope",
            "changed",
            "--base",
            "main",
            "--include-untracked",
        ])
        .unwrap();
        assert_eq!(changed.scope, Scope::Changed);
        assert_eq!(changed.base.as_deref(), Some("main"));
        assert!(changed.include_untracked);

        let files = Cli::try_parse_from([
            "rust-doctor",
            "--scope",
            "files",
            "--files",
            "src/lib.rs,src/main.rs",
        ])
        .unwrap();
        assert_eq!(files.scope, Scope::Files);
        assert_eq!(files.files.len(), 2);

        assert!(Cli::try_parse_from(["rust-doctor", "--staged", "--baseline"]).is_err());
    }

    #[test]
    fn test_max_duration_must_be_positive() {
        assert!(Cli::try_parse_from(["rust-doctor", "--max-duration", "0"]).is_err());
        let cli = Cli::try_parse_from(["rust-doctor", "--max-duration", "30"]).unwrap();
        assert_eq!(cli.max_duration, Some(30));
    }

    #[test]
    fn test_project_single() {
        let cli = Cli::try_parse_from(["rust-doctor", "--project", "core"]).unwrap();
        assert_eq!(cli.project, vec!["core"]);
    }

    #[test]
    fn test_project_comma_separated() {
        let cli = Cli::try_parse_from(["rust-doctor", "--project", "core,api,web"]).unwrap();
        assert_eq!(cli.project, vec!["core", "api", "web"]);
    }

    #[test]
    fn test_project_empty_by_default() {
        let cli = Cli::try_parse_from(["rust-doctor"]).unwrap();
        assert!(cli.project.is_empty());
    }

    #[test]
    fn test_project_rejects_empty_name() {
        let result = Cli::try_parse_from(["rust-doctor", "--project", ",api"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_version_flag() {
        let result = Cli::try_parse_from(["rust-doctor", "--version"]);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::DisplayVersion);
    }

    #[test]
    fn test_help_flag() {
        let result = Cli::try_parse_from(["rust-doctor", "--help"]);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::DisplayHelp);
    }

    #[test]
    fn test_install_deps_flag() {
        let cli = Cli::try_parse_from(["rust-doctor", "--install-deps"]).unwrap();
        assert!(cli.install_deps);
    }

    #[test]
    fn test_setup_subcommand() {
        let cli = Cli::try_parse_from(["rust-doctor", "setup"]).unwrap();
        assert!(matches!(cli.command, Some(Command::Install(_))));
    }

    #[test]
    fn test_rules_and_why_subcommands() {
        let rules = Cli::try_parse_from([
            "rust-doctor",
            "rules",
            "list",
            "--category",
            "security",
            "--json",
        ])
        .unwrap();
        assert!(matches!(rules.command, Some(Command::Rules(_))));

        let why = Cli::try_parse_from(["rust-doctor", "why", "src/lib.rs:42:7"]).unwrap();
        assert!(matches!(why.command, Some(Command::Why(_))));
    }

    #[test]
    fn test_ci_subcommands_and_boolean_channels() {
        let install = Cli::try_parse_from([
            "rust-doctor",
            "ci",
            "install",
            "--scope",
            "baseline",
            "--comment=false",
            "--review-comments=true",
            "--dry-run",
        ])
        .unwrap();
        let Some(Command::Ci(arguments)) = install.command else {
            panic!("expected ci command");
        };
        let CiCommand::Install(arguments) = arguments.command else {
            panic!("expected ci install command");
        };
        assert!(!arguments.comment);
        assert!(arguments.review_comments);
        assert_eq!(arguments.scope, CiScope::Baseline);

        let config =
            Cli::try_parse_from(["rust-doctor", "ci", "config", "--commit-status=false"]).unwrap();
        let Some(Command::Ci(arguments)) = config.command else {
            panic!("expected ci command");
        };
        let CiCommand::Config(arguments) = arguments.command else {
            panic!("expected ci config command");
        };
        assert_eq!(arguments.commit_status, Some(false));

        assert!(
            Cli::try_parse_from(["rust-doctor", "ci", "upgrade", "--version", "latest"]).is_err()
        );
    }

    #[test]
    fn test_scan_contract_rejects_value_dependent_conflicts() {
        let diff =
            Cli::try_parse_from(["rust-doctor", "--diff", "main", "--scope", "changed"]).unwrap();
        assert!(diff.validate_contract().is_err());

        let files = Cli::try_parse_from(["rust-doctor", "--scope", "files"]).unwrap();
        assert!(files.validate_contract().is_err());

        let valid = Cli::try_parse_from([
            "rust-doctor",
            "--scope",
            "changed",
            "--base",
            "main",
            "--jobs",
            "2",
            "--blocking",
            "warning",
        ])
        .unwrap();
        assert!(valid.validate_contract().is_ok());
    }

    #[test]
    fn test_output_modes_have_stable_conflicts() {
        assert!(Cli::try_parse_from(["rust-doctor", "--sarif", "--json-out", "x"]).is_err());
        assert!(Cli::try_parse_from(["rust-doctor", "--score", "--json-compact"]).is_err());
        assert!(Cli::try_parse_from(["rust-doctor", "--share", "--json"]).is_err());
        assert!(
            Cli::try_parse_from(["rust-doctor", "--blocking", "error", "--fail-on", "warning"])
                .is_err()
        );
    }

    #[test]
    fn test_share_is_explicit_and_local() {
        assert!(!Cli::try_parse_from(["rust-doctor"]).unwrap().share);
        let cli = Cli::try_parse_from(["rust-doctor", "--share", "--offline"]).unwrap();
        assert!(cli.share);
        assert!(cli.offline);
    }

    #[test]
    fn test_all_flags_combined() {
        let cli = Cli::try_parse_from([
            "rust-doctor",
            "/my/project",
            "--verbose",
            "--score",
            "--diff",
            "develop",
            "--fail-on",
            "warning",
            "--offline",
            "--project",
            "core,api",
        ])
        .unwrap();

        assert_eq!(cli.directory, PathBuf::from("/my/project"));
        assert!(cli.verbose);
        assert!(cli.score);
        assert!(!cli.json);
        assert_eq!(cli.diff, Some("develop".to_string()));
        assert_eq!(cli.scope, Scope::Full);
        assert_eq!(cli.fail_on, Some(FailOn::Warning));
        assert!(cli.offline);
        assert_eq!(cli.project, vec!["core", "api"]);
    }
}
