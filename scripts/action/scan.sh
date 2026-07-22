#!/usr/bin/env bash
set -euo pipefail

write_output() {
  printf '%s=%s\n' "$1" "$2" >> "$GITHUB_OUTPUT"
}

if [[ "$SKIP_SCAN" == true ]]; then
  write_output score ""
  write_output errors 0
  write_output warnings 0
  write_output outcome skipped
  write_output completeness complete
  write_output report-file ""
  write_output exit-code 0
  exit 0
fi

case "$INPUT_BLOCKING" in
  error|warning|info|none) ;;
  *) echo "::error::blocking must be error, warning, info, or none"; exit 2 ;;
esac
case "$INPUT_REQUIRE_COMPLETE" in
  true|false) ;;
  *) echo "::error::require-complete must be true or false"; exit 2 ;;
esac

REPORT_FILE=$(mktemp "$RUNNER_TEMP/rust-doctor-report.XXXXXX.json")
ARGS=("$SCAN_ROOT" --json-out "$REPORT_FILE" --blocking "$INPUT_BLOCKING")
if [[ "$INPUT_REQUIRE_COMPLETE" == true ]]; then
  ARGS+=(--require-complete)
fi
if [[ -n "$INPUT_PROJECT" ]]; then
  ARGS+=(--project "$INPUT_PROJECT")
fi
if [[ -n "$INPUT_MAX_DURATION" ]]; then
  [[ "$INPUT_MAX_DURATION" =~ ^[1-9][0-9]*$ ]] || {
    echo "::error::max-duration must be a positive integer"
    exit 2
  }
  ARGS+=(--max-duration "$INPUT_MAX_DURATION")
fi

case "$RESOLVED_SCOPE" in
  full) ARGS+=(--scope full) ;;
  changed) ARGS+=(--scope changed --base "$BASE_SHA") ;;
  baseline) ARGS+=(--baseline --base "$BASE_SHA") ;;
  staged) ARGS+=(--staged) ;;
  api-files)
    ARGS+=(--scope files)
    FILE_COUNT=0
    while IFS= read -r -d '' path; do
      case "$path" in
        *.rs)
          candidate="$path"
          if [[ "$candidate" != /* ]]; then
            candidate="$(git -C "$SCAN_ROOT" rev-parse --show-toplevel)/$candidate"
          fi
          if [[ "$candidate" == "$SCAN_ROOT"/* ]]; then
            ARGS+=(--files "$candidate")
            FILE_COUNT=$((FILE_COUNT + 1))
          fi
          ;;
      esac
    done < "$CHANGED_PATHS_FILE"
    if [[ "$FILE_COUNT" -eq 0 ]]; then
      ARGS=("$SCAN_ROOT" --json-out "$REPORT_FILE" --blocking "$INPUT_BLOCKING" --scope full)
      [[ "$INPUT_REQUIRE_COMPLETE" == true ]] && ARGS+=(--require-complete)
    fi
    ;;
  *) echo "::error::internal invalid resolved scope '$RESOLVED_SCOPE'"; exit 2 ;;
esac

if [[ -n "$DEGRADED_REASON" ]]; then
  echo "::warning::$DEGRADED_REASON"
fi

set +e
rust-doctor "${ARGS[@]}"
EXIT_CODE=$?
set -e
[[ "$EXIT_CODE" =~ ^[0-4]$ ]] || EXIT_CODE=2

if [[ ! -s "$REPORT_FILE" ]] || ! jq -e '.schema_version | startswith("1")' "$REPORT_FILE" >/dev/null 2>&1; then
  echo "::error::rust-doctor did not produce a valid Report V1"
  rm -f "$REPORT_FILE"
  write_output score ""
  write_output errors 0
  write_output warnings 0
  write_output outcome failed
  write_output completeness incomplete
  write_output report-file ""
  write_output exit-code 2
  exit 0
fi

SCORE=$(jq -r '.summary.score // empty' "$REPORT_FILE")
ERRORS=$(jq -r '.summary.error_count // 0' "$REPORT_FILE")
WARNINGS=$(jq -r '.summary.warning_count // 0' "$REPORT_FILE")
OUTCOME=$(jq -r '.outcome' "$REPORT_FILE")
COMPLETENESS=$(jq -r '.completeness.state' "$REPORT_FILE")
[[ "$SCORE" =~ ^[0-9]+$ || -z "$SCORE" ]] || SCORE=""
[[ "$ERRORS" =~ ^[0-9]+$ ]] || ERRORS=0
[[ "$WARNINGS" =~ ^[0-9]+$ ]] || WARNINGS=0

write_output score "$SCORE"
write_output errors "$ERRORS"
write_output warnings "$WARNINGS"
write_output outcome "$OUTCOME"
write_output completeness "$COMPLETENESS"
write_output report-file "$REPORT_FILE"
write_output exit-code "$EXIT_CODE"
