use std::env;
use std::path::{Path, PathBuf};

pub(crate) const STDERR_LIMIT: usize = 65_536;

const EXCERPT_BYTES: usize = 1_020;

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

pub(crate) fn excerpt(
    stderr: &[u8],
    workspace_root: &Path,
    target_directory: Option<&Path>,
) -> Option<String> {
    let home = crate::report::home_directory();
    let cargo_home = env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| home.as_ref().map(|home| home.join(".cargo")));
    let rustup_home = env::var_os("RUSTUP_HOME")
        .map(PathBuf::from)
        .or_else(|| home.as_ref().map(|home| home.join(".rustup")));
    let text = String::from_utf8_lossy(stderr);
    let text = scrub(
        &text,
        &[
            (Some(workspace_root.to_path_buf()), "."),
            (target_directory.map(Path::to_path_buf), "$CARGO_TARGET_DIR"),
            (cargo_home, "$CARGO_HOME"),
            (rustup_home, "$RUSTUP_HOME"),
            (home, "~"),
        ],
    );
    let text = crate::repo_hygiene::mask_credentials(&text);
    let lines: Vec<String> = text
        .lines()
        .map(plain)
        .filter(|line| !is_progress(line))
        .map(|line| line.trim().to_owned())
        .filter(|line| !line.is_empty())
        .collect();
    tail(&lines)
}

fn plain(line: &str) -> String {
    let mut plain = String::with_capacity(line.len());
    let mut characters = line.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '\u{1b}' {
            if characters.next_if_eq(&'[').is_some() {
                for next in characters.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            }
        } else if !character.is_control() {
            plain.push(character);
        }
    }
    plain
}

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
            text = crate::report::replace_path(&text, form, replacement);
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
    fn the_excerpt_returns_on_every_line_of_the_non_ascii_table() {
        for line in crate::boundary_table::lines() {
            let excerpt = excerpt(line.as_bytes(), Path::new("/work"), None);
            assert!(excerpt.is_some(), "{line:?}");
        }
    }

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
        assert!(published.ends_with("TOML parse error at line 1, column 6 bold"), "{published}");
        assert_eq!(
            excerpt(b"   Compiling a v0.1.0\n", Path::new("/work"), None),
            None
        );
    }

    #[test]
    fn a_colored_stderr_publishes_the_same_words() {
        let stderr = b"\x1b[1m\x1b[92m    Checking\x1b[0m a v0.1.0 (/work/a)\n\
\x1b[1m\x1b[91merror\x1b[0m\x1b[1m: failed to parse lock file at: /work/Cargo.lock\x1b[0m\n";
        assert_eq!(
            excerpt(stderr, Path::new("/work"), None).as_deref(),
            Some("error: failed to parse lock file at: ./Cargo.lock")
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
    fn a_key_a_build_script_echoed_is_masked_before_the_excerpt_cuts_it() {
        let body = "Q+/Z9".repeat(13);
        let mut stderr = format!(
            "error: failed to run custom build command for `probe`\n--- stderr\n-----BEGIN {}PRIVATE KEY-----\n",
            "RSA "
        );
        for _ in 0..120 {
            stderr.push_str(&body);
            stderr.push('\n');
        }
        stderr.push_str(&format!("-----END {}PRIVATE KEY-----\n", "RSA "));
        let unmasked: Vec<String> = stderr.lines().map(str::to_owned).collect();
        assert!(tail(&unmasked).unwrap().starts_with(&body));

        let published = excerpt(stderr.as_bytes(), Path::new("/work"), None).unwrap();
        let leaked = body
            .as_bytes()
            .windows(3)
            .filter_map(|window| std::str::from_utf8(window).ok())
            .any(|window| published.contains(window));
        assert!(!leaked, "{published}");
        assert!(published.starts_with("<redacted> <redacted>"), "{published}");
        assert!(published.ends_with("-----END RSA PRIVATE KEY-----"), "{published}");
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

    #[test]
    fn a_prefix_is_replaced_only_where_a_path_component_ends() {
        let scrubbed = scrub(
            "/home/u/.cargo-target/x /home/u/.cargo/registry /home/u/.cargo. /home/u/.cargo.d/y",
            &[
                (Some(PathBuf::from("/home/u/.cargo")), "$CARGO_HOME"),
                (Some(PathBuf::from("/home/u")), "~"),
            ],
        );
        assert_eq!(scrubbed, "~/.cargo-target/x $CARGO_HOME/registry $CARGO_HOME. ~/.cargo.d/y");
    }
}
