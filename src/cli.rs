use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

/// Exit-code reference shown after `--help` (clap `after_help`).
///
/// Mirrors the constants in `run.rs` (`EXIT_SCAN_ERROR` = 2,
/// `EXIT_GATE_FAILURE` = 3) and the "Exit Codes" section of the README so CI
/// authors can distinguish a quality-gate failure from a crash.
const EXIT_CODES_HELP: &str = "\
Exit codes:
  0  Success — scan completed and all quality gates passed
  1  Setup error — MCP server, setup wizard, or --install-deps failed
  2  Scan error — project discovery/compile failure or output rendering failed
  3  Quality gate failed: score, --fail-on, or --require-complete threshold reached

CI gating example:
  rust-doctor --fail-on error; if [ $? -eq 3 ]; then echo 'quality gate failed'; fi";

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

    /// Print only the bare integer score (for CI piping)
    #[arg(long, conflicts_with = "json")]
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
    #[arg(long, conflicts_with_all = ["score", "json"])]
    pub sarif: bool,

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
    #[arg(long, value_enum)]
    pub fail_on: Option<FailOn>,

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

    /// Run as an MCP (Model Context Protocol) stdio server for AI tool integration
    #[arg(long, conflicts_with_all = ["score", "json", "json_compact", "json_out"])]
    pub mcp: bool,

    /// Ignore the project's rust-doctor.toml config file
    #[arg(long)]
    pub no_project_config: bool,

    /// Scan only specific workspace members (comma-separated)
    #[arg(long, value_delimiter = ',', value_name = "NAMES", value_parser = parse_non_empty)]
    pub project: Vec<String>,
}

impl Cli {
    pub const fn wants_json(&self) -> bool {
        self.json || self.json_compact || self.json_out.is_some()
    }
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

/// Subcommands (optional — default behavior is scanning).
#[derive(Subcommand, Debug, Clone)]
pub enum Command {
    /// Interactive setup wizard — configure rust-doctor for your AI coding agent
    Setup,
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
        for code in ["  0 ", "  1 ", "  2 ", "  3 "] {
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
    fn test_fail_on_default() {
        let cli = Cli::try_parse_from(["rust-doctor"]).unwrap();
        assert_eq!(cli.fail_on, Option::None);
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
        assert!(matches!(cli.command, Some(Command::Setup)));
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
