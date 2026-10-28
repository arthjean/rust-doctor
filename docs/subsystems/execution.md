# Running the producers

`src/execution.rs` is the orchestration and nothing else: what a run holds
constant, the toolchain it resolves, the order the five producers run in, and
the shape of what came back. `execution/clippy.rs` is the pass that compiles,
`execution/messages.rs` the `--message-format=json` stream it answers on,
`execution/baseline.rs` the dual run a `--scope baseline` comparison needs, and
each carries its tests in a file of its own.

Five rules hold it together, and each of them replaced something that had a
cost.

One list of the producers that degrade rather than abort, a panic included.
`ExecutionResult::producer_errors` is that list, and `is_complete` and
`report::errors` both read it. It used to be written twice, and `cargo_health`
was in the second list only: a `.cargo/config.toml` that could not be read
published a `dependencies` error under `"status": "complete"`.
`every_producer_error_drops_the_authoritative_flag_at_its_own_stage` asserts the
four together. Every pass runs under `execution/containment.rs`: one that panics
is dropped and published as `pass-panicked` at its stage, with a fixed sentence
and never the payload, which can quote the tree, and a child nobody waited on is
killed with its group on drop. `tests/pass_containment.rs` injects each panic
through `RUST_DOCTOR_INJECT_PANIC`, which only a debug build reads.

The advisory database is read, never fetched. `cargo_health/advisories.rs`
judges each crates.io package `Cargo.lock` resolves against the RustSec
database `--advisory-db` names, or the one `cargo audit` or `cargo deny` left
under `$CARGO_HOME`, and only `main.rs` looks for one: a library caller of
`inspect` names its database or reads none. Discovery accepts only a clone
whose `.git/config`, read as text, names the RustSec remote: `cargo deny`
clones any database a `deny.toml` names beside it, and that one deciding the
verdicts in silence was the alternative. A machine without one publishes
both advisory rules `advisory-db-missing` and keeps the score authoritative,
since a check nobody could run is not a failed stage. Fetching it would have
been the tool's first network call. The report
publishes where the database came from and its date, never its path, and a
database older than thirty days is read and its age printed.

The result is assembled from what came back, never asked back for what it was
given. `ExecutionContext::run` owns the metadata to the end, which retired
three `Option` dances no input could make `None` and a workspace-root clone,
and carries what seven positional parameters used to.

Every bound is a budget on work, never a filter on meaning. `LINE_MAX_BYTES`
bounds what one line of Cargo's stream may contribute, `MESSAGE_MAX_COUNT` how
many messages one scan may keep, `RUSTUP_OUTPUT_LIMIT` what a toolchain probe
may print, and none of them decides which diagnostic the report may publish: an
oversized line is counted malformed and the stream keeps going, a flood stops
with an error at the `parsing` stage. The reader had no bound at all, and the
scanned workspace's procedural macros decide how many diagnostics Cargo emits.
`src/bounded_read.rs` is the one primitive both layers read streams through.
A compiler message keeps its `children` and the replacement each of their spans
proposes, which is what the report's `suggestion` is read from; the text of a
child stays with the toolchain.

Three versions or none, and each probe names its own remedy. `Toolchain` is the
three the report attributes its findings to, held by one `Option`:
`resolve_toolchain` returns all three or fails before any producer starts, so a
report naming a cargo and no rustc was a shape the type allowed and no code path
could produce. Clippy is one of the three, so a toolchain without the component
fails the scan at stage `execution` before anything runs, which is why the
generated workflow and every CI that runs this tool install `components:
clippy`. A `Probe` carries what its tool is called and what to do when it fails,
and a probe reads both of its streams bounded, stderr on a thread of its own, so
the failure quotes what the toolchain itself said. The message used to be
`Clippy exited with status exit status: 101` and nothing more: it named neither
the missing component nor the one command that installs it, and stderr, where
cargo had already written both, was sent to `/dev/null`.

A scan with no Clippy rule starts no program the scanned tree selects. It
probes no version and publishes all three null, which reads "not probed":
`cargo clippy --version` runs a workspace `[alias] clippy`, and every probe
runs whatever a `rust-toolchain.toml` selects. Its `cargo metadata` runs under
`RUSTUP_TOOLCHAIN` set to a name by `execution/toolchain.rs`, the inherited one
or else the first field of `rustup show active-toolchain`; only ASCII
alphanumerics, `-`, `_` and `.` pass, so a path toolchain is refused with exit
2 rather than run, and without rustup no proxy reads the file and no pin is
set. The working directory stays the manifest's, so `build.target-dir` keeps
its meaning. `tests/toolchain_pin_assumptions.rs` checks, on the toolchain CI
runs, that metadata starts nothing `.cargo/config.toml` names and that a name
outranks a path toolchain. The request decides the pin, since `rust-doctor.toml`
is read only once metadata names the root and a request override outranks it.

The Clippy child answers for itself. Its stderr is drained under
`src/cargo_stderr.rs`'s 65,536-byte bound on a thread of its own, and a failed
pass ends its `clippy-exit` error with the scrubbed tail of it: workspace root,
target directory, `CARGO_HOME` and `HOME` become `.`, `$CARGO_TARGET_DIR`,
`$CARGO_HOME` and `~`. `cargo metadata`
takes the same path. `--keep-going` lints every member that compiles, and
`execution/members.rs` names the ones with no `compiler-artifact` record under
`packages-unlinted`. Lint-level tokens are stripped from the caller's rustflags
by `execution/rustflags.rs`, and only when present, so a user's fingerprints
survive. The workspace's `[build] rustflags = ["-D", "warnings"]` fails the
build on 1.97.1 (observed 2026-09-25), so it is stripped the same way unless a
`[target.*] rustflags` overrides it. `execution/lint_list.rs` reads
`clippy-driver -W help` under ten seconds before any `-W` is passed, and a
catalogued rule it does not list is published as not evaluated. A `-D warnings`
inherited from a `[target.<cfg>] rustflags` table or `CARGO_BUILD_RUSTFLAGS`
cannot be stripped from the command line, so the command ends with
`-W warnings`: on 1.97.1 the last level given wins and the build completes
(observed 2026-10-14, pinned by
`a_target_table_or_cargo_build_rustflags_cannot_deny_warnings`).

A scan leaves `Cargo.lock` as it found it. `cargo clippy` writes the lockfile
it resolved, so a lockless binary crate scored 94 with `missing_lockfile` and
99 on the next scan, which read the file the first one wrote.
`execution/lockfile.rs` keeps the lockfile's bytes before Clippy starts and,
whatever ended the pass, removes one that did not exist and writes back one
Cargo changed; one Cargo left alone is not written, so its modification time
survives. `--locked` refuses a missing lockfile and `--offline` resolves
differently, which is why the file is restored rather than protected. A
removal that fails is one stderr line naming the file, never a report field.
The race is accepted: a lockfile another process creates during the scan is
removed with it.

`--max-duration` arms `execution/process.rs`'s watchdog, and every child the
run starts goes through it: the version probes, `cargo metadata`, every git
call, the snapshot and Clippy. A child still running at the deadline is killed
with `deadline-exceeded` at its own stage. Clippy cut short names the members
it never linted, their score is withheld rather than computed on nothing, and
the message names the lock Cargo's stderr shows it waited on, the build
directory's or the package cache's, and never one it did not. On Unix a bounded child leads its own process group, killed through
`libc::kill`. `libc` is a normal dependency for that and for
`execution/interrupt.rs`: a first SIGINT, SIGTERM or SIGHUP kills every
registered group and the run ends with one failed report, no score, exit 130
and no snapshot left behind. SIGHUP is a closed terminal tab, and was the one
signal that left Cargo compiling. A second signal within 500 ms of the first
is the same delivery: a terminal's Ctrl-C through the npm launcher arrives
once from the terminal and once from the launcher's forward, and took the
immediate exit with nothing on stdout. A second signal after that exits at
once. A terminal's Ctrl-C no longer reaches a child in its own group, which is
why the handler kills them. While the interactive report holds the terminal,
a signal restores the mode it found and shows the cursor before the process
exits. SIGKILL runs nothing, so the first snapshot a baseline or staged scan
creates sweeps the `rust-doctor-baseline-<pid>-<n>` directories this user owns
whose process is gone, never a live one's, and one it cannot remove stays.

A snapshot is a tree Cargo walks up from, so `src/baseline/parent.rs` creates
it under the first of the temporary directory, `$XDG_RUNTIME_DIR` and the
cache home whose every ancestor only this user or root can write: a 1777
`/tmp` would let another user plant a `.cargo/config.toml` above it.
Discovery stops at a snapshot's `tree`. On
Windows `taskkill /T` kills the tree it can see, no signal is handled, and
nothing tests either.

The run overlaps what does not depend on itself. The three version probes and
the lint list run concurrently. The dependency pack and the repository pass
read the manifests, the lockfile and git on one thread while the walk reads the
tree on the other, all of it before Clippy starts, so `build.rs` cannot write
into what they measure. Clippy then runs on a thread of its own, the
structural pass on another, and the source kernel and the dependency-truth
rules on the calling thread; the deadline decides which of the three start
before any does. The reports are byte-identical to the serial order on every
fixture. `--timings`, and `--verbose` outside `--json`, publish each
phase in milliseconds; `tests/performance.rs` medians them cold and warm with
`cargo test --release --test performance -- --ignored --nocapture`.

A baseline comparison reads its base side from
`execution/side_cache.rs` when nothing it depends on moved: the base tree, the
toolchain and lint list, the policy, the packages, the advisory database, the
`CARGO`/`RUST`/`CC`/`CFLAGS`/`CXX` environment and the Cargo configuration
outside the repository. The cache lives under the target directory, written
through `workspace_write` like the installers' files: never through a symlinked
component, and staged in a file created only where nothing sits.
`RUST_DOCTOR_NO_CACHE=1` turns it off, a corrupt file is a miss, and only
`--verbose` says which it was: the JSON is the same bytes either way.

A member selection narrows the work, not only the report. `--package` reaches
Clippy as one `-p <NAME>` per member where `--workspace` stood, narrows the
walk to the units those members reach, and keeps the repository pass from
running, since a repository is no member: its rules are published
`not_evaluated: "package-scoped"`. A name that is not a member is refused before
any process starts. `exclusions.rs` gathers what the report leaves out, the
paths `.gitattributes` declares generated or vendored through
`git check-attr --stdin`, read once from the workspace and applied to every
side of a comparison since a snapshot sits outside git, and the suppression
directives of the files the scan read; a git that fails answers nothing, and
only the generator header applies.

`the_execution_holds_the_size_bound_it_scans_for` keeps every file of the module
under the 1000 lines `oversized_unit` reports, tests included. The module was
one file of 977 lines with no such test, the only one of the crate both near the
bound and unguarded, and it carried the whole Clippy story that `mod clippy`
was supposed to name.

