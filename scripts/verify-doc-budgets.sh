#!/usr/bin/env bash
# Checks docs/doc-budgets.json against what is on disk.
#
# `budgets` holds the standing documents and their word ceilings. A file over
# its ceiling fails, and a file under 85 percent of it fails too, so a ceiling
# ratchets down with the words the document no longer needs rather than
# accumulating slack: a new ceiling leaves 5 percent of headroom, the band a
# document may grow in before it has to earn more.
#
# `frozen` holds the measurement records, which carry no ceiling. A markdown
# file under docs/ that is in neither list fails, because otherwise the way out
# of a ceiling is a new unlisted file. docs/doc-gates.md holds the reasoning.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
manifest="$root/docs/doc-budgets.json"
headroom_permille=1050
floor_permille=850

[ -f "$manifest" ] || { echo "missing manifest: $manifest" >&2; exit 1; }

status=0
listed=""

budgets="$(sed -n '/^  "budgets": {/,/^  },\{0,1\}$/p' "$manifest" \
  | sed -n 's/^[[:space:]]*"\([^"]*\.md\)"[[:space:]]*:[[:space:]]*\([0-9]\{1,\}\).*/\1\t\2/p')"
frozen="$(sed -n '/^  "frozen": \[/,/^  \]/p' "$manifest" \
  | sed -n 's/^[[:space:]]*"\([^"]*\.md\)".*/\1/p')"

if [ -z "$budgets" ]; then
  echo "FAIL: docs/doc-budgets.json listed no budgeted document" >&2
  exit 1
fi

while IFS='	' read -r path cap; do
  [ -n "$path" ] || continue
  listed="$listed$path"$'\n'
  file="$root/$path"
  if [ ! -f "$file" ]; then
    echo "FAIL $path: budgeted in doc-budgets.json but not on disk" >&2
    status=1
    continue
  fi
  words="$(wc -w < "$file" | tr -d ' ')"
  if [ "$words" -gt "$cap" ]; then
    echo "FAIL $path: $words words over the $cap ceiling. Move the reasoning to its own document, or raise the ceiling and justify it in the pull request." >&2
    status=1
  elif [ $((words * 1000)) -lt $((cap * floor_permille)) ]; then
    tight=$(( (words * headroom_permille + 999) / 1000 ))
    echo "FAIL $path: $words words under a $cap ceiling. Ratchet it down to $tight in docs/doc-budgets.json." >&2
    status=1
  else
    echo "ok   $path: $words / $cap"
  fi
done <<< "$budgets"

while read -r path; do
  [ -n "$path" ] || continue
  listed="$listed$path"$'\n'
  if [ ! -f "$root/$path" ]; then
    echo "FAIL $path: frozen in doc-budgets.json but not on disk" >&2
    status=1
  else
    echo "ok   $path: frozen record"
  fi
done <<< "$frozen"

while read -r path; do
  [ -n "$path" ] || continue
  case "$listed" in
    *"$path"$'\n'*) ;;
    *)
      echo "FAIL $path: not in docs/doc-budgets.json. Give it a ceiling under \"budgets\", or list it under \"frozen\" if it is a measurement record." >&2
      status=1
      ;;
  esac
done < <(cd "$root" && find docs -name '*.md' | sort)

exit "$status"
