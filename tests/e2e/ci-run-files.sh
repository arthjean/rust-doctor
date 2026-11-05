set -uo pipefail

if [ "$#" -eq 0 ]; then
  printf 'usage: %s <test-file>...\n' "$0" >&2
  exit 2
fi

cd "$(dirname "$0")" || exit 1

tmux_root="${RUNNER_TEMP:-${TMPDIR:-/tmp}}/rust-doctor-e2e-tmux"
status=0
index=0

reset_tmux() {
  local socket
  for socket in "$1"/*; do
    [ -S "$socket" ] && tmux -S "$socket" kill-server 2>/dev/null
  done
  return 0
}

for test_file in "$@"; do
  tmux_dir="$tmux_root-$index"
  mkdir -p "$tmux_dir"
  if ! TMUX_TMPDIR="$tmux_dir" bun test "./$test_file"; then
    printf 'Retrying failed e2e file after resetting tmux: %s\n' "$test_file"
    reset_tmux "$tmux_dir"
    if TMUX_TMPDIR="$tmux_dir" bun test "./$test_file"; then
      printf '::warning title=E2E passed only on retry::%s failed and then passed on retry. Investigate it as a possible race.\n' "$test_file"
    else
      status=1
    fi
  fi
  reset_tmux "$tmux_dir"
  index=$((index + 1))
done

exit "$status"
