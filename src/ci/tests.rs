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
/// `EMPTY_REPORT` leaves both empty; `CLEAN` is written as the scan's clean
/// flag. `git` logs its arguments, answers `SHALLOW` (default `false`) to
/// `--is-shallow-repository`, and fails a fetch under `FETCH_FAILS`. `npm
/// prefix -g` answers a prefix whose `bin/rust-doctor` logs as the `rust-doctor`
/// function does, under its absolute path, and `{prefix}` in an environment
/// value stands for that prefix.
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
    if let Some((_, clean)) = environment.iter().find(|(name, _)| *name == "CLEAN") {
        fs::write(root.join("rust-doctor/clean"), format!("{clean}\n")).unwrap();
    }
    let prefix = root.join("npm-prefix");
    fs::create_dir_all(prefix.join("bin")).unwrap();
    let launcher = prefix.join("bin/rust-doctor");
    fs::write(
        &launcher,
        "#!/bin/bash\nprintf '%s\\037' \"$0\" \"$@\" >> \"$ARGV_LOG\"; printf '\\n' >> \"$ARGV_LOG\"\n\
         if [ \"${1:-}\" = report ]; then echo '## rust-doctor'; exit 0; fi\n\
         if [ \"${1:-}\" = --version ]; then echo 'rust-doctor 0.0.0'; exit 0; fi\n\
         exit \"${SCAN_STATUS:-0}\"\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&launcher, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let prefix_text = prefix.to_string_lossy().into_owned();
    let environment: Vec<(String, String)> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.replace("{prefix}", &prefix_text)))
        .collect();
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
npm() { if [ "${1:-}" = prefix ]; then echo "$NPM_PREFIX"; fi; }
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
        .env("NPM_PREFIX", &prefix)
        .envs(environment.iter().map(|(name, value)| (name.as_str(), value.as_str())))
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
        let printed_version = parsed
            .as_ref()
            .err()
            .is_some_and(|error| error.kind() == clap::error::ErrorKind::DisplayVersion);
        assert!(parsed.is_ok() || printed_version, "the CLI refuses {argv:?}: {:?}", parsed.err());
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
            let argv = invocations
                .iter()
                .find(|argv| argv.get(2).map(String::as_str) == Some("--yes"))
                .cloned()
                .unwrap();
            assert!(
                Path::new(&argv[0]).is_absolute() && argv[0].ends_with("/npm-prefix/bin/rust-doctor"),
                "the launcher runs by the path npm installed it at: {argv:?}"
            );
            command_line(&argv)
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
            pull_request.iter().any(|argv| {
                argv.get(1..3) == Some(&["report".to_owned(), "markdown".to_owned()][..])
                    && argv[0].ends_with("/npm-prefix/bin/rust-doctor")
            }),
            "the summary is rendered by `report markdown`, through the same launcher"
        );
    }
}

/// An invocation as a command line, the program named by its file name.
fn command_line(argv: &[String]) -> String {
    let program = Path::new(&argv[0])
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    std::iter::once(program)
        .chain(argv[1..].iter().cloned())
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn the_scan_workflow_runs_read_only_and_never_interpolates_into_a_shell() {
    let workflow = scan_workflow(&workflow_options(None));
    assert!(workflow.starts_with(MARKER));
    assert!(workflow.contains("permissions:\n  contents: read\n\njobs:"));
    assert!(!workflow.contains("pull-requests") && !workflow.contains("<!-- rust-doctor -->"));
    assert!(!workflow.contains("\n  comment:\n"));
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

/// `--comment` adds a second job, the only one granted the write permission,
/// which needs the scan, checks nothing out and runs the script
/// `comment/action.yml` runs, byte for byte.
#[test]
fn the_comment_is_opt_in_and_the_action_runs_the_same_script() {
    let workflow = scan_workflow(&WorkflowOptions {
        comment: true,
        ..workflow_options(None)
    });
    assert!(workflow.contains("permissions:\n  contents: read\n\njobs:"));
    assert_eq!(workflow.matches("pull-requests: write").count(), 1);
    let comment = workflow.split("\n  comment:\n").nth(1).unwrap();
    assert!(comment.contains("    needs: inspect\n"), "{comment}");
    assert!(comment.contains("    if: always() && github.event_name == 'pull_request'\n"));
    assert!(comment.contains("    permissions:\n      pull-requests: write\n"));
    assert!(!comment.contains("actions/checkout") && !comment.contains("npm install"));
    assert!(comment.contains(
        "uses: actions/download-artifact@d3f86a106a0bac45b974a628896c90dbdf5c8093 # v4.3.0"
    ));
    assert!(!workflow.contains("pull_request_target"));
    let blocks = run_blocks(&workflow);
    assert_eq!(blocks.len(), 3, "install, scan, comment");
    assert_eq!(blocks.last().map(String::as_str), Some(COMMENT_SCRIPT.trim_end()));
    for block in &blocks {
        assert!(!block.contains("${{"), "a run block interpolates: {block}");
    }
    assert!(!ACTION.contains("<!-- rust-doctor -->"), "action.yml no longer comments");
    let action_comment = run_blocks(COMMENT_ACTION)
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
const COMMENT_ACTION: &str = include_str!("../../comment/action.yml");

/// The default an input of `action.yml` declares, unquoted.
fn input_default(name: &str) -> Option<String> {
    declared_default(ACTION, name)
}

/// The default an input of an action declares, unquoted.
fn declared_default(action: &str, name: &str) -> Option<String> {
    let mut lines = action
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
        "cache",
    ] {
        assert!(input_default(input).is_some(), "action.yml declares no default for `{input}`");
    }
    assert_eq!(input_default("version").as_deref(), Some(env!("CARGO_PKG_VERSION")));
    assert_eq!(input_default("toolchain").as_deref(), Some(VALIDATED_TOOLCHAIN));
    for removed in ["comment", "comment-on-clean"] {
        assert!(!ACTION.contains(&format!("\n  {removed}:\n")), "action.yml still declares `{removed}`");
    }
    assert_eq!(declared_default(COMMENT_ACTION, "comment-on-clean").as_deref(), Some("true"));
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
            ("LAUNCHER", "{prefix}/bin/rust-doctor"),
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
            .find(|argv| argv.get(2).map(String::as_str) == Some("--yes"));
        assert_eq!(scan.map(|argv| command_line(argv)).as_deref(), Some(expected), "{inputs:?}");
        for argv in &invocations {
            assert!(
                argv[0].ends_with("/npm-prefix/bin/rust-doctor"),
                "the Action runs the launcher it installed: {argv:?}"
            );
        }
        assert!(
            invocations.iter().any(|argv| argv.get(1).map(String::as_str) == Some("report")),
            "the job summary is rendered by `report markdown`"
        );
    }
}

/// The install step resolves the launcher once, by the absolute path npm
/// installed it at, and the scan and summary steps run that path. With
/// `version: preinstalled` the step is skipped, its output is empty and both
/// fall back to the `rust-doctor` on PATH.
#[test]
fn the_action_runs_the_launcher_it_installed_by_its_absolute_path() {
    let install = run_blocks(ACTION).remove(0);
    let installed = run_stubbed(&install, &[("VERSION", "0.9.0")]);
    assert_eq!(installed.status, Some(0));
    let launcher = installed.outputs.trim().strip_prefix("launcher=").unwrap();
    assert!(Path::new(launcher).is_absolute() && launcher.ends_with("/npm-prefix/bin/rust-doctor"));
    assert_eq!(installed.stdout, "rust-doctor 0.0.0\n");
    let header = ACTION.split("- name: Install rust-doctor").nth(1).unwrap();
    assert!(header.contains("id: install\n") && header.contains("if: inputs.version != 'preinstalled'"));
    for step in ["- name: Scan", "- name: Summarize"] {
        let block = ACTION.split(step).nth(1).unwrap().split("run: |").next().unwrap();
        assert!(block.contains("LAUNCHER: ${{ steps.install.outputs.launcher }}"), "{step}");
    }
    let mut environment = SCAN_INPUTS.to_vec();
    environment.push(("PULL_REQUEST_BASE", ""));
    environment.push(("LAUNCHER", ""));
    let preinstalled = run_stubbed(&action_scan(), &environment);
    assert_eq!(preinstalled.invocations[0][0], "rust-doctor", "preinstalled keeps the PATH lookup");
}

/// This repository scans itself through the Action from its own checkout,
/// with the permission its comment needs.
#[test]
fn this_repository_runs_the_action_it_ships() {
    let dogfood = include_str!("../../.github/workflows/dogfood.yml");
    assert!(dogfood.starts_with("name: Rust Doctor\n"));
    assert!(dogfood.contains("uses: ./\n"));
    assert!(dogfood.contains("pull_request:"));
    let comment = dogfood.split("\n  comment:\n").nth(1).unwrap();
    assert!(comment.contains("      - uses: ./comment\n"));
    assert!(comment.contains("      pull-requests: write\n"));
    let grants = dogfood.lines().filter(|line| line.trim() == "pull-requests: write").count();
    assert_eq!(grants, 1, "only the comment job writes to the pull request");
}

/// A scan that exits 2 still leaves an exit code for the summary, the upload
/// and the outputs to run after, each of which runs `if: always()`, and the
/// last step fails with that code.
#[test]
fn a_failed_scan_still_publishes_and_then_fails_the_step() {
    let blocks = run_blocks(ACTION);
    let scan = blocks
        .iter()
        .find(|block| block.contains(SCAN_COMMAND))
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
        .find(|block| block.contains(SCAN_COMMAND))
        .unwrap()
}

/// The line the Action's scan step runs the scan on.
const SCAN_COMMAND: &str = "\"${LAUNCHER:-rust-doctor}\" \"${arguments[@]}\"";

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
    assert_eq!(declared_default(COMMENT_ACTION, "comment-on-clean").as_deref(), Some("true"));
    assert_eq!(input_default("cache").as_deref(), Some("true"));
    assert!(COMMENT_ACTION.contains("COMMENT_ON_CLEAN: ${{ inputs.comment-on-clean }}"));
    assert!(ACTION.contains("echo \"$clean\" > \"$RUNNER_TEMP/rust-doctor/clean\""));
    assert!(ACTION.contains("${{ runner.temp }}/rust-doctor/clean\n"));
}

/// Every action the shipped YAML uses is pinned to a full commit, with the
/// ref it was read from beside it, and every rust-cache step saves on
/// failure, from the default branch only, and never caches `~/.cargo/bin`.
#[test]
fn every_shipped_action_is_pinned_to_a_commit() {
    let workflow = scan_workflow(&WorkflowOptions {
        comment: true,
        ..workflow_options(None)
    });
    for (name, yaml) in [
        ("action.yml", ACTION),
        ("the generated workflow", workflow.as_str()),
        ("comment/action.yml", COMMENT_ACTION),
    ] {
        let uses: Vec<&str> = yaml
            .lines()
            .filter_map(|line| line.trim_start().trim_start_matches("- ").strip_prefix("uses: "))
            .filter(|target| !target.starts_with("./"))
            .collect();
        assert!(!uses.is_empty(), "{name}: {uses:?}");
        let count = uses.len();
        for target in uses {
            let pinned = target.split_once('@').is_some_and(|(_, reference)| {
                reference.get(..40).is_some_and(|sha| {
                    sha.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                }) && reference.get(40..).is_some_and(|rest| rest.starts_with(" # ") && rest.len() > 3)
            });
            assert!(pinned, "{name}: `{target}` is not pinned to a 40-hex SHA and a `# <ref>` comment");
        }
        if name == "comment/action.yml" {
            assert!(!yaml.contains("rust-cache") && !yaml.contains("actions/checkout"), "{name}");
            continue;
        }
        assert!(count >= 3, "{name}: {count} actions");
        let caches = yaml.matches("Swatinem/rust-cache@").count();
        assert_eq!(caches, 1, "{name}");
        let cache = yaml.split("Swatinem/rust-cache@").nth(1).unwrap();
        let settings: Vec<&str> = cache
            .lines()
            .skip(1)
            .take_while(|line| !line.trim().is_empty())
            .map(str::trim)
            .collect();
        for setting in ["cache-on-failure: true", "cache-bin: false", SAVE_IF] {
            assert!(settings.contains(&setting), "{name}: {settings:?} lacks {setting}");
        }
    }
}

/// What both rust-cache steps save under: the default branch, and nothing else.
const SAVE_IF: &str =
    "save-if: ${{ github.ref == format('refs/heads/{0}', github.event.repository.default_branch) }}";

/// The `save-if` expression of [`SAVE_IF`], evaluated as GitHub would for a
/// run on `reference` in a repository whose payload names `default_branch`:
/// `format` fills `{0}`, a missing property is the empty string.
fn saves(reference: &str, default_branch: Option<&str>) -> bool {
    let expression = SAVE_IF
        .strip_prefix("save-if: ${{ ")
        .and_then(|rest| rest.strip_suffix(" }}"))
        .unwrap();
    let (left, right) = expression.split_once(" == ").unwrap();
    assert_eq!(left, "github.ref");
    let (template, argument) = right
        .strip_prefix("format('")
        .and_then(|rest| rest.strip_suffix(')'))
        .and_then(|rest| rest.split_once("', "))
        .unwrap();
    assert_eq!(argument, "github.event.repository.default_branch");
    reference == template.replace("{0}", default_branch.unwrap_or_default())
}

/// EP-005 US-019, unhappy path: a pull request's ref is its merge ref, so the
/// expression is false and the cache step restores without saving; a
/// scheduled payload without the default branch saves nothing either.
#[test]
fn only_a_run_on_the_default_branch_saves_the_cache() {
    assert!(!saves("refs/pull/7/merge", Some("main")), "a pull request never saves");
    assert!(saves("refs/heads/main", Some("main")), "the default branch saves");
    assert!(!saves("refs/heads/feature", Some("main")), "another branch never saves");
    assert!(!saves("refs/heads/main", None), "a payload without the default branch never saves");
}

/// The jobs of a workflow, each with its id and the lines it spans, after
/// the lines before `jobs:`.
fn workflow_jobs(yaml: &str) -> (Vec<&str>, Vec<(String, Vec<&str>)>) {
    let (preamble, jobs) = yaml.split_once("\njobs:\n").unwrap_or((yaml, ""));
    let mut parsed: Vec<(String, Vec<&str>)> = Vec::new();
    for line in jobs.lines() {
        match line.strip_prefix("  ").and_then(|rest| rest.strip_suffix(':')) {
            Some(id) if !id.starts_with([' ', '#']) => parsed.push((id.to_owned(), Vec::new())),
            _ => {
                if let Some((_, lines)) = parsed.last_mut() {
                    lines.push(line);
                }
            }
        }
    }
    (preamble.lines().collect(), parsed)
}

/// Whether line `index` of a job runs pull request code: a checkout of
/// anything but the base commit, the scan Action, rust-doctor, or Cargo.
fn runs_pull_request_code(lines: &[&str], index: usize) -> bool {
    let code = lines[index].trim().trim_start_matches("- ");
    if code.starts_with('#') {
        return false;
    }
    if code.starts_with("uses: actions/checkout@") {
        return !lines[index + 1..]
            .iter()
            .take_while(|next| !next.trim_start().starts_with("- "))
            .any(|next| next.trim() == "ref: ${{ github.event.pull_request.base.sha }}");
    }
    let scans = ["uses: ./", "uses: arthjean/rust-doctor@", "rust-doctor ", "run: rust-doctor"];
    let builds = ["npm install -g rust-doctor", "\"$launcher\"", "cargo "];
    code == scans[0]
        || scans[1..].iter().any(|prefix| code.starts_with(prefix))
        || builds.iter().any(|fragment| code.contains(fragment))
}

/// Why a job of a workflow could run pull request code beside the token that
/// writes to the pull request: it holds `pull-requests: write`, its own or the
/// workflow's, and a line of it runs that code.
pub(crate) fn writing_jobs_that_run_pull_request_code(yaml: &str) -> Vec<String> {
    let grants = |lines: &[&str]| lines.iter().any(|line| line.trim() == "pull-requests: write");
    let (preamble, jobs) = workflow_jobs(yaml);
    let workflow_grants = grants(&preamble);
    let mut violations = Vec::new();
    for (id, lines) in jobs {
        let own = lines.iter().any(|line| line.trim() == "permissions:");
        if !(grants(&lines) || (workflow_grants && !own)) {
            continue;
        }
        for index in (0..lines.len()).filter(|index| runs_pull_request_code(&lines, *index)) {
            violations.push(format!("job `{id}`: {}", lines[index].trim()));
        }
    }
    violations
}

/// EP-005 US-020, unhappy path: in the generated workflow, no job holding
/// `pull-requests: write` checks out the pull request or runs rust-doctor,
/// and the check refuses each way a writing job could.
#[test]
fn no_job_that_writes_to_the_pull_request_runs_its_code() {
    for comment in [false, true] {
        let workflow = scan_workflow(&WorkflowOptions {
            comment,
            ..workflow_options(None)
        });
        assert_eq!(writing_jobs_that_run_pull_request_code(&workflow), Vec::<String>::new());
    }
    let dogfood = include_str!("../../.github/workflows/dogfood.yml");
    assert_eq!(writing_jobs_that_run_pull_request_code(dogfood), Vec::<String>::new());
    let documented: Vec<&str> = include_str!("../../README.md")
        .split("```yaml\n")
        .skip(1)
        .filter_map(|block| block.split("```").next())
        .filter(|block| block.contains("pull-requests: write"))
        .collect();
    assert_eq!(documented.len(), 1, "the README documents the two-job layout once");
    for layout in documented {
        assert!(layout.contains("uses: arthjean/rust-doctor/comment@v0"), "{layout}");
        assert_eq!(writing_jobs_that_run_pull_request_code(layout), Vec::<String>::new());
    }

    let job = |permissions: &str, steps: &str| {
        format!("permissions:\n  contents: read{permissions}\n\njobs:\n  scan:\n    steps:\n{steps}\n")
    };
    for steps in [
        "      - uses: actions/checkout@fbc6f3992d24b796d5a048ff273f7fcc4a7b6c09 # v5.1.0",
        "      - uses: ./",
        "      - uses: arthjean/rust-doctor@v0",
        "      - run: npm install -g rust-doctor@0.9.0",
        "      - run: rust-doctor . --yes",
        "      - run: cargo build",
    ] {
        let granted = job("\n  pull-requests: write", steps);
        assert_eq!(writing_jobs_that_run_pull_request_code(&granted).len(), 1, "{steps}");
        assert!(writing_jobs_that_run_pull_request_code(&job("", steps)).is_empty(), "{steps}");
    }
    let base = "      - uses: actions/checkout@v5\n        with:\n          ref: ${{ github.event.pull_request.base.sha }}\n      - uses: ./comment";
    assert!(writing_jobs_that_run_pull_request_code(&job("\n  pull-requests: write", base)).is_empty());
}

/// A workspace below the repository's top is the path the workflow scans and
/// the one rust-cache keys its target on.
#[test]
fn a_nested_workspace_is_scanned_and_cached_by_its_path() {
    let workflow = scan_workflow(&WorkflowOptions {
        workspace: "src-tauri".to_owned(),
        ..workflow_options(None)
    });
    assert!(workflow.contains(&format!("{SAVE_IF}\n          workspaces: src-tauri\n")), "{workflow}");
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
