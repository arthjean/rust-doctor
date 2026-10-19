use super::{compiling_command, compiling_steps_with_secrets, holds_registry_token, is_cargo_credentials, is_workflow};

const RELEASE: &str = r#"name: release
on:
  push:
    tags: ["v*"]
env:
  CARGO_TERM_COLOR: always
jobs:
  publish:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Verify
        run: cargo package --locked
      - name: Publish
        env:
          CARGO_REGISTRY_TOKEN: ${{ secrets.CRATES_IO_TOKEN }}
        run: |
          set -eu
          cargo publish --locked
"#;

#[test]
fn a_compiling_step_is_reported_at_its_run_line_with_the_narrowest_scope() {
    let findings = compiling_steps_with_secrets(RELEASE);
    assert_eq!(findings.len(), 1, "{findings:?}");
    let finding = &findings[0];
    assert_eq!(finding.subcommand, "publish");
    assert_eq!(finding.scope, "step");
    assert_eq!(finding.variable, "CARGO_REGISTRY_TOKEN");
    assert_eq!(finding.line, 19);
    assert_eq!(finding.column, 11);

    let verified = RELEASE.replace("cargo publish --locked", "cargo publish --locked --no-verify");
    assert!(compiling_steps_with_secrets(&verified).is_empty());
}

#[test]
fn a_secret_in_the_job_or_workflow_env_reaches_every_step() {
    let job = "jobs:\n  test:\n    env:\n      API: ${{ secrets.API }}\n    steps:\n      - run: cargo test --all\n";
    let findings = compiling_steps_with_secrets(job);
    assert_eq!(findings.len(), 1);
    assert_eq!((findings[0].scope, findings[0].variable.as_str()), ("job", "API"));

    let workflow = "env:\n  DEPLOY: ${{ secrets.DEPLOY }}\njobs:\n  lint:\n    steps:\n      - run: cargo +nightly clippy -- -D warnings\n";
    let findings = compiling_steps_with_secrets(workflow);
    assert_eq!(findings.len(), 1);
    assert_eq!((findings[0].scope, findings[0].subcommand.as_str()), ("workflow", "clippy"));

    let unrelated = "env:\n  RUSTFLAGS: -Dwarnings\njobs:\n  test:\n    steps:\n      - run: cargo test\n";
    assert!(compiling_steps_with_secrets(unrelated).is_empty());
}

#[test]
fn anchors_and_reusable_workflow_calls_abstain() {
    let anchored = "env: &shared\n  TOKEN: ${{ secrets.T }}\njobs:\n  test:\n    steps:\n      - run: cargo test\n";
    assert!(compiling_steps_with_secrets(anchored).is_empty());
    let reusable = "env:\n  TOKEN: ${{ secrets.T }}\njobs:\n  call:\n    uses: org/repo/.github/workflows/build.yml@v1\n  test:\n    steps:\n      - run: cargo build\n";
    let findings = compiling_steps_with_secrets(reusable);
    assert_eq!(findings.len(), 1, "only the job that runs its own steps is judged");
    let anchored_step = "jobs:\n  test:\n    steps:\n      - env:\n          T: ${{ secrets.T }}\n        run: echo\n      - &build\n        run: cargo build\n";
    assert!(compiling_steps_with_secrets(anchored_step).is_empty());
    let aliased_step = "jobs:\n  test:\n    env:\n      T: ${{ secrets.T }}\n    steps:\n      - *build\n      - run: cargo build\n";
    assert!(compiling_steps_with_secrets(aliased_step).is_empty());
}

/// A `\` continuation is read with its command: `--no-verify` written on the
/// next line exempts the publish, and the span stays on the line it starts on.
#[test]
fn a_continued_command_is_read_whole_and_a_comment_is_not_a_command() {
    let continued = RELEASE.replace("cargo publish --locked", "cargo publish \\\n            --locked --no-verify");
    assert!(compiling_steps_with_secrets(&continued).is_empty(), "{continued}");
    let built = RELEASE.replace("cargo publish --locked", "cargo build \\\n            --release");
    let findings = compiling_steps_with_secrets(&built);
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!((findings[0].line, findings[0].end - findings[0].column), (19, "cargo build \\".len()));
    let commented = RELEASE.replace("cargo publish --locked", "# cargo build runs in the verify job");
    assert!(compiling_steps_with_secrets(&commented).is_empty());
}

#[test]
fn only_compiling_subcommands_count() {
    for line in [
        "cargo build --release",
        "cd crate && cargo test",
        "~/.cargo/bin/cargo doc",
        "cargo +1.97.1 check",
        "cargo fmt --check && cargo build",
        "\"cargo build --release\"",
        "cargo --locked -q test",
        "cargo publish --no-verify; cargo build",
    ] {
        assert!(compiling_command(line).is_some(), "{line}");
    }
    for line in [
        "cargo fmt --check",
        "cargo publish --no-verify",
        "echo done",
        "cargo-deny check",
        "# cargo build",
        "echo ok # then cargo build",
    ] {
        assert!(compiling_command(line).is_none(), "{line}");
    }
}

#[test]
fn workflow_and_credentials_paths_are_matched_as_paths() {
    assert!(is_workflow(".github/workflows/ci.yml"));
    assert!(is_workflow(".github/workflows/release.yaml"));
    assert!(!is_workflow(".github/workflows/nested/ci.yml"));
    assert!(!is_workflow("docs/.github/workflows/ci.yml"));
    assert!(is_cargo_credentials(".cargo/credentials.toml"));
    assert!(is_cargo_credentials("member/.cargo/credentials"));
    assert!(!is_cargo_credentials("credentials.toml"));
    assert!(!is_cargo_credentials(".cargo/config.toml"));
}

#[test]
fn a_credentials_file_counts_only_with_a_registry_token() {
    assert!(holds_registry_token("[registry]\ntoken = \"cio-abc\"\n"));
    assert!(holds_registry_token("[registries.internal]\ntoken = \"abc\"\n"));
    assert!(!holds_registry_token("[registry]\ntoken = \"\"\n"));
    assert!(!holds_registry_token(""));
    assert!(!holds_registry_token("[net]\ntoken = \"abc\"\n"));
    assert!(holds_registry_token("registry.token = \"abc\"\n"));
    assert!(holds_registry_token("[registries]\ninternal = { token = \"abc\" }\n"));
    assert!(holds_registry_token("[registry] # crates.io\ntoken = \"abc\"\n"));
    assert!(!holds_registry_token("[registry]\ntoken = \"\" # fill in\n"));
}
