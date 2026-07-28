use std::path::PathBuf;

/// Top-level error for the rust-doctor scan pipeline.
#[derive(thiserror::Error, Debug)]
pub enum ScanError {
    #[error(transparent)]
    Discovery(#[from] DiscoveryError),

    #[error("workspace resolution failed: {0}")]
    Workspace(#[from] WorkspaceError),

    #[error("diff resolution failed: {0}")]
    Diff(#[from] DiffError),

    #[error("canonical rule catalog is invalid: {0}")]
    Catalog(String),

    #[error("invalid resolved rule policy: {0}")]
    InvalidPolicy(String),

    #[error("score model is unavailable: {0}")]
    ScoreModel(String),
}

/// Errors from workspace member resolution.
#[derive(thiserror::Error, Debug)]
pub enum WorkspaceError {
    #[error("unknown workspace member '{name}'. Available members: {available}")]
    UnknownMember { name: String, available: String },

    #[error("ambiguous workspace selector '{selector}'. Matches: {matches}")]
    AmbiguousSelector { selector: String, matches: String },

    #[error("workspace has no members")]
    NoMembers,
}

/// Errors from diff mode resolution.
#[derive(thiserror::Error, Debug)]
pub enum DiffError {
    #[error("invalid ref '{name}': {reason}")]
    InvalidRef { name: String, reason: String },

    #[error("git is not available or directory is not a git repository")]
    GitNotFound,

    #[error("failed to find merge base: {0}")]
    MergeBaseFailed(String),

    #[error("invalid scan scope: {0}")]
    InvalidScope(String),

    #[error("Git index has unresolved conflicts: {0}")]
    IndexConflict(String),

    #[error("staged snapshot failed: {0}")]
    StagedSnapshot(String),

    #[error("baseline is unavailable: {0}")]
    BaselineUnavailable(String),

    #[error("{0}")]
    Other(String),
}

/// Errors from project discovery via `cargo metadata`.
#[derive(thiserror::Error, Debug)]
pub enum DiscoveryError {
    #[error("cargo metadata failed: {source}")]
    CargoMetadata {
        #[source]
        source: cargo_metadata::Error,
    },

    #[error("no packages found in workspace")]
    NoPackages,
}

/// Errors from individual analysis passes.
#[derive(thiserror::Error, Debug)]
pub enum PassError {
    #[error("{pass}: {message}")]
    Failed { pass: String, message: String },

    #[error("{pass}: analysis pass panicked")]
    Panicked { pass: String },

    #[error("{pass}: skipped ({reason})")]
    Skipped { pass: String, reason: String },

    #[error("{pass}: timed out ({reason})")]
    TimedOut { pass: String, reason: String },

    #[error("{pass}: cancelled ({reason})")]
    Cancelled { pass: String, reason: String },
}

/// Errors from project bootstrapping (shared between CLI and MCP).
#[derive(thiserror::Error, Debug)]
pub enum BootstrapError {
    #[error("invalid directory '{path}': {source}")]
    InvalidDirectory {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("no Cargo.toml found in '{}'", path.display())]
    NoCargo { path: PathBuf },

    #[error(transparent)]
    Discovery(#[from] DiscoveryError),

    #[error(transparent)]
    Config(#[from] ConfigError),
}

/// Errors from the interactive setup wizard.
#[derive(thiserror::Error, Debug)]
pub enum SetupError {
    #[error(transparent)]
    Prompt(#[from] dialoguer::Error),

    #[error("{0}")]
    NotInteractive(String),

    #[error("installation failed{path_suffix}: {message}", path_suffix = path.as_ref().map_or_else(String::new, |value| format!(" for '{}'", value.display())))]
    Install {
        path: Option<PathBuf>,
        message: String,
    },
}

/// Errors from the post-scan GitHub Actions onboarding prompt.
#[derive(thiserror::Error, Debug)]
pub enum CiSetupPromptError {
    #[error("Rust Doctor state directory is unavailable")]
    StateDirectoryUnavailable,

    #[error("GitHub Actions prompt failed: {0}")]
    Prompt(#[from] dialoguer::Error),

    #[error("GitHub Actions installation failed: {0}")]
    Install(String),

    #[error("failed to access CI prompt state '{}': {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Errors from loading the config file (`rust-doctor.toml`).
#[derive(thiserror::Error, Debug)]
pub enum ConfigError {
    #[error("failed to read config file '{}': {source}", path.display())]
    Io {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse config file '{}': {source}", path.display())]
    Parse {
        path: std::path::PathBuf,
        #[source]
        source: toml::de::Error,
    },

    #[error("failed to parse [package.metadata.rust-doctor] in Cargo.toml: {0}")]
    MetadataParse(#[from] serde_json::Error),

    #[error("invalid rule catalog while loading '{}': {message}", path.display())]
    Catalog { path: PathBuf, message: String },

    #[error("unknown rule '{rule}' in '{}'; configuration was not applied", path.display())]
    UnknownRule { path: PathBuf, rule: String },

    #[error("unknown category '{category}' in '{}'; configuration was not applied", path.display())]
    UnknownCategory { path: PathBuf, category: String },

    #[error("unknown tag '{tag}' in '{}'; configuration was not applied", path.display())]
    UnknownTag { path: PathBuf, tag: String },

    #[error("rule '{rule}' does not support a numeric threshold in '{}'", path.display())]
    UnsupportedThreshold { path: PathBuf, rule: String },

    #[error("threshold {value} for rule '{rule}' is outside {min}..={max} in '{}'", path.display())]
    ThresholdOutOfRange {
        path: PathBuf,
        rule: String,
        value: u32,
        min: u32,
        max: u32,
    },

    #[error("invalid path override pattern '{pattern}' in '{}'; configuration was not applied", path.display())]
    InvalidPathOverride { path: PathBuf, pattern: String },

    #[error("configuration in '{}' declares {actual} policy entries; the maximum is {limit}", path.display())]
    PolicyLimitExceeded {
        path: PathBuf,
        limit: usize,
        actual: usize,
    },

    #[error("configuration in '{}' declares {actual} glob patterns; the maximum is {limit}", path.display())]
    GlobLimitExceeded {
        path: PathBuf,
        limit: usize,
        actual: usize,
    },

    #[error("rule '{rule}' has conflicting canonical and legacy policy in '{}'; configuration was not applied", path.display())]
    ConflictingRulePolicy { path: PathBuf, rule: String },
}

/// Errors while serializing or routing machine output.
#[derive(thiserror::Error, Debug)]
pub enum OutputError {
    #[error("failed to serialize Report V1: {0}")]
    Serialize(serde_json::Error),

    #[error("failed to write Report V1 to '{}': {source}", path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to write Report V1 to stdout: {0}")]
    Stdout(std::io::Error),
}
