set -euo pipefail

repository="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
replay_root="${RUST_DOCTOR_REPLAY_ROOT:-$HOME/.cache/rust-doctor/replay}"
pinned=1.99.1
msrv=1.95
launcher_bun=1.3.14
all_jobs=(clippy docs test msrv launcher release packed e2e)

fail() {
  echo "replay: $*" >&2
  exit 1
}

usage() {
  echo "usage: bash scripts/linux-ci-replay.sh setup" >&2
  echo "       bash scripts/linux-ci-replay.sh <commit> [${all_jobs[*]}] [-- <arguments for the one job>]" >&2
  exit 2
}

e2e_bun() {
  sed -n 's/.*"packageManager": *"bun@\([0-9.]*\)".*/\1/p' "$1/tests/e2e/package.json"
}

bun_dir() {
  printf '%s/bun/%s/bun-linux-x64' "$replay_root" "$1"
}

install_bun() {
  local version="$1" dir="$replay_root/bun/$1"
  local release="https://github.com/oven-sh/bun/releases/download/bun-v$version"
  [ -x "$(bun_dir "$version")/bun" ] && return 0
  mkdir -p "$dir"
  curl -fsSL -o "$dir/bun-linux-x64.zip" "$release/bun-linux-x64.zip"
  curl -fsSL -o "$dir/SHASUMS256.txt" "$release/SHASUMS256.txt"
  (cd "$dir" && grep ' bun-linux-x64.zip$' SHASUMS256.txt | sha256sum --check --strict -)
  unzip -q -o "$dir/bun-linux-x64.zip" -d "$dir"
}

setup() {
  rustup toolchain install "$pinned" --profile minimal --component clippy --target x86_64-pc-windows-msvc
  rustup toolchain install "$msrv" --profile minimal
  rustup toolchain install stable --profile minimal --component clippy
  local tool missing=()
  for tool in tmux strace jq unzip; do
    [ -x "/usr/bin/$tool" ] || missing+=("$tool")
  done
  [ "${#missing[@]}" -eq 0 ] || sudo dnf install -y "${missing[@]}"
  install_bun "$launcher_bun"
  install_bun "$(e2e_bun "$repository")"
  unshare --user --map-root-user --net true || sudo sysctl -w user.max_user_namespaces=65536
  echo "replay: setup done"
}

pin_default_toolchain() {
  local home="$replay_root/rustup" shared="${RUSTUP_HOME:-$HOME/.rustup}"
  mkdir -p "$home"
  ln -sfn "$shared/toolchains" "$home/toolchains"
  ln -sfn "$shared/update-hashes" "$home/update-hashes"
  printf 'version = "12"\ndefault_toolchain = "%s-x86_64-unknown-linux-gnu"\nprofile = "minimal"\n\n[overrides]\n' \
    "$pinned" > "$home/settings.toml"
  export RUSTUP_HOME="$home"
}

refuse_target_dir() {
  local config
  for config in "$1/config.toml" "$1/config"; do
    if [ -f "$config" ] && grep -Eq '^[[:space:]]*(build\.)?target-dir[[:space:]]*=' "$config"; then
      fail "$config sets target-dir; every fixture must build into its own target/"
    fi
  done
}

preflight() {
  local checkout="$1" dir tool
  [ -z "${CARGO_TARGET_DIR:-}" ] || fail "CARGO_TARGET_DIR is set; unset it so every fixture builds into its own target/"
  [ -z "${CARGO_BUILD_TARGET_DIR:-}" ] || fail "CARGO_BUILD_TARGET_DIR is set; unset it"
  dir="$checkout"
  while :; do
    refuse_target_dir "$dir/.cargo"
    [ "$dir" = / ] && break
    dir="$(dirname "$dir")"
  done
  refuse_target_dir "${CARGO_HOME:-$HOME/.cargo}"
  for tool in tmux strace jq; do
    [ "$(command -v "$tool")" = "/usr/bin/$tool" ] || fail "$tool must resolve to /usr/bin/$tool"
  done
  (cd / && env -u RUSTUP_TOOLCHAIN rustc --version) | grep -q "^rustc $pinned " \
    || fail "the replay's default toolchain is not $pinned"
  rustup component list --toolchain "$pinned" --installed | grep -q '^clippy' || fail "toolchain $pinned lacks clippy"
  rustup target list --toolchain "$pinned" --installed | grep -qx x86_64-pc-windows-msvc \
    || fail "toolchain $pinned lacks the x86_64-pc-windows-msvc target"
  rustup run "$msrv" rustc --version > /dev/null 2>&1 || fail "toolchain $msrv is not installed"
  rustup component list --toolchain stable --installed | grep -q '^clippy' || fail "toolchain stable lacks clippy"
  [ -x "$(bun_dir "$launcher_bun")/bun" ] || fail "bun $launcher_bun is missing: bash scripts/linux-ci-replay.sh setup"
  [ -x "$(bun_dir "$(e2e_bun "$checkout")")/bun" ] || fail "bun $(e2e_bun "$checkout") is missing: bash scripts/linux-ci-replay.sh setup"
  unshare --user --map-root-user --net true || fail "unprivileged user namespaces are refused, and E2E_REQUIRE_NETNS=1 needs them"
}

counts() {
  sed 's/\x1b\[[0-9;?]*[A-Za-z]//g' "$1" | awk '
    /^test result: / {
      for (i = 1; i <= NF; i++) {
        if ($(i + 1) ~ /^passed;?$/) passed += $i
        if ($(i + 1) ~ /^failed;?$/) failed += $i
        if ($(i + 1) ~ /^ignored;?$/) ignored += $i
      }
    }
    /^ *[0-9]+ pass$/ { passed += $1 }
    /^ *[0-9]+ fail$/ { failed += $1 }
    /^ *[0-9]+ skip$/ { ignored += $1 }
    END { printf "passed=%d failed=%d ignored=%d", passed, failed, ignored }
  '
}

with_bun() {
  local version="$1"
  shift
  PATH="$(bun_dir "$version"):$PATH" "$@"
}

e2e() {
  local checkout="$1" bun_version shard files status=0
  shift
  bun_version="$(e2e_bun "$checkout")"
  cd "$checkout/tests/e2e" || return 1
  with_bun "$bun_version" bun install --frozen-lockfile || return 1
  with_bun "$bun_version" bun run typecheck || return 1
  if [ "$#" -gt 0 ]; then
    with_bun "$bun_version" bash ci-run-files.sh "$@"
    return
  fi
  for shard in 0 1 2 3 4; do
    mapfile -t files < <(with_bun "$bun_version" bun ci-shards.ts --shard-count 5 --shard-index "$shard")
    [ "${#files[@]}" -gt 0 ] && [ -n "${files[0]}" ] || return 1
    echo "replay: e2e shard $shard"
    with_bun "$bun_version" bash ci-run-files.sh "${files[@]}" || status=1
  done
  return "$status"
}

job() {
  local checkout="$1" name="$2"
  shift 2
  cd "$checkout" || return 1
  case "$name" in
    clippy) cargo clippy --all-targets --no-deps --locked -- -D warnings ;;
    docs) bash scripts/verify-doc-budgets.sh && bash scripts/verify-doc-refs.sh ;;
    test) RUST_DOCTOR_BENCHMARK_ALLOWANCE=3 cargo test --locked --no-fail-fast "$@" ;;
    msrv) cargo "+$msrv" check --locked --all-targets ;;
    launcher) cd npm/rust-doctor && with_bun "$launcher_bun" bun test tests ;;
    release) cargo build --release --locked ;;
    packed)
      cd npm/rust-doctor || return 1
      RUST_DOCTOR_BINARY="$checkout/target/release/rust-doctor" with_bun "$launcher_bun" bun run smoke:packed
      ;;
    e2e)
      RUST_DOCTOR_E2E_BIN="$checkout/target/release/rust-doctor" \
        E2E_REQUIRE_TMUX=1 E2E_REQUIRE_NETNS=1 RUST_DOCTOR_BENCHMARK_ALLOWANCE=3 \
        RUST_DOCTOR_E2E_ROOT="$RUNNER_TEMP/rust-doctor-e2e" \
        e2e "$checkout" "$@"
      ;;
    *) usage ;;
  esac
}

replay() {
  local commit tree checkout record started status=0 name log result
  commit="$(git -C "$repository" rev-parse --verify "$1^{commit}")"
  shift
  local jobs=() arguments=()
  while [ "$#" -gt 0 ]; do
    if [ "$1" = -- ]; then
      shift
      arguments=("$@")
      break
    fi
    jobs+=("$1")
    shift
  done
  [ "${#jobs[@]}" -gt 0 ] || jobs=("${all_jobs[@]}")
  [ "${#arguments[@]}" -eq 0 ] || [ "${#jobs[@]}" -eq 1 ] || fail "arguments after -- need exactly one job"
  checkout="$replay_root/$commit"
  if [ ! -d "$checkout/.git" ]; then
    git clone --quiet --shared --no-checkout "$repository" "$checkout"
    git -C "$checkout" remote set-url origin "$(git -C "$repository" remote get-url origin)"
    git -C "$checkout" checkout --quiet --detach "$commit"
  fi
  [ "$(git -C "$checkout" rev-parse HEAD)" = "$commit" ] && [ -z "$(git -C "$checkout" status --porcelain --untracked-files=all)" ] \
    || fail "$checkout differs from $commit; delete it and rerun"
  tree="$(git -C "$checkout" write-tree)"
  [ "$tree" = "$(git -C "$repository" rev-parse "$commit^{tree}")" ] || fail "the checkout's index is not the tree of $commit"
  [ -z "${RUSTUP_TOOLCHAIN:-}" ] || fail "RUSTUP_TOOLCHAIN is set; CI resolves the toolchain from its default"
  pin_default_toolchain
  preflight "$checkout"
  export RUNNER_TEMP="$replay_root/temp/$commit"
  export TMPDIR="$RUNNER_TEMP/tmp"
  mkdir -p "$TMPDIR"
  record="$replay_root/$commit.record"
  echo "replay: commit $commit tree $tree"
  (cd "$checkout" && cargo fetch --locked)
  for name in "${jobs[@]}"; do
    log="$replay_root/$commit.$name.log"
    started="$(date +%s)"
    if (job "$checkout" "$name" "${arguments[@]}") 2>&1 | tee "$log"; then result=pass; else result=fail; status=1; fi
    printf '%s\t%s\t%s\t%ss\t%s\ttree=%s\t%s\n' "$(date -u +%FT%TZ)" "$name" "$result" "$(($(date +%s) - started))" \
      "$(counts "$log")" "$tree" "$(cd "$checkout" && rustc --version)" | tee -a "$record"
  done
  echo "replay: record in $record"
  return "$status"
}

[ "$#" -gt 0 ] || usage
case "$1" in
  setup) setup ;;
  -h | --help) usage ;;
  *) replay "$@" ;;
esac
