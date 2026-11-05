use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::{STAGE, temp_unavailable};
use crate::internal_error::InternalError;

#[derive(Debug)]
pub(super) struct Candidate {
    pub(super) path: PathBuf,
    pub(super) create_under: Option<PathBuf>,
}

pub(super) fn candidates() -> Vec<Candidate> {
    let mut candidates = vec![Candidate {
        path: env::temp_dir(),
        create_under: None,
    }];
    if cfg!(unix) {
        let absolute = |name: &str| {
            env::var_os(name)
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
        };
        if let Some(runtime) = absolute("XDG_RUNTIME_DIR") {
            candidates.push(Candidate {
                path: runtime,
                create_under: None,
            });
        }
        let cache = absolute("XDG_CACHE_HOME")
            .map(|cache| (cache.join("rust-doctor"), cache))
            .or_else(|| absolute("HOME").map(|home| (home.join(".cache/rust-doctor"), home)));
        if let Some((path, base)) = cache {
            candidates.push(Candidate {
                path,
                create_under: Some(base),
            });
        }
    }
    candidates
}

pub(super) fn choose(
    repository: &Path,
    candidates: impl IntoIterator<Item = Candidate>,
) -> Result<PathBuf, InternalError> {
    let repository = repository.canonicalize().map_err(|_| temp_unavailable())?;
    let mut first_refusal = None;
    for candidate in candidates {
        match accept(&repository, &candidate) {
            Ok(parent) => return Ok(parent),
            Err(refusal) => {
                first_refusal.get_or_insert(refusal);
            }
        }
    }
    Err(InternalError::new(
        STAGE,
        "baseline-temp-unavailable",
        format!(
            "no private directory for the snapshot: {}",
            first_refusal.unwrap_or_else(|| "no candidate directory is set".to_owned())
        ),
    ))
}

fn accept(repository: &Path, candidate: &Candidate) -> Result<PathBuf, String> {
    let unavailable = || format!("{} is unavailable", candidate.path.display());
    let canonical = |path: &Path| path.canonicalize().map_err(|_| unavailable());
    if let Some(base) = &candidate.create_under {
        let base = canonical(base)?;
        if !base.is_dir() {
            return Err(unavailable());
        }
        vet(repository, &base)?;
        create_owner_only(&candidate.path).map_err(|_| unavailable())?;
    }
    let parent = canonical(&candidate.path)?;
    if !parent.is_dir() {
        return Err(unavailable());
    }
    vet(repository, &parent)?;
    Ok(parent)
}

fn vet(repository: &Path, canonical: &Path) -> Result<(), String> {
    if canonical.starts_with(repository) {
        return Err(format!("{} is inside the repository", canonical.display()));
    }
    refusal(canonical).map_or(Ok(()), Err)
}

#[cfg(unix)]
fn create_owner_only(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;

    fs::DirBuilder::new().recursive(true).mode(0o700).create(path)
}

#[cfg(not(unix))]
fn create_owner_only(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)
}

#[cfg(unix)]
fn refusal(canonical: &Path) -> Option<String> {
    use std::os::unix::fs::MetadataExt;

    let user = crate::execution::effective_user();
    for ancestor in canonical.ancestors() {
        let Ok(metadata) = fs::metadata(ancestor) else {
            return Some(format!("{} is unavailable", ancestor.display()));
        };
        if metadata.uid() != user && metadata.uid() != 0 {
            return Some(format!("{} is owned by another user", ancestor.display()));
        }
        if metadata.mode() & 0o022 != 0 {
            return Some(format!("{} is writable by other users", ancestor.display()));
        }
    }
    None
}

#[cfg(not(unix))]
fn refusal(_canonical: &Path) -> Option<String> {
    None
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    fn scratch() -> (PathBuf, PathBuf) {
        let base = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/baseline-parent")
            .join(format!("{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("repository")).unwrap();
        let base = base.canonicalize().unwrap();
        fs::set_permissions(&base, fs::Permissions::from_mode(0o700)).unwrap();
        (base.clone(), base.join("repository"))
    }

    fn directory(path: &Path, mode: u32) -> Candidate {
        fs::create_dir_all(path).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
        Candidate {
            path: path.to_path_buf(),
            create_under: None,
        }
    }

    #[test]
    fn an_owner_only_directory_is_accepted() {
        let (base, repository) = scratch();
        let parent = choose(&repository, [directory(&base.join("private"), 0o700)]).unwrap();
        assert_eq!(parent, base.join("private"));
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn a_sticky_world_writable_or_group_writable_directory_is_refused() {
        let (base, repository) = scratch();
        for (name, mode) in [("sticky", 0o1777), ("group", 0o775)] {
            let error = choose(&repository, [directory(&base.join(name), mode)]).unwrap_err();
            assert_eq!(error.code, "baseline-temp-unavailable");
            assert_eq!(
                error.message,
                format!(
                    "no private directory for the snapshot: {} is writable by other users",
                    base.join(name).display()
                )
            );
        }
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn a_private_directory_under_a_writable_ancestor_is_refused() {
        let (base, repository) = scratch();
        let open = directory(&base.join("open"), 0o777);
        let inner = directory(&open.path.join("inner"), 0o700);
        let error = choose(&repository, [inner]).unwrap_err();
        assert!(error.message.ends_with(&format!("{} is writable by other users", open.path.display())));
        fs::set_permissions(&open.path, fs::Permissions::from_mode(0o700)).unwrap();
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn a_symlinked_ancestor_is_resolved_before_the_check() {
        let (base, repository) = scratch();
        let open = directory(&base.join("open"), 0o1777);
        directory(&open.path.join("inner"), 0o700);
        let private = directory(&base.join("private"), 0o700);
        symlink(&open.path, private.path.join("to-open")).unwrap();
        symlink(&private.path, base.join("to-private")).unwrap();

        let through_link = Candidate {
            path: private.path.join("to-open/inner"),
            create_under: None,
        };
        let error = choose(&repository, [through_link]).unwrap_err();
        assert!(error.message.ends_with(&format!("{} is writable by other users", open.path.display())));

        let linked = Candidate {
            path: base.join("to-private"),
            create_under: None,
        };
        assert_eq!(choose(&repository, [linked]).unwrap(), private.path);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn the_first_private_candidate_wins_and_the_cache_home_is_created_owner_only() {
        let (base, repository) = scratch();
        let inside = directory(&repository.join("tmp"), 0o700);
        let cache = base.join("cache/rust-doctor");
        let parent = choose(
            &repository,
            [
                directory(&base.join("sticky"), 0o1777),
                Candidate {
                    path: base.join("missing"),
                    create_under: None,
                },
                inside,
                Candidate {
                    path: cache.clone(),
                    create_under: Some(base.clone()),
                },
            ],
        )
        .unwrap();
        assert_eq!(parent, cache);
        assert_eq!(fs::metadata(&cache).unwrap().permissions().mode() & 0o777, 0o700);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn a_cache_home_under_a_writable_parent_is_refused_uncreated() {
        let (base, repository) = scratch();
        let home = directory(&base.join("home"), 0o775);
        let cache = home.path.join(".cache/rust-doctor");
        let error = choose(
            &repository,
            [Candidate {
                path: cache,
                create_under: Some(home.path.clone()),
            }],
        )
        .unwrap_err();
        assert!(error.message.ends_with(&format!("{} is writable by other users", home.path.display())));
        assert!(!home.path.join(".cache").exists());

        let missing = base.join("missing-home");
        let error = choose(
            &repository,
            [Candidate {
                path: missing.join(".cache/rust-doctor"),
                create_under: Some(missing.clone()),
            }],
        )
        .unwrap_err();
        assert!(error.message.ends_with("rust-doctor is unavailable"), "{}", error.message);
        assert!(!missing.exists());
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn no_candidate_names_the_first_refusal() {
        let (base, repository) = scratch();
        let inside = directory(&repository.join("tmp"), 0o700);
        let error = choose(&repository, [inside]).unwrap_err();
        assert_eq!(
            error.message,
            format!(
                "no private directory for the snapshot: {} is inside the repository",
                repository.join("tmp").display()
            )
        );
        let error = choose(&repository, []).unwrap_err();
        assert_eq!(
            error.message,
            "no private directory for the snapshot: no candidate directory is set"
        );
        fs::remove_dir_all(base).unwrap();
    }
}
