# Replaying CI's Linux jobs on Fedora

A Linux verdict reached on the maintainer's Fedora machine has to mean what a CI
run means, so the Linux jobs of `.github/workflows/ci.yml` replay through one
script rather than through commands retyped from memory:
`scripts/linux-ci-replay.sh`.

## Setup

`bash scripts/linux-ci-replay.sh setup` installs what the jobs install:
toolchain 1.99.1 with clippy and the `x86_64-pc-windows-msvc` target, which the
`Tests` job adds; toolchain 1.95 for the MSRV check; `stable` with clippy, which
the baseline fixture's `rust-toolchain.toml` pins; tmux, strace and jq under
`/usr/bin` through dnf; bun 1.3.14 for the launcher jobs and the version
`tests/e2e/package.json` names for the end-to-end suite, each from its release
archive checked against the release's `SHASUMS256.txt`. It allows unprivileged
user namespaces when `unshare` is refused, so `E2E_REQUIRE_NETNS=1` holds.

## A run

`bash scripts/linux-ci-replay.sh <commit> [job...]` replays `clippy`, `docs`,
`test`, `msrv`, `launcher`, `release`, `packed` and `e2e`, or the jobs named.
It checks the commit out as a shared clone with the full history, as CI's
`fetch-depth: 0` does, under `~/.cache/rust-doctor/replay/<commit>`: a worktree
would make `.git` a file, which the packed smoke reads as a directory. It
refuses a checkout that differs from the commit, and prints the checkout's `git write-tree`, which is the tree id the
record names.

It then holds CI's environment. CI's default toolchain is 1.99.1, and tests that
remove `RUSTUP_TOOLCHAIN` resolve to it, so the run gets a private `RUSTUP_HOME`
sharing the installed toolchains with that default, and the machine's own default
stays as it is. It refuses `CARGO_TARGET_DIR`, and a `target-dir` in any
`config.toml` Cargo reads from the checkout's ancestors or from `CARGO_HOME`, so
each fixture builds into its own `target/`. It fetches once with
`cargo fetch --locked`, passes `--locked` to every Cargo command, runs the
tests with `--no-fail-fast` so one run lists every failure, and sets
`RUST_DOCTOR_BENCHMARK_ALLOWANCE=3` for every test run. The end-to-end suite runs
with `E2E_REQUIRE_TMUX=1`, `E2E_REQUIRE_NETNS=1` and `RUST_DOCTOR_E2E_BIN`,
through `bash ci-run-files.sh` on the files `bun ci-shards.ts` plans for each of
the five shards. `TMPDIR` points under the replay root, since `/tmp` is
quota-limited on this machine.

Each job appends one line to `<commit>.record`: the date, the job, pass or fail,
its duration, its pass, fail and skip counts, the tree id and the rustc version.
Its output stays beside it in `<commit>.<job>.log`.

## Judging a failure

A failure on the reviewed tree counts only once the same test has run alone on
the tree's parent commit in the same environment:
`bash scripts/linux-ci-replay.sh <commit>^ test -- --test <binary> <name>`, or
`e2e -- <file>`. A failure both share is recorded as pre-existing, never as a
pass. A wall-clock budget is judged the same way.

A test that passes on Fedora and fails in CI, or the reverse, is recorded as an
environment difference, never as a pass. Fedora ships GNU coreutils, while CI's
ubuntu-26.04 image ships uutils.
