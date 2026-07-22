#!/usr/bin/env bash
set -euo pipefail

write_output() {
  printf '%s=%s\n' "$1" "$2" >> "$GITHUB_OUTPUT"
}

case "$INPUT_SCOPE" in
  auto|full|changed|baseline|staged) ;;
  *) echo "::error::scope must be auto, full, changed, baseline, or staged"; exit 2 ;;
esac

SCAN_ROOT=$(realpath "$INPUT_DIRECTORY")
[[ -d "$SCAN_ROOT" ]] || { echo "::error::directory is not a directory"; exit 2; }
GIT_ROOT=$(git -C "$SCAN_ROOT" rev-parse --show-toplevel 2>/dev/null) || {
  echo "::error::Rust Doctor Action requires a Git checkout"
  exit 2
}
GIT_ROOT=$(realpath "$GIT_ROOT")
CHANGED_PATHS_FILE=$(mktemp "$RUNNER_TEMP/rust-doctor-changed.XXXXXX.paths")
BASE_SHA_VALUE="${PR_BASE_SHA:-}"
RESOLVED_SCOPE="$INPUT_SCOPE"
DEGRADED_REASON=""
SKIP_SCAN=false
PATHS_RESOLVED=false

if [[ "$RESOLVED_SCOPE" == auto ]]; then
  if [[ "$EVENT_NAME" == pull_request || "$EVENT_NAME" == pull_request_target ]]; then
    RESOLVED_SCOPE=baseline
  else
    RESOLVED_SCOPE=full
  fi
fi

if [[ "$RESOLVED_SCOPE" == baseline || "$RESOLVED_SCOPE" == changed ]]; then
  LOCAL_BASE=false
  if [[ "$BASE_SHA_VALUE" =~ ^[0-9a-fA-F]{40}$ ]]; then
    if ! git -C "$GIT_ROOT" cat-file -e "${BASE_SHA_VALUE}^{commit}" 2>/dev/null; then
      git -C "$GIT_ROOT" fetch --no-tags --depth=100 origin "$BASE_SHA_VALUE" >/dev/null 2>&1 || true
    fi
    if git -C "$GIT_ROOT" merge-base "$BASE_SHA_VALUE" HEAD >/dev/null 2>&1; then
      LOCAL_BASE=true
    fi
  fi

  if [[ "$LOCAL_BASE" == true ]]; then
    git -C "$GIT_ROOT" diff --name-only -z "$BASE_SHA_VALUE...HEAD" -- > "$CHANGED_PATHS_FILE"
    PATHS_RESOLVED=true
  elif [[ -n "${GH_TOKEN:-}" && "${PR_NUMBER:-}" =~ ^[0-9]+$ && -n "${REPOSITORY:-}" ]]; then
    API_PATHS=$(mktemp "$RUNNER_TEMP/rust-doctor-api-paths.XXXXXX")
    if gh api --paginate "repos/${REPOSITORY}/pulls/${PR_NUMBER}/files?per_page=100" \
      --jq '.[] | .filename | @base64' > "$API_PATHS" 2>/dev/null; then
      while IFS= read -r encoded; do
        printf '%s' "$encoded" | base64 --decode >> "$CHANGED_PATHS_FILE"
        printf '\0' >> "$CHANGED_PATHS_FILE"
      done < "$API_PATHS"
      PATHS_RESOLVED=true
      RESOLVED_SCOPE=api-files
      DEGRADED_REASON="base history unavailable; changed paths resolved through the GitHub API"
    else
      RESOLVED_SCOPE=full
      DEGRADED_REASON="base history and GitHub changed-path API unavailable; running a full scan"
    fi
    rm -f "$API_PATHS"
  else
    RESOLVED_SCOPE=full
    DEGRADED_REASON="base history unavailable; running a full scan"
  fi
fi

if [[ -s "$CHANGED_PATHS_FILE" ]]; then
  RELEVANT=false
  FORCE_FULL=false
  while IFS= read -r -d '' path; do
    case "$path" in
      *.rs) RELEVANT=true ;;
      Cargo.toml|*/Cargo.toml|Cargo.lock|*/Cargo.lock|rust-doctor.toml|*/rust-doctor.toml|build.rs|*/build.rs)
        RELEVANT=true
        FORCE_FULL=true
        ;;
    esac
  done < "$CHANGED_PATHS_FILE"
  if [[ "$RELEVANT" == false ]]; then
    SKIP_SCAN=true
    DEGRADED_REASON="pull request changes do not affect Rust analysis"
  elif [[ "$FORCE_FULL" == true && "$RESOLVED_SCOPE" == api-files ]]; then
    RESOLVED_SCOPE=full
    DEGRADED_REASON="base history unavailable and project metadata changed; running all package checks"
  fi
elif [[ "$PATHS_RESOLVED" == true ]]; then
  SKIP_SCAN=true
  DEGRADED_REASON="pull request contains no changed paths"
fi

COMPILER_FINGERPRINT=$(rustc -vV 2>/dev/null | sha256sum | cut -d' ' -f1)
RULESET_FINGERPRINT=$(printf 'rust-doctor-rules:%s' "$INPUT_VERSION" | sha256sum | cut -d' ' -f1)
CONFIG_FINGERPRINT=$(
  {
    printf '%s\0' "$INPUT_VERSION"
    find "$SCAN_ROOT" -type f \
      \( -name Cargo.toml -o -name Cargo.lock -o -name rust-doctor.toml \) \
      -print0 | sort -z | xargs -0 -r sha256sum
  } | sha256sum | cut -d' ' -f1
)

write_output scan-root "$SCAN_ROOT"
write_output git-root "$GIT_ROOT"
write_output scope "$RESOLVED_SCOPE"
write_output base-sha "$BASE_SHA_VALUE"
write_output changed-paths-file "$CHANGED_PATHS_FILE"
write_output scan-cache-path "$SCAN_ROOT/.rust-doctor-cache.json"
write_output compiler-fingerprint "$COMPILER_FINGERPRINT"
write_output ruleset-fingerprint "$RULESET_FINGERPRINT"
write_output config-fingerprint "$CONFIG_FINGERPRINT"
write_output degraded-reason "$DEGRADED_REASON"
write_output skip "$SKIP_SCAN"
