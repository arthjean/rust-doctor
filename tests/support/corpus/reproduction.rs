//! Whether a run is reproducing the corpus, and what it says when it is not.
//!
//! The two reproduction tests replay eighteen repositories out of a clone cache
//! named by `RUST_DOCTOR_CORPUS_DIR`, so a machine without that cache has to be
//! able to run `cargo test`. That gate used to be a `let ... else { return }`:
//! the test passed, the suite was green, and nothing anywhere said that the one
//! check able to locate a published site had not run. The two failure shapes it
//! hid are different, and only one of them is a machine without a cache.
//!
//! A run that sets neither variable is not attempting a reproduction, and it is
//! told which two variables would make it one. A run that sets one of them is
//! attempting a reproduction and got it wrong, and a misconfigured reproduction
//! that silently passes is the failure this module exists to make loud.

use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};

use super::{ARTIFACTS_DIRECTORY_ENV, CACHE_DIRECTORY_ENV};

/// What a run may do with the corpus.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Reproduction {
    /// Both directories are named: replay.
    Run {
        artifacts: PathBuf,
        cache: PathBuf,
    },
    /// A reproduction was attempted and cannot be trusted to mean anything.
    Misconfigured(String),
    /// No reproduction was attempted, and the reason is printed rather than
    /// left to be inferred from a green run.
    Skipped(String),
}

/// Decides from the two variables alone, so the decision is testable without a
/// test mutating the environment of every other test in the binary.
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
    /// The two directories, or the reason there is nothing to do. Printing the
    /// skip is the point: a reader of the run learns the reproduction did not
    /// happen from the run itself.
    pub(crate) fn directories(self, repository: &Path) -> Option<(PathBuf, PathBuf)> {
        match self {
            Self::Run { artifacts, cache } => {
                assert!(
                    !artifacts.starts_with(repository) && !cache.starts_with(repository),
                    "the corpus and its artifacts belong outside this repository"
                );
                Some((cache, artifacts))
            }
            // Not a skip: a reproduction was attempted with half its
            // configuration, and one that returns quietly reads as a corpus
            // that reproduced. Spelled as an assertion rather than `panic!`,
            // which every target of this crate denies, tests included.
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

/// Writes the reason where libtest cannot swallow it.
///
/// Not `println!`. libtest captures the `print!` family for a test that passes,
/// and a skipped reproduction passes by construction, so the reason reached
/// nobody under a plain `cargo test`: the run was exactly as silent as the bare
/// `return` this module replaced, which is the whole defect. The capture hook
/// lives in `std::io::_print` alone, so writing the bytes to the stream itself
/// goes to the terminal whatever libtest is doing with its own output.
fn announce(reason: &str) {
    let mut stderr = std::io::stderr();
    let _ = stderr.write_all(reason.as_bytes());
    let _ = stderr.write_all(b"\n");
    let _ = stderr.flush();
}

/// The gate the two reproduction tests open on, read from the process.
pub(crate) fn requested() -> Reproduction {
    decide(
        std::env::var_os(CACHE_DIRECTORY_ENV),
        std::env::var_os(ARTIFACTS_DIRECTORY_ENV),
    )
}
