//! The changed-work scope an invocation asked for, checked before any process
//! starts. It lives outside `main.rs` so that file stays under the size bound
//! its own scan reports.

use clap::ValueEnum;
use clap::error::ErrorKind;
use rust_doctor::ScopeMode;

use crate::InspectArgs;

/// A changed-work scope the invocation asked for, checked for combinations
/// that mean nothing before any process starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScopeSelection {
    pub(crate) mode: ScopeMode,
    pub(crate) base: Option<String>,
    pub(crate) staged: bool,
    pub(crate) include_untracked: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum ScopeArgument {
    Full,
    Files,
    Lines,
    Contextual,
    Baseline,
}

pub(crate) fn validate_scope(
    arguments: &InspectArgs,
) -> Result<Option<ScopeSelection>, (ErrorKind, &'static str)> {
    let mode = match arguments.scope {
        None | Some(ScopeArgument::Full) => {
            return match (
                arguments.base.is_some(),
                arguments.staged,
                arguments.include_untracked,
            ) {
                (false, false, false) => Ok(None),
                (true, _, _) => Err((
                    ErrorKind::ArgumentConflict,
                    "--base <REF> requires a files, lines, contextual or baseline scope",
                )),
                (_, true, _) => Err((
                    ErrorKind::ArgumentConflict,
                    "--staged cannot judge the full codebase: pass --scope files, lines, contextual or baseline",
                )),
                (_, _, true) => Err((
                    ErrorKind::ArgumentConflict,
                    "--include-untracked requires a files, lines or contextual scope",
                )),
            };
        }
        Some(ScopeArgument::Files) => ScopeMode::Files,
        Some(ScopeArgument::Lines) => ScopeMode::Lines,
        Some(ScopeArgument::Contextual) => ScopeMode::Contextual,
        Some(ScopeArgument::Baseline) => ScopeMode::Baseline,
    };
    if arguments.include_untracked && mode == ScopeMode::Baseline {
        return Err((
            ErrorKind::ArgumentConflict,
            "--include-untracked requires a files, lines or contextual scope",
        ));
    }
    if arguments.include_untracked && arguments.staged {
        return Err((
            ErrorKind::ArgumentConflict,
            "--include-untracked cannot be combined with --staged: the index holds no untracked file",
        ));
    }
    Ok(Some(ScopeSelection {
        mode,
        base: arguments.base.clone(),
        staged: arguments.staged,
        include_untracked: arguments.include_untracked,
    }))
}
