use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Result of diff mode resolution.
pub struct DiffContext {
    /// Base branch or commit used for diff.
    pub base: String,
    /// Set of changed `.rs` file paths (relative to project root).
    pub changed_files: HashSet<PathBuf>,
}

/// Resolve the diff base and collect changed `.rs` files.
///
/// `base_hint` is the user-provided `--diff` value:
/// - `"auto"` → auto-detect via `git merge-base HEAD main` then `master`
/// - any other string → use as the base branch/commit directly
///
/// Returns `Ok(DiffContext)` on success, or `Err` with a typed error.
pub fn resolve_diff(
    project_root: &Path,
    base_hint: &str,
) -> Result<DiffContext, crate::error::DiffError> {
    use crate::error::DiffError;

    // Check if we're in a git repo
    if !is_git_repo(project_root) {
        return Err(DiffError::GitNotFound);
    }

    let base = if base_hint == "auto" {
        auto_detect_base(project_root).map_err(DiffError::MergeBaseFailed)?
    } else {
        validate_ref_name(base_hint).map_err(|reason| DiffError::InvalidRef {
            name: base_hint.to_string(),
            reason,
        })?;
        // Verify the branch/commit exists
        verify_ref_exists(project_root, base_hint).map_err(|reason| DiffError::InvalidRef {
            name: base_hint.to_string(),
            reason,
        })?;
        base_hint.to_string()
    };

    let changed_files = get_changed_rs_files(project_root, &base).map_err(DiffError::Other)?;

    Ok(DiffContext {
        base,
        changed_files,
    })
}

/// Check if the directory is inside a git repository.
fn is_git_repo(dir: &Path) -> bool {
    Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(dir)
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Auto-detect the base branch by trying `main` then `master`.
fn auto_detect_base(dir: &Path) -> Result<String, String> {
    // Try merge-base with main
    for branch in ["main", "master"] {
        let output = Command::new("git")
            .args(["merge-base", "HEAD", branch])
            .current_dir(dir)
            .output();

        if let Ok(output) = output
            && output.status.success()
        {
            let commit = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !commit.is_empty() {
                return Ok(commit);
            }
        }
    }

    // Fallback: use HEAD~1
    let output = Command::new("git")
        .args(["rev-parse", "HEAD~1"])
        .current_dir(dir)
        .output()
        .map_err(|e| format!("git rev-parse failed: {e}"))?;

    if output.status.success() {
        let commit = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !commit.is_empty() {
            return Ok(commit);
        }
    }

    Err("Could not auto-detect base branch. Specify one with `--diff <branch>`".into())
}

/// Validate that a ref name is safe to pass as a git argument.
/// Rejects values that could cause git argument injection or unexpected behavior.
fn validate_ref_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Ref name cannot be empty".into());
    }
    if name.starts_with('-') {
        return Err(format!(
            "Invalid ref name '{name}': must not start with '-'"
        ));
    }
    if name.contains('\0') {
        return Err(format!("Invalid ref name '{name}': contains null byte"));
    }
    if name.contains("..") {
        return Err(format!("Invalid ref name '{name}': contains '..' sequence"));
    }
    if name.contains(|c: char| c.is_ascii_control()) {
        return Err(format!(
            "Invalid ref name '{name}': contains control character"
        ));
    }
    if name.contains(' ') {
        return Err(format!("Invalid ref name '{name}': contains space"));
    }
    if name.contains(':') {
        return Err(format!("Invalid ref name '{name}': contains colon"));
    }
    Ok(())
}

/// Verify a git ref (branch, tag, commit) exists.
fn verify_ref_exists(dir: &Path, ref_name: &str) -> Result<(), String> {
    let output = Command::new("git")
        .args(["rev-parse", "--verify", ref_name])
        .current_dir(dir)
        .output()
        .map_err(|e| format!("git rev-parse failed: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "Base ref '{ref_name}' not found. Try `--diff main` or `--diff HEAD~1`"
        ));
    }
    Ok(())
}

/// Get the list of changed `.rs` files between base and HEAD.
fn get_changed_rs_files(dir: &Path, base: &str) -> Result<HashSet<PathBuf>, String> {
    // Use merge-base to find the common ancestor, then diff against HEAD.
    // This avoids interpolating user input into a single argument with `...`.
    let merge_base_output = Command::new("git")
        .args(["merge-base", base, "HEAD"])
        .current_dir(dir)
        .output()
        .map_err(|e| format!("git merge-base failed: {e}"))?;

    let effective_base = if merge_base_output.status.success() {
        let candidate = String::from_utf8_lossy(&merge_base_output.stdout)
            .trim()
            .to_string();
        // Validate merge-base output looks like a hex SHA to prevent injection
        if !candidate.is_empty()
            && candidate.len() <= 40
            && candidate.chars().all(|c| c.is_ascii_hexdigit())
        {
            candidate
        } else {
            base.to_string()
        }
    } else {
        // Fallback: use base directly (e.g., for commit SHAs)
        base.to_string()
    };

    let mut child = Command::new("git")
        .args(["diff", "--name-only", &effective_base, "HEAD"])
        .current_dir(dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("git diff failed: {e}"))?;

    // Cap output to prevent OOM on pathological repositories
    const MAX_DIFF_OUTPUT_BYTES: u64 = 1024 * 1024; // 1 MB
    let mut stdout_data = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        use std::io::Read;
        let _ = stdout
            .take(MAX_DIFF_OUTPUT_BYTES)
            .read_to_end(&mut stdout_data);
    }

    let status = child
        .wait()
        .map_err(|e| format!("git diff wait failed: {e}"))?;
    if !status.success() {
        return Err(format!("git diff failed for base '{base}'"));
    }

    Ok(parse_changed_rs_files(&stdout_data))
}

fn parse_changed_rs_files(output: &[u8]) -> HashSet<PathBuf> {
    String::from_utf8_lossy(output)
        .lines()
        .filter(|line| {
            std::path::Path::new(line)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("rs"))
        })
        .map(|line| PathBuf::from(line.trim()))
        .collect()
}

/// Normalize a path by stripping a leading `./` and converting to forward slashes.
/// This handles mismatches between git output (always forward slashes) and
/// diagnostic paths that may use OS-specific separators or `./` prefixes.
fn normalize_path(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    let stripped = s.strip_prefix("./").unwrap_or(&s);
    PathBuf::from(stripped.replace('\\', "/"))
}

/// Filter diagnostics to only include those from changed files.
///
/// Uses normalized path comparison to handle mismatches between
/// git diff output (relative to repo root) and diagnostic paths
/// (which may be absolute or relative to a workspace member).
pub fn filter_to_changed_files(
    diagnostics: Vec<crate::diagnostics::Diagnostic>,
    changed_files: &HashSet<PathBuf>,
) -> Vec<crate::diagnostics::Diagnostic> {
    let normalized_changed: HashSet<PathBuf> =
        changed_files.iter().map(|p| normalize_path(p)).collect();

    diagnostics
        .into_iter()
        .filter(|d| {
            let norm_diag = normalize_path(&d.file_path);

            // Exact match after normalization
            normalized_changed.contains(&norm_diag)
                // Suffix match: diagnostic path ends with a changed file path
                // (handles absolute diagnostic paths vs relative git paths)
                || normalized_changed.iter().any(|cf| norm_diag.ends_with(cf))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_changed_rs_files() {
        let output = b"src/main.rs\nsrc/lib.rs\nREADME.md\ntests/test.rs\nCargo.toml\n";
        let files = parse_changed_rs_files(output);
        assert_eq!(files.len(), 3);
        assert!(files.contains(&PathBuf::from("src/main.rs")));
        assert!(files.contains(&PathBuf::from("src/lib.rs")));
        assert!(files.contains(&PathBuf::from("tests/test.rs")));
    }

    #[test]
    fn test_parse_empty_output() {
        let files = parse_changed_rs_files(b"");
        assert!(files.is_empty());
    }

    #[test]
    fn test_parse_no_rs_files() {
        let output = b"README.md\nCargo.toml\n.gitignore\n";
        let files = parse_changed_rs_files(output);
        assert!(files.is_empty());
    }

    #[test]
    fn test_filter_to_changed_files() {
        use crate::diagnostics::{Category, Diagnostic, Severity};

        let diags = vec![
            Diagnostic {
                file_path: PathBuf::from("src/main.rs"),
                rule: "test".into(),
                category: Category::Style,
                severity: Severity::Warning,
                message: "test".into(),
                help: None,
                line: Some(1),
                column: None,
                fix: None,
            },
            Diagnostic {
                file_path: PathBuf::from("src/lib.rs"),
                rule: "test".into(),
                category: Category::Style,
                severity: Severity::Warning,
                message: "test".into(),
                help: None,
                line: Some(1),
                column: None,
                fix: None,
            },
        ];

        let changed: HashSet<PathBuf> = HashSet::from([PathBuf::from("src/main.rs")]);
        let filtered = filter_to_changed_files(diags, &changed);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].file_path, PathBuf::from("src/main.rs"));
    }

    #[test]
    fn test_filter_handles_dot_slash_prefix() {
        use crate::diagnostics::{Category, Diagnostic, Severity};

        let diags = vec![Diagnostic {
            file_path: PathBuf::from("./src/main.rs"),
            rule: "test".into(),
            category: Category::Style,
            severity: Severity::Warning,
            message: "test".into(),
            help: None,
            line: Some(1),
            column: None,
            fix: None,
        }];

        let changed: HashSet<PathBuf> = HashSet::from([PathBuf::from("src/main.rs")]);
        let filtered = filter_to_changed_files(diags, &changed);
        assert_eq!(filtered.len(), 1);
    }

    #[test]
    fn test_filter_handles_absolute_diagnostic_path() {
        use crate::diagnostics::{Category, Diagnostic, Severity};

        let diags = vec![Diagnostic {
            file_path: PathBuf::from("/home/user/project/src/main.rs"),
            rule: "test".into(),
            category: Category::Style,
            severity: Severity::Warning,
            message: "test".into(),
            help: None,
            line: Some(1),
            column: None,
            fix: None,
        }];

        let changed: HashSet<PathBuf> = HashSet::from([PathBuf::from("src/main.rs")]);
        let filtered = filter_to_changed_files(diags, &changed);
        assert_eq!(filtered.len(), 1);
    }

    #[test]
    fn test_validate_ref_name_rejects_dash_prefix() {
        assert!(validate_ref_name("--upload-pack=evil").is_err());
        assert!(validate_ref_name("-f").is_err());
    }

    #[test]
    fn test_validate_ref_name_accepts_valid_refs() {
        assert!(validate_ref_name("main").is_ok());
        assert!(validate_ref_name("feature/my-branch").is_ok());
        assert!(validate_ref_name("HEAD~1").is_ok());
        assert!(validate_ref_name("abc123def").is_ok());
    }

    #[test]
    fn test_validate_ref_name_rejects_empty() {
        assert!(validate_ref_name("").is_err());
    }

    #[test]
    fn test_validate_ref_name_rejects_double_dot() {
        assert!(validate_ref_name("main..HEAD").is_err());
        assert!(validate_ref_name("../../etc/passwd").is_err());
    }

    #[test]
    fn test_validate_ref_name_rejects_control_chars() {
        assert!(validate_ref_name("main\nHEAD").is_err());
        assert!(validate_ref_name("main\x00HEAD").is_err());
    }

    #[test]
    fn test_validate_ref_name_rejects_space() {
        assert!(validate_ref_name("main HEAD").is_err());
    }

    #[test]
    fn test_validate_ref_name_rejects_colon() {
        assert!(validate_ref_name("refs/heads/main:refs/heads/foo").is_err());
    }

    #[test]
    fn test_is_git_repo_on_self() {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        assert!(is_git_repo(manifest_dir));
    }

    #[test]
    fn test_is_git_repo_on_tmp() {
        assert!(!is_git_repo(Path::new("/tmp")));
    }
}
