set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

status=0
checked=0

mapfile -t files < <(git ls-files --cached --others --exclude-standard \
  | grep -E '\.(md|rs|sh|json|ya?ml|toml|ts|js)$')

[ "${#files[@]}" -gt 0 ] || { echo "FAIL: no text file to check" >&2; exit 1; }

slugs_of() {
  sed -n 's/^#\{1,6\}[[:space:]]\{1,\}//p' "$1" \
    | tr '[:upper:]' '[:lower:]' \
    | sed 's/`//g; s/[^a-z0-9 _-]//g; s/^[[:space:]]*//; s/[[:space:]]*$//; s/[[:space:]]\{1,\}/-/g'
}

while IFS=: read -r file line ref; do
  [ -n "$ref" ] || continue
  case "$ref" in *.mdx) continue ;; esac
  checked=$((checked + 1))
  if [ ! -f "$ref" ]; then
    echo "FAIL $file:$line names \`$ref\`, which is not on disk" >&2
    status=1
  fi
done < <({
  printf '%s\n' "${files[@]}" | grep -vE '\.json$' | xargs -r grep -HnoE 'docs/[A-Za-z0-9._/-]*\.mdx?' || true
  printf '%s\n' "${files[@]}" | grep -E '\.json$' | while read -r json; do
    sed 's/^[[:space:]]*"quote":.*$//' "$json" | { grep -noE 'docs/[A-Za-z0-9._/-]*\.mdx?' || true; } | sed "s|^|$json:|"
  done
} | sort -u)

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
done < <(printf '%s\n' "${files[@]}" | grep -E '\.md$' | while read -r markdown; do
  sed 's/`[^`]*`//g' "$markdown" | { grep -noE '\]\([^) ]+\)' || true; } | sed "s|^|$markdown:|"
done | sort -u)

echo "ok   $checked documentation references resolve"
exit "$status"
