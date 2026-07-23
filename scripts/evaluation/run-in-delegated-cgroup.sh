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
  parent=/sys/fs/cgroup
else
  parent="/sys/fs/cgroup$relative_cgroup"
fi
delegated="$parent/rust-doctor-delegated-$$"
delegated_created=false

cleanup() {
  local cleanup_status=0
  if [[ "$delegated_created" == true ]]; then
    sudo rmdir -- "$delegated" || cleanup_status=$?
  fi
  return "$cleanup_status"
}
trap cleanup EXIT

for controller in memory pids; do
  if ! grep -qw "$controller" "$parent/cgroup.controllers"; then
    echo "required cgroup controller is unavailable: $controller" >&2
    exit 1
  fi
done

sudo mkdir -- "$delegated"
delegated_created=true
sudo sh -c 'printf "%s\n" "+memory +pids" > "$1/cgroup.subtree_control"' sh "$delegated"
sudo chown "$(id -u):$(id -g)" \
  "$delegated" \
  "$delegated/cgroup.procs" \
  "$delegated/cgroup.threads" \
  "$delegated/cgroup.subtree_control"

set +e
RUST_DOCTOR_CGROUP_ROOT="$delegated" "$@"
command_status=$?
cleanup_status=0
cleanup || cleanup_status=$?
trap - EXIT
set -e

if [[ "$command_status" -ne 0 ]]; then
  exit "$command_status"
fi
exit "$cleanup_status"
