#!/usr/bin/env bash
set -euo pipefail

MARKER='<!-- rust-doctor-report:v1 -->'
TARGET_URL="${SERVER_URL}/${REPOSITORY}/actions/runs/${RUN_ID}/attempts/${RUN_ATTEMPT}"

warn_channel() {
  echo "::warning::Rust Doctor $1 channel skipped: $2"
}

status_state() {
  if [[ "$SKIP_SCAN" == true ]]; then
    printf 'success'
  elif [[ "$EXIT_CODE" == 0 ]]; then
    printf 'success'
  elif [[ "$EXIT_CODE" == 3 ]]; then
    printf 'failure'
  else
    printf 'error'
  fi
}

status_description() {
  if [[ "$SKIP_SCAN" == true ]]; then
    printf 'Skipped: no Rust-relevant changes'
  elif [[ -s "${REPORT_FILE:-}" ]]; then
    jq -r '
      if .completeness.state != "complete" then
        "Incomplete: required analysis did not complete"
      elif .outcome == "clean" or .outcome == "nothing_to_scan" then
        "Passed: no blocking findings"
      elif .outcome == "findings" then
        "Blocked: " + (.summary.error_count | tostring) + " errors, " + (.summary.warning_count | tostring) + " warnings"
      else
        "Failed: Rust Doctor could not complete"
      end
    ' "$REPORT_FILE" | cut -c1-140
  else
    printf 'Failed: Report V1 unavailable'
  fi
}

if [[ "$COMMIT_STATUS_ENABLED" == true ]]; then
  STATE=$(status_state)
  DESCRIPTION=$(status_description)
  if ! gh api "repos/${REPOSITORY}/statuses/${COMMIT_SHA}" \
    -X POST \
    -f state="$STATE" \
    -f context='Rust Doctor' \
    -f description="$DESCRIPTION" \
    -f target_url="$TARGET_URL" >/dev/null 2>&1; then
    warn_channel status "permission denied or API unavailable"
  fi
fi

INLINE_POSTED=0
INLINE_OMITTED=0
if [[ "$REVIEW_COMMENTS_ENABLED" == true && "$EVENT_NAME" == pull_request && -s "${REPORT_FILE:-}" ]]; then
  if [[ ! "$PR_NUMBER" =~ ^[0-9]+$ || ! "$BASE_SHA" =~ ^[0-9a-fA-F]{40}$ ]]; then
    warn_channel review-comment "pull-request number or base commit is unavailable"
  else
    CHANGED_LINES=$(mktemp "${RUNNER_TEMP:-/tmp}/rust-doctor-lines.XXXXXX")
    git -c core.quotePath=false diff --unified=0 --no-color --no-ext-diff "$BASE_SHA...$COMMIT_SHA" -- 2>/dev/null | \
      perl -ne '
        if (/^\+\+\+ b\/(.*)$/) { $path=$1; next }
        if (/^@@ .* \+([0-9]+)(?:,([0-9]+))? /) {
          $start=$1;
          $count=defined($2) ? $2 : 1;
          for ($line=$start; $line<$start+$count; $line++) {
            print "$path\t$line\n";
          }
        }
      ' > "$CHANGED_LINES" || true

    EXISTING=$(mktemp "${RUNNER_TEMP:-/tmp}/rust-doctor-existing.XXXXXX")
    gh api --paginate "repos/${REPOSITORY}/pulls/${PR_NUMBER}/comments?per_page=100" \
      --jq '.[].body' > "$EXISTING" 2>/dev/null || true
    declare -A SEEN=()
    while IFS=$'\t' read -r site_id rule message path line; do
      [[ "$site_id" =~ ^[0-9a-fA-F]+$ ]] || continue
      [[ "$line" =~ ^[1-9][0-9]*$ ]] || continue
      [[ -z "${SEEN[$site_id]:-}" ]] || continue
      SEEN[$site_id]=1
      if ! awk -F $'\t' -v path="$path" -v line="$line" '$1 == path && $2 == line { found=1 } END { exit !found }' "$CHANGED_LINES"; then
        continue
      fi
      if [[ "$INLINE_POSTED" -ge 50 ]]; then
        INLINE_OMITTED=$((INLINE_OMITTED + 1))
        continue
      fi
      inline_marker="<!-- rust-doctor-inline:${site_id} -->"
      if grep -Fq "$inline_marker" "$EXISTING"; then
        continue
      fi
      safe_rule=$(printf '%s' "$rule" | tr -d '\000-\010\013\014\016-\037\177' | sed 's/`/\\`/g; s/</\&lt;/g; s/>/\&gt;/g')
      safe_message=$(printf '%s' "$message" | tr '\n\r\t' '   ' | tr -d '\000-\010\013\014\016-\037\177' | sed 's/`/\\`/g; s/</\&lt;/g; s/>/\&gt;/g')
      body=$(printf '%s\n**%s**: %s' "$inline_marker" "$safe_rule" "$safe_message")
      if gh api "repos/${REPOSITORY}/pulls/${PR_NUMBER}/comments" \
        -X POST \
        -f body="$body" \
        -f commit_id="$COMMIT_SHA" \
        -f path="$path" \
        -F line="$line" \
        -f side=RIGHT >/dev/null 2>&1; then
        INLINE_POSTED=$((INLINE_POSTED + 1))
      else
        warn_channel review-comment "permission denied or changed-line position rejected"
        break
      fi
    done < <(
      jq -r '.diagnostics[] | select(.visible_on | index("pr-comment")) | select(.location.kind == "source") | [.site_id, .rule, .message, .location.path, .location.range.start.line] | @tsv' "$REPORT_FILE"
    )
    rm -f "$CHANGED_LINES" "$EXISTING"
  fi
fi

if [[ "$COMMENT_ENABLED" == true && "$EVENT_NAME" == pull_request ]]; then
  if [[ ! "$PR_NUMBER" =~ ^[0-9]+$ ]]; then
    warn_channel comment "pull-request number is unavailable"
  elif [[ ! -s "${REPORT_FILE:-}" ]]; then
    warn_channel comment "Report V1 is unavailable"
  else
    COMMENT_FILE=$(mktemp "${RUNNER_TEMP:-/tmp}/rust-doctor-comment.XXXXXX.md")
    jq -r \
      --arg marker "$MARKER" \
      --arg target "$TARGET_URL" \
      --argjson inline_posted "$INLINE_POSTED" \
      --argjson inline_omitted "$INLINE_OMITTED" '
      def safe:
        tostring
        | gsub("[\\u0000-\\u001f\\u007f]"; " ")
        | gsub("\\|"; "\\\\|")
        | gsub("`"; "\\\\`")
        | gsub("<"; "&lt;")
        | gsub(">"; "&gt;");
      def score: if .summary.score == null then "unavailable" else (.summary.score | tostring) + "/100" end;
      def baseline: .baseline // {new_count: .summary.diagnostic_count, fixed_count: 0};
      def pr_diagnostics: [.diagnostics[] | select(.visible_on | index("pr-comment"))];
      def packages: ([.projects[].cargo_package_id] | unique | map(safe) | join(", "));
      def top_rules:
        [pr_diagnostics | group_by(.rule)[] | {rule: .[0].rule, count: length}]
        | sort_by(-.count, .rule)
        | .[:5]
        | if length == 0 then "None" else map((.rule | safe) + " (" + (.count | tostring) + ")") | join(", ") end;
      $marker + "\n" +
      "## Rust Doctor\n\n" +
      "| Metric | Result |\n|---|---:|\n" +
      "| Completeness | " + (.completeness.state | safe) + " |\n" +
      "| Score | " + score + " |\n" +
      "| Introduced | " + (pr_diagnostics | length | tostring) + " |\n" +
      "| Fixed | " + (baseline.fixed_count | tostring) + " |\n" +
      "| Errors | " + (pr_diagnostics | map(select(.severity == "error")) | length | tostring) + " |\n" +
      "| Warnings | " + (pr_diagnostics | map(select(.severity == "warning")) | length | tostring) + " |\n\n" +
      "Affected packages: " + packages + "\n\n" +
      "Top rule groups: " + top_rules + "\n\n" +
      (if $inline_posted + $inline_omitted > 0 then
        "Inline findings: " + ($inline_posted | tostring) + " posted, " + ($inline_omitted | tostring) + " omitted by the 50-comment cap.\n\n"
       else "" end) +
      "[Open workflow run](" + $target + ")"
    ' "$REPORT_FILE" > "$COMMENT_FILE"

    COMMENT_ID=$(gh api --paginate "repos/${REPOSITORY}/issues/${PR_NUMBER}/comments?per_page=100" \
      --jq ".[] | select(.body | contains(\"$MARKER\")) | .id" 2>/dev/null | head -n1 || true)
    if [[ -n "$COMMENT_ID" ]]; then
      if ! gh api "repos/${REPOSITORY}/issues/comments/${COMMENT_ID}" \
        -X PATCH -F body=@"$COMMENT_FILE" >/dev/null 2>&1; then
        warn_channel comment "permission denied while updating sticky summary"
      fi
    elif ! gh api "repos/${REPOSITORY}/issues/${PR_NUMBER}/comments" \
      -X POST -F body=@"$COMMENT_FILE" >/dev/null 2>&1; then
      warn_channel comment "permission denied while creating sticky summary"
    fi
    rm -f "$COMMENT_FILE"
  fi
fi
