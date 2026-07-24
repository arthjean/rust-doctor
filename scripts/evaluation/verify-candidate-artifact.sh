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
environment_name=ep006-protected-evidence

jq -e '
  .schema_version == "1.0"
  and (.subject_sha256 | test("^[0-9a-f]{64}$"))
  and (.head_sha | test("^[0-9a-f]{40}$"))
  and (.run_id | type == "number" and . > 0)
  and (.artifact_id | type == "number" and . > 0)
  and (.artifact_digest | test("^sha256:[0-9a-f]{64}$"))
  and (.reviewed_by | type == "string" and length > 0)
  and (.reviewed_at | test("^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$"))
  and .review_source == "protected-environment"
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
source_author=$(gh api "repos/${repository}/pulls/${pull_request_number}" \
  --jq '.user.login')

jobs_json=$(gh api "repos/${repository}/actions/runs/${run_id}/jobs?per_page=100")
jq -e 'any(.jobs[]; .name == "corpus" and .conclusion == "success")' \
  <<<"$jobs_json" >/dev/null
corpus_job_id=$(jq -er '
  [.jobs[] | select(.name == "corpus" and .conclusion == "success")]
  | sort_by(.id)
  | last.id
' <<<"$jobs_json")
expected_job_url="https://github.com/${repository}/actions/runs/${run_id}/job/${corpus_job_id}"

deployments_json=$(gh api --method GET "repos/${repository}/deployments" \
  -f sha="$head_sha" \
  -f environment="$environment_name" \
  -f per_page=100)
deployment_id=""
while IFS= read -r candidate_deployment_id; do
  deployment_statuses=$(gh api --method GET \
    "repos/${repository}/deployments/${candidate_deployment_id}/statuses" \
    -f per_page=100)
  if jq -e --arg job_url "$expected_job_url" \
    'any(.[]; .state == "success" and .log_url == $job_url)' \
    <<<"$deployment_statuses" >/dev/null; then
    deployment_id=$candidate_deployment_id
    break
  fi
done < <(jq -r '
  [
    .[]
    | select(
        .environment == $environment
        and .sha == $sha
        and .performed_via_github_app.slug == "github-actions"
      )
  ]
  | sort_by(.id)
  | reverse[]
  | .id
' --arg environment "$environment_name" --arg sha "$head_sha" <<<"$deployments_json")
if [[ -z "$deployment_id" ]]; then
  echo "candidate corpus job has no successful protected-environment deployment" >&2
  exit 1
fi

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

current_head_sha=$(git rev-parse HEAD)
current_run_json=$(gh api "repos/${repository}/actions/runs/${GITHUB_RUN_ID:?}")
jq -e \
  --arg repository "$repository" \
  --arg sha "$current_head_sha" \
  '
    .event == "pull_request"
    and .head_repository.full_name == $repository
    and .head_sha == $sha
    and .path == ".github/workflows/ep006-promotion.yml"
  ' <<<"$current_run_json" >/dev/null
environment_json=$(gh api \
  "repos/${repository}/environments/${environment_name}")
jq -e \
  --arg author "$source_author" \
  --arg reviewer "$reviewed_by" \
  '
    .can_admins_bypass == false
    and .deployment_branch_policy.custom_branch_policies == true
    and any(
      .protection_rules[];
      .type == "required_reviewers"
      and .prevent_self_review == true
      and any(
        .reviewers[];
        .type == "User"
        and (.reviewer.login | ascii_downcase) == ($reviewer | ascii_downcase)
        and (.reviewer.login | ascii_downcase) != ($author | ascii_downcase)
      )
    )
  ' <<<"$environment_json" >/dev/null
branch_policies_json=$(gh api \
  "repos/${repository}/environments/${environment_name}/deployment-branch-policies")
jq -e \
  --arg branch "$(jq -r '.head_branch' <<<"$run_json")" \
  '
    (.branch_policies | length) == 1
    and .branch_policies[0].name == $branch
    and .branch_policies[0].type == "branch"
  ' <<<"$branch_policies_json" >/dev/null
current_jobs_json=$(gh api \
  "repos/${repository}/actions/runs/${GITHUB_RUN_ID}/jobs?per_page=100")
promotion_job_id=$(jq -er '
  [
    .jobs[]
    | select(.name == "promotion" and (.status == "in_progress" or .conclusion == "success"))
  ]
  | sort_by(.id)
  | last.id
' <<<"$current_jobs_json")
promotion_job_url="https://github.com/${repository}/actions/runs/${GITHUB_RUN_ID}/job/${promotion_job_id}"
promotion_deployments=$(gh api --method GET "repos/${repository}/deployments" \
  -f environment="$environment_name" \
  -f per_page=100)
promotion_authorized_at=""
while IFS= read -r promotion_deployment_id; do
  promotion_statuses=$(gh api --method GET \
    "repos/${repository}/deployments/${promotion_deployment_id}/statuses" \
    -f per_page=100)
  promotion_authorized_at=$(jq -er --arg job_url "$promotion_job_url" '
    [
      .[]
      | select(
          (.state == "in_progress" or .state == "success")
          and .log_url == $job_url
        )
    ]
    | sort_by(.created_at)
    | first.created_at
  ' <<<"$promotion_statuses" 2>/dev/null || true)
  if [[ -n "$promotion_authorized_at" ]]; then
    break
  fi
done < <(jq -r '
  [
    .[]
    | select(
        .environment == $environment
        and .performed_via_github_app.slug == "github-actions"
      )
  ]
  | sort_by(.id)
  | reverse[]
  | .id
' --arg environment "$environment_name" <<<"$promotion_deployments")
if [[ -z "$promotion_authorized_at" ]]; then
  echo "promotion job has no protected-environment authorization" >&2
  exit 1
fi
if ! jq -en \
  --arg reviewed_at "$reviewed_at" \
  --arg authorized_at "$promotion_authorized_at" \
  '($authorized_at | fromdateiso8601) >= ($reviewed_at | fromdateiso8601)' >/dev/null; then
  echo "protected environment authorization predates the declared candidate review" >&2
  exit 1
fi

printf 'verified %s from candidate run %s artifact %s\n' \
  "$subject_sha256" "$run_id" "$artifact_id"
