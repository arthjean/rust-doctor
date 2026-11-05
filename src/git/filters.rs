use std::collections::BTreeMap;
use std::ffi::OsString;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use super::GitCall;
use crate::internal_error::InternalError;

pub(super) const ENUMERATION_LIMIT: usize = 65_536;

const CANARY_KEY: &str = "rust-doctor.canary";
const CANARY_VALUE: &str = "1";
const PRESENT: &str = "rust-doctor.present=1";
const PATTERN: &str = r"^(filter\.|rust-doctor\.(canary|present)$)";

const NO_PAIRS: &str =
    "git 2.31 or later is required to scan a repository that configures filter drivers";

static GENERATION: AtomicU64 = AtomicU64::new(0);

type Enumerated = BTreeMap<Vec<OsString>, (u64, Arc<[String]>)>;

static ENUMERATED: Mutex<Enumerated> = Mutex::new(BTreeMap::new());

pub(crate) fn forget() {
    GENERATION.fetch_add(1, Ordering::SeqCst);
}

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

pub(super) fn canary_pairs() -> Vec<(String, &'static str)> {
    vec![(CANARY_KEY.to_owned(), CANARY_VALUE)]
}

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

pub(super) fn drivers(
    key: Vec<OsString>,
    call: &GitCall,
    enumerate: impl FnOnce() -> Result<Vec<u8>, InternalError>,
) -> Result<Arc<[String]>, InternalError> {
    let generation = GENERATION.load(Ordering::SeqCst);
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

fn parse(answer: &[u8], call: &GitCall) -> Result<Vec<String>, InternalError> {
    let mut names = Vec::new();
    let mut canary = false;
    for key in answer.split(|byte| *byte == 0).filter(|key| !key.is_empty()) {
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
        assert!(parse(b"rust-doctor.present\0", &call()).unwrap().is_empty());
    }
}
