use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScoreReason {
    ScanIncomplete,
    StageFailed,
    CategoryMappingConflict,
    DeadlineExceeded,
    PackagesUnlinted,
    RulesNotEvaluated,
}

impl ScoreReason {
    pub(crate) fn of_error(code: &str) -> Option<Self> {
        match code {
            "deadline-exceeded" => Some(Self::DeadlineExceeded),
            "packages-unlinted" => Some(Self::PackagesUnlinted),
            "lint-list-unavailable" | crate::delta::LINE_MAP_NOTICE => None,
            _ => Some(Self::StageFailed),
        }
    }

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
