#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 4 ]]; then
  echo "usage: assemble-certification-evidence.sh <release-binary> <tool-revision> <generated-at> <output>" >&2
  exit 2
fi

release_binary=$1
tool_revision=$2
generated_at=$3
output=$4
binary_sha256=$(sha256sum "$release_binary" | cut -d' ' -f1)
gates=(
  quality-gates
  cross-surface
  artifact-smoke
  workspace-scale
  decision-overhead
  corpus-runtime
  interruption
)

records=$(mktemp)
trap 'rm -f "$records"' EXIT
for gate in "${gates[@]}"; do
  artifact="evaluation/certifications/evidence/$gate.json"
  jq -e \
    --arg gate "$gate" \
    --arg binary_sha256 "$binary_sha256" \
    --arg tool_revision "$tool_revision" \
    --argjson generated_at "$generated_at" \
    '
      .schema_version == "1.0"
      and .gate == $gate
      and .release_binary_sha256 == $binary_sha256
      and .tool_revision == $tool_revision
      and .generated_at_utc_unix_seconds == $generated_at
      and .passed == true
    ' "$artifact" >/dev/null
  sha256=$(sha256sum "$artifact" | cut -d' ' -f1)
  jq -c \
    --arg path "$artifact" \
    --arg sha256 "$sha256" \
    '. + {artifact: {path: $path, sha256: $sha256}} |
      {gate, passed, command, detail, artifact}' \
    "$artifact" >> "$records"
done

mkdir -p "$(dirname "$output")"
temporary=$(mktemp "$(dirname "$output")/.release-evidence.XXXXXX")
jq -s \
  --arg binary_sha256 "$binary_sha256" \
  --arg tool_revision "$tool_revision" \
  --argjson generated_at "$generated_at" \
  '{
    schema_version: "1.0",
    release_binary_sha256: $binary_sha256,
    tool_revision: $tool_revision,
    generated_at_utc_unix_seconds: $generated_at,
    gates: .
  }' "$records" > "$temporary"
mv "$temporary" "$output"
