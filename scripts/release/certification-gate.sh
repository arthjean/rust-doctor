#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 4 ]]; then
  echo "usage: certification-gate.sh <gate> <release-binary> <tool-revision> <generated-at>" >&2
  exit 2
fi

gate=$1
release_binary=$2
tool_revision=$3
generated_at=$4
eval_binary=target/release/rust-doctor-eval

[[ -x "$release_binary" && -x "$eval_binary" ]]
[[ "$tool_revision" =~ ^[0-9a-fA-F]{40}$ ]]
[[ "$generated_at" =~ ^[1-9][0-9]*$ ]]

case "$gate" in
  quality-gates)
    cargo fmt --check
    cargo +1.97 check --all-targets
    cargo check --all-targets --all-features
    cargo build
    cargo build --no-default-features
    cargo clippy --all-targets --all-features -- -W clippy::all -W clippy::pedantic -W clippy::nursery -D warnings
    cargo test --all-features
    cargo test --test deadlock_regression
    ;;
  cross-surface)
    cargo test --test cross_surface_ordering
    ;;
  artifact-smoke)
    : "${CERT_NO_DEFAULT_BINARY:?}"
    : "${CERT_LSP_BINARY:?}"
    : "${CERT_NPM_PLATFORM_PACKAGE:?}"
    : "${CERT_NPM_WRAPPER_PACKAGE:?}"
    : "${CERT_NATIVE_ARCHIVE:?}"
    : "${CERT_CRATE_PACKAGE:?}"
    "$eval_binary" smoke \
      --binary "$release_binary" \
      --no-default-binary "$CERT_NO_DEFAULT_BINARY" \
      --lsp-binary "$CERT_LSP_BINARY" \
      --schema schemas/report-v1.schema.json \
      --npm-platform-package "$CERT_NPM_PLATFORM_PACKAGE" \
      --npm-wrapper-package "$CERT_NPM_WRAPPER_PACKAGE" \
      --bun "${CERT_BUN:-bun}" \
      --archive "$CERT_NATIVE_ARCHIVE" \
      --crate-package "$CERT_CRATE_PACKAGE" \
      --action action.yml
    ;;
  workspace-scale)
    cargo test --release --test deadlock_regression \
      release_scale_workspace_is_deterministic_without_deadlock \
      -- --ignored --exact
    ;;
  decision-overhead)
    cargo test --release --lib ordering::tests::canonical_decision_scale_gate -- --ignored --exact
    ;;
  corpus-runtime)
    : "${CERT_CORPUS_BASELINE:?}"
    : "${CERT_CORPUS_CANDIDATE:?}"
    : "${CERT_PERFORMANCE_BASELINE:?}"
    mkdir -p target/certification
    "$eval_binary" benchmark \
      --manifest evaluation/benchmarks-v1.json \
      --binary "$release_binary" \
      --baseline "$CERT_PERFORMANCE_BASELINE" \
      --baseline-approval evaluation/approvals/performance-baseline.json \
      --output target/certification/benchmark-candidate.json \
      --repetitions 3
    "$eval_binary" delta \
      --baseline "$CERT_CORPUS_BASELINE" \
      --baseline-approval evaluation/approvals/corpus-baseline.json \
      --candidate "$CERT_CORPUS_CANDIDATE" \
      --output target/certification/corpus-delta.json
    jq -e '
      .blocked == false
      and .median_runtime_delta_percent <= 10
    ' target/certification/corpus-delta.json >/dev/null
    ;;
  interruption)
    cargo test --release --test release_interruption \
      unix::signals_terminate_analyzer_groups_within_two_seconds \
      -- --ignored --exact
    ;;
  *)
    echo "unknown certification gate: $gate" >&2
    exit 2
    ;;
esac

binary_sha256=$(sha256sum "$release_binary" | cut -d' ' -f1)
command="bash scripts/release/certification-gate.sh $gate"
detail="$gate gate completed against the release binary"
destination="evaluation/certifications/evidence/$gate.json"
mkdir -p "$(dirname "$destination")"
temporary=$(mktemp "$(dirname "$destination")/.${gate}.XXXXXX")
jq -n \
  --arg gate "$gate" \
  --arg binary_sha256 "$binary_sha256" \
  --arg tool_revision "$tool_revision" \
  --argjson generated_at "$generated_at" \
  --arg command "$command" \
  --arg detail "$detail" \
  '{
    schema_version: "1.0",
    gate: $gate,
    release_binary_sha256: $binary_sha256,
    tool_revision: $tool_revision,
    generated_at_utc_unix_seconds: $generated_at,
    passed: true,
    command: $command,
    detail: $detail
  }' > "$temporary"
mv "$temporary" "$destination"
