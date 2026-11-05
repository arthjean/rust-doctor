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
    pub(crate) generated: BTreeSet<String>,
    pub(crate) directives: Vec<Directive>,
    pub(crate) packages: Option<BTreeSet<String>>,
}

const LISTED_MEMBERS: usize = 20;

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
    directives.sort_by(|left, right| (&left.path, left.line).cmp(&(&right.path, right.line)));
    directives.dedup();
    ScanExclusions {
        generated,
        directives,
        packages: packages.cloned(),
    }
}
