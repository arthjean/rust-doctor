use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use cargo_metadata::{Message, Metadata};

use super::messages::CapturedMessage;
use crate::progress::{Progress, ProgressSink};

pub(super) struct Members {
    names: BTreeMap<String, String>,
}

impl Members {
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

    pub(super) fn observe_dependency(&self, message: &Message, last: &mut Option<Instant>) {
        if let Message::CompilerArtifact(artifact) = message
            && !self.names.contains_key(&artifact.package_id.repr)
        {
            *last = Some(Instant::now());
        }
    }

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
