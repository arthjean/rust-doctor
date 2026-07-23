use crate::discovery::WorkspaceMember;
use crate::error::WorkspaceError;
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

/// Resolve names, package-relative paths, comma-expanded selectors, or `*`.
/// With no selector, Cargo `default-members` determine the result.
pub fn resolve_members<'a>(
    members: &'a [WorkspaceMember],
    workspace_root: &Path,
    default_member_ids: &[String],
    selectors: &[String],
) -> Result<Vec<&'a WorkspaceMember>, WorkspaceError> {
    if members.is_empty() {
        return Err(WorkspaceError::NoMembers);
    }
    if selectors.iter().any(|selector| selector == "*") {
        return Ok(members.iter().collect());
    }
    if selectors.is_empty() {
        let defaults: Vec<_> = members
            .iter()
            .filter(|member| default_member_ids.contains(&member.package_id))
            .collect();
        return Ok(if defaults.is_empty() {
            members.iter().collect()
        } else {
            defaults
        });
    }

    let candidates = valid_candidates(members, workspace_root);
    let mut selected_ids = BTreeSet::new();
    for selector in selectors {
        let normalized_selector = normalize_selector(selector);
        let matches: Vec<_> = members
            .iter()
            .filter(|member| {
                member.name == *selector
                    || relative_member_root(workspace_root, member) == normalized_selector
            })
            .collect();
        match matches.as_slice() {
            [] => {
                return Err(WorkspaceError::UnknownMember {
                    name: selector.clone(),
                    available: candidates.join(", "),
                });
            }
            [member] => {
                selected_ids.insert(member.package_id.as_str());
            }
            _ => {
                return Err(WorkspaceError::AmbiguousSelector {
                    selector: selector.clone(),
                    matches: matches
                        .iter()
                        .map(|member| member.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                });
            }
        }
    }
    Ok(members
        .iter()
        .filter(|member| selected_ids.contains(member.package_id.as_str()))
        .collect())
}

/// Restrict selected members to owners of changed files. Root manifests and
/// lockfiles affect all selected members. Longest-root matching handles nested
/// workspace member paths without suffix ambiguity.
pub fn affected_members<'a>(
    selected: Vec<&'a WorkspaceMember>,
    workspace_root: &Path,
    changed_paths: &BTreeSet<PathBuf>,
) -> Vec<&'a WorkspaceMember> {
    if changed_paths.is_empty()
        || changed_paths.iter().any(|path| {
            matches!(
                path.to_string_lossy().replace('\\', "/").as_str(),
                "Cargo.toml"
                    | "Cargo.lock"
                    | "rust-doctor.toml"
                    | "rust-toolchain"
                    | "rust-toolchain.toml"
            )
        })
    {
        return selected;
    }

    let mut affected_ids = BTreeSet::new();
    for path in changed_paths {
        let cargo_owner = cargo_owner_root(workspace_root, path);
        let mut owners: Vec<_> = selected
            .iter()
            .filter_map(|member| {
                let relative_root = relative_member_root(workspace_root, member);
                let owns = cargo_owner.as_ref().map_or_else(
                    || {
                        relative_root.as_os_str().is_empty()
                            || path == &relative_root
                            || path.starts_with(&relative_root)
                    },
                    |owner| member.root_dir.as_path() == owner.as_path(),
                );
                owns.then_some((*member, relative_root.components().count()))
            })
            .collect();
        owners.sort_by_key(|(_, depth)| std::cmp::Reverse(*depth));
        if let Some((owner, _)) = owners.first() {
            affected_ids.insert(owner.package_id.as_str());
        }
    }
    selected
        .into_iter()
        .filter(|member| affected_ids.contains(member.package_id.as_str()))
        .collect()
}

fn cargo_owner_root(workspace_root: &Path, path: &Path) -> Option<PathBuf> {
    let absolute = workspace_root.join(path);
    let mut current = if absolute.is_dir() {
        Some(absolute.as_path())
    } else {
        absolute.parent()
    };
    while let Some(directory) = current {
        if !directory.starts_with(workspace_root) {
            return None;
        }
        if directory.join("Cargo.toml").is_file() {
            return Some(directory.to_path_buf());
        }
        if directory == workspace_root {
            return None;
        }
        current = directory.parent();
    }
    None
}

pub fn member_for_root<'a>(
    members: &'a [WorkspaceMember],
    root: &Path,
) -> Option<&'a WorkspaceMember> {
    members.iter().find(|member| member.root_dir == root)
}

fn valid_candidates(members: &[WorkspaceMember], workspace_root: &Path) -> Vec<String> {
    let mut candidates = Vec::new();
    for member in members {
        candidates.push(member.name.clone());
        let path = relative_member_root(workspace_root, member);
        if path.as_os_str().is_empty() {
            candidates.push(".".to_string());
        } else {
            candidates.push(path.to_string_lossy().replace('\\', "/"));
        }
    }
    candidates.sort();
    candidates.dedup();
    candidates
}

fn relative_member_root(workspace_root: &Path, member: &WorkspaceMember) -> PathBuf {
    member
        .root_dir
        .strip_prefix(workspace_root)
        .map_or_else(|_| normalize_path(&member.root_dir), normalize_path)
}

fn normalize_selector(selector: &str) -> PathBuf {
    let normalized = selector.trim_end_matches(['/', '\\']).replace('\\', "/");
    let without_manifest = normalized
        .strip_suffix("/Cargo.toml")
        .unwrap_or(&normalized);
    normalize_path(Path::new(without_manifest))
}

fn normalize_path(path: &Path) -> PathBuf {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_members() -> Vec<WorkspaceMember> {
        vec![
            member("root", "/ws", "root-id"),
            member("core", "/ws/crates/core", "core-id"),
            member("api", "/ws/crates/api", "api-id"),
        ]
    }

    fn member(name: &str, root: &str, package_id: &str) -> WorkspaceMember {
        WorkspaceMember {
            name: name.to_string(),
            root_dir: PathBuf::from(root),
            package_id: package_id.to_string(),
            targets: vec![format!("{name}:[Lib]")],
            frameworks: vec![],
            framework_capabilities: vec![],
            rust_version: Some("1.85".to_string()),
        }
    }

    #[test]
    fn resolves_default_members_and_star() {
        let members = make_members();
        let defaults =
            resolve_members(&members, Path::new("/ws"), &["core-id".into()], &[]).unwrap();
        assert_eq!(defaults.len(), 1);
        assert_eq!(defaults[0].name, "core");
        assert_eq!(
            resolve_members(&members, Path::new("/ws"), &[], &["*".into()])
                .unwrap()
                .len(),
            3
        );
    }

    #[test]
    fn resolves_names_and_package_relative_paths() {
        let members = make_members();
        let selected = resolve_members(
            &members,
            Path::new("/ws"),
            &[],
            &["core".into(), "crates/api/Cargo.toml".into()],
        )
        .unwrap();
        assert_eq!(
            selected
                .iter()
                .map(|member| member.name.as_str())
                .collect::<Vec<_>>(),
            vec!["core", "api"]
        );
    }

    #[test]
    fn unknown_selector_lists_names_and_paths() {
        let members = make_members();
        let error = resolve_members(&members, Path::new("/ws"), &[], &["missing".into()])
            .unwrap_err()
            .to_string();
        assert!(error.contains("core"));
        assert!(error.contains("crates/core"));
    }

    #[test]
    fn maps_changes_to_the_longest_owning_root() {
        let members = make_members();
        let selected: Vec<_> = members.iter().collect();
        let affected = affected_members(
            selected,
            Path::new("/ws"),
            &BTreeSet::from([PathBuf::from("crates/core/src/lib.rs")]),
        );
        assert_eq!(affected.len(), 1);
        assert_eq!(affected[0].name, "core");
    }

    #[test]
    fn root_manifest_affects_every_selected_member() {
        let members = make_members();
        let selected: Vec<_> = members.iter().collect();
        let affected = affected_members(
            selected,
            Path::new("/ws"),
            &BTreeSet::from([PathBuf::from("Cargo.lock")]),
        );
        assert_eq!(affected.len(), 3);
    }
}
