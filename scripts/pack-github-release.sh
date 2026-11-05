set -euo pipefail

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
  for license in LICENSE-APACHE LICENSE-MIT; do
    git -C "$repo" cat-file blob "HEAD:$license" > "$stage/$root/$license"
  done

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
