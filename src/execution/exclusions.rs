//! What a run gathers so the report can leave findings out: the files the
//! repository declares generated, the suppression directives written in the
//! files the scan read, and the members `--package` selected.
//!
//! Nothing is decided here. The report applies all three, beside the ignored
//! paths and the overrides the plan carries, in one place.

use std::collections::BTreeSet;
use std::path::Path;

use cargo_metadata::Metadata;

use crate::directive::{self, Directive};
use crate::generated;
use crate::internal_error::InternalError;
use crate::source_kernel::Enumeration;
use crate::workspace_path;

#[derive(Debug, Default)]
pub(crate) struct ScanExclusions {
    /// Workspace-relative paths `.gitattributes` marks generated or vendored,
    /// or whose Rust source opens on a generator header.
    pub(crate) generated: BTreeSet<String>,
    pub(crate) directives: Vec<Directive>,
    /// The members `--package` selected, `None` when every member is scanned.
    pub(crate) packages: Option<BTreeSet<String>>,
}

/// How many member names an unknown `--package` lists before it stops.
const LISTED_MEMBERS: usize = 20;

/// Refuses a selection naming anything but a workspace member, listing the
/// members it could have named. The name that was refused is not repeated:
/// the reader wrote it.
pub(crate) fn check_packages(
    metadata: &Metadata,
    selected: &BTreeSet<String>,
) -> Result<(), InternalError> {
    let members = metadata
        .packages
        .iter()
        .filter(|package| metadata.workspace_members.contains(&package.id))
        .map(|package| package.name.as_str())
        .collect::<BTreeSet<_>>();
    if selected.iter().all(|name| members.contains(name.as_str())) {
        return Ok(());
    }
    let listed = members
        .iter()
        .take(LISTED_MEMBERS)
        .copied()
        .collect::<Vec<_>>()
        .join(", ");
    let more = members
        .len()
        .checked_sub(LISTED_MEMBERS)
        .filter(|more| *more > 0)
        .map(|more| format!(" and {more} more"))
        .unwrap_or_default();
    Err(InternalError::new(
        "policy",
        "unknown-package",
        format!("--package names no workspace member. Members: {listed}{more}."),
    ))
}

/// The TOML files a directive may sit in besides the member manifests.
const CARGO_CONFIG: &str = ".cargo/config.toml";

pub(super) fn gather(
    metadata: &Metadata,
    enumeration: &Enumeration,
    packages: Option<&BTreeSet<String>>,
    declared: &BTreeSet<String>,
) -> ScanExclusions {
    let workspace_root = metadata.workspace_root.as_std_path();
    let mut generated = declared.clone();
    let mut directives = Vec::new();
    // Read over every unit the walk loaded: a headed file is already out of
    // the scanned ones, and its Clippy findings still have to be left out.
    for unit in enumeration.reached() {
        if generated::has_generator_header(unit.source()) {
            generated.insert(unit.relative_path().to_owned());
        }
    }
    for unit in enumeration.units() {
        directives.extend(directive::in_rust(
            unit.relative_path(),
            unit.source(),
            &unit.tree(),
        ));
    }
    let manifests = metadata
        .packages
        .iter()
        .filter(|package| metadata.workspace_members.contains(&package.id))
        .filter(|package| packages.is_none_or(|selected| selected.contains(package.name.as_str())))
        .map(|package| package.manifest_path.as_std_path().to_path_buf())
        .chain([
            workspace_root.join("Cargo.toml"),
            workspace_root.join(CARGO_CONFIG),
        ])
        .collect::<BTreeSet<_>>();
    for manifest in manifests {
        if let Some(relative) = workspace_path::normalize(workspace_root, &manifest) {
            directives.extend(directive::in_toml_file(&relative, Path::new(&manifest)));
        }
    }
    // The same file parsed under two editions is two units and one set of
    // comments.
    directives.sort_by(|left, right| (&left.path, left.line).cmp(&(&right.path, right.line)));
    directives.dedup();
    ScanExclusions {
        generated,
        directives,
        packages: packages.cloned(),
    }
}
