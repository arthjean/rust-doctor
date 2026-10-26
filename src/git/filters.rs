//! The filter drivers a repository configures, read once per scan and per git
//! location, so every call can switch each of them off.
//!
//! A `.gitattributes` the tree commits routes paths to a driver, and the
//! repository's configuration names the program the driver runs: `diff`
//! runs its `clean`, `checkout-index` its `smudge`, both its `process`. A
//! fixed list of known names, which LFS alone used to be, leaves every other
//! name to run. The names are therefore read from the effective
//! configuration, includes resolved, and each is blanked through
//! `GIT_CONFIG_COUNT` pairs: those are command-scope configuration, which
//! outranks every file. `-c` would rank the same, but git splits its
//! argument at the first `=`, so `-c filter.p=q.process=` sets a key named
//! `filter.p` and leaves the driver `p=q` running.
//!
//! The enumeration carries a canary pair and must read it back. A git that
//! ignores the pairs (before 2.31) cannot neutralize anything, so a
//! repository that configures a driver other than LFS is refused rather than
//! scanned with the driver live. LFS keeps `GIT_LFS_SKIP_SMUDGE` on such a git.
//!
//! LFS is blanked whether or not it is configured: checking a snapshot out
//! would smudge each pointer file into a download, and comparing the working
//! tree runs its clean filter, which writes under `.git/lfs`. A snapshot holds
//! the pointer files the commit records.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use super::GitCall;
use crate::internal_error::InternalError;

/// The most the enumeration may answer. A configuration that names more
/// drivers than fit here is refused, never half neutralized.
pub(super) const ENUMERATION_LIMIT: usize = 65_536;

const CANARY_KEY: &str = "rust-doctor.canary";
const CANARY_VALUE: &str = "1";
/// Set through `-c`, which every git honors, so the enumeration always
/// matches a key: `--get-regexp` exits 1 on no match, and a git that ignores
/// the canary pair would otherwise fail every call of a repository with no
/// driver at all.
const PRESENT: &str = "rust-doctor.present=1";
const PATTERN: &str = r"^(filter\.|rust-doctor\.(canary|present)$)";

const NO_PAIRS: &str =
    "git 2.31 or later is required to scan a repository that configures filter drivers";

/// Bumped when a scan starts, so a configuration changed between two scans of
/// one process (the interactive report rescans) is read again.
static GENERATION: AtomicU64 = AtomicU64::new(0);

type Enumerated = BTreeMap<Vec<OsString>, (u64, Arc<[String]>)>;

static ENUMERATED: Mutex<Enumerated> = Mutex::new(BTreeMap::new());

/// Forgets every enumeration: the next call of each location reads its
/// configuration again.
pub(crate) fn forget() {
    GENERATION.fetch_add(1, Ordering::SeqCst);
}

/// The `KEY`, `VALUE` pairs that switch off LFS and every driver `location`
/// configures.
pub(super) fn neutralizing_pairs(names: &[String]) -> Vec<(String, &'static str)> {
    std::iter::once("lfs")
        .chain(names.iter().map(String::as_str).filter(|name| *name != "lfs"))
        .flat_map(|name| {
            [
                (format!("filter.{name}.clean"), ""),
                (format!("filter.{name}.smudge"), ""),
                (format!("filter.{name}.process"), ""),
                (format!("filter.{name}.required"), "false"),
            ]
        })
        .collect()
}

/// The pair the enumeration call carries and must read back.
pub(super) fn canary_pairs() -> Vec<(String, &'static str)> {
    vec![(CANARY_KEY.to_owned(), CANARY_VALUE)]
}

/// The arguments of the enumeration, under the same global options as the
/// call it serves.
pub(super) fn enumeration_arguments(location: &[OsString]) -> Vec<OsString> {
    location
        .iter()
        .cloned()
        .chain(
            ["-c", PRESENT, "config", "-z", "--name-only", "--get-regexp", PATTERN]
                .into_iter()
                .map(OsString::from),
        )
        .collect()
}

/// The global options at the head of `arguments`: the `-c`, `-C`,
/// `--git-dir` and `--work-tree` every call of this crate opens with, which
/// select the repository and the configuration it reads. Parsing stops at the
/// operation, so a path operand spelled like an option is never read as one.
pub(super) fn location(arguments: &[OsString]) -> &[OsString] {
    let mut index = 0;
    while let Some(argument) = arguments.get(index) {
        let text = argument.to_string_lossy();
        index += match text.as_ref() {
            "-c" | "-C" => 2,
            "--no-pager" => 1,
            _ if text.starts_with("--git-dir=") || text.starts_with("--work-tree=") => 1,
            _ => break,
        };
    }
    &arguments[..index.min(arguments.len())]
}

/// The driver names the call `key` identifies (its git, its working
/// directory and its [`location`]) configures, read through `enumerate` on
/// the first call of the current scan and remembered after.
pub(super) fn drivers(
    key: Vec<OsString>,
    call: &GitCall,
    enumerate: impl FnOnce() -> Result<Vec<u8>, InternalError>,
) -> Result<Arc<[String]>, InternalError> {
    let generation = GENERATION.load(Ordering::SeqCst);
    // Held across the enumeration, so two threads of one scan asking for
    // one location start one git between them.
    let mut enumerated = ENUMERATED.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((seen, names)) = enumerated.get(&key)
        && *seen == generation
    {
        return Ok(Arc::clone(names));
    }
    let names: Arc<[String]> = parse(&enumerate()?, call)?.into();
    enumerated.insert(key, (generation, Arc::clone(&names)));
    Ok(names)
}

/// The driver names of an enumeration's answer: each key's text between its
/// first and its last dot, so `filter.p=q.process` names `p=q` and
/// `filter.a.b.clean` names `a.b`.
fn parse(answer: &[u8], call: &GitCall) -> Result<Vec<String>, InternalError> {
    let mut names = Vec::new();
    let mut canary = false;
    for key in answer.split(|byte| *byte == 0).filter(|key| !key.is_empty()) {
        // A name that is not UTF-8 cannot be written back into a pair this
        // crate builds as text, so its repository is refused.
        let key = std::str::from_utf8(key).map_err(|_| call.failure.error(call.stage))?;
        if key == CANARY_KEY {
            canary = true;
            continue;
        }
        let Some(rest) = key.strip_prefix("filter.") else {
            continue;
        };
        if let Some((name, _)) = rest.rsplit_once('.')
            && !names.iter().any(|known| known == name)
        {
            names.push(name.to_owned());
        }
    }
    if !canary && names.iter().any(|name| name != "lfs") {
        return Err(InternalError::new(call.stage, call.failure.code(), NO_PAIRS));
    }
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{GitFailure, OUTPUT_TOO_LARGE};

    fn call() -> GitCall {
        GitCall {
            arguments: Vec::new(),
            stdout_limit: 1,
            stage: "scope",
            failure: GitFailure::new("git-diff-failed", "Git changed files could not be read."),
            overflow: OUTPUT_TOO_LARGE,
        }
    }

    #[test]
    fn a_name_is_the_text_between_the_first_and_the_last_dot() {
        let answer = b"filter.Ab.clean\0filter.p=q.process\0filter.a.b.smudge\0filter.Ab.smudge\0filter.bare\0rust-doctor.canary\0";
        assert_eq!(parse(answer, &call()).unwrap(), ["Ab", "p=q", "a.b"]);
    }

    #[test]
    fn a_git_that_drops_the_canary_is_refused_once_a_foreign_driver_is_named() {
        assert_eq!(parse(b"filter.lfs.clean\0", &call()).unwrap(), ["lfs"]);
        assert!(parse(b"", &call()).unwrap().is_empty());
        let error = parse(b"filter.lfs.clean\0filter.x.clean\0", &call()).unwrap_err();
        assert_eq!((error.stage, error.code), ("scope", "git-diff-failed"));
        assert!(error.message.contains("git 2.31"), "{}", error.message);
    }

    #[test]
    fn every_name_and_lfs_get_four_blanked_keys() {
        let pairs = neutralizing_pairs(&["p=q".to_owned(), "lfs".to_owned()]);
        assert_eq!(
            pairs,
            [
                ("filter.lfs.clean".to_owned(), ""),
                ("filter.lfs.smudge".to_owned(), ""),
                ("filter.lfs.process".to_owned(), ""),
                ("filter.lfs.required".to_owned(), "false"),
                ("filter.p=q.clean".to_owned(), ""),
                ("filter.p=q.smudge".to_owned(), ""),
                ("filter.p=q.process".to_owned(), ""),
                ("filter.p=q.required".to_owned(), "false"),
            ]
        );
    }

    #[test]
    fn the_location_stops_at_the_operation() {
        let arguments: Vec<OsString> = [
            "-c",
            "color.ui=false",
            "--no-pager",
            "-C",
            "/w",
            "--git-dir=/w/.git",
            "--work-tree=/s",
            "check-ignore",
            "--",
            "--git-dir=x",
        ]
        .into_iter()
        .map(OsString::from)
        .collect();
        assert_eq!(location(&arguments), &arguments[..7]);
        assert!(location(&[]).is_empty());
    }

    #[test]
    fn the_enumeration_always_matches_a_key_it_sets_itself() {
        let arguments = enumeration_arguments(&[]);
        let position = arguments.iter().position(|argument| argument == "-c").unwrap();
        assert_eq!(arguments[position + 1], PRESENT);
        assert!(position < arguments.iter().position(|argument| argument == "config").unwrap());
        // Read back, the sentinel is neither the canary nor a driver.
        assert!(parse(b"rust-doctor.present\0", &call()).unwrap().is_empty());
    }
}
