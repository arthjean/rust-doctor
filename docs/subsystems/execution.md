# Running the producers

`src/execution.rs` is the orchestration and nothing else: what a run holds
constant, the toolchain it resolves, the order the five producers run in, and
the shape of what came back. `execution/clippy.rs` is the pass that compiles,
`execution/messages.rs` the `--message-format=json` stream it answers on,
`execution/baseline.rs` the dual run a `--scope baseline` comparison needs, and
each carries its tests in a file of its own.

Four rules hold it together, and each of them replaced something that had a
cost.

One list of the producers that degrade rather than abort.
`ExecutionResult::producer_errors` is that list, and `is_complete` and
`report::errors` both read it. It used to be written twice, as a four-clause
conjunction in `execution.rs` and four `if let` blocks with the stage spelled
again in `report.rs`, and `cargo_health` was in the second list only: a
workspace whose `.cargo/config.toml` could not be read published a
`dependencies` error under `"status": "complete"`, with the score still calling
itself authoritative.
`every_producer_error_drops_the_authoritative_flag_at_its_own_stage` asserts the
four together, because the defect was never the missing clause.

The result is assembled from what came back, never asked back for what it was
given. `ExecutionContext::run` owns the metadata to the end, so the three
`Option` dances that no input could make `None`, and the workspace-root clone
that only existed to survive an early move, are gone with them. The same run
carries `ExecutionContext` rather than seven positional parameters, four of
which were identical on both sides of a baseline comparison.

Every bound is a budget on work, never a filter on meaning. `LINE_MAX_BYTES`
bounds what one line of Cargo's stream may contribute, `MESSAGE_MAX_COUNT` how
many messages one scan may keep, `RUSTUP_OUTPUT_LIMIT` what a toolchain probe
may print, and none of them decides which diagnostic the report may publish: an
oversized line is counted malformed and the stream keeps going, a flood stops
with an error at the `parsing` stage. The reader had no bound at all in a crate
whose git layer documents that every way out of it is bounded, and the scanned
workspace's procedural macros are what decide how many diagnostics Cargo emits.
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
catalogued rule it does not list is published as not evaluated.

`--max-duration` arms `execution/process.rs`'s watchdog. On Unix a bounded child
leads its own process group, and the whole group is killed through `kill(1)`,
which keeps `libc` a dev-dependency. Only bounded children get a group, because
a terminal's Ctrl-C no longer reaches one. On Windows `taskkill /T` kills the
tree it can see, and nothing tests it. Passes the deadline has not started are
skipped at their own stage, and the structure budget is capped by what is left.

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

