# The end-to-end suite

`tests/e2e/` is a private bun package that drives the release binary the way a
user does: in a copy of a workspace, through a real terminal when the subject
is the interactive report, on git histories built for the test. The Rust
integration tests stay the regression home of every fix; this suite is the
layer that sees what only the real binary, a real terminal or a real git
history shows. Its scenarios come from the catalogs of `tasks/e2e-catalog/`,
and a test names the catalog row it automates.

## Running it

```bash
cargo build --release
cd tests/e2e
bun install --frozen-lockfile
bun test cli-surface.test.ts
```

`bun test` alone runs every file. `RUST_DOCTOR_E2E_BIN` points the suite at
another binary, and a missing one fails every file with the build instruction.
`RUST_DOCTOR_E2E_ROOT` moves the scratch root, which defaults to
`rust-doctor-e2e` under the system temporary directory. The interactive-report
files need tmux: without it they skip with one stated reason, and
`E2E_REQUIRE_TMUX=1` turns that skip into a failure, which is what CI sets.
`bun run typecheck` checks the suite under strict TypeScript.

## What every test holds to

Every spawn goes through `e2e-helpers.ts`. The environment is built from an
allowlist, never inherited: a private HOME and TMPDIR, the machine's
`RUSTUP_HOME` and `CARGO_HOME`, `LC_ALL=C`, a pinned `TERM`, no system or
global git configuration, and `CARGO_NET_OFFLINE=true`, so no test reaches the
network. The agent markers and `CI` are absent unless a test sets one, and
their list is read from `src/terminal_profile.rs`, so a marker added there is
scrubbed the same day. PATH holds links to the toolchain and the core
utilities and nothing else; `fakes.ts` puts recording stand-ins for agents,
clipboards, browsers, `gh` and `git-lfs` in front of it, so no test can start
the machine's real ones.

A scan runs in a copy, never in this repository, with its own
`CARGO_TARGET_DIR` and `--advisory-db /dev/null` unless the test opts out. A
watchdog kills a process that outlives it, and the preload kills and names any
process or tmux server a test leaves running. `treeFingerprint` hashes a tree
without spawning git, `.git/index` time included, for a test that asserts a
scan wrote nothing. Credential-shaped values are built at runtime by
`credential()` in `git-scenario.ts`: a literal one under `tests/e2e/` would
be reported by this repository's own scan, and a test fails on one.

Terminal tests poll and never sleep blind. `tmux-helpers.ts` gives each
session its own server with escape time zero and an explicit size, and starts
the program only once the byte log is attached; a dead pane is read with its
history, since tmux scrolls the program's last lines up. `pty.ts` is the lighter tool for a test that only needs the binary to
see terminal streams.

## The self-scan rule

A test that scans this repository asserts contracts, never this repository's
score, label, counts or specific findings, as [testing.md](testing.md) requires
of the Rust suite: the self-scan is a gate, and evidence that a rule fires
belongs on a fixture.

## Known defects

Every defect the catalogs confirmed has a stable identifier in the ledger of
`tasks/e2e-catalog/README.md`, mirrored entry for entry by
`tests/e2e/known-bugs.json`. An open defect is a `test.failing` row titled with
its catalog row and its identifier, whose body asserts the correct behavior:
it passes while the defect stands and fails the day it is fixed.
`known-bugs.test.ts` fails when an open entry has no such row, when a row
names an identifier the ledger lacks, or when it names one already fixed.

A defect found end to end lands first as a Rust regression test in the test
file that owns its module. The commit that fixes it turns its `test.failing`
row into a plain test, marks its ledger entry fixed in both files, and keeps
the regression test.

## In CI

`ci.yml` builds the release binary once and runs the files in weighted
shards ([ci-workflows.md](ci-workflows.md)). A new test file needs its entry
in `ci-shard-weights.json`, an estimate in seconds on a hosted runner, or the
planner fails naming it.
