set -euo pipefail

BINSTALL_VERSION=1.26.2
BINSTALL_SHA256=9e0e064f189a9abb9f8df5fb93c3c810ef5cc19e7212d998c7a64c9e4d3527d5

fail() {
  echo "binstall-proof: $*" >&2
  exit 1
}

repo="$(cd "$(dirname "$0")/.." && pwd)"
scratch="$(mktemp -d)"
server=""
cleanup() {
  rm -f "$scratch/serving"
  [ -z "$server" ] || wait "$server" 2>/dev/null || true
  rm -rf "$scratch"
}
trap cleanup EXIT

version="$(sed -n 's/^version = "\([^"]*\)"$/\1/p' "$repo/Cargo.toml" | head -n 1)"
[ -n "$version" ] || fail "Cargo.toml declares no package version"

binstall="${CARGO_BINSTALL:-}"
if [ -z "$binstall" ]; then
  archive="cargo-binstall-x86_64-unknown-linux-musl.tgz"
  curl -fsSL -o "$scratch/$archive" \
    "https://github.com/cargo-bins/cargo-binstall/releases/download/v$BINSTALL_VERSION/$archive"
  echo "$BINSTALL_SHA256  $scratch/$archive" | sha256sum -c - >/dev/null \
    || fail "$archive does not match the pinned digest"
  tar -xzf "$scratch/$archive" -C "$scratch" cargo-binstall
  binstall="$scratch/cargo-binstall"
fi

case "${1:-}" in
  local)
    [ $# -eq 2 ] || fail "usage: $0 local <archives>"
    mkdir -p "$scratch/www"
    cp "$2"/*.tar.gz "$scratch/www/"
    (
      cd "$scratch"
      export MSYS_NO_PATHCONV=1
      openssl req -x509 -newkey rsa:2048 -nodes -days 1 -subj "/CN=binstall-proof CA" \
        -keyout ca.key -out ca.crt 2>/dev/null
      openssl req -newkey rsa:2048 -nodes -subj "/CN=127.0.0.1" \
        -keyout server.key -out server.csr 2>/dev/null
      printf 'subjectAltName=IP:127.0.0.1\nbasicConstraints=critical,CA:FALSE\nextendedKeyUsage=serverAuth\n' \
        > server.ext
      openssl x509 -req -in server.csr -CA ca.crt -CAkey ca.key -CAcreateserial -days 1 \
        -extfile server.ext -out server.crt 2>/dev/null
    ) || fail "openssl could not issue the throwaway certificate"
    touch "$scratch/serving"
    (
      cd "$scratch"
      exec python3 - >server.log 2>&1 <<'PYTHON'
import functools, http.server, os, ssl
handler = functools.partial(http.server.SimpleHTTPRequestHandler, directory="www")
server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
context.load_cert_chain("server.crt", "server.key")
server.socket = context.wrap_socket(server.socket, server_side=True)
server.timeout = 0.2
print(f"port {server.server_address[1]}", flush=True)
while os.path.exists("serving"):
    server.handle_request()
PYTHON
    ) &
    server=$!
    port=""
    for _ in $(seq 100); do
      port="$(sed -n 's/^port \([0-9]*\)$/\1/p' "$scratch/server.log" 2>/dev/null)"
      if [ -n "$port" ] && curl -fsSk -o /dev/null "https://127.0.0.1:$port/" 2>/dev/null; then
        break
      fi
      port=""
      sleep 0.1
    done
    [ -n "$port" ] || { cat "$scratch/server.log" >&2; fail "the archive server never answered"; }
    source=(--manifest-path "$repo" --root-certificates "$scratch/ca.crt"
      --pkg-url "https://127.0.0.1:$port/{ name }-{ target }.tar.gz")
    host="127.0.0.1"
    ;;
  registry)
    [ $# -eq 1 ] || fail "usage: $0 registry"
    source=(--version "$version")
    host="github.com"
    ;;
  *)
    fail "usage: $0 local <archives> | $0 registry"
    ;;
esac

resolve() {
  "$binstall" rust-doctor "${source[@]}" --strategies crate-meta-data \
    --disable-telemetry --no-confirm --force --no-track --root "$scratch/root" \
    --log-level info "$@" 2>&1
}

judge() {
  local label="$1" output="$2"
  if grep -q "installed from source" <<<"$output" \
    || ! grep -q "has been downloaded from $host" <<<"$output"; then
    echo "$output" >&2
    fail "$label did not resolve a prebuilt archive from $host"
  fi
}

for target in $(bash "$repo/scripts/pack-github-release.sh" --targets); do
  output="$(resolve --targets "$target" --dry-run)" || { echo "$output" >&2; fail "$target failed"; }
  judge "$target" "$output"
  echo "resolved $target from $host"
done

output="$(resolve)" || { echo "$output" >&2; fail "the host install failed"; }
judge "the host install" "$output"
binary="$scratch/root/bin/rust-doctor"
[ -f "$binary" ] || binary="$binary.exe"
printed="$("$binary" --version)"
[ "$printed" = "rust-doctor $version" ] || fail "the installed binary printed '$printed', expected 'rust-doctor $version'"
echo "installed for the host: $printed"
