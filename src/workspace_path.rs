use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

pub(crate) fn normalize(workspace_root: &Path, path: &Path) -> Option<String> {
    let workspace_root = lexical_normalize(workspace_root)?;
    let physical_candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace_root.join(path)
    };
    let candidate = lexical_normalize(&physical_candidate)?;
    let relative = candidate.strip_prefix(&workspace_root).ok()?;
    let canonical_workspace = workspace_root.canonicalize().ok()?;
    let existing_ancestor = existing_ancestor(&physical_candidate)?;
    let canonical_ancestor = existing_ancestor.canonicalize().ok()?;
    if !canonical_ancestor.starts_with(&canonical_workspace) {
        return None;
    }
    normalize_relative(relative)
}

pub(crate) fn normalize_relative(relative: &Path) -> Option<String> {
    normalize_components(relative, true)
}

pub(crate) fn normalize_changed(path: &str) -> Option<String> {
    if path.is_empty() || path.split('/').any(str::is_empty) {
        return None;
    }
    normalize_components(Path::new(path), false)
}

fn normalize_components(path: &Path, allow_current_directory: bool) -> Option<String> {
    if path.is_absolute() {
        return None;
    }
    let components: Option<Vec<_>> = path
        .components()
        .map(|component| match component {
            Component::Normal(value) => safe_component(value),
            Component::CurDir if allow_current_directory => Some(".".to_owned()),
            _ => None,
        })
        .collect();
    let normalized = components?.join("/");
    if normalized.is_empty() {
        allow_current_directory.then(|| ".".to_owned())
    } else {
        Some(normalized)
    }
}

fn existing_ancestor(path: &Path) -> Option<&Path> {
    existing_ancestor_with(path, |candidate| candidate.symlink_metadata().map(|_| ()))
}

fn existing_ancestor_with(
    path: &Path,
    mut probe: impl FnMut(&Path) -> std::io::Result<()>,
) -> Option<&Path> {
    for ancestor in path.ancestors() {
        match probe(ancestor) {
            Ok(()) => return Some(ancestor),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return None,
        }
    }
    None
}

fn safe_component(value: &OsStr) -> Option<String> {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";

    let value = value.to_str()?;
    let mut encoded = String::with_capacity(value.len());
    for character in value.chars() {
        if character == '%' || character.is_control() {
            let mut buffer = [0; 4];
            for byte in character.encode_utf8(&mut buffer).bytes() {
                encoded.push('%');
                encoded.push(char::from(HEX[usize::from(byte >> 4)]));
                encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
            }
        } else {
            encoded.push(character);
        }
    }
    Some(encoded)
}

fn lexical_normalize(path: &Path) -> Option<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return None;
                }
            }
            Component::Normal(value) => normalized.push(value),
        }
    }
    Some(normalized)
}

#[cfg(test)]
mod tests {
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::symlink;

    use super::*;

    #[test]
    fn changed_paths_use_the_same_safe_representation_as_diagnostics() {
        assert_eq!(
            normalize_changed("src/100%\u{001b}[31mline\n.rs").as_deref(),
            Some("src/100%25%1B[31mline%0A.rs")
        );
        for invalid in [
            "",
            "/absolute",
            "./relative",
            "parent/../escape",
            "double//component",
        ] {
            assert!(normalize_changed(invalid).is_none(), "{invalid:?}");
        }
    }

    #[test]
    fn physical_containment_stops_on_non_not_found_errors() {
        let mut probes = 0;
        let ancestor = existing_ancestor_with(Path::new("one/two/three.rs"), |_| {
            probes += 1;
            match probes {
                1 => Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "missing leaf",
                )),
                2 => Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "unreadable parent",
                )),
                _ => Ok(()),
            }
        });

        assert!(ancestor.is_none());
        assert_eq!(probes, 2);
    }

    #[cfg(unix)]
    #[test]
    fn paths_crossing_symlinks_outside_the_workspace_are_null() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!("workspace-paths-{}", std::process::id()));
        if root.exists() {
            fs::remove_dir_all(&root).unwrap();
        }
        let workspace = root.join("workspace");
        let outside = root.join("outside");
        fs::create_dir_all(workspace.join("src")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("external.rs"), "pub fn external() {}\n").unwrap();
        symlink(&outside, workspace.join("linked")).unwrap();
        symlink(
            outside.join("external.rs"),
            workspace.join("direct-link.rs"),
        )
        .unwrap();

        assert_eq!(
            normalize(&workspace, &workspace.join("src/future.rs")).as_deref(),
            Some("src/future.rs")
        );
        for external in [
            "linked/external.rs",
            "direct-link.rs",
            "linked/future.rs",
            "linked/../outside/external.rs",
        ] {
            assert_eq!(normalize(&workspace, &workspace.join(external)), None);
        }

        fs::remove_dir_all(root).unwrap();
    }
}
