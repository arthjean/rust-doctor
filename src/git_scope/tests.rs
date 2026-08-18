use std::cell::RefCell;
use std::collections::VecDeque;
use std::ffi::{OsStr, OsString};

use super::*;

const BASE: &str = "1111111111111111111111111111111111111111";
const MERGE_BASE: &str = "2222222222222222222222222222222222222222";

fn files(base: &str) -> ScopeRequest {
    ScopeRequest::Files {
        base: base.to_owned(),
    }
}

fn validated_files(base: &str) -> ValidatedScope {
    files(base).validate().unwrap()
}

fn output(stdout: impl Into<Vec<u8>>) -> Result<Vec<u8>, InternalError> {
    Ok(stdout.into())
}

#[test]
fn closed_base_grammar_accepts_only_named_selectors_and_full_oids() {
    for accepted in [
        "main",
        "release/1.2.3",
        "refs/remotes/origin/main",
        BASE,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    ] {
        assert!(files(accepted).validate().is_ok(), "{accepted}");
    }
    for rejected in [
        "",
        "-main",
        ".hidden",
        "feature/.hidden",
        "feature/",
        "feature//child",
        "feature/../main",
        "main.",
        "main.lock",
        "HEAD~1",
        "main^{commit}",
        "révision",
    ] {
        let error = files(rejected).validate().unwrap_err();
        assert_eq!((error.stage, error.code), ("scope", "invalid-base"));
        if !rejected.is_empty() {
            assert!(!error.message.contains(rejected));
        }
    }
    assert!(files(&"a".repeat(256)).validate().is_err());
    assert!(ScopeRequest::Full.validate().is_ok());
}

/// A validated selector never prints the branch it holds, in any trace that
/// reaches a log or an error.
#[test]
fn a_validated_selector_redacts_itself() {
    let scope = validated_files("release/1.2.3");
    let rendered = format!("{scope:?}");
    assert!(!rendered.contains("release"), "{rendered}");
    assert_eq!(rendered, "Files(<redacted>)");
    assert_eq!(
        format!("{:?}", files("release/1.2.3")),
        "Files { base: <redacted> }"
    );
}

#[test]
fn full_returns_without_observing_git() {
    let calls = RefCell::new(0);
    let scope = resolve_with(&ValidatedScope::Full, Path::new("/workspace"), |_| {
        *calls.borrow_mut() += 1;
        output(Vec::new())
    })
    .unwrap();

    assert_eq!(*calls.borrow(), 0);
    assert_eq!(scope, ScopeReport::full());
}

#[test]
fn files_runs_three_exact_calls_and_normalizes_the_result() {
    let responses = RefCell::new(VecDeque::from([
        format!("{BASE}\n").into_bytes(),
        format!("{MERGE_BASE}\n").into_bytes(),
        b"src/z.rs\0src/a.rs\0src/z.rs\0".to_vec(),
    ]));
    let calls = RefCell::new(Vec::new());
    let scope = resolve_with(&validated_files("main"), Path::new("/workspace"), |call| {
        calls.borrow_mut().push(call.arguments.clone());
        assert_eq!(call.stage, "scope");
        output(responses.borrow_mut().pop_front().unwrap())
    })
    .unwrap();

    assert_eq!(calls.borrow().len(), 3);
    assert_eq!(
        calls.borrow()[0],
        [
            "-c",
            "color.ui=false",
            "-c",
            "core.fsmonitor=false",
            "--no-pager",
            "-C",
            "/workspace",
            "rev-parse",
            "--verify",
            "--quiet",
            "--end-of-options",
            "main^{commit}",
        ]
        .map(OsString::from)
    );
    assert_eq!(calls.borrow()[1][7], OsStr::new("merge-base"));
    assert_eq!(calls.borrow()[2][7], OsStr::new("diff"));
    assert_eq!(calls.borrow()[2][10], OsStr::new("--relative"));
    assert_eq!(scope.mode(), ScopeMode::Files);
    assert_eq!(scope.comparison_base(), Some(MERGE_BASE));
    assert_eq!(
        scope.files(),
        Some(&["src/a.rs".to_owned(), "src/z.rs".to_owned()][..])
    );
}

/// A baseline scope stops at the merge base and never asks for a diff.
#[test]
fn baseline_resolves_the_base_and_carries_no_file_list() {
    let responses = RefCell::new(VecDeque::from([
        format!("{BASE}\n").into_bytes(),
        format!("{MERGE_BASE}\n").into_bytes(),
    ]));
    let calls = RefCell::new(0);
    let scope = resolve_with(
        &ScopeRequest::Baseline {
            base: "main".to_owned(),
        }
        .validate()
        .unwrap(),
        Path::new("/workspace"),
        |_| {
            *calls.borrow_mut() += 1;
            output(responses.borrow_mut().pop_front().unwrap())
        },
    )
    .unwrap();

    assert_eq!(*calls.borrow(), 2);
    assert_eq!(scope.mode(), ScopeMode::Baseline);
    assert_eq!(scope.comparison_base(), Some(MERGE_BASE));
    assert_eq!(scope.files(), None);
}

#[test]
fn empty_diff_and_sha256_oids_are_closed_successes() {
    let oid64 = "a".repeat(64);
    let responses = RefCell::new(VecDeque::from([
        format!("{oid64}\n").into_bytes(),
        format!("{oid64}\n").into_bytes(),
        Vec::new(),
    ]));
    let scope = resolve_with(&validated_files(&oid64), Path::new("/workspace"), |_| {
        output(responses.borrow_mut().pop_front().unwrap())
    })
    .unwrap();
    assert_eq!(scope.comparison_base(), Some(oid64.as_str()));
    assert_eq!(scope.files(), Some(&[][..]));
}

#[test]
fn failures_stop_before_later_calls_and_never_transport_hostile_output() {
    for (failing_call, expected) in [
        (0, "base-unavailable"),
        (1, "merge-base-unavailable"),
        (2, "git-diff-failed"),
    ] {
        let calls = RefCell::new(0);
        let error = resolve_with(&validated_files("main"), Path::new("/workspace"), |call| {
            let index = *calls.borrow();
            *calls.borrow_mut() += 1;
            if index == failing_call {
                return Err(call.failure.error(call.stage));
            }
            match index {
                0 => output(format!("{BASE}\n")),
                1 => output(format!("{MERGE_BASE}\n")),
                _ => output(Vec::new()),
            }
        })
        .unwrap_err();
        assert_eq!((error.stage, error.code), ("scope", expected));
        assert_eq!(*calls.borrow(), failing_call + 1);
        assert!(!error.message.contains("credential=secret"));
    }
}

#[test]
fn missing_and_ambiguous_merge_bases_fail_before_diff() {
    for (merge_output, expected) in [
        (Vec::new(), "merge-base-unavailable"),
        (
            format!("{BASE}\n{MERGE_BASE}\n").into_bytes(),
            "merge-base-ambiguous",
        ),
        (b"not-an-oid\n".to_vec(), "merge-base-unavailable"),
        // A merge base of another hash length is not an answer about the
        // commit that was asked for.
        (
            format!("{}\n", "a".repeat(64)).into_bytes(),
            "merge-base-unavailable",
        ),
    ] {
        let responses = RefCell::new(VecDeque::from([
            format!("{BASE}\n").into_bytes(),
            merge_output,
        ]));
        let calls = RefCell::new(0);
        let error = resolve_with(&validated_files("main"), Path::new("/workspace"), |_| {
            *calls.borrow_mut() += 1;
            output(responses.borrow_mut().pop_front().unwrap())
        })
        .unwrap_err();
        assert_eq!(error.code, expected);
        assert_eq!(*calls.borrow(), 2);
    }
}

#[test]
fn all_output_and_path_boundaries_fail_atomically() {
    assert!(parse_single_oid(format!("{BASE}\n{MERGE_BASE}\n").as_bytes()).is_none());
    assert!(parse_single_oid(b"not-an-oid\n").is_none());
    assert_eq!(parse_paths(&[]).unwrap(), Vec::<String>::new());
    // Normalization is what the parser answers; the ordering is the
    // constructor's, and `files_scope` is where it is stated.
    assert_eq!(
        parse_paths(b"space name\0tab\tname\0line\nname\0percent%name\0").unwrap(),
        ["space name", "tab%09name", "line%0Aname", "percent%25name"]
    );

    let too_many = b"a\0".repeat(FILE_LIMIT + 1);
    assert_eq!(parse_paths(&too_many).unwrap_err().code, "too-many-files");
    let long_path = [vec![b'a'; PATH_LIMIT + 1], vec![0]].concat();
    assert_eq!(
        parse_paths(&long_path).unwrap_err().code,
        "git-path-invalid"
    );
    for invalid in [
        vec![0],
        b"/absolute\0".to_vec(),
        b"./relative\0".to_vec(),
        b"parent/../escape\0".to_vec(),
        b"double//component\0".to_vec(),
        vec![0xff, 0],
        b"unterminated".to_vec(),
    ] {
        assert_eq!(parse_paths(&invalid).unwrap_err().code, "git-path-invalid");
    }
}

/// One constructor owns the order `includes` searches, so a scope built from
/// unsorted, duplicated paths still answers correctly.
#[test]
fn the_file_constructor_owns_the_order_membership_is_searched_in() {
    let scope = ScopeReport::files_scope(
        MERGE_BASE.to_owned(),
        vec![
            "src/z.rs".to_owned(),
            "src/a.rs".to_owned(),
            "src/z.rs".to_owned(),
        ],
    )
    .unwrap();

    assert_eq!(
        scope.files(),
        Some(&["src/a.rs".to_owned(), "src/z.rs".to_owned()][..])
    );
    assert!(scope.includes(Some("src/a.rs")));
    assert!(scope.includes(Some("src/z.rs")));
    assert!(!scope.includes(Some("src/missing.rs")));
    assert!(!scope.includes(None));

    // Every other shape admits everything, with or without a path.
    for open in [
        ScopeReport::full(),
        ScopeReport::baseline_scope(MERGE_BASE.to_owned()),
    ] {
        assert!(open.includes(Some("src/anything.rs")));
        assert!(open.includes(None));
    }
}

#[test]
fn serialized_scope_limit_accepts_the_last_byte_and_rejects_the_boundary() {
    let overhead = {
        let empty = ScopeReport::files_scope(MERGE_BASE.to_owned(), vec![String::new()]).unwrap();
        serde_json::to_vec(&empty).unwrap().len()
    };
    let of_serialized_size = |size: usize| {
        ScopeReport::files_scope(MERGE_BASE.to_owned(), vec!["x".repeat(size - overhead)])
    };

    let last_valid = of_serialized_size(SCOPE_OUTPUT_LIMIT - 1).unwrap();
    assert_eq!(
        serde_json::to_vec(&last_valid).unwrap().len(),
        SCOPE_OUTPUT_LIMIT - 1
    );
    assert_eq!(
        of_serialized_size(SCOPE_OUTPUT_LIMIT).unwrap_err().code,
        "git-output-too-large"
    );
}

/// A diff inside the transport bound can still normalize past the report bound,
/// which is why the report is measured after normalization and not before.
#[test]
fn normalized_scope_cannot_expand_beyond_the_report_limit() {
    let mut diff = Vec::new();
    for index in 0..FILE_LIMIT {
        let prefix = format!("{index:04}-");
        let path = format!("{prefix}{}", "%".repeat(PATH_LIMIT - prefix.len()));
        if diff.len() + path.len() + 1 > DIFF_OUTPUT_LIMIT {
            break;
        }
        diff.extend_from_slice(path.as_bytes());
        diff.push(0);
    }
    assert!(diff.len() <= DIFF_OUTPUT_LIMIT);

    let responses = RefCell::new(VecDeque::from([
        format!("{BASE}\n").into_bytes(),
        format!("{MERGE_BASE}\n").into_bytes(),
        diff,
    ]));
    let error = resolve_with(&validated_files("main"), Path::new("/workspace"), |_| {
        output(responses.borrow_mut().pop_front().unwrap())
    })
    .unwrap_err();

    assert_eq!((error.stage, error.code), ("scope", "git-output-too-large"));
}

/// Every file of the module stays under the bound `oversized_unit` reports at,
/// tests included: the pass that scopes the scan has to pass the rule it
/// raises.
#[test]
fn the_scope_holds_the_size_bound_it_reports_for() {
    for own in [include_str!("../git_scope.rs"), include_str!("tests.rs")] {
        let lines = own.lines().count();
        assert!(
            lines < crate::structure::FILE_LINES,
            "a file of the scope is {lines} lines long, over the {} it publishes",
            crate::structure::FILE_LINES
        );
    }
}
