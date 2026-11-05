use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};

use super::{ARTIFACTS_DIRECTORY_ENV, CACHE_DIRECTORY_ENV};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Reproduction {
    Run {
        artifacts: PathBuf,
        cache: PathBuf,
    },
    Misconfigured(String),
    Skipped(String),
}

pub(crate) fn decide(cache: Option<OsString>, artifacts: Option<OsString>) -> Reproduction {
    match (cache, artifacts) {
        (Some(cache), Some(artifacts)) => Reproduction::Run {
            artifacts: PathBuf::from(artifacts),
            cache: PathBuf::from(cache),
        },
        (None, None) => Reproduction::Skipped(format!(
            "skipped: no corpus reproduction was attempted. Set {CACHE_DIRECTORY_ENV} to a clone \
             cache and {ARTIFACTS_DIRECTORY_ENV} to a scratch directory, both outside this \
             repository, to replay the pinned corpus and confirm every published site."
        )),
        (Some(_), None) => Reproduction::Misconfigured(format!(
            "{CACHE_DIRECTORY_ENV} is set without {ARTIFACTS_DIRECTORY_ENV}: a reproduction has \
             nowhere to write its reports, and a half-configured reproduction that returns \
             quietly reads as a corpus that reproduced."
        )),
        (None, Some(_)) => Reproduction::Misconfigured(format!(
            "{ARTIFACTS_DIRECTORY_ENV} is set without {CACHE_DIRECTORY_ENV}: a reproduction has \
             no clone cache to replay from, and a half-configured reproduction that returns \
             quietly reads as a corpus that reproduced."
        )),
    }
}

impl Reproduction {
    pub(crate) fn directories(self, repository: &Path) -> Option<(PathBuf, PathBuf)> {
        match self {
            Self::Run { artifacts, cache } => {
                assert!(
                    !artifacts.starts_with(repository) && !cache.starts_with(repository),
                    "the corpus and its artifacts belong outside this repository"
                );
                Some((cache, artifacts))
            }
            Self::Misconfigured(reason) => {
                assert!(reason.is_empty(), "{reason}");
                None
            }
            Self::Skipped(reason) => {
                announce(&reason);
                None
            }
        }
    }
}

fn announce(reason: &str) {
    let mut stderr = std::io::stderr();
    let _ = stderr.write_all(reason.as_bytes());
    let _ = stderr.write_all(b"\n");
    let _ = stderr.flush();
}

pub(crate) fn requested() -> Reproduction {
    decide(
        std::env::var_os(CACHE_DIRECTORY_ENV),
        std::env::var_os(ARTIFACTS_DIRECTORY_ENV),
    )
}
