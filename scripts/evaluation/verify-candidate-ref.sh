#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 3 ]]; then
  echo "usage: $0 CANDIDATE_PR CANDIDATE_SHA TRUSTED_REPOSITORY" >&2
  exit 2
fi

candidate_pr=$1
candidate_sha=$2
trusted_repository=$3

if [[ "${GITHUB_EVENT_NAME:-}" != "workflow_dispatch" ||
      "${GITHUB_REF_NAME:-}" != "master" ]]; then
  echo "protected evidence must be dispatched from master" >&2
  exit 1
fi
if [[ ! "$candidate_pr" =~ ^[1-9][0-9]*$ ]]; then
  echo "candidate pull request must be a positive integer" >&2
  exit 1
fi
if [[ ! "$candidate_sha" =~ ^[0-9a-f]{40}$ ]]; then
  echo "candidate SHA must be a full lowercase commit SHA" >&2
  exit 1
fi
if [[ ! "${GITHUB_SHA:-}" =~ ^[0-9a-f]{40}$ ||
      -z "${GITHUB_REPOSITORY:-}" ]]; then
  echo "trusted workflow identity is unavailable" >&2
  exit 1
fi

trusted_head=$(git -C "$trusted_repository" rev-parse HEAD)
if [[ "$trusted_head" != "$GITHUB_SHA" ]]; then
  echo "trusted checkout does not match the dispatched workflow revision" >&2
  exit 1
fi
if [[ "$candidate_sha" == "$trusted_head" ]]; then
  echo "candidate SHA must differ from the trusted base revision" >&2
  exit 1
fi

pull_json=$(gh api \
  "repos/${GITHUB_REPOSITORY}/pulls/${candidate_pr}")
jq -e \
  --arg repository "$GITHUB_REPOSITORY" \
  --arg sha "$candidate_sha" \
  '
    .state == "open"
    and .base.ref == "master"
    and .head.repo.full_name == $repository
    and .head.sha == $sha
  ' <<<"$pull_json" >/dev/null

if ! git -C "$trusted_repository" cat-file -e "${candidate_sha}^{commit}" 2>/dev/null; then
  git -C "$trusted_repository" fetch --no-tags origin "$candidate_sha"
fi
if ! git -C "$trusted_repository" merge-base --is-ancestor "$trusted_head" "$candidate_sha"; then
  echo "candidate does not contain the exact trusted master revision" >&2
  exit 1
fi

printf 'verified PR #%s candidate %s from trusted revision %s\n' \
  "$candidate_pr" "$candidate_sha" "$trusted_head"
