set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ledger="$root/tests/windows-known-failures.txt"

suite_started() {
  tr -d '\r' < "$1" | awk '
    /Running (unittests )?[^ ]+ \(|Doc-tests [^ ]+$/ { started = 1; exit }
    /^error: could not compile / { exit }
    END { exit started ? 0 : 1 }
  '
}

compare() {
  local output="$1" status="$2" ledger="$3"
  local escape
  escape="$(printf '\033')"
  if [ "$status" -ne 0 ] && ! suite_started "$output"; then
    echo "cargo test exited $status before running the suite; no comparison was made" >&2
    return "$status"
  fi
  [ -f "$ledger" ] || { echo "missing ledger: $ledger" >&2; return 1; }
  tr -d '\r' < "$output" | sed "s/${escape}\[[0-9;?]*[A-Za-z]//g" | awk -v classes="path-form separator unix-only toolchain wall-clock" -v ledger_file="$ledger" '
    BEGIN {
      split(classes, list, " ")
      for (i in list) known_class[list[i]] = 1
    }
    FILENAME == ledger_file {
      sub(/\r$/, "")
      if ($0 == "") next
      if (NF != 3 || !($3 in known_class) || ($1 " " $2) in class) {
        malformed[++malformed_count] = FNR ": " $0
        next
      }
      key = $1 " " $2
      class[key] = $3
      order[++ledger_count] = key
      next
    }
    /^---- .* ----$/ { captured = 1; next }
    /^failures:$/ { captured = 0; next }
    captured { next }
    match($0, /Running (unittests )?[^ ]+ \(/) {
      split(substr($0, RSTART, RLENGTH), field, " ")
      target = (field[2] == "unittests") ? field[3] : field[2]
      parts = split(target, path, /[\/\\]/)
      binary = path[parts]
      sub(/\.rs$/, "", binary)
      binaries[++binary_count] = binary
      next
    }
    match($0, /Doc-tests [^ ]+$/) {
      binary = "doc"
      binaries[++binary_count] = binary
      next
    }
    /test result: / {
      finished[binary] = 1
      if (match($0, /[0-9]+ failed/)) counted[binary] += substr($0, RSTART, RLENGTH) + 0
      next
    }
    match($0, /test [^ ]+( - should panic)? \.\.\. (ok|FAILED|ignored)/) {
      fields = split(substr($0, RSTART, RLENGTH), field, " ")
      key = binary " " field[2]
      result[key] = (field[fields] == "FAILED") ? "failed" : "passed"
      if (field[fields] == "FAILED") { failed[++failed_count] = key; parsed[binary]++ }
      next
    }
    END {
      for (i = 1; i <= malformed_count; i++) {
        if (i == 1) print "malformed ledger entry (binary, test, class among: " classes "):"
        print "  " malformed[i]
      }
      for (i = 1; i <= binary_count; i++) {
        if (finished[binaries[i]] || seen_binary[binaries[i]]++) continue
        if (!unfinished++) print "did not finish:"
        print "  " binaries[i]
      }
      for (i = 1; i <= binary_count; i++) {
        b = binaries[i]
        if (!finished[b] || seen_count[b]++ || counted[b] + 0 == parsed[b] + 0) continue
        if (!uncounted++) print "failures the summary counts but no test line names:"
        print "  " b " counts " (counted[b] + 0) " failed, names " (parsed[b] + 0)
      }
      for (i = 1; i <= failed_count; i++) {
        if (failed[i] in class) continue
        if (!fresh++) print "new failure:"
        print "  " failed[i]
      }
      for (i = 1; i <= ledger_count; i++) {
        key = order[i]
        if (!(key in result) || result[key] != "passed") continue
        if (!fixed++) print "passes now, remove it:"
        print "  " key " " class[key]
      }
      for (i = 1; i <= ledger_count; i++) {
        key = order[i]
        if (key in result) continue
        if (!unknown++) print "unknown test:"
        print "  " key " " class[key]
      }
      if (malformed_count + unfinished + uncounted + fresh + fixed + unknown > 0) {
        print "windows test gate: red, the failures differ from tests/windows-known-failures.txt"
        exit 1
      }
      print "windows test gate: green, " failed_count " failures, each one listed in tests/windows-known-failures.txt"
    }
  ' "$ledger" -
}

if [ "${1:-}" = "--compare" ]; then
  [ "$#" -eq 4 ] || { echo "usage: bash scripts/windows-test-gate.sh --compare <cargo output> <cargo status> <ledger>" >&2; exit 2; }
  compare "$2" "$3" "$4"
  exit $?
fi

export RUST_DOCTOR_BENCHMARK_ALLOWANCE="${RUST_DOCTOR_BENCHMARK_ALLOWANCE:-3}"
output="$(mktemp)"
trap 'rm -f "$output"' EXIT
status=0
(cd "$root" && cargo test --locked --no-fail-fast --color never) 2>&1 | tee "$output" || status=$?
compare "$output" "$status" "$ledger"
