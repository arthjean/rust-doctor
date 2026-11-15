use std::path::Path;

use super::sanitize::{HomePaths, shown_path};
use super::{InspectReport, NotEvaluated};
use crate::cargo_health::{AdvisoryOutcome, UNREADABLE};
use crate::score_block::UncheckedAdvisories;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum AdvisoryCheck {
    #[default]
    Published,
    TurnedOff,
    Unreadable {
        database: String,
        skipped: usize,
        files: usize,
    },
}

impl AdvisoryCheck {
    pub(super) fn of(
        outcome: Option<&AdvisoryOutcome>,
        workspace_root: Option<&Path>,
        home: &HomePaths,
    ) -> Self {
        match outcome {
            Some(AdvisoryOutcome::TurnedOff) => Self::TurnedOff,
            Some(AdvisoryOutcome::Unreadable(unreadable)) => Self::Unreadable {
                database: shown_path(&unreadable.path, workspace_root, home),
                skipped: unreadable.skipped,
                files: unreadable.files,
            },
            _ => Self::Published,
        }
    }
}

pub(super) const fn not_evaluated(outcome: &AdvisoryOutcome) -> Option<NotEvaluated> {
    match outcome {
        AdvisoryOutcome::Missing | AdvisoryOutcome::TurnedOff | AdvisoryOutcome::Unreadable(_) => {
            Some(NotEvaluated::AdvisoryDbMissing)
        }
        AdvisoryOutcome::NotRequested | AdvisoryOutcome::Consulted(_) => None,
    }
}

impl InspectReport {
    pub fn unchecked_advisories(&self) -> Option<UncheckedAdvisories> {
        let unreadable = self.errors.iter().any(|error| error.code == UNREADABLE);
        let missing = !unreadable
            && self.policy.as_ref()?.rules.iter().any(|rule| {
                rule.not_evaluated == Some(NotEvaluated::AdvisoryDbMissing)
            });
        missing.then(|| {
            if self.scan.advisory_check == AdvisoryCheck::TurnedOff {
                UncheckedAdvisories::TurnedOff
            } else {
                UncheckedAdvisories::NoDatabase
            }
        })
    }
}
