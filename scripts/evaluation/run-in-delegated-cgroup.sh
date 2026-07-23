#!/usr/bin/env bash

set -euo pipefail

if [[ $# -eq 0 ]]; then
  echo "usage: run-in-delegated-cgroup.sh COMMAND [ARG...]" >&2
  exit 64
fi

relative_cgroup=$(
  awk -F: '$1 == "0" && $2 == "" { print $3; exit }' /proc/self/cgroup
)
if [[ -z "$relative_cgroup" ]]; then
  echo "unified cgroup v2 membership is unavailable" >&2
  exit 1
fi

if [[ "$relative_cgroup" == "/" ]]; then
  current=/sys/fs/cgroup
else
  current="/sys/fs/cgroup$relative_cgroup"
fi
parent="$current"
while true; do
  enabled=$(<"$parent/cgroup.subtree_control")
  if grep -qw memory <<<"$enabled" && grep -qw pids <<<"$enabled"; then
    break
  fi
  if [[ "$parent" == /sys/fs/cgroup ]]; then
    echo "memory and pids controllers are not delegated by the cgroup v2 root" >&2
    exit 1
  fi
  parent=${parent%/*}
done
delegated="$parent/rust-doctor-delegated-$$"
runner="$delegated/runner"
delegated_created=false
runner_created=false

cleanup() {
  local cleanup_status=0
  if [[ "$runner_created" == true ]]; then
    sudo rmdir -- "$runner" || cleanup_status=$?
  fi
  if [[ "$delegated_created" == true ]]; then
    sudo rmdir -- "$delegated" || cleanup_status=$?
  fi
  return "$cleanup_status"
}
trap cleanup EXIT

sudo mkdir -- "$delegated"
delegated_created=true
for controller in memory pids; do
  if ! grep -qw "$controller" "$delegated/cgroup.controllers"; then
    echo "required cgroup controller was not delegated: $controller" >&2
    exit 1
  fi
done
sudo sh -c 'printf "%s\n" "+memory +pids" > "$1/cgroup.subtree_control"' sh "$delegated"
sudo chown "$(id -u):$(id -g)" \
  "$delegated" \
  "$delegated/cgroup.procs" \
  "$delegated/cgroup.threads" \
  "$delegated/cgroup.subtree_control"
sudo mkdir -- "$runner"
runner_created=true
sudo chown "$(id -u):$(id -g)" \
  "$runner" \
  "$runner/cgroup.procs" \
  "$runner/cgroup.threads"

set +e
sudo sh -c '
  runner=$1
  delegated=$2
  uid=$3
  shift 3
  printf "%s\n" "$$" > "$runner/cgroup.procs"
  exec sudo --non-interactive --user="#$uid" -- \
    env RUST_DOCTOR_CGROUP_ROOT="$delegated" "$@"
' sh "$runner" "$delegated" "$(id -u)" "$@"
command_status=$?
cleanup_status=0
cleanup || cleanup_status=$?
trap - EXIT
set -e

if [[ "$command_status" -ne 0 ]]; then
  exit "$command_status"
fi
exit "$cleanup_status"
