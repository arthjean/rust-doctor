#!/usr/bin/env bash
set -euo pipefail

VERSION="${1:?Usage: publish.sh <version> <packed-artifacts-dir>}"
ARTIFACTS="${2:?Usage: publish.sh <version> <packed-artifacts-dir>}"
SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
REPOSITORY_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)

command -v bun >/dev/null 2>&1 || {
  echo "bun is required to verify and publish Rust Doctor packages" >&2
  exit 1
}

cd "$REPOSITORY_ROOT"
RESOLVED_VERSION=$(bun scripts/release/packages.ts validate "v${VERSION}")
[[ "$RESOLVED_VERSION" == "$VERSION" ]] || {
  echo "Cargo resolved version $RESOLVED_VERSION, expected $VERSION" >&2
  exit 1
}
bun scripts/release/packages.ts publish "$ARTIFACTS"
