#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 5 ]]; then
  echo "usage: $0 APPROVAL SUBJECT EXPECTED_ARTIFACT EXPECTED_WORKFLOW EXPECTED_RUN_ID" >&2
  exit 2
fi

approval=$1
subject=$2
expected_artifact=$3
expected_workflow=$4
expected_run_id=$5

jq -e '
  .schema_version == "1.0"
  and (.subject_sha256 | test("^[0-9a-f]{64}$"))
  and (.head_sha | test("^[0-9a-f]{40}$"))
  and (.run_id | type == "number" and . > 0)
  and (.artifact_id | type == "number" and . > 0)
  and (.artifact_digest | test("^sha256:[0-9a-f]{64}$"))
  and (.reviewed_by | type == "string" and length > 0)
  and (.reviewed_at | test("^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$"))
  and .review_source == "codeowners"
' "$approval" >/dev/null

repository=$(jq -r '.repository' "$approval")
head_sha=$(jq -r '.head_sha' "$approval")
run_id=$(jq -r '.run_id' "$approval")
artifact_id=$(jq -r '.artifact_id' "$approval")
artifact_name=$(jq -r '.artifact_name' "$approval")
artifact_digest=$(jq -r '.artifact_digest' "$approval")
artifact_url=$(jq -r '.artifact_url' "$approval")
subject_sha256=$(jq -r '.subject_sha256' "$approval")
reviewed_by=$(jq -r '.reviewed_by' "$approval")
reviewed_at=$(jq -r '.reviewed_at' "$approval")

if [[ "$run_id" != "$expected_run_id" ]]; then
  echo "candidate approval run ID does not match the requested immutable run" >&2
  exit 1
fi
if [[ "$repository" != "$GITHUB_REPOSITORY" || "$artifact_name" != "$expected_artifact" ]]; then
  echo "candidate approval repository or artifact name does not match this gate" >&2
  exit 1
fi

expected_url="https://github.com/${repository}/actions/runs/${run_id}/artifacts/${artifact_id}"
if [[ "$artifact_url" != "$expected_url" ]]; then
  echo "candidate approval artifact URL is not canonical" >&2
  exit 1
fi

run_json=$(gh api "repos/${repository}/actions/runs/${run_id}")
jq -e \
  --arg repository "$repository" \
  --arg sha "$head_sha" \
  --arg workflow "$expected_workflow" \
  '
    .status == "completed"
    and .event == "pull_request"
    and .head_repository.full_name == $repository
    and .head_sha == $sha
    and .path == $workflow
    and (.pull_requests | length == 1)
    and (.pull_requests[0].head.sha | test("^[0-9a-f]{40}$"))
  ' <<<"$run_json" >/dev/null
source_head_sha=$(jq -r '.pull_requests[0].head.sha' <<<"$run_json")
pull_request_number=$(jq -r '.pull_requests[0].number' <<<"$run_json")

jobs_json=$(gh api "repos/${repository}/actions/runs/${run_id}/jobs?per_page=100")
jq -e 'any(.jobs[]; .name == "corpus" and .conclusion == "success")' \
  <<<"$jobs_json" >/dev/null

artifact_json=$(gh api "repos/${repository}/actions/artifacts/${artifact_id}")
jq -e \
  --argjson id "$artifact_id" \
  --argjson run_id "$run_id" \
  --arg sha "$head_sha" \
  --arg name "$artifact_name" \
  --arg digest "$artifact_digest" \
  '
    .id == $id
    and .name == $name
    and .expired == false
    and .digest == $digest
    and .workflow_run.id == $run_id
    and .workflow_run.head_sha == $sha
  ' <<<"$artifact_json" >/dev/null

actual_subject_sha256=$(sha256sum "$subject" | cut -d' ' -f1)
if [[ "$actual_subject_sha256" != "$subject_sha256" ]]; then
  echo "downloaded candidate does not match its approved subject SHA-256" >&2
  exit 1
fi

if ! git cat-file -e "${source_head_sha}^{commit}" 2>/dev/null; then
  git fetch --no-tags origin "$source_head_sha"
fi
git merge-base --is-ancestor "$source_head_sha" HEAD
while IFS= read -r changed_path; do
  case "$changed_path" in
    evaluation/approvals/ep006-candidate.json | evaluation/approvals/ep006-labels.json | tasks/prd-react-doctor-parity-status.json)
      ;;
    *)
      echo "candidate evidence is stale because $changed_path changed after the corpus run" >&2
      exit 1
      ;;
  esac
done < <(git diff --name-only "$source_head_sha" HEAD)

reviews_json=$(gh api "repos/${repository}/pulls/${pull_request_number}/reviews?per_page=100")
review_json=$(jq -c \
  --arg reviewer "$reviewed_by" \
  '
    [
      .[]
      | select(
          .state == "APPROVED"
          and (.user.login | ascii_downcase) == ($reviewer | ascii_downcase)
          and (.commit_id | test("^[0-9a-f]{40}$"))
        )
    ]
    | sort_by(.submitted_at)
    | last // empty
  ' <<<"$reviews_json")
if [[ -z "$review_json" ]]; then
  echo "candidate approval has no matching APPROVED GitHub review" >&2
  exit 1
fi
review_commit=$(jq -r '.commit_id' <<<"$review_json")
review_submitted_at=$(jq -r '.submitted_at' <<<"$review_json")
if ! jq -en \
  --arg reviewed_at "$reviewed_at" \
  --arg submitted_at "$review_submitted_at" \
  '($submitted_at | fromdateiso8601) >= ($reviewed_at | fromdateiso8601)' >/dev/null; then
  echo "GitHub review predates the declared candidate review" >&2
  exit 1
fi
if ! git cat-file -e "${review_commit}^{commit}" 2>/dev/null; then
  git fetch --no-tags origin "$review_commit"
fi
git merge-base --is-ancestor "$source_head_sha" "$review_commit"
git merge-base --is-ancestor "$review_commit" HEAD
while IFS= read -r changed_path; do
  if [[ "$changed_path" != "tasks/prd-react-doctor-parity-status.json" ]]; then
    echo "only status evidence may change after the approved GitHub review" >&2
    exit 1
  fi
done < <(git diff --name-only "$review_commit" HEAD)

printf 'verified %s from candidate run %s artifact %s\n' \
  "$subject_sha256" "$run_id" "$artifact_id"
