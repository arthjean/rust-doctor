#!/usr/bin/env bash
# Fails on a documentation reference that no longer resolves.
#
# Two kinds are checked. A path under `docs/` named anywhere in the repository,
# source comments and skills and workflows included, has to be a file on disk.
# A relative markdown link between two documents has to resolve, and so does the
# `#fragment` of one, against the headings of the document it points into.
#
# Renaming a document breaks every citation of it in silence: the reader follows
# a path, opens nothing, and falls back to the map. docs/doc-gates.md holds the
# reasoning.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

status=0
checked=0

# Tracked files plus new ones, minus anything gitignored, so a document added in
# the working tree is checked before it is committed.
mapfile -t files < <(git ls-files --cached --others --exclude-standard \
  | grep -E '\.(md|rs|sh|json|ya?ml|toml|ts|js)$')

[ "${#files[@]}" -gt 0 ] || { echo "FAIL: no text file to check" >&2; exit 1; }

# The heading anchors of a markdown file, slugged the way a markdown renderer
# slugs them: lowercased, punctuation dropped, spaces hyphenated.
slugs_of() {
  sed -n 's/^#\{1,6\}[[:space:]]\{1,\}//p' "$1" \
    | tr '[:upper:]' '[:lower:]' \
    | sed 's/`//g; s/[^a-z0-9 _-]//g; s/^[[:space:]]*//; s/[[:space:]]*$//; s/[[:space:]]\{1,\}/-/g'
}

while IFS=: read -r file line ref; do
  [ -n "$ref" ] || continue
  checked=$((checked + 1))
  if [ ! -f "$ref" ]; then
    echo "FAIL $file:$line names \`$ref\`, which is not on disk" >&2
    status=1
  fi
done < <(grep -HnoE 'docs/[A-Za-z0-9._/-]*\.md' -- "${files[@]}" 2>/dev/null | sort -u)

while IFS=: read -r file line link; do
  link="${link#](}"
  link="${link%)}"
  case "$link" in
    http://*|https://*|mailto:*) continue ;;
  esac
  target="${link%%#*}"
  fragment=""
  case "$link" in *#*) fragment="${link#*#}" ;; esac
  if [ -n "$target" ]; then
    path="$(dirname "$file")/$target"
  else
    path="$file"
  fi
  checked=$((checked + 1))
  if [ ! -e "$path" ]; then
    echo "FAIL $file:$line links to \`$link\`, which is not on disk" >&2
    status=1
    continue
  fi
  if [ -n "$fragment" ] && [ -f "$path" ]; then
    case "$path" in
      *.md)
        if ! slugs_of "$path" | grep -qx -- "$fragment"; then
          echo "FAIL $file:$line links to \`#$fragment\`, which is not a heading of $target" >&2
          status=1
        fi
        ;;
    esac
  fi
done < <(printf '%s\n' "${files[@]}" | grep -E '\.md$' \
  | xargs grep -HnoE '\]\([^) ]+\)' 2>/dev/null | sort -u)

echo "ok   $checked documentation references resolve"
exit "$status"
