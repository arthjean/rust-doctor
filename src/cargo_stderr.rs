//! What Cargo said on stderr when it failed, in the words a report may publish.
//!
//! Cargo writes its cause there, "failed to parse lock file", "could not
//! compile", "Blocking waiting for file lock", and the report used to publish
//! the exit status alone because the stream went to `/dev/null`. Every Cargo
//! child whose failure ends a stage is now drained through
//! [`crate::bounded_read::collect_bounded`] under [`STDERR_LIMIT`], and its
//! failure quotes the tail of what it said through [`excerpt`]: bounded,
//! stripped of control characters, and with the paths that would name the
//! machine replaced by names that do not.

use std::env;
use std::path::{Path, PathBuf};

/// How much of a Cargo child's stderr this process keeps.
///
/// A cold build prints one status line per crate, and two hundred members fit
/// in a fraction of this. The read continues past it, so a verbose build can
/// never block Cargo on a full pipe.
pub(crate) const STDERR_LIMIT: usize = 65_536;

/// How much of it reaches the published message, the bound `bounded_stderr`
/// sets on everything the binary prints on its own stderr.
const EXCERPT_BYTES: usize = 1_020;

/// The status lines Cargo prints while it works. They say what it did, never
/// why it stopped, and a failing build's last kilobyte is otherwise mostly
/// them.
const PROGRESS_VERBS: [&str; 12] = [
    "Adding",
    "Blocking",
    "Building",
    "Checking",
    "Compiling",
    "Documenting",
    "Downloaded",
    "Downloading",
    "Finished",
    "Fresh",
    "Locking",
    "Updating",
];

/// The last lines of `stderr` that fit the published bound, on one line, with
/// the workspace root, the target directory, `CARGO_HOME` and `HOME` replaced
/// by `.`, `$CARGO_TARGET_DIR`, `$CARGO_HOME` and `~`. `None` when Cargo said
/// nothing but progress.
///
/// The target directory is named apart from the root because it need not sit
/// under it: a `CARGO_TARGET_DIR` elsewhere, or the base side of a baseline,
/// which builds from a snapshot into the real workspace's target directory.
pub(crate) fn excerpt(
    stderr: &[u8],
    workspace_root: &Path,
    target_directory: Option<&Path>,
) -> Option<String> {
    let home = env::var_os("HOME").map(PathBuf::from);
    let cargo_home = env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| home.as_ref().map(|home| home.join(".cargo")));
    let text = String::from_utf8_lossy(stderr);
    let text = scrub(
        &text,
        &[
            (Some(workspace_root.to_path_buf()), "."),
            (target_directory.map(Path::to_path_buf), "$CARGO_TARGET_DIR"),
            (cargo_home, "$CARGO_HOME"),
            (home, "~"),
        ],
    );
    let lines: Vec<String> = text
        .lines()
        .filter(|line| !is_progress(line))
        .map(|line| {
            line.chars()
                .filter(|character| !character.is_control())
                .collect::<String>()
                .trim()
                .to_owned()
        })
        .filter(|line| !line.is_empty())
        .collect();
    tail(&lines)
}

/// Replaces every prefix, in the order given, in both its lexical and its
/// canonical spelling: a workspace reached through a symlink is printed by
/// Cargo under the path it resolved.
fn scrub(text: &str, prefixes: &[(Option<PathBuf>, &str)]) -> String {
    let mut text = text.to_owned();
    for (path, replacement) in prefixes {
        let Some(path) = path else { continue };
        let mut forms = vec![path.to_string_lossy().into_owned()];
        if let Ok(canonical) = path.canonicalize() {
            forms.push(canonical.to_string_lossy().into_owned());
        }
        forms.sort_by_key(|form| std::cmp::Reverse(form.len()));
        forms.dedup();
        for form in forms.iter().filter(|form| form.len() > 1) {
            text = text.replace(form.as_str(), replacement);
        }
    }
    text
}

fn is_progress(line: &str) -> bool {
    line.starts_with(' ')
        && line
            .split_whitespace()
            .next()
            .is_some_and(|verb| PROGRESS_VERBS.contains(&verb))
}

/// The longest suffix of `lines` whose joined form fits [`EXCERPT_BYTES`], cut
/// at a character boundary when the last line alone does not.
fn tail(lines: &[String]) -> Option<String> {
    let mut kept: Vec<&str> = Vec::new();
    let mut length = 0_usize;
    for line in lines.iter().rev() {
        let added = line.len() + usize::from(!kept.is_empty());
        if length + added > EXCERPT_BYTES {
            if kept.is_empty() {
                let mut start = line.len() - EXCERPT_BYTES;
                while !line.is_char_boundary(start) {
                    start += 1;
                }
                kept.push(line.get(start..).unwrap_or_default());
            }
            break;
        }
        length += added;
        kept.push(line);
    }
    kept.reverse();
    (!kept.is_empty()).then(|| kept.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_excerpt_keeps_the_cause_and_drops_the_progress() {
        let stderr = b"    Checking a v0.1.0 (/work/a)\n\
error: failed to parse lock file at: /work/Cargo.lock\n\nCaused by:\n  \
TOML parse error at line 1, column 6\n\x1b[1mbold\x1b[0m\n";
        let published = excerpt(stderr, Path::new("/work"), None).unwrap();
        assert!(published.starts_with("error: failed to parse lock file at: ./Cargo.lock"));
        assert!(published.contains("TOML parse error at line 1, column 6"));
        assert!(!published.contains("Checking"));
        assert!(!published.contains('\x1b'));
        assert_eq!(
            excerpt(b"   Compiling a v0.1.0\n", Path::new("/work"), None),
            None
        );
    }

    #[test]
    fn the_excerpt_is_the_tail_and_fits_the_bound() {
        let lines: Vec<String> = (0..200).map(|index| format!("line {index:03}")).collect();
        let joined = tail(&lines).unwrap();
        assert!(joined.len() <= EXCERPT_BYTES);
        assert!(joined.ends_with("line 199"));
        let long = vec!["é".repeat(EXCERPT_BYTES)];
        let cut = tail(&long).unwrap();
        assert!(cut.len() <= EXCERPT_BYTES);
    }

    #[test]
    fn a_target_directory_outside_the_workspace_is_named_not_published() {
        // The base side of a baseline: a snapshot root, and the real
        // workspace's target directory, neither under the other.
        let published = excerpt(
            b"error: failed to write /srv/repo/target/rust-doctor/baseline/debug/x.rmeta\n",
            Path::new("/tmp/snapshot"),
            Some(Path::new("/srv/repo/target/rust-doctor/baseline")),
        )
        .unwrap();
        assert_eq!(
            published,
            "error: failed to write $CARGO_TARGET_DIR/debug/x.rmeta"
        );
    }

    #[test]
    fn the_three_prefixes_are_replaced_most_specific_first() {
        let scrubbed = scrub(
            "/home/u/w/src /home/u/.cargo/registry /home/u/x",
            &[
                (Some(PathBuf::from("/home/u/w")), "."),
                (Some(PathBuf::from("/home/u/.cargo")), "$CARGO_HOME"),
                (Some(PathBuf::from("/home/u")), "~"),
            ],
        );
        assert_eq!(scrubbed, "./src $CARGO_HOME/registry ~/x");
    }
}
