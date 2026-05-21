use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

/// Diagnose your Rust project's health with a single command.
///
/// rust-doctor scans Rust codebases for security, performance, correctness,
/// architecture, and dependency issues, producing a 0-100 health score
/// with actionable diagnostics.
#[derive(Parser, Debug)]
#[command(version, about, long_about = None, args_conflicts_with_subcommands = true)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Directory to scan (defaults to current directory)
    #[arg(default_value = ".")]
    pub directory: PathBuf,

    /// Show detailed file:line information per diagnostic
    #[arg(long, short = 'v')]
    pub verbose: bool,

    /// Filter verbose output to a specific category
    #[arg(long, value_enum, value_name = "CATEGORY")]
    pub category: Option<CategoryFilter>,

    /// Print only the bare integer score (for CI piping)
    #[arg(long, conflicts_with = "json")]
    pub score: bool,

    /// Output full scan results as JSON
    #[arg(long, conflicts_with_all = ["score", "sarif"])]
    pub json: bool,

    /// Output results in SARIF 2.1.0 format (for GitHub Code Scanning, GitLab SAST)
    #[arg(long, conflicts_with_all = ["score", "json"])]
    pub sarif: bool,

    /// Scan only changed files vs a base branch
    #[arg(long, num_args = 0..=1, default_missing_value = "auto", value_name = "BASE")]
    pub diff: Option<String>,

    /// Exit with code 1 when this severity is reached
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
    #[arg(long, conflicts_with_all = ["score", "json"])]
    pub mcp: bool,

    /// Ignore the project's rust-doctor.toml config file
    #[arg(long)]
    pub no_project_config: bool,

    /// Scan only specific workspace members (comma-separated)
    #[arg(long, value_delimiter = ',', value_name = "NAMES", value_parser = parse_non_empty)]
    pub project: Vec<String>,
}

/// Reject empty project name segments (e.g. `--project ,api` or `--project core,`)
fn parse_non_empty(s: &str) -> Result<String, String> {
    if s.is_empty() {
        Err("project name cannot be empty".to_string())
    } else {
        Ok(s.to_string())
    }
}

/// When to exit with a non-zero status code
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailOn {
    /// Exit 1 if any errors found
    Error,
    /// Exit 1 if any errors or warnings found
    Warning,
    /// Exit 1 if any errors, warnings, or info findings found
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

/// Category filter for `--category` flag.
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum CategoryFilter {
    /// Error handling rules (unwrap-in-production, panic-in-library, etc.)
    ErrorHandling,
    /// Performance rules (excessive-clone, collect-then-iterate, etc.)
    Performance,
    /// Security rules (hardcoded-secrets, unsafe-block-audit, etc.)
    Security,
    /// Correctness rules
    Correctness,
    /// Architecture rules (high-cyclomatic-complexity, etc.)
    Architecture,
    /// Dependency audit rules
    Dependencies,
    /// Async rules (blocking-in-async, block-on-in-async)
    Async,
    /// Framework rules (tokio-main-missing, axum-handler-not-async, etc.)
    Framework,
    /// Cargo rules
    Cargo,
    /// Style rules (clippy style lints)
    Style,
}

impl CategoryFilter {
    /// Check if this filter matches the given diagnostic category.
    pub const fn matches(&self, category: &crate::diagnostics::Category) -> bool {
        matches!(
            (self, category),
            (
                Self::ErrorHandling,
                crate::diagnostics::Category::ErrorHandling
            ) | (Self::Performance, crate::diagnostics::Category::Performance)
                | (Self::Security, crate::diagnostics::Category::Security)
                | (Self::Correctness, crate::diagnostics::Category::Correctness)
                | (
                    Self::Architecture,
                    crate::diagnostics::Category::Architecture
                )
                | (
                    Self::Dependencies,
                    crate::diagnostics::Category::Dependencies
                )
                | (Self::Async, crate::diagnostics::Category::Async)
                | (Self::Framework, crate::diagnostics::Category::Framework)
                | (Self::Cargo, crate::diagnostics::Category::Cargo)
                | (Self::Style, crate::diagnostics::Category::Style)
        )
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
    use clap::Parser;

    #[test]
    fn test_default_directory() {
        let cli = Cli::try_parse_from(["rust-doctor"]).unwrap();
        assert_eq!(cli.directory, PathBuf::from("."));
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
        assert_eq!(cli.fail_on, Some(FailOn::Warning));
        assert!(cli.offline);
        assert_eq!(cli.project, vec!["core", "api"]);
    }
}
