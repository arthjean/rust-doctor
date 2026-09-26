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
/// answers `JQ_1` then `JQ_2`; `gh` logs its arguments; `NO_REPORT` leaves the
/// uploaded report out.
pub(crate) fn run_stubbed(block: &str, environment: &[(&str, &str)]) -> Execution {
    let root = scratch("ci-tests", "execute");
    let log = root.join("argv");
    fs::create_dir_all(root.join("rust-doctor")).unwrap();
    if !environment.iter().any(|(name, _)| *name == "NO_REPORT") {
        fs::write(root.join("rust-doctor/report.json"), "{}").unwrap();
    }
    let prelude = r#"
rust-doctor() {
  printf '%s\037' rust-doctor "$@" >> "$ARGV_LOG"; printf '\n' >> "$ARGV_LOG"
  if [ "${1:-}" = report ]; then echo '## rust-doctor'; return 0; fi
  return "${SCAN_STATUS:-0}"
}
git() { :; }
npm() { :; }
gh() { echo "$*" >> "$GH_LOG"; echo '[]'; }
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
    // constant: the command line is never assembled from text.
    let output = Command::new("bash")
        .args(["-c", "eval \"$STUBS\"\neval \"$BLOCK\""])
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
        blocking,
        branch: "trunk".to_owned(),
        toolchain: VALIDATED_TOOLCHAIN.to_owned(),
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
fn every_command_the_comment_workflow_runs_parses_against_the_cli() {
    let workflow = comment_workflow();
    let invocations: Vec<Vec<String>> = run_blocks(&workflow)
        .iter()
        .flat_map(|block| execute(block, &[("HEAD_SHA", "abc"), ("HEAD_OWNER", "fork"), ("HEAD_BRANCH", "topic"), ("REPOSITORY", "o/r")]))
        .collect();
    assert!(!invocations.is_empty());
    assert_parses(&invocations);
}

#[test]
fn the_scan_workflow_runs_read_only_and_never_interpolates_into_a_shell() {
    let workflow = scan_workflow(&workflow_options(None));
    assert!(workflow.starts_with(MARKER));
    assert!(workflow.contains("permissions:\n  contents: read\n"));
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

#[test]
fn the_comment_workflow_holds_the_only_write_token_and_never_builds_the_pull_request() {
    let workflow = comment_workflow();
    assert!(workflow.starts_with(MARKER));
    assert!(workflow.contains("workflow_run:\n    workflows: [Rust Doctor]"));
    assert!(workflow.contains("\npermissions: {}\n"));
    assert_eq!(workflow.matches("pull-requests: write").count(), 1);
    assert!(workflow.contains("<!-- rust-doctor -->"));
    for absent in ["actions/checkout", "cargo ", "\n  pull_request_target"] {
        assert!(!workflow.contains(absent), "the comment workflow names {absent}");
    }
    for block in run_blocks(&workflow) {
        assert!(!block.contains("${{"), "a run block interpolates: {block}");
    }
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
    options(root, None, Some(branch.to_owned()), toolchain.to_owned())
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
    for input in ["version", "toolchain", "scope", "base", "blocking", "working-directory", "args"] {
        assert!(input_default(input).is_some(), "action.yml declares no default for `{input}`");
    }
    assert_eq!(input_default("version").as_deref(), Some(env!("CARGO_PKG_VERSION")));
    assert_eq!(input_default("toolchain").as_deref(), Some(VALIDATED_TOOLCHAIN));
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
    assert_eq!(blocks.len(), 5, "install, scan, summary, outputs, exit");
    let cases: [(&[(&str, &str)], &str); 4] = [
        (
            &[("PULL_REQUEST_BASE", "main")],
            "rust-doctor . --yes --json --scope baseline --base origin/main",
        ),
        (&[], "rust-doctor . --yes --json --scope full"),
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

/// This repository runs what `ci install --comment` writes, byte for byte,
/// and scans itself through the Action from its own checkout.
#[test]
fn this_repository_runs_the_comment_workflow_and_the_action_it_ships() {
    assert_eq!(
        include_str!("../../.github/workflows/rust-doctor-comment.yml"),
        comment_workflow(),
        "regenerate it with `rust-doctor ci install --comment --update`"
    );
    let dogfood = include_str!("../../.github/workflows/dogfood.yml");
    assert!(dogfood.starts_with("name: Rust Doctor\n"), "the comment workflow follows it by name");
    assert!(dogfood.contains("uses: ./\n"));
    assert!(dogfood.contains("pull_request:"));
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
        4,
        "summary, upload, outputs and exit run after a failed scan"
    );
}

/// The comment step as a second push, a first push, a scan that uploaded
/// nothing, and a commit no open pull request is at.
#[test]
fn the_comment_is_edited_in_place_and_skipped_without_a_report() {
    let workflow = comment_workflow();
    let post = run_blocks(&workflow).pop().unwrap();
    let head = [
        ("HEAD_SHA", "abc"),
        ("HEAD_OWNER", "fork"),
        ("HEAD_BRANCH", "topic"),
        ("REPOSITORY", "o/r"),
    ];
    let with = |extra: &[(&'static str, &'static str)]| {
        let mut environment = head.to_vec();
        environment.extend_from_slice(extra);
        run_stubbed(&post, &environment)
    };

    let edited = with(&[("JQ_1", "7"), ("JQ_2", "9")]);
    assert_eq!(edited.status, Some(0));
    assert!(edited.gh.contains("-X GET repos/o/r/pulls -f state=open -f head=fork:topic"), "{}", edited.gh);
    assert!(edited.gh.contains("-X PATCH repos/o/r/issues/comments/9 -f body=<!-- rust-doctor -->"), "{}", edited.gh);
    assert!(!edited.gh.contains("-X POST"));

    let created = with(&[("JQ_1", "7"), ("JQ_2", "")]);
    assert_eq!(created.status, Some(0));
    assert!(created.gh.contains("-X POST repos/o/r/issues/7/comments -f body=<!-- rust-doctor -->"), "{}", created.gh);

    let missing = with(&[("NO_REPORT", "1")]);
    assert_eq!(missing.status, Some(0));
    assert!(missing.gh.is_empty(), "nothing is posted without a report");
    assert!(missing.stdout.contains("::notice::"), "{}", missing.stdout);

    let closed = with(&[("JQ_1", "")]);
    assert_eq!(closed.status, Some(0));
    assert!(!closed.gh.contains("comments"), "{}", closed.gh);
    assert!(closed.stdout.contains("::notice::"));
}
