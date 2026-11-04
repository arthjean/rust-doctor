#!/usr/bin/env bash
# Packs the five release binaries into the archives a GitHub Release carries,
# the ones `cargo binstall rust-doctor` downloads through the
# `[package.metadata.binstall]` table of Cargo.toml instead of compiling.
#
#   scripts/pack-github-release.sh --targets
#   scripts/pack-github-release.sh <artifacts> <output>
#
# `--targets` prints the five Rust targets, one per line. Otherwise <artifacts>
# holds one `rust-doctor-<platform>/` directory per platform with the binary
# inside, the layout the build matrix uploads and pack-release.mjs reads too,
# and <output> receives one `rust-doctor-<target>.tar.gz` per target:
#
#   rust-doctor-<target>/rust-doctor[.exe]   0755
#   rust-doctor-<target>/LICENSE-APACHE      0644
#   rust-doctor-<target>/LICENSE-MIT         0644
#
# One format for every target, Windows included: binstall extracts a tarball on
# any host, Windows 10 and later ship `tar`, and one code path packs all five.
# The archives are reproducible: ustar entries in a fixed order, owned by 0:0
# with no names, stamped 1985-10-26T08:15:00Z (the instant npm stamps every
# tarball entry with), gzip without a name or a timestamp. The same binaries
# give the same bytes, so a re-run never ships something new under an old name.
# docs/publishing.md holds the reasoning.
set -euo pipefail

# Each npm platform key with the Rust target its binary is built for, as the
# build matrix of .github/workflows/release.yml pairs them.
# tests/binstall_contract.rs holds this table to the matrix and to Cargo.toml.
PLATFORMS=(
  "linux-x64 x86_64-unknown-linux-gnu"
  "linux-arm64 aarch64-unknown-linux-gnu"
  "darwin-x64 x86_64-apple-darwin"
  "darwin-arm64 aarch64-apple-darwin"
  "win32-x64 x86_64-pc-windows-msvc"
)
EPOCH=499162500

fail() {
  echo "pack-github-release: $*" >&2
  exit 1
}

if [ "${1:-}" = "--targets" ]; then
  for pair in "${PLATFORMS[@]}"; do
    echo "${pair#* }"
  done
  exit 0
fi

[ $# -eq 2 ] || fail "usage: $0 --targets | $0 <artifacts> <output>"
repo="$(cd "$(dirname "$0")/.." && pwd)"
artifacts="$(cd "$1" && pwd)"
mkdir -p "$2"
output="$(cd "$2" && pwd)"
stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT

for pair in "${PLATFORMS[@]}"; do
  platform="${pair%% *}"
  target="${pair#* }"
  binary=rust-doctor
  case "$target" in *-windows-*) binary=rust-doctor.exe ;; esac
  source="$artifacts/rust-doctor-$platform/$binary"
  [ -f "$source" ] && [ -s "$source" ] || fail "no binary for $platform at $source"

  root="rust-doctor-$target"
  rm -rf "${stage:?}/$root" "$stage/archive.tar"
  mkdir "$stage/$root"
  cp "$source" "$stage/$root/$binary"
  # From the committed blob rather than the working tree, whose line endings
  # depend on the checkout: a Windows one writes CRLF.
  for license in LICENSE-APACHE LICENSE-MIT; do
    git -C "$repo" cat-file blob "HEAD:$license" > "$stage/$root/$license"
  done

  # The mode is written, not read: a workflow artifact comes back without its
  # executable bit, and a Windows checkout has no bit to read at all.
  tar=(tar --format=ustar --owner=0 --group=0 --numeric-owner
    --mtime="@$EPOCH" --no-recursion -C "$stage")
  "${tar[@]}" --mode=0755 -cf "$stage/archive.tar" "$root" "$root/$binary"
  "${tar[@]}" --mode=0644 -rf "$stage/archive.tar" "$root/LICENSE-APACHE" "$root/LICENSE-MIT"
  gzip -n -9 -c "$stage/archive.tar" > "$output/$root.tar.gz"

  listed="$(tar -tzf "$output/$root.tar.gz" | tr '\n' ' ')"
  expected="$root/ $root/$binary $root/LICENSE-APACHE $root/LICENSE-MIT "
  [ "$listed" = "$expected" ] || fail "$root.tar.gz holds '$listed', expected '$expected'"
  echo "$(sha256sum "$output/$root.tar.gz" | cut -d' ' -f1)  $root.tar.gz"
done
