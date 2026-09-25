//! Why a score is not authoritative, as closed values.
//!
//! `score.authoritative` used to be the only answer, and the terminal turned it
//! into one sentence covering two causes. A reader branching on the report,
//! an agent or a CI step, needs to tell a broken toolchain from a timeout
//! without re-deriving it from `errors[]`, so every cause that drops the flag
//! is published as one of these values, and the flag is true exactly when
//! there are none.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScoreReason {
    /// The scan stopped before covering the whole workspace.
    ScanIncomplete,
    /// A stage failed, and `errors[]` names it with its code.
    StageFailed,
    /// A finding could not be placed on exactly one score dimension.
    CategoryMappingConflict,
    /// The run reached the limit `--max-duration` set.
    DeadlineExceeded,
    /// Some packages did not compile and went unlinted.
    PackagesUnlinted,
    /// Some active rules were not evaluated: the installed Clippy does not
    /// know them, or `--package` left no repository to judge them on.
    RulesNotEvaluated,
}

impl ScoreReason {
    /// The reason a published error gives, if any. A deadline and a member
    /// that did not compile have reasons of their own; a notice, which reports
    /// a narrower check rather than a failed one, has none; every other error
    /// is a stage that failed.
    pub(crate) fn of_error(code: &str) -> Option<Self> {
        match code {
            "deadline-exceeded" => Some(Self::DeadlineExceeded),
            "packages-unlinted" => Some(Self::PackagesUnlinted),
            "lint-list-unavailable" => None,
            _ => Some(Self::StageFailed),
        }
    }

    /// The cause and the next step, in the one line both reports print.
    pub const fn explanation(self) -> &'static str {
        match self {
            Self::ScanIncomplete => {
                "Score is partial: the scan did not cover the whole workspace. Rerun it to completion."
            }
            Self::StageFailed => {
                "Score is partial: a stage of the scan failed. Fix what its error reports, then rerun."
            }
            Self::CategoryMappingConflict => {
                "Score is partial: a finding maps to no single score dimension. Report it with the --json output."
            }
            Self::DeadlineExceeded => {
                "Score is partial: the scan hit its --max-duration limit. Raise the limit or narrow the scope."
            }
            Self::PackagesUnlinted => {
                "Score is partial: some packages did not compile and were not linted. Fix their build, then rerun."
            }
            Self::RulesNotEvaluated => {
                "Score is partial: some active rules were not evaluated, by an older Clippy or under --package. policy.rules in --json names them."
            }
        }
    }
}
