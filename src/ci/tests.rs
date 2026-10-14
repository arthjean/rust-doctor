//! The workflows as a runner would run them: every `run` block is checked by
//! `bash -n`, then executed with the tools it calls replaced by stubs, and every
//! `rust-doctor` invocation it made is parsed with the binary's own clap
//! definition. A command line the CLI refuses is a CI gate that dies on its
//! first run, and reading the template for it would miss the arguments the
//! shell assembles.

use super::*;
use crate::test_scratch::scratch;
use clap::Parser as _;
use std::process::Command;

/// The body of every `run` step, dedented, in order.
pub(crate) fn run_blocks(yaml: &str) -> Vec<String> {
    let lines: Vec<&str> = yaml.lines().collect();
    let mut blocks = Vec::new();
    let mut index = 0;
    while let Some(line) = lines.get(index) {
        index += 1;
        let trimmed = line.trim_start();
        let Some(rest) = trimmed
            .strip_prefix("run: ")
            .or_else(|| trimmed.strip_prefix("- run: "))
        else {
            continue;
        };
        if rest.trim() != "|" {
            blocks.push(rest.trim().to_owned());
            continue;
        }
        let indent = line.len() - trimmed.len();
        let mut body = Vec::new();
        while let Some(next) = lines.get(index) {
            let depth = next.len() - next.trim_start().len();
            if !next.trim().is_empty() && depth <= indent {
                break;
            }
            body.push(next.get(indent + 2..).unwrap_or("").to_owned());
            index += 1;
        }
        blocks.push(body.join("\n"));
    }
    blocks
}

/// What a run block did under stubs.
pub(crate) struct Execution {
    invocations: Vec<Vec<String>>,
    status: Option<i32>,
    outputs: String,
    stdout: String,
    gh: String,
    git: String,
}

/// Runs `block` under bash with every external tool stubbed, and answers with
/// each `rust-doctor` command line it ran, asserting the block succeeded.
pub(crate) fn execute(block: &str, environment: &[(&str, &str)]) -> Vec<Vec<String>> {
    let execution = run_stubbed(block, environment);
    assert_eq!(execution.status, Some(0), "a run block failed under stubs:\n{block}");
    execution.invocations
}

/// Runs `block` under bash with every external tool stubbed. A scan the stub
/// runs exits with `SCAN_STATUS`, 0 unless the environment sets it; `jq`
/// answers `JQ_1` then `JQ_2`; `gh` logs its arguments and, under
/// `GH_REFUSES`, refuses every write as a fork's read-only token would.
/// `NO_REPORT` leaves the report and its rendered summary out, and
/// `EMPTY_REPORT` leaves both empty. `git` logs its arguments, answers
/// `SHALLOW` (default `false`) to `--is-shallow-repository`, and fails a
/// fetch under `FETCH_FAILS`.
pub(crate) fn run_stubbed(block: &str, environment: &[(&str, &str)]) -> Execution {
    let root = scratch("ci-tests", "execute");
    let log = root.join("argv");
    fs::create_dir_all(root.join("rust-doctor")).unwrap();
    if !environment.iter().any(|(name, _)| *name == "NO_REPORT") {
        let empty = environment.iter().any(|(name, _)| *name == "EMPTY_REPORT");
        fs::write(root.join("rust-doctor/report.json"), if empty { "" } else { "{}" }).unwrap();
        let summary = if empty { "" } else { "## rust-doctor\n" };
        fs::write(root.join("rust-doctor/summary.md"), summary).unwrap();
    }
    let prelude = r#"
rust-doctor() {
  printf '%s\037' rust-doctor "$@" >> "$ARGV_LOG"; printf '\n' >> "$ARGV_LOG"
  if [ "${1:-}" = report ]; then echo '## rust-doctor'; return 0; fi
  return "${SCAN_STATUS:-0}"
}
git() {
  echo "$*" >> "$GH_LOG.git"
  case " $* " in
    *" --is-shallow-repository "*) echo "${SHALLOW:-false}" ;;
    *" fetch "*) [ -z "${FETCH_FAILS:-}" ] ;;
  esac
}
npm() { :; }
gh() {
  echo "$*" >> "$GH_LOG"
  if [ "${1:-}" = api ] && [ "${2:-}" = -X ] && [ -n "${GH_REFUSES:-}" ]; then return 1; fi
  echo '[]'
}
jq() {
  cat > /dev/null
  # Each call runs in a command substitution, so the count lives in a file.
  echo x >> "$GH_LOG.jq"
  if [ "$(wc -l < "$GH_LOG.jq")" -eq 1 ]; then echo "${JQ_1-7}"; else echo "${JQ_2-9}"; fi
}
"#;
    let syntax = Command::new("bash")
        .args(["-n", "-c", block])
        .output()
        .unwrap();
    assert!(
        syntax.status.success(),
        "bash -n refused a run block: {}\n{block}",
        String::from_utf8_lossy(&syntax.stderr)
    );
    // The stubs and the block reach bash as data, and the script it runs is a
    // constant: the command line is never assembled from text. The options
    // are the ones GitHub gives `shell: bash`, so a command that fails
    // unguarded ends the step here as it does on a runner.
    let output = Command::new("bash")
        .args(["--noprofile", "--norc", "-eo", "pipefail", "-c", "eval \"$STUBS\"\neval \"$BLOCK\""])
        .env("STUBS", prelude)
        .env("BLOCK", block)
        .current_dir(&root)
        .env("ARGV_LOG", &log)
        .env("GH_LOG", root.join("gh"))
        .env("RUNNER_TEMP", &root)
        .env("GITHUB_STEP_SUMMARY", root.join("summary"))
        .env("GITHUB_OUTPUT", root.join("output"))
        .envs(environment.iter().copied())
        .output()
        .unwrap();
    let argv = fs::read_to_string(&log).unwrap_or_default();
    let outputs = fs::read_to_string(root.join("output")).unwrap_or_default();
    let gh = fs::read_to_string(root.join("gh")).unwrap_or_default();
    let git = fs::read_to_string(root.join("gh.git")).unwrap_or_default();
    fs::remove_dir_all(root).unwrap();
    let invocations = argv
        .lines()
        .map(|line| {
            line.split('\u{1f}')
                .filter(|argument| !argument.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .collect();
    Execution {
        invocations,
        status: output.status.code(),
        outputs,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        gh,
        git,
    }
}

pub(crate) fn assert_parses(invocations: &[Vec<String>]) {
    for argv in invocations {
        let parsed = crate::Cli::try_parse_from(argv);
        assert!(parsed.is_ok(), "the CLI refuses {argv:?}: {:?}", parsed.err());
    }
}

fn workflow_options(blocking: Option<BlockingLevel>) -> WorkflowOptions {
    WorkflowOptions {
        workspace: ".".to_owned(),
        blocking,
        branch: "trunk".to_owned(),
        toolchain: VALIDATED_TOOLCHAIN.to_owned(),
        comment: false,
    }
}

#[test]
fn every_command_the_scan_workflow_runs_parses_against_the_cli() {
    for blocking in [None, Some(BlockingLevel::Warning)] {
        let workflow = scan_workflow(&workflow_options(blocking));
        let blocks = run_blocks(&workflow);
        assert_eq!(blocks.len(), 2, "the install step and the scan step");
        let mut pull_request = Vec::new();
        let mut push = Vec::new();
        for block in &blocks {
            pull_request.extend(execute(block, &[("BASE_REF", "main")]));
            push.extend(execute(block, &[("BASE_REF", "")]));
        }
        assert_parses(&pull_request);
        assert_parses(&push);

        let scan = |invocations: &[Vec<String>]| {
            invocations
                .iter()
                .find(|argv| argv.get(2).map(String::as_str) == Some("--yes"))
                .cloned()
                .unwrap()
                .join(" ")
        };
        let pull_request_scan = scan(&pull_request);
        assert!(
            pull_request_scan.starts_with("rust-doctor . --yes --json --scope baseline --base origin/main"),
            "{pull_request_scan}"
        );
        assert_eq!(
            pull_request_scan.contains("--blocking warning"),
            blocking.is_some(),
            "{pull_request_scan}"
        );
        assert_eq!(
            scan(&push),
            "rust-doctor . --yes --json --scope full --blocking none"
        );
        assert!(
            pull_request
                .iter()
                .any(|argv| argv.get(1..3) == Some(&["report".to_owned(), "markdown".to_owned()][..])),
            "the summary is rendered by `report markdown`"
        );
    }
}

#[test]
fn the_scan_workflow_runs_read_only_and_never_interpolates_into_a_shell() {
    let workflow = scan_workflow(&workflow_options(None));
    assert!(workflow.starts_with(MARKER));
    assert!(workflow.contains("permissions:\n  contents: read\n\njobs:"));
    assert!(!workflow.contains("pull-requests") && !workflow.contains("<!-- rust-doctor -->"));
    assert!(workflow.contains("persist-credentials: false"));
    assert!(workflow.contains("branches: ['trunk']"));
    assert!(workflow.contains(&format!("toolchain: '{VALIDATED_TOOLCHAIN}'")));
    assert!(workflow.contains(&format!(
        "npm install -g rust-doctor@{}",
        env!("CARGO_PKG_VERSION")
    )));
    assert!(workflow.contains("BASE_REF: ${{ github.base_ref }}"));
    assert!(workflow.contains("name: rust-doctor-report"));
    for placeholder in ["{MARKER}", "{BRANCH}", "{TOOLCHAIN}", "{VERSION}", "{BLOCKING}"] {
        assert!(!workflow.contains(placeholder), "{placeholder} survived");
    }
    for block in run_blocks(&workflow) {
        assert!(!block.contains("${{"), "a run block interpolates: {block}");
    }
}

/// `--comment` grants the one write permission and adds the step, and the
/// Action runs the same script byte for byte.
#[test]
fn the_comment_is_opt_in_and_the_action_runs_the_same_script() {
    let workflow = scan_workflow(&WorkflowOptions {
        comment: true,
        ..workflow_options(None)
    });
    assert!(workflow.contains("permissions:\n  contents: read\n  pull-requests: write\n\njobs:"));
    assert_eq!(workflow.matches("pull-requests: write").count(), 1);
    assert!(workflow.contains("if: always() && github.event_name == 'pull_request'"));
    assert!(!workflow.contains("pull_request_target"));
    let blocks = run_blocks(&workflow);
    assert_eq!(blocks.len(), 3, "install, scan, comment");
    assert_eq!(blocks.last().map(String::as_str), Some(COMMENT_SCRIPT.trim_end()));
    for block in &blocks {
        assert!(!block.contains("${{"), "a run block interpolates: {block}");
    }
    let action_comment = run_blocks(ACTION)
        .into_iter()
        .find(|block| block.contains("<!-- rust-doctor -->"));
    assert_eq!(action_comment.as_deref().map(str::trim_end), Some(COMMENT_SCRIPT.trim_end()));
}

#[test]
fn a_branch_or_toolchain_that_could_escape_its_scalar_is_refused() {
    let root = Path::new(".");
    for branch in ["", "-x", "main'", "a b", "a\nb", "a:b", "a\"b"] {
        assert_eq!(
            options_for(root, branch, VALIDATED_TOOLCHAIN),
            Err(CiError::InvalidBranch),
            "{branch:?}"
        );
    }
    for toolchain in ["", "1.97.1'", "stable\n", "$(id)"] {
        assert_eq!(
            options_for(root, "main", toolchain),
            Err(CiError::InvalidToolchain),
            "{toolchain:?}"
        );
    }
    assert!(options_for(root, "release/1.x", "nightly-2026-01-01").is_ok());
}

fn options_for(root: &Path, branch: &str, toolchain: &str) -> Result<WorkflowOptions, CiError> {
    options(root, None, Some(branch.to_owned()), toolchain.to_owned(), false)
}

const ACTION: &str = include_str!("../../action.yml");

/// The default an input of `action.yml` declares, unquoted.
fn input_default(name: &str) -> Option<String> {
    let mut lines = ACTION
        .lines()
        .skip_while(|line| *line != format!("  {name}:"))
        .skip(1)
        .take_while(|line| line.starts_with("    "));
    lines
        .find_map(|line| line.trim().strip_prefix("default: "))
        .map(|value| value.trim_matches('"').to_owned())
}

#[test]
fn the_action_declares_its_inputs_and_defaults_to_this_release() {
    for input in [
        "version",
        "toolchain",
        "scope",
        "base",
        "blocking",
        "warning-exit-code",
        "working-directory",
        "args",
        "comment",
    ] {
        assert!(input_default(input).is_some(), "action.yml declares no default for `{input}`");
    }
    assert_eq!(input_default("version").as_deref(), Some(env!("CARGO_PKG_VERSION")));
    assert_eq!(input_default("toolchain").as_deref(), Some(VALIDATED_TOOLCHAIN));
    assert_eq!(input_default("comment").as_deref(), Some("true"));
    for output in ["score", "authoritative", "introduced", "fixed", "exit-code"] {
        assert!(ACTION.contains(&format!("\n  {output}:\n")), "action.yml has no `{output}` output");
    }
    assert!(!ACTION.contains("pull_request_target"));
    for block in run_blocks(ACTION) {
        assert!(!block.contains("${{"), "a run block of action.yml interpolates: {block}");
    }
}

#[test]
fn every_command_the_action_runs_parses_against_the_cli() {
    let blocks = run_blocks(ACTION);
    assert_eq!(blocks.len(), 6, "install, scan, summary, comment, outputs, exit");
    let cases: [(&[(&str, &str)], &str); 5] = [
        (
            &[("PULL_REQUEST_BASE", "main")],
            "rust-doctor . --yes --json --scope baseline --base origin/main",
        ),
        (&[], "rust-doctor . --yes --json --scope full"),
        (
            &[("WARNING_EXIT_CODE", "78")],
            "rust-doctor . --yes --json --scope full --warning-exit-code 78",
        ),
        (
            &[("SCOPE", "files"), ("BASE", "v1.0"), ("BLOCKING", "warning")],
            "rust-doctor . --yes --json --scope files --base v1.0 --blocking warning",
        ),
        (
            &[("ARGS", "--package core --max-duration 600"), ("WORKSPACE", "crates/core")],
            "rust-doctor crates/core --yes --json --scope full --package core --max-duration 600",
        ),
    ];
    for (inputs, expected) in cases {
        let mut environment = vec![
            ("SCOPE", ""),
            ("BASE", ""),
            ("BLOCKING", ""),
            ("ARGS", ""),
            ("WORKSPACE", "."),
            ("PULL_REQUEST_BASE", ""),
            ("VERSION", "0.9.0"),
            ("REPORT", "report.json"),
            ("EXIT_CODE", "0"),
            ("REPOSITORY", "o/r"),
            ("PULL_REQUEST", "7"),
        ];
        environment.retain(|(name, _)| !inputs.iter().any(|(input, _)| input == name));
        environment.extend_from_slice(inputs);
        let invocations: Vec<Vec<String>> = blocks
            .iter()
            .flat_map(|block| execute(block, &environment))
            .collect();
        assert_parses(&invocations);
        let scan = invocations
            .iter()
            .find(|argv| argv.get(2).map(String::as_str) == Some("--yes"))
            .map(|argv| argv.join(" "));
        assert_eq!(scan.as_deref(), Some(expected), "{inputs:?}");
        assert!(
            invocations.iter().any(|argv| argv.get(1).map(String::as_str) == Some("report")),
            "the job summary is rendered by `report markdown`"
        );
    }
}

/// This repository scans itself through the Action from its own checkout,
/// with the permission its comment needs.
#[test]
fn this_repository_runs_the_action_it_ships() {
    let dogfood = include_str!("../../.github/workflows/dogfood.yml");
    assert!(dogfood.starts_with("name: Rust Doctor\n"));
    assert!(dogfood.contains("uses: ./\n"));
    assert!(dogfood.contains("pull_request:"));
    assert!(dogfood.contains("  pull-requests: write\n"));
}

/// A scan that exits 2 still leaves an exit code for the summary, the upload
/// and the outputs to run after, each of which runs `if: always()`, and the
/// last step fails with that code.
#[test]
fn a_failed_scan_still_publishes_and_then_fails_the_step() {
    let blocks = run_blocks(ACTION);
    let scan = blocks
        .iter()
        .find(|block| block.contains("rust-doctor \"${arguments[@]}\""))
        .unwrap();
    let environment = [
        ("SCOPE", ""),
        ("BASE", ""),
        ("BLOCKING", ""),
        ("ARGS", ""),
        ("WORKSPACE", "."),
        ("PULL_REQUEST_BASE", ""),
        ("SCAN_STATUS", "2"),
    ];
    let execution = run_stubbed(scan, &environment);
    assert_eq!(execution.status, Some(0), "the scan step keeps the code rather than failing");
    assert!(execution.outputs.contains("exit-code=2"), "{}", execution.outputs);
    let exit = blocks.last().unwrap();
    assert_eq!(run_stubbed(exit, &[("EXIT_CODE", "2")]).status, Some(2));
    let after_scan = ACTION.split("id: scan").nth(1).unwrap();
    assert_eq!(
        after_scan.matches("if: always()").count(),
        5,
        "summary, upload, comment, outputs and exit run after a failed scan"
    );
}

/// The comment step as a second push, a first push, a fork's read-only token,
/// a scan that rendered nothing or an empty summary.
#[test]
fn the_comment_is_edited_in_place_and_never_fails_the_job() {
    let pull_request = [("REPOSITORY", "o/r"), ("PULL_REQUEST", "7")];
    let with = |extra: &[(&'static str, &'static str)]| {
        let mut environment = pull_request.to_vec();
        environment.extend_from_slice(extra);
        run_stubbed(COMMENT_SCRIPT, &environment)
    };

    let edited = with(&[("JQ_1", "9")]);
    assert_eq!(edited.status, Some(0));
    assert!(edited.gh.contains("api --paginate repos/o/r/issues/7/comments"), "{}", edited.gh);
    assert!(edited.gh.contains("-X PATCH repos/o/r/issues/comments/9 -f body=<!-- rust-doctor -->\n\n## rust-doctor"), "{}", edited.gh);
    assert!(!edited.gh.contains("-X POST"));

    let created = with(&[("JQ_1", "")]);
    assert_eq!(created.status, Some(0));
    assert!(created.gh.contains("-X POST repos/o/r/issues/7/comments -f body=<!-- rust-doctor -->"), "{}", created.gh);

    let fork = with(&[("JQ_1", ""), ("GH_REFUSES", "1")]);
    assert_eq!(fork.status, Some(0), "a read-only token never fails the job");
    assert!(fork.gh.contains("-X POST"), "{}", fork.gh);
    assert!(fork.stdout.contains("::warning::"), "{}", fork.stdout);

    for absent in ["NO_REPORT", "EMPTY_REPORT"] {
        let skipped = with(&[(absent, "1")]);
        assert_eq!(skipped.status, Some(0), "{absent}");
        assert!(skipped.gh.is_empty(), "nothing is posted without a summary: {}", skipped.gh);
        assert!(skipped.stdout.contains("::notice::"), "{}", skipped.stdout);
    }
}

/// The scan step of the Action, found by the command it runs.
fn action_scan() -> String {
    run_blocks(ACTION)
        .into_iter()
        .find(|block| block.contains("rust-doctor \"${arguments[@]}\""))
        .unwrap()
}

const SCAN_INPUTS: [(&str, &str); 6] = [
    ("SCOPE", ""),
    ("BASE", ""),
    ("BLOCKING", ""),
    ("WARNING_EXIT_CODE", ""),
    ("ARGS", ""),
    ("WORKSPACE", "."),
];

/// A gate failure (1) fails only a pull request; a scan that could not run
/// (2) fails every event; in the Action and in the generated workflow alike.
#[test]
fn only_a_pull_request_fails_on_the_gate() {
    let exit = run_blocks(ACTION).pop().unwrap();
    for (code, event, expected) in [
        ("1", "push", 0),
        ("1", "pull_request", 1),
        ("2", "push", 2),
        ("2", "pull_request", 2),
        ("0", "push", 0),
    ] {
        let execution = run_stubbed(&exit, &[("EXIT_CODE", code), ("EVENT_NAME", event)]);
        assert_eq!(execution.status, Some(expected), "action: {code} on {event}");
    }
    // The warning code a caller chose fails a pull request and nothing else.
    for (event, expected) in [("push", 0), ("pull_request", 78)] {
        let environment = [("EXIT_CODE", "78"), ("EVENT_NAME", event), ("WARNING_EXIT_CODE", "78")];
        let execution = run_stubbed(&exit, &environment);
        assert_eq!(execution.status, Some(expected), "action: warning code on {event}");
    }
    let workflow = scan_workflow(&workflow_options(None));
    let scan = run_blocks(&workflow).pop().unwrap();
    for (code, event, base, expected) in [
        ("1", "push", "", 0),
        ("1", "pull_request", "main", 1),
        ("2", "push", "", 2),
        ("2", "pull_request", "main", 2),
    ] {
        let environment = [("SCAN_STATUS", code), ("EVENT_NAME", event), ("BASE_REF", base)];
        let execution = run_stubbed(&scan, &environment);
        assert_eq!(execution.status, Some(expected), "workflow: {code} on {event}");
    }
    // The summary of a push is rendered all the same: no event guards it.
    let summarize = ACTION.split("- name: Summarize").nth(1).unwrap();
    let header = summarize.split("run: |").next().unwrap();
    assert!(header.contains("if: always()") && !header.contains("event_name"), "{header}");
}

/// A pull request scanned from a depth-1 checkout fetches its base once, and a
/// fetch that fails never fails the step. A full clone, or a full scope,
/// fetches nothing.
#[test]
fn a_shallow_pull_request_checkout_fetches_its_base_once() {
    let scan = action_scan();
    let with = |extra: &[(&'static str, &'static str)]| {
        let mut environment = SCAN_INPUTS.to_vec();
        environment.push(("PULL_REQUEST_BASE", "main"));
        environment.extend_from_slice(extra);
        run_stubbed(&scan, &environment)
    };
    let fetch = "-C . fetch --no-tags --unshallow origin +refs/heads/main:refs/remotes/origin/main\n";

    let shallow = with(&[("SHALLOW", "true")]);
    assert_eq!(shallow.status, Some(0));
    assert_eq!(shallow.git.matches(" fetch ").count(), 1, "{}", shallow.git);
    assert!(shallow.git.ends_with(fetch), "{}", shallow.git);

    let refused = with(&[("SHALLOW", "true"), ("FETCH_FAILS", "1")]);
    assert_eq!(refused.status, Some(0), "a failed fetch never fails the step");
    assert!(refused.git.contains(" fetch "), "{}", refused.git);
    assert!(refused.outputs.contains("exit-code=0"), "{}", refused.outputs);

    let full_clone = with(&[("SHALLOW", "false")]);
    assert!(!full_clone.git.contains(" fetch "), "{}", full_clone.git);

    let full_scope = with(&[("SHALLOW", "true"), ("SCOPE", "full")]);
    assert!(full_scope.git.is_empty(), "{}", full_scope.git);
}

/// With `comment-on-clean: false`, a clean pull request gets no new comment,
/// one already posted is still edited, and a pull request with findings is
/// commented as before.
#[test]
fn a_clean_pull_request_stays_quiet_when_asked() {
    let with = |extra: &[(&'static str, &'static str)]| {
        let mut environment = vec![("REPOSITORY", "o/r"), ("PULL_REQUEST", "7")];
        environment.extend_from_slice(extra);
        run_stubbed(COMMENT_SCRIPT, &environment)
    };
    let quiet = with(&[("CLEAN", "true"), ("COMMENT_ON_CLEAN", "false"), ("JQ_1", "")]);
    assert_eq!(quiet.status, Some(0));
    assert!(!quiet.gh.contains("-X"), "{}", quiet.gh);
    assert!(quiet.stdout.contains("comment-on-clean is false"), "{}", quiet.stdout);

    let updated = with(&[("CLEAN", "true"), ("COMMENT_ON_CLEAN", "false"), ("JQ_1", "9")]);
    assert!(updated.gh.contains("-X PATCH repos/o/r/issues/comments/9"), "{}", updated.gh);

    for extra in [
        &[("CLEAN", "false"), ("COMMENT_ON_CLEAN", "false"), ("JQ_1", "")][..],
        &[("CLEAN", "true"), ("COMMENT_ON_CLEAN", "true"), ("JQ_1", "")][..],
    ] {
        let posted = with(extra);
        assert!(posted.gh.contains("-X POST repos/o/r/issues/7/comments"), "{extra:?}: {}", posted.gh);
    }
    assert_eq!(input_default("comment-on-clean").as_deref(), Some("true"));
    assert_eq!(input_default("cache").as_deref(), Some("true"));
    assert!(ACTION.contains("CLEAN: ${{ steps.summarize.outputs.clean }}"));
}

/// Every action the shipped YAML uses is pinned to a full commit, with the
/// ref it was read from beside it, and every rust-cache step saves on
/// failure.
#[test]
fn every_shipped_action_is_pinned_to_a_commit() {
    let workflow = scan_workflow(&WorkflowOptions {
        comment: true,
        ..workflow_options(None)
    });
    for (name, yaml) in [("action.yml", ACTION), ("the generated workflow", workflow.as_str())] {
        let uses: Vec<&str> = yaml
            .lines()
            .filter_map(|line| line.trim_start().trim_start_matches("- ").strip_prefix("uses: "))
            .filter(|target| !target.starts_with("./"))
            .collect();
        assert!(uses.len() >= 3, "{name}: {uses:?}");
        for target in uses {
            let pinned = target.split_once('@').is_some_and(|(_, reference)| {
                reference.get(..40).is_some_and(|sha| {
                    sha.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                }) && reference.get(40..).is_some_and(|rest| rest.starts_with(" # ") && rest.len() > 3)
            });
            assert!(pinned, "{name}: `{target}` is not pinned to a 40-hex SHA and a `# <ref>` comment");
        }
        let caches = yaml.matches("Swatinem/rust-cache@").count();
        assert_eq!(caches, 1, "{name}");
        assert!(yaml.contains("cache-on-failure: true"), "{name}");
    }
}

/// A workspace below the repository's top is the path the workflow scans and
/// the one rust-cache keys its target on.
#[test]
fn a_nested_workspace_is_scanned_and_cached_by_its_path() {
    let workflow = scan_workflow(&WorkflowOptions {
        workspace: "src-tauri".to_owned(),
        ..workflow_options(None)
    });
    assert!(workflow.contains("cache-on-failure: true\n          workspaces: src-tauri\n"), "{workflow}");
    let scan = run_blocks(&workflow).pop().unwrap();
    let invocations = execute(&scan, &[("BASE_REF", "main")]);
    assert_parses(&invocations);
    assert_eq!(invocations[0].get(1).map(String::as_str), Some("src-tauri"));
    assert!(!scan_workflow(&workflow_options(None)).contains("workspaces:"));
    for (path, accepted) in [
        (".", true),
        ("src-tauri", true),
        ("crates/app_1.x", true),
        ("..", false),
        ("a/../b", false),
        ("-x", false),
        ("a b", false),
        ("a'b", false),
        ("/abs", false),
        ("", false),
    ] {
        assert_eq!(is_workspace_path(path), accepted, "{path:?}");
    }
}
