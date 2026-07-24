#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 4 ]]; then
  echo "usage: $0 APPROVAL SUBJECT EXPECTED_ARTIFACT EXPECTED_WORKFLOW" >&2
  exit 2
fi

approval=$1
subject=$2
expected_artifact=$3
expected_workflow=$4

jq -e '
  .schema_version == "1.0"
  and (.subject_sha256 | test("^[0-9a-f]{64}$"))
  and (.head_sha | test("^[0-9a-f]{40}$"))
  and (.run_id | type == "number" and . > 0)
  and (.artifact_id | type == "number" and . > 0)
  and (.artifact_digest | test("^sha256:[0-9a-f]{64}$"))
  and (.reviewed_by | type == "string" and length > 0)
  and (.reviewed_at | type == "string" and length > 0)
  and .review_source == "protected-ci"
' "$approval" >/dev/null

repository=$(jq -r '.repository' "$approval")
head_sha=$(jq -r '.head_sha' "$approval")
run_id=$(jq -r '.run_id' "$approval")
artifact_id=$(jq -r '.artifact_id' "$approval")
artifact_name=$(jq -r '.artifact_name' "$approval")
artifact_digest=$(jq -r '.artifact_digest' "$approval")
artifact_url=$(jq -r '.artifact_url' "$approval")
subject_sha256=$(jq -r '.subject_sha256' "$approval")

if [[ "$repository" != "$GITHUB_REPOSITORY" || "$artifact_name" != "$expected_artifact" ]]; then
  echo "approval repository or artifact name does not match this gate" >&2
  exit 1
fi

expected_url="https://github.com/${repository}/actions/runs/${run_id}/artifacts/${artifact_id}"
if [[ "$artifact_url" != "$expected_url" ]]; then
  echo "approval artifact URL is not canonical" >&2
  exit 1
fi

run_json=$(gh api "repos/${repository}/actions/runs/${run_id}")
jq -e \
  --arg sha "$head_sha" \
  --arg workflow "$expected_workflow" \
  '
    .status == "completed"
    and .conclusion == "success"
    and .event == "workflow_dispatch"
    and .head_branch == "master"
    and .head_sha == $sha
    and .path == $workflow
  ' <<<"$run_json" >/dev/null

artifact_json=$(gh api "repos/${repository}/actions/artifacts/${artifact_id}")
jq -e \
  --argjson id "$artifact_id" \
  --argjson run_id "$run_id" \
  --arg name "$artifact_name" \
  --arg digest "$artifact_digest" \
  --arg head_sha "$head_sha" \
  '
    .id == $id
    and .name == $name
    and .expired == false
    and .digest == $digest
    and .workflow_run.id == $run_id
    and .workflow_run.head_sha == $head_sha
  ' <<<"$artifact_json" >/dev/null

actual_subject_sha256=$(sha256sum "$subject" | cut -d' ' -f1)
if [[ "$actual_subject_sha256" != "$subject_sha256" ]]; then
  echo "downloaded baseline does not match its approved subject SHA-256" >&2
  exit 1
fi

if ! git cat-file -e "${head_sha}^{commit}" 2>/dev/null; then
  git fetch --no-tags origin "$head_sha"
fi
git merge-base --is-ancestor "$head_sha" origin/master

printf 'verified %s from protected run %s artifact %s\n' \
  "$subject_sha256" "$run_id" "$artifact_id"
