use std::cell::RefCell;
use std::collections::VecDeque;
use std::ffi::{OsStr, OsString};

use super::*;

const BASE: &str = "1111111111111111111111111111111111111111";
const MERGE_BASE: &str = "2222222222222222222222222222222222222222";

fn changed(mode: ChangeMode, base: Option<&str>, options: ChangeOptions) -> ScopeRequest {
    ScopeRequest::Changed {
        mode,
        base: base.map(str::to_owned),
        options,
    }
}

fn files(base: &str) -> ScopeRequest {
    changed(ChangeMode::Files, Some(base), ChangeOptions::default())
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
    for rendered in [format!("{scope:?}"), format!("{:?}", files("release/1.2.3"))] {
        assert!(!rendered.contains("release"), "{rendered}");
        assert!(rendered.contains("<redacted>"), "{rendered}");
    }
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
        b"src/new.rs\0".to_vec(),
    ]));
    let calls = RefCell::new(Vec::new());
    let scope = resolve_with(&validated_files("main"), Path::new("/workspace"), |call| {
        calls.borrow_mut().push(call.arguments.clone());
        assert_eq!(call.stage, "scope");
        output(responses.borrow_mut().pop_front().unwrap())
    })
    .unwrap();

    assert_eq!(calls.borrow().len(), 4);
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
    // The fourth call counts the untracked Rust files the scope left out.
    assert_eq!(calls.borrow()[3][7], OsStr::new("ls-files"));
    assert_eq!(calls.borrow()[3].last().unwrap(), OsStr::new("*.rs"));
    assert_eq!(scope.untracked_unreported(), Some(1));
    // A base the caller named is never published.
    assert_eq!(scope.base_ref(), None);
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
        &changed(ChangeMode::Baseline, Some("main"), ChangeOptions::default())
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
        // A merge base that failed asks once more, whether the clone is
        // shallow; nothing else runs after a failure.
        let probes = usize::from(expected == "merge-base-unavailable");
        assert_eq!(*calls.borrow(), failing_call + 1 + probes);
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
            output(responses.borrow_mut().pop_front().unwrap_or_default())
        })
        .unwrap_err();
        assert_eq!(error.code, expected);
        // The shallow probe answers nothing here, so the code stands.
        assert_eq!(*calls.borrow(), 2 + usize::from(expected == "merge-base-unavailable"));
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

/// Answers each call by the git operation it runs, recording the operations.
fn scripted<'a>(
    calls: &'a RefCell<Vec<Vec<String>>>,
    answer: impl Fn(&[String]) -> Result<Vec<u8>, ()> + 'a,
) -> impl FnMut(&GitCall) -> Result<Vec<u8>, InternalError> + 'a {
    move |call| {
        let operation: Vec<String> = call.arguments[7..]
            .iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect();
        calls.borrow_mut().push(operation.clone());
        answer(&operation).map_err(|()| call.failure.error(call.stage))
    }
}

fn lines_scope(base: Option<&str>, options: ChangeOptions) -> ValidatedScope {
    changed(ChangeMode::Lines, base, options).validate().unwrap()
}

/// Without `--base`, the first candidate git resolves is the base, and the
/// report names it.
#[test]
fn a_missing_base_is_the_first_default_branch_that_resolves() {
    for (remote_head, resolvable, expected) in [
        (Some("refs/remotes/origin/trunk"), "origin/trunk", "origin/trunk"),
        (None, "origin/master", "origin/master"),
        (None, "master", "master"),
        (Some("refs/remotes/origin/gone"), "main", "main"),
    ] {
        let calls = RefCell::new(Vec::new());
        let scope = resolve_with(
            &lines_scope(None, ChangeOptions::default()),
            Path::new("/workspace"),
            scripted(&calls, |operation| match operation[0].as_str() {
                "symbolic-ref" if operation.last().is_some_and(|last| last == "HEAD") => {
                    Ok(b"feature\n".to_vec())
                }
                "symbolic-ref" => remote_head.map(|head| format!("{head}\n").into_bytes()).ok_or(()),
                "rev-parse" if operation.last().is_some_and(|last| last.starts_with(resolvable)) => {
                    Ok(format!("{BASE}\n").into_bytes())
                }
                "rev-parse" if operation.last().is_some_and(|last| last == "HEAD^{commit}") => {
                    Ok(format!("{BASE}\n").into_bytes())
                }
                "rev-parse" => Err(()),
                "merge-base" => Ok(format!("{MERGE_BASE}\n").into_bytes()),
                _ => Ok(Vec::new()),
            }),
        )
        .unwrap();
        assert_eq!(scope.base_ref(), Some(expected), "{remote_head:?}");
        assert_eq!(scope.mode(), ScopeMode::Lines);
    }
}

/// On the branch the base names, the work left to judge is what has not been
/// committed, so the base becomes `HEAD`.
#[test]
fn on_the_default_branch_the_base_is_head() {
    let calls = RefCell::new(Vec::new());
    let scope = resolve_with(
        &lines_scope(None, ChangeOptions::default()),
        Path::new("/workspace"),
        scripted(&calls, |operation| match operation[0].as_str() {
            "symbolic-ref" if operation.last().is_some_and(|last| last == "HEAD") => {
                Ok(b"master\n".to_vec())
            }
            "symbolic-ref" => Err(()),
            "rev-parse" if operation.last().is_some_and(|last| last.starts_with("origin/")) => Err(()),
            "rev-parse" if operation.last().is_some_and(|last| last.starts_with("main")) => Err(()),
            "rev-parse" => Ok(format!("{BASE}\n").into_bytes()),
            "merge-base" => Ok(format!("{BASE}\n").into_bytes()),
            _ => Ok(Vec::new()),
        }),
    )
    .unwrap();
    assert_eq!(scope.base_ref(), Some("HEAD"));
    assert!(
        calls
            .borrow()
            .iter()
            .any(|operation| operation.last().is_some_and(|last| last == "HEAD^{commit}"))
    );
}

#[test]
fn no_resolvable_candidate_fails_with_base_undetected() {
    let calls = RefCell::new(Vec::new());
    let error = resolve_with(
        &lines_scope(None, ChangeOptions::default()),
        Path::new("/workspace"),
        scripted(&calls, |_| Err(())),
    )
    .unwrap_err();
    assert_eq!((error.stage, error.code), ("scope", "base-undetected"));
    assert!(error.message.contains("--base"), "{}", error.message);
    // The four fallbacks plus origin/HEAD, and no diff.
    assert!(calls.borrow().iter().all(|operation| operation[0] != "diff"));
}

#[test]
fn a_merge_base_missing_from_a_shallow_clone_says_so() {
    for (shallow, expected) in [("true", "shallow-clone"), ("false", "merge-base-unavailable")] {
        let calls = RefCell::new(Vec::new());
        let error = resolve_with(
            &changed(ChangeMode::Baseline, Some("main"), ChangeOptions::default())
                .validate()
                .unwrap(),
            Path::new("/workspace"),
            scripted(&calls, |operation| match operation[0].as_str() {
                "rev-parse" if operation[1] == "--is-shallow-repository" => {
                    Ok(format!("{shallow}\n").into_bytes())
                }
                "rev-parse" => Ok(format!("{BASE}\n").into_bytes()),
                _ => Err(()),
            }),
        )
        .unwrap_err();
        assert_eq!((error.stage, error.code), ("scope", expected));
        if expected == "shallow-clone" {
            assert!(error.message.contains("fetch-depth: 0"), "{}", error.message);
        }
    }
}

/// A staged scan without `--base` compares the index with `HEAD`, through
/// `diff --cached`, and never asks which branch is the default.
#[test]
fn a_staged_scope_diffs_the_index_against_head() {
    let calls = RefCell::new(Vec::new());
    let staged = ChangeOptions {
        staged: true,
        include_untracked: false,
    };
    let scope = resolve_with(
        &lines_scope(None, staged),
        Path::new("/workspace"),
        scripted(&calls, |operation| match operation[0].as_str() {
            "rev-parse" | "merge-base" => Ok(format!("{BASE}\n").into_bytes()),
            "diff" => Ok(b"+++ b/src/lib.rs\n@@ -1 +1,2 @@\n-a\n+b\n+c\n".to_vec()),
            _ => Err(()),
        }),
    )
    .unwrap();
    assert!(scope.staged());
    assert_eq!(scope.base_ref(), Some("HEAD"));
    assert_eq!(scope.untracked_unreported(), None);
    let calls = calls.borrow();
    assert!(calls.iter().all(|operation| operation[0] != "symbolic-ref"));
    let diff = calls.iter().find(|operation| operation[0] == "diff").unwrap();
    assert_eq!(diff[1..3], ["--no-ext-diff", "--cached"]);
    assert!(scope.includes_span(Some("src/lib.rs"), Some((2, 2))));
    assert!(!scope.includes_span(Some("src/lib.rs"), Some((3, 3))));
}

/// A change on lines 10 to 12 leaves a finding on line 40 out of a lines
/// scope; an untracked file included is changed whole; a finding with no span
/// or no path is out.
#[test]
fn a_lines_scope_keeps_a_finding_only_where_its_span_meets_a_change() {
    let calls = RefCell::new(Vec::new());
    let untracked = ChangeOptions {
        staged: false,
        include_untracked: true,
    };
    let scope = resolve_with(
        &lines_scope(Some("main"), untracked),
        Path::new("/workspace"),
        scripted(&calls, |operation| match operation[0].as_str() {
            "rev-parse" => Ok(format!("{BASE}\n").into_bytes()),
            "merge-base" => Ok(format!("{MERGE_BASE}\n").into_bytes()),
            "diff" => Ok(b"+++ b/src/lib.rs\n@@ -10,3 +10,3 @@\n-a\n-b\n-c\n+A\n+B\n+C\n+++ b/src/gone.rs\n@@ -4 +3,0 @@\n-d\n".to_vec()),
            "ls-files" => Ok(b"src/new.rs\0".to_vec()),
            _ => Err(()),
        }),
    )
    .unwrap();
    assert_eq!(
        scope.files(),
        Some(&["src/lib.rs".to_owned(), "src/new.rs".to_owned()][..])
    );
    assert!(!scope.includes_span(Some("src/lib.rs"), Some((40, 40))));
    assert!(scope.includes_span(Some("src/lib.rs"), Some((12, 20))));
    assert!(scope.includes_span(Some("src/lib.rs"), Some((1, 10))));
    assert!(!scope.includes_span(Some("src/lib.rs"), None));
    assert!(!scope.includes_span(None, Some((10, 10))));
    assert!(scope.includes_span(Some("src/new.rs"), Some((400, 400))));
    assert!(!scope.includes_span(Some("src/gone.rs"), Some((3, 3))));
    assert_eq!(scope.untracked_unreported(), None);
}

#[test]
fn a_lines_diff_past_its_bound_is_diff_too_large() {
    let calls = RefCell::new(Vec::new());
    let error = resolve_with(
        &lines_scope(Some("main"), ChangeOptions::default()),
        Path::new("/workspace"),
        move |call| {
            calls.borrow_mut().push(());
            match call.arguments[7].to_str() {
                Some("diff") => Err(call.overflow.error(call.stage)),
                _ => output(format!("{BASE}\n")),
            }
        },
    )
    .unwrap_err();
    assert_eq!((error.stage, error.code), ("scope", "diff-too-large"));
    assert!(error.message.contains("--scope files"), "{}", error.message);
}

#[test]
fn untracked_files_join_only_a_working_tree_files_or_lines_scope() {
    for (mode, staged) in [(ChangeMode::Baseline, false), (ChangeMode::Lines, true)] {
        let options = ChangeOptions {
            staged,
            include_untracked: true,
        };
        let error = changed(mode, None, options).validate().unwrap_err();
        assert_eq!((error.stage, error.code), ("scope", "untracked-unsupported"));
    }
}

/// Every file of the module stays under the bound `oversized_unit` reports at,
/// tests included: the pass that scopes the scan has to pass the rule it
/// raises.
#[test]
fn the_scope_holds_the_size_bound_it_reports_for() {
    for own in [
        include_str!("../git_scope.rs"),
        include_str!("base.rs"),
        include_str!("lines.rs"),
        include_str!("tests.rs"),
    ] {
        let lines = own.lines().count();
        assert!(
            lines < crate::structure::FILE_LINES,
            "a file of the scope is {lines} lines long, over the {} it publishes",
            crate::structure::FILE_LINES
        );
    }
}
