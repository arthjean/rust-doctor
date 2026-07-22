#!/bin/bash
set -euo pipefail

# Publishes all npm packages for a given release.
# Usage: ./npm/publish.sh <version> <artifacts-dir>
#
# Auth: npm Trusted Publishing (OIDC) — no NPM_TOKEN. The Release workflow runs
# with `id-token: write`; npm mints a short-lived credential per publish and
# attaches a provenance attestation (--provenance). Configured ONCE PER PACKAGE
# on npmjs.com (Settings > Trusted Publisher), all pointing at repo
# arthjean/rust-doctor, workflow release.yml:
#   rust-doctor, @rust-doctor/darwin-x64, @rust-doctor/darwin-arm64,
#   @rust-doctor/linux-x64, @rust-doctor/linux-arm64, @rust-doctor/win32-x64
#
# Expected artifacts directory layout:
#   rust-doctor-x86_64-apple-darwin.tar.gz
#   rust-doctor-aarch64-apple-darwin.tar.gz
#   rust-doctor-x86_64-unknown-linux-gnu.tar.gz
#   rust-doctor-aarch64-unknown-linux-gnu.tar.gz
#   rust-doctor-x86_64-pc-windows-msvc.zip

VERSION="${1:?Usage: publish.sh <version> <artifacts-dir>}"
ARTIFACTS="${2:?Usage: publish.sh <version> <artifacts-dir>}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
WORK_DIR=$(mktemp -d)

trap 'rm -rf "$WORK_DIR"' EXIT

# Map: Rust target -> npm package dir
declare -A TARGET_MAP=(
  ["x86_64-apple-darwin"]="darwin-x64"
  ["aarch64-apple-darwin"]="darwin-arm64"
  ["x86_64-unknown-linux-gnu"]="linux-x64"
  ["aarch64-unknown-linux-gnu"]="linux-arm64"
  ["x86_64-pc-windows-msvc"]="win32-x64"
)

echo "Publishing rust-doctor npm packages v${VERSION}"

# Step 1: Extract binaries into platform packages
for target in "${!TARGET_MAP[@]}"; do
  npm_dir="${TARGET_MAP[$target]}"
  pkg_dir="${WORK_DIR}/${npm_dir}"
  cp -r "${SCRIPT_DIR}/${npm_dir}" "${pkg_dir}"

  # Update version in package.json
  sed -i "s/\"version\": \".*\"/\"version\": \"${VERSION}\"/" "${pkg_dir}/package.json"

  # Ensure bin/ directory exists (git doesn't track empty dirs)
  mkdir -p "${pkg_dir}/bin"

  # Extract binary
  archive_base="rust-doctor-${target}"
  if [[ "$target" == *"windows"* ]]; then
    archive="${ARTIFACTS}/${archive_base}.zip"
    if [ ! -f "$archive" ]; then
      echo "WARNING: Missing archive ${archive}, skipping ${npm_dir}"
      continue
    fi
    unzip -o "$archive" -d "${pkg_dir}/bin/"
  else
    archive="${ARTIFACTS}/${archive_base}.tar.gz"
    if [ ! -f "$archive" ]; then
      echo "WARNING: Missing archive ${archive}, skipping ${npm_dir}"
      continue
    fi
    tar xzf "$archive" -C "${pkg_dir}/bin/"
  fi

  chmod +x "${pkg_dir}/bin/"* 2>/dev/null || true

  echo "Publishing @rust-doctor/${npm_dir}@${VERSION}..."
  (cd "${pkg_dir}" && npm publish --provenance --access public)
done

# Step 2: Publish main package
main_dir="${WORK_DIR}/rust-doctor"
cp -r "${SCRIPT_DIR}/rust-doctor" "${main_dir}"

# Update version in main package and optionalDependencies
sed -i "s/\"version\": \".*\"/\"version\": \"${VERSION}\"/" "${main_dir}/package.json"
for npm_dir in "${TARGET_MAP[@]}"; do
  sed -i "s/\"@rust-doctor\/${npm_dir}\": \".*\"/\"@rust-doctor\/${npm_dir}\": \"${VERSION}\"/" "${main_dir}/package.json"
done

echo "Publishing rust-doctor@${VERSION}..."
(cd "${main_dir}" && npm publish --provenance --access public)

echo "All npm packages published successfully!"
