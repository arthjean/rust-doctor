#!/usr/bin/env bash
set -euo pipefail

[[ -s "$REPORT_FILE" ]] || { echo "::warning::Report V1 is unavailable; skipping SARIF"; exit 0; }
SARIF_FILE=$(mktemp "$RUNNER_TEMP/rust-doctor.XXXXXX.sarif")

jq '{
  version: "2.1.0",
  "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
  runs: [{
    tool: {driver: {
      name: "rust-doctor",
      version: .tool_version,
      informationUri: "https://github.com/arthjean/rust-doctor",
      rules: ([.diagnostics[] | select(.visible_on | index("sarif")) | {
        id: .rule,
        shortDescription: {text: .title},
        helpUri: .url,
        defaultConfiguration: {level: (if .severity == "error" then "error" elif .severity == "warning" then "warning" else "note" end)}
      }] | unique_by(.id))
    }},
    results: [.diagnostics[] | select(.visible_on | index("sarif")) | {
      ruleId: .rule,
      level: (if .severity == "error" then "error" elif .severity == "warning" then "warning" else "note" end),
      message: {text: (if .help then (.message + ": " + .help) else .message end)},
      locations: (if .location.kind == "source" then [{physicalLocation: {
        artifactLocation: {uri: .location.path, uriBaseId: "%SRCROOT%"},
        region: {
          startLine: .location.range.start.line,
          startColumn: .location.range.start.column,
          endLine: .location.range.end.line,
          endColumn: .location.range.end.column
        }
      }}] else [] end),
      partialFingerprints: {"rustDoctorSiteId/v1": .site_id}
    }]
  }]
}' "$REPORT_FILE" > "$SARIF_FILE"

jq -e '.version == "2.1.0" and (.runs | length == 1)' "$SARIF_FILE" >/dev/null
printf 'sarif-file=%s\n' "$SARIF_FILE" >> "$GITHUB_OUTPUT"
