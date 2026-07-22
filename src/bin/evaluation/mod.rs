#![expect(
    clippy::redundant_pub_crate,
    reason = "pub(crate) documents the contracts shared by sibling modules in this private binary subtree"
)]

pub(crate) mod benchmark;
pub(crate) mod corpus;
pub(crate) mod delta;
pub(crate) mod manifest;
pub(crate) mod model;
pub(crate) mod process;
pub(crate) mod sandbox;
pub(crate) mod smoke;

use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub(crate) enum EvalError {
    #[error("{action} '{}': {source}", path.display())]
    Io {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid JSON in '{}': {source}", path.display())]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("invalid evaluation manifest: {0}")]
    InvalidManifest(String),
    #[error("evaluation command failed: {0}")]
    Command(String),
    #[error("evaluation gate failed: {0}")]
    GateFailed(String),
    #[error("unsupported evaluation environment: {0}")]
    Unsupported(String),
}

impl EvalError {
    pub(crate) fn io(action: &'static str, path: impl AsRef<Path>, source: std::io::Error) -> Self {
        Self::Io {
            action,
            path: path.as_ref().to_path_buf(),
            source,
        }
    }

    pub(crate) const fn exit_code(&self) -> u8 {
        match self {
            Self::GateFailed(_) => 3,
            Self::Io { .. }
            | Self::Json { .. }
            | Self::InvalidManifest(_)
            | Self::Command(_)
            | Self::Unsupported(_) => 2,
        }
    }
}

pub(crate) type Result<T> = std::result::Result<T, EvalError>;
