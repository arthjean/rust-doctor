//! The agent skill the CLI offers to install.
//!
//! The skill is compiled into the binary rather than fetched, so an install
//! never reaches the network and a binary always carries the skill of its own
//! version: the commands it documents are the commands it accepts, and
//! `tests/skill_contract.rs` is what keeps that true.
//!
//! Like the CI workflow, this writes into the workspace only when asked and
//! never over what is already there. The refusal is the creation of the skill
//! directory itself: `create_dir` fails when it exists, so either the whole
//! skill lands or nothing does, and no half-installed skill is left pointing at
//! a reference that was never written.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const SKILL_DIRECTORY: &str = ".claude/skills/rust-doctor";

const DOCUMENTS: [(&str, &str); 2] = [
    (
        "SKILL.md",
        include_str!("../skills/rust-doctor/SKILL.md"),
    ),
    (
        "references/expert-review.md",
        include_str!("../skills/rust-doctor/references/expert-review.md"),
    ),
];

#[derive(Debug, PartialEq, Eq)]
pub enum SkillError {
    AlreadyPresent,
    Write(io::ErrorKind),
}

impl fmt::Display for SkillError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyPresent => write!(formatter, "{SKILL_DIRECTORY} already exists"),
            Self::Write(kind) => write!(formatter, "{}", write_reason(*kind)),
        }
    }
}

const fn write_reason(kind: io::ErrorKind) -> &'static str {
    match kind {
        io::ErrorKind::PermissionDenied => "permission denied",
        io::ErrorKind::NotFound => "directory not found",
        io::ErrorKind::ReadOnlyFilesystem => "read-only filesystem",
        _ => "write error",
    }
}

/// Writes the skill and answers with the workspace-relative paths it created,
/// which is what the caller prints: the tool publishes no absolute path.
pub fn install(workspace_root: &Path) -> Result<Vec<PathBuf>, SkillError> {
    let root = workspace_root.join(SKILL_DIRECTORY);
    if let Some(parent) = root.parent() {
        fs::create_dir_all(parent).map_err(|error| SkillError::Write(error.kind()))?;
    }
    fs::create_dir(&root).map_err(|error| match error.kind() {
        io::ErrorKind::AlreadyExists => SkillError::AlreadyPresent,
        kind => SkillError::Write(kind),
    })?;

    let mut written = Vec::with_capacity(DOCUMENTS.len());
    for (name, content) in DOCUMENTS {
        let destination = root.join(name);
        if let Some(directory) = destination.parent() {
            fs::create_dir_all(directory).map_err(|error| SkillError::Write(error.kind()))?;
        }
        fs::write(&destination, content).map_err(|error| SkillError::Write(error.kind()))?;
        written.push(Path::new(SKILL_DIRECTORY).join(name));
    }
    Ok(written)
}

#[cfg(test)]
mod tests {

    use super::*;
use crate::test_scratch::scratch;

    

    #[test]
    fn the_skill_installs_once_and_never_over_an_existing_one() {
        let root = scratch("skill-tests", "install");
        let written = install(&root).unwrap_or_default();
        assert_eq!(
            written,
            vec![
                Path::new(SKILL_DIRECTORY).join("SKILL.md"),
                Path::new(SKILL_DIRECTORY).join("references/expert-review.md"),
            ]
        );

        let skill = fs::read_to_string(root.join(SKILL_DIRECTORY).join("SKILL.md"))
            .unwrap_or_default();
        assert!(skill.starts_with("---\nname: rust-doctor\n"));
        assert!(skill.contains("rust-doctor . --json"));

        assert_eq!(install(&root), Err(SkillError::AlreadyPresent));
        fs::remove_dir_all(root).unwrap_or_default();
    }
}
