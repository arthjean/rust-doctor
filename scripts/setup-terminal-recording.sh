#!/usr/bin/env bash

set -euo pipefail

TERMINAL_RECORDING_REPOSITORY_ROOT="$PWD"
TERMINAL_RECORDING_DIRECTORY="$(mktemp -d)"
trap 'rm -rf "$TERMINAL_RECORDING_DIRECTORY"' EXIT

mkdir -p \
  "$TERMINAL_RECORDING_DIRECTORY/crates/api/src" \
  "$TERMINAL_RECORDING_DIRECTORY/crates/cli/src"

cat > "$TERMINAL_RECORDING_DIRECTORY/Cargo.toml" <<'EOF'
[workspace]
members = ["crates/api", "crates/cli"]
resolver = "2"
EOF

cat > "$TERMINAL_RECORDING_DIRECTORY/crates/api/Cargo.toml" <<'EOF'
[package]
name = "sample-api"
version = "0.1.0"
edition = "2024"
EOF

cat > "$TERMINAL_RECORDING_DIRECTORY/crates/api/src/lib.rs" <<'EOF'
pub fn require_configuration(value: Option<&str>) -> &str {
    match value {
        Some(value) => value,
        None => panic!("missing configuration"),
    }
}
EOF

cat > "$TERMINAL_RECORDING_DIRECTORY/crates/cli/Cargo.toml" <<'EOF'
[package]
name = "sample-cli"
version = "0.1.0"
edition = "2024"
EOF

cat > "$TERMINAL_RECORDING_DIRECTORY/crates/cli/src/lib.rs" <<'EOF'
pub fn run() -> &'static str {
    "ready"
}
EOF

git -C "$TERMINAL_RECORDING_DIRECTORY" init --quiet --initial-branch=master
git -C "$TERMINAL_RECORDING_DIRECTORY" add .
git -C "$TERMINAL_RECORDING_DIRECTORY" \
  -c user.name="Rust Doctor" \
  -c user.email="rust-doctor@example.invalid" \
  commit --quiet -m "fixture"

export RUST_DOCTOR_FORCE_ONBOARDING=1
export RUST_DOCTOR_TELEMETRY=0
export XDG_CONFIG_HOME="$TERMINAL_RECORDING_DIRECTORY/.config"
unset CI GITHUB_ACTIONS GITLAB_CI BUILDKITE JENKINS_URL TF_BUILD CODEBUILD_BUILD_ID
unset TEAMCITY_VERSION BITBUCKET_BUILD_NUMBER CIRCLECI TRAVIS DRONE GIT_DIR
unset CLAUDECODE CLAUDE_CODE CURSOR_AGENT CODEX_CI CODEX_SANDBOX
unset CODEX_SANDBOX_NETWORK_DISABLED OPENCODE GOOSE_TERMINAL AGENT_SESSION_ID
unset AMP_THREAD_ID AGENT_THREAD_ID AGENT

rust-doctor() {
  "$TERMINAL_RECORDING_REPOSITORY_ROOT/target/debug/rust-doctor" "$@"
}
export -f rust-doctor

cd "$TERMINAL_RECORDING_DIRECTORY"
clear
printf 'terminal-recording-ready\n'

set +e +u
set +o pipefail
