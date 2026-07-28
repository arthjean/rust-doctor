#!/usr/bin/env bash
set -euo pipefail

MARKER='<!-- rust-doctor-report:v1 -->'
INLINE_PREFIX='<!-- rust-doctor-inline:'
REVIEW_MARKER='<!-- rust-doctor-review:v1 -->'
TARGET_URL="${SERVER_URL}/${REPOSITORY}/actions/runs/${RUN_ID}/attempts/${RUN_ATTEMPT}"

warn_channel() {
  echo "::warning::Rust Doctor $1 channel skipped: $2"
}

status_state() {
  if [[ "$SKIP_SCAN" == true ]]; then
    printf 'success'
  elif [[ -n "${DEGRADED_REASON:-}" ]]; then
    printf 'error'
  elif [[ "$EXIT_CODE" == 0 ]]; then
    printf 'success'
  elif [[ -s "${REPORT_FILE:-}" ]] && jq -e '.gate_result == "failed"' "$REPORT_FILE" >/dev/null 2>&1; then
    printf 'failure'
  else
    printf 'error'
  fi
}

status_description() {
  if [[ "$SKIP_SCAN" == true ]]; then
    printf 'Skipped: no Rust-relevant changes'
  elif [[ -n "${DEGRADED_REASON:-}" ]]; then
    printf 'Incomplete: requested scope degraded'
  elif [[ -s "${REPORT_FILE:-}" ]]; then
    jq -r '
      if (.summary.score == null and (.summary.score_reasons | length) > 0 and (.outcome == "partial" or .outcome == "failed")) then
        "Incomplete: " + .summary.score_reasons[0]
      elif .completeness.state != "complete" then
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

if [[ -n "${GITHUB_STEP_SUMMARY:-}" && -s "${REPORT_FILE:-}" ]]; then
  jq -r '
    def score:
      if .summary.score == null then "n/a" else (.summary.score | tostring) end;
    def score_label:
      if .summary.score_label == null then "unavailable" else .summary.score_label end;
    def authority:
      if (.summary.score_reasons | length) == 0
      then "authoritative"
      else (.summary.score_reasons | join(", "))
      end;
    "## Rust Doctor decision\n\n" +
    score + "/100 (" + score_label + ")\n\n" +
    "Authority: " + authority + "\n\n" +
    (.summary.error_count | tostring) + " errors | " +
    (.summary.warning_count | tostring) + " warnings | " +
    (.summary.info_count | tostring) + " info\n\n" +
    ([.diagnostics[] | select(.score_impact == "scored")] | length | tostring) + " scored | " +
    ([.diagnostics[] | select(.score_impact != "scored" and .trust_tier != "audit-only")] | length | tostring) + " advisory | " +
    ([.diagnostics[] | select(.trust_tier == "audit-only")] | length | tostring) + " audit\n\n" +
    "### Top remediations\n" +
    ([((.root_causes // [])[:3])[] | "- `" + .rule + "` [" + .key + "]: " + .title] | join("\n"))
  ' "$REPORT_FILE" >> "$GITHUB_STEP_SUMMARY"
fi

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

SCAN_PREFIX=$(realpath --relative-to="$GIT_ROOT" "$SCAN_ROOT")

build_changed_lines() {
  local destination=$1
  local ndjson
  ndjson=$(mktemp "${RUNNER_TEMP:-/tmp}/rust-doctor-lines.XXXXXX.ndjson")
  while IFS= read -r -d '' path; do
    while IFS= read -r line; do
      [[ "$line" =~ ^[1-9][0-9]*$ ]] || continue
      jq -cn --arg path "$path" --argjson line "$line" '{path: $path, line: $line}' >> "$ndjson"
    done < <(
      git diff --unified=3 --no-color --no-ext-diff "$BASE_SHA...$COMMIT_SHA" -- "$path" 2>/dev/null |
        awk '
          /^@@ / {
            split($0, fields, " ")
            right = fields[3]
            sub(/^\+/, "", right)
            sub(/,.*/, "", right)
            right += 0
            in_hunk = 1
            next
          }
          in_hunk && /^\\/ { next }
          in_hunk && /^-/ { next }
          in_hunk && /^\+/ { print right; right += 1; next }
          in_hunk && /^ / { print right; right += 1 }
        '
    )
  done < <(git diff --name-only -z "$BASE_SHA...$COMMIT_SHA" --)
  jq -s 'unique_by([.path, .line])' "$ndjson" > "$destination"
  rm -f "$ndjson"
}

INLINE_POSTED=0
INLINE_OMITTED=0
if [[ "$REVIEW_COMMENTS_ENABLED" == true && "$EVENT_NAME" == pull_request && -s "${REPORT_FILE:-}" ]]; then
  if [[ ! "$PR_NUMBER" =~ ^[0-9]+$ || ! "$BASE_SHA" =~ ^[0-9a-fA-F]{40}$ ]]; then
    warn_channel review-comment "pull-request number or base commit is unavailable"
  elif ! jq -e '
    .report_constructed == true
    and .completeness.state == "complete"
    and (.outcome != "partial" and .outcome != "failed")
  ' "$REPORT_FILE" >/dev/null; then
    warn_channel review-comment "replacement report is incomplete; preserving prior inline findings"
  else
    CHANGED_LINES=$(mktemp "${RUNNER_TEMP:-/tmp}/rust-doctor-lines.XXXXXX.json")
    COMMENTS_ALL=$(mktemp "${RUNNER_TEMP:-/tmp}/rust-doctor-comments.XXXXXX.json")
    COMMENTS_NEW=$(mktemp "${RUNNER_TEMP:-/tmp}/rust-doctor-comments-new.XXXXXX.json")
    EXISTING_PAGES=$(mktemp "${RUNNER_TEMP:-/tmp}/rust-doctor-existing-pages.XXXXXX.json")
    EXISTING=$(mktemp "${RUNNER_TEMP:-/tmp}/rust-doctor-existing.XXXXXX.json")
    REVIEW_PAYLOAD=$(mktemp "${RUNNER_TEMP:-/tmp}/rust-doctor-review.XXXXXX.json")
    build_changed_lines "$CHANGED_LINES"
    jq \
      --arg git_root "$GIT_ROOT" \
      --arg scan_root "$SCAN_ROOT" \
      --arg scan_prefix "$SCAN_PREFIX" \
      --slurpfile changed "$CHANGED_LINES" '
      def safe:
        tostring
        | gsub("[\u0000-\u001f\u007f]"; " ")
        | gsub("`"; "\\\\`")
        | gsub("<"; "&lt;")
        | gsub(">"; "&gt;");
      def repository_path:
        if startswith($git_root + "/") then .[($git_root | length) + 1:]
        elif startswith($scan_root + "/") then
          (if $scan_prefix == "." then "" else $scan_prefix + "/" end)
          + .[($scan_root | length) + 1:]
        elif startswith("/") then null
        elif $scan_prefix == "." then .
        else $scan_prefix + "/" + .
        end;
      [
        # `.diagnostics` already arrives in the canonical priority order every
        # Rust Doctor surface shares. `order` preserves it through the
        # deduplication step instead of re-sorting by path (US-015 AC-1).
        (.diagnostics | to_entries[] | .value + {rust_doctor_order: .key})
        | select(.visible_on | index("pr-comment"))
        | select(.location.kind == "source")
        | (.location.path | repository_path) as $path
        | select($path != null)
        | .location.range.start.line as $line
        | select(any($changed[0][]; .path == $path and .line == $line))
        | {
            site_id,
            rust_doctor_order,
            path: $path,
            line: $line,
            side: "RIGHT",
            body: ("<!-- rust-doctor-inline:" + .site_id + " -->\n**" + (.rule | safe) + "**: " + (.message | safe))
          }
      ]
      | unique_by(.site_id)
      | sort_by(.rust_doctor_order)
      | map(del(.rust_doctor_order))
    ' "$REPORT_FILE" > "$COMMENTS_ALL"
    COMMENT_COUNT=$(jq 'length' "$COMMENTS_ALL")
    if (( COMMENT_COUNT > 50 )); then
      INLINE_OMITTED=$((COMMENT_COUNT - 50))
    fi
    jq '.[0:50] | map(del(.site_id))' "$COMMENTS_ALL" > "$COMMENTS_NEW"

    EXISTING_READY=true
    if gh api --paginate --slurp \
      "repos/${REPOSITORY}/pulls/${PR_NUMBER}/comments?per_page=100" > "$EXISTING_PAGES" 2>/dev/null; then
      jq --arg prefix "$INLINE_PREFIX" \
        '[.[][] | select(.body | startswith($prefix)) | {id, body}]' \
        "$EXISTING_PAGES" > "$EXISTING"
    else
      EXISTING_READY=false
      warn_channel review-comment "existing inline findings could not be loaded"
    fi

    NEW_COUNT=$(jq 'length' "$COMMENTS_NEW")
    OLD_COUNT=0
    if [[ "$EXISTING_READY" == true ]]; then
      OLD_COUNT=$(jq 'length' "$EXISTING")
    fi
    if [[ "$EXISTING_READY" == true && ( "$NEW_COUNT" -gt 0 || "$OLD_COUNT" -gt 0 ) ]]; then
      jq -n --arg body "$REVIEW_MARKER" --slurpfile comments "$COMMENTS_NEW" \
        '{body: $body, event: "COMMENT", comments: $comments[0]}' > "$REVIEW_PAYLOAD"
      if gh api "repos/${REPOSITORY}/pulls/${PR_NUMBER}/reviews" \
        -X POST --input "$REVIEW_PAYLOAD" >/dev/null 2>&1; then
        INLINE_POSTED=$NEW_COUNT
        while IFS= read -r comment_id; do
          if ! gh api "repos/${REPOSITORY}/pulls/comments/${comment_id}" \
            -X DELETE >/dev/null 2>&1; then
            warn_channel review-comment "new review published but an older inline comment could not be retired"
          fi
        done < <(jq -r '.[].id' "$EXISTING")
      else
        warn_channel review-comment "permission denied or changed-line positions rejected; preserving prior inline findings"
      fi
    fi
    rm -f "$CHANGED_LINES" "$COMMENTS_ALL" "$COMMENTS_NEW" "$EXISTING_PAGES" "$EXISTING" "$REVIEW_PAYLOAD"
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
      --arg degraded "${DEGRADED_REASON:-}" \
      --argjson inline_posted "$INLINE_POSTED" \
      --argjson inline_omitted "$INLINE_OMITTED" '
      def safe:
        tostring
        | gsub("[\u0000-\u001f\u007f]"; " ")
        | gsub("\\|"; "\\\\|")
        | gsub("`"; "\\\\`")
        | gsub("<"; "&lt;")
        | gsub(">"; "&gt;");
      def score: if .summary.score == null then "unavailable" else (.summary.score | tostring) + "/100" end;
      def authority_reason: if (.summary.score_reasons | length) == 0 then "none" else (.summary.score_reasons[0] | safe) end;
      def baseline_available: .mode == "baseline" and .baseline != null and $degraded == "";
      def introduced: if baseline_available then (.baseline.new_count | tostring) else "unavailable" end;
      def fixed: if baseline_available then (.baseline.fixed_count | tostring) else "unavailable" end;
      def pr_diagnostics: [.diagnostics[] | select(.visible_on | index("pr-comment"))];
      def packages: ([.projects[].cargo_package_id] | unique | map(safe) | join(", "));
      def top_rules:
        reduce pr_diagnostics[] as $diagnostic
          ({order: [], groups: {}};
            ($diagnostic.root_cause_key // $diagnostic.rule) as $key
            | if .groups[$key] then
                .groups[$key].count += 1
              else
                .order += [$key]
                | .groups[$key] = {rule: $diagnostic.rule, count: 1}
              end
          )
        | [.order[] as $key | .groups[$key]]
        | .[:5]
        | if length == 0 then "None" else map((.rule | safe) + " (" + (.count | tostring) + ")") | join(", ") end;
      $marker + "\n" +
      "## Rust Doctor\n\n" +
      "| Metric | Result |\n|---|---:|\n" +
      "| Completeness | " + (.completeness.state | safe) + " |\n" +
      "| Score | " + score + " |\n" +
      "| Score authority | " + authority_reason + " |\n" +
      "| Introduced | " + introduced + " |\n" +
      "| Fixed | " + fixed + " |\n" +
      "| Errors | " + (pr_diagnostics | map(select(.severity == "error")) | length | tostring) + " |\n" +
      "| Warnings | " + (pr_diagnostics | map(select(.severity == "warning")) | length | tostring) + " |\n\n" +
      (if $degraded == "" then "" else "Scope degraded: " + ($degraded | safe) + "\n\n" end) +
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
