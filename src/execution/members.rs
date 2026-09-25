//! The workspace members a Clippy run was asked to lint, and which of them it
//! did.
//!
//! Under `--keep-going` Cargo lints every member that compiles and skips the
//! rest, including every member that only failed because a dependency did.
//! Cargo announces what it checked with one `compiler-artifact` record per
//! target, so a member with no record, or with an error of its own, is a
//! member the report must name: its silence is not a clean bill of health.
//! A build script's record does not count: it compiles without waiting on the
//! member's dependencies, so a member that failed only through one still has it.
//! The same records drive the progress line while the run is still going.

use std::collections::{BTreeMap, BTreeSet};

use cargo_metadata::{Message, Metadata};

use super::messages::CapturedMessage;
use crate::progress::{Progress, ProgressSink};

pub(super) struct Members {
    /// Package id to package name, for workspace members only.
    names: BTreeMap<String, String>,
}

impl Members {
    /// The members a run lints: every one, or the ones `--package` selected.
    pub(super) fn of(metadata: &Metadata, selected: Option<&BTreeSet<String>>) -> Self {
        let names = metadata
            .packages
            .iter()
            .filter(|package| metadata.workspace_members.contains(&package.id))
            .filter(|package| {
                selected.is_none_or(|selected| selected.contains(package.name.as_str()))
            })
            .map(|package| (package.id.repr.clone(), package.name.to_string()))
            .collect();
        Self { names }
    }

    /// Records a member Cargo just finished checking, and reports it.
    pub(super) fn observe(
        &self,
        message: &Message,
        linted: &mut BTreeSet<String>,
        progress: Option<&ProgressSink>,
    ) {
        let Message::CompilerArtifact(artifact) = message else {
            return;
        };
        if artifact.target.is_custom_build() {
            return;
        }
        let Some(name) = self.names.get(&artifact.package_id.repr) else {
            return;
        };
        if linted.insert(artifact.package_id.repr.clone())
            && let Some(progress) = progress
        {
            progress.report(Progress::Linting {
                package: name,
                done: linted.len(),
                total: self.names.len(),
            });
        }
    }

    /// Package names, sorted, of the members with no artifact or with an
    /// error of their own. Names only: a path would carry the machine.
    pub(super) fn unlinted(&self, messages: &[CapturedMessage]) -> Vec<String> {
        let mut checked = BTreeSet::new();
        let mut failed = BTreeSet::new();
        for message in messages {
            match message {
                CapturedMessage::Known(message) => {
                    if let Message::CompilerArtifact(artifact) = message.as_ref()
                        && !artifact.target.is_custom_build()
                    {
                        checked.insert(artifact.package_id.repr.as_str());
                    }
                }
                CapturedMessage::Compiler(data) if data.message.level.starts_with("error") => {
                    failed.insert(data.package_id.as_str());
                }
                CapturedMessage::Compiler(_) | CapturedMessage::Unknown => {}
            }
        }
        let mut names: Vec<String> = self
            .names
            .iter()
            .filter(|(id, _)| !checked.contains(id.as_str()) || failed.contains(id.as_str()))
            .map(|(_, name)| name.clone())
            .collect();
        names.sort();
        names.dedup();
        names
    }
}
