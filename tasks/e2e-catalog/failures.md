# rust-doctor e2e catalog: failure modes, environment, output contracts, privacy

Binary: `target/release/rust-doctor` 0.7.0 built from `b5389f7`. Host toolchain 1.98.0 (rustup 1.29.0), toolchains 1.95.0 to 1.98.1 installed, all with clippy. Measured 2026-10-20.

## Conventions

- `rd <ws> ARGS` means the binary run under `env -i` with only `HOME=<fake home>`, `PATH=~/.cargo/bin:/usr/bin:/bin`, `RUSTUP_HOME=~/.rustup`, `CARGO_HOME=<scratch>`, `CARGO_TARGET_DIR=<scratch>/targets/<id>`, `TMPDIR=<scratch>`, `LANG=C.UTF-8`, stdin `/dev/null`. The scrub matters: the harness itself runs under agent markers (`CLAUDECODE`, `AI_AGENT`, ...), and `CLAUDECODE` alone changes stderr (skill notice) and disables the interactive report.
- `ws/<name>` is a copy of `tests/fixtures/projects/<name>` or `tests/fixtures/scan-failures/<name>` (prefixed `sf-`) without its `target/`, or a crate written for the scenario (setup column says which).
- "JSON doc" = stdout is exactly one JSON object, one trailing `\n`, `schema_version: 23`.
- Rust coverage names the existing test (file::fn) or `none`. "unit" means only an in-crate unit test covers the logic, not the binary.
- Verified = observed on the binary today with the exact output recorded. `no` = not run, expectation derived from source.
- Scripts used: `bin/rd`, `bin/raw`, `bin/sig.sh`, `bin/sig-native.sh`, `bin/pty_run.py`, `bin/priv.sh`, `bin/sum.py` in this directory.

## Harness lessons (apply to every test file)

1. Spawn the binary directly with `Bun.spawn` and an explicit `env` object. A child started by a non-interactive shell with `&` inherits SIGINT/SIGQUIT as ignored (SIGQUIT test below was a no-op for that reason; SIGINT only works because the binary installs its own handler).
2. One `CARGO_TARGET_DIR` and one `CARGO_HOME` per test. A shared target dir serializes on Cargo's build lock (see SG-09).
3. No `script`/`expect` on this host: the pty helper is a 90-line python3 `pty`/`TIOCSWINSZ` script. Keep it as a test helper rather than adding a dependency.
4. Network-isolated runs: `unshare -rn` and `bwrap --dev-bind / / --unshare-net` both work unprivileged here; gate with `test.skipIf(!available)`.
5. `strace -f -e trace=socket,connect` is available and is the strongest "no network" oracle: zero socket syscalls across the whole process tree on a cached workspace.

---

## EX: exit status matrix

| ID | Setup | Command | Expected observable | Rust coverage | Verified |
|---|---|---|---|---|---|
| EX-01 | ws/clean | `rd ws/clean --yes --json` | exit 0, status `complete`, gate `passed`, score 100 authoritative, stderr empty | product_proof::fixtures_prove_complete_incomplete_and_source_preservation | yes |
| EX-02 | ws/clippy-warning (one `clippy::unwrap_used` warning) | `rd ws/clippy-warning --yes --json` | exit 0 (warnings do not block at default `error`), gate passed, 1 warning | local_cli_experience::terminal_default_verbose_clean_partial_and_no_rust_modes_are_truthful | yes |
| EX-03 | ws/clippy-warning | `... --blocking warning` | exit 1, gate `failed`, `blocking_diagnostics: 1`, status still `complete` | policy_gate_product_proof (matrix) | yes |
| EX-04 | ws/clippy-warning | `... --warning-exit-code 42` | exit 42, gate passed | warning_exit_code::a_passing_run_with_warnings_exits_with_the_chosen_code_and_nothing_else_does | yes |
| EX-05 | ws/clean | `... --warning-exit-code 42` | exit 0 (nothing would block under warning) | same | yes |
| EX-06 | ws/clippy-warning | `... --blocking none --warning-exit-code 42` | exit 0 (`--warning-exit-code` ignored under `none`) | same | yes |
| EX-07 | ws/compile-error | `... --json` | exit 1, status `incomplete`, gate `not-evaluated`, errors `clippy/packages-unlinted`, `execution/build-failed`, `execution/clippy-exit`; E0308 published | scan_failures::a_broken_member_leaves_the_others_linted_and_is_named | yes |
| EX-08 | ws/compile-error | `... --warning-exit-code 42` | exit 1 (precedence 2, 1, warning, 0) | none at binary level | yes |
| EX-09 | ws/sf-sleeping-build | `... --max-duration 3 --json` | exit 2 after 3.0 s, status `incomplete` (not failed), `clippy/deadline-exceeded` "The scan stopped at the 3 s limit set by --max-duration. Not linted: sleeper.", findings of `quick` kept (clippy::todo), score 94 non-authoritative | scan_failures::the_deadline_ends_the_scan_kills_the_build_script_and_keeps_what_was_found | yes |
| EX-10 | ws/sf-sleeping-build | `... --max-duration 2 --warning-exit-code 9` | exit 2 (deadline outranks warning code) | none | yes |
| EX-11 | non-Cargo dir | `rd ws/not-cargo --yes --json` | exit 2, status `failed`, `discovery/no-manifest` "no Cargo.toml found in this path or its ancestors", `audit.score: null`, `policy/scope/project: null` | product_proof::failed_json_report_is_valid_and_returns_two | yes |
| EX-12 | nonexistent path | `rd ws/does-not-exist --yes --json` | exit 2, `discovery/no-manifest` "path is not a directory" | none | yes |
| EX-13 | file path | `rd ws/clean/src/lib.rs --yes --json` | exit 2, `no-manifest` "path is not a directory" (no upward discovery from a file) | none | yes |
| EX-14 | ws/clippy-warning, stdout to `/dev/full` with gate failing | `rd ... --blocking warning > /dev/full` | exit 2 (any non-broken-pipe write error returns 2, outranking gate 1), stderr `Failed to write report: could not serialize report: No space left on device (os error 28)` | none | partial (observed on ws/clean; gate-failing variant follows from `main.rs` and is not run) |
| EX-15 | any scan, SIGINT | see SG-01 | exit 130 | scan_failures::a_sigint_kills_the_build_and_leaves_one_failed_report_and_no_snapshot | yes |
| EX-16 | `--help` | `rust-doctor --help` | exit 0, lists the five exit statuses and "The precedence is 2, 1, the warning code, then 0." | product_proof::default_path_terminal_help_and_clap_errors_follow_cli_contract | no |

## AR: argument validation (all must stop before any process starts)

| ID | Setup | Command | Expected observable | Rust coverage | Verified |
|---|---|---|---|---|---|
| AR-01 | ws/clean | `--max-duration 0` | exit 2, stdout empty, stderr `error: invalid value '0' for '--max-duration <SECONDS>': 0 is not in 1..=86400` | scan_failures::a_max_duration_outside_its_range_is_refused_by_clap | yes |
| AR-02 | ws/clean | `--max-duration 86401` | exit 2, `86401 is not in 1..=86400` | same | yes |
| AR-03 | ws/clean | `--warning-exit-code 2` / `126` | exit 2, `2 is not in 3..=125` / `126 is not in 3..=125` | none | yes |
| AR-04 | ws/clean | `--bogus` | exit 2, `error: unexpected argument '--bogus' found` + tip + usage | product_proof::default_path_terminal_help... | yes |
| AR-05 | ws/clean | `--blocking fatal` | exit 2, `[possible values: none, error, warning]` | none | yes |
| AR-06 | ws/clean | `--rule nonsense` / `--category x` | exit 2, stdout empty, stderr exactly `error: expected KEY=LEVEL with LEVEL one of: off, warn, error` with NO trailing newline and no usage/help hint (BUG-10); value not echoed (redaction intended) | none | yes |
| AR-07 | ws/clean | `--rule clippy::does_not_exist=warn` | exit 2, JSON doc, `policy/unknown-rule` "Unknown rule selector." (selector not echoed) | unit (policy.rs) | yes |
| AR-08 | ws/clean | `--rule clippy::todo=off --rule clippy::todo=warn` | exit 2, `policy/duplicate-rule-override` | unit | yes |
| AR-09 | ws/clean | `--category nope=warn` | exit 2, `policy/unknown-category` | unit | yes |
| AR-10 | ws/clean | `--rule 'clippy::*=off'` | exit 2, `policy/invalid-rule-selector` (no globs) | unit | yes |
| AR-11 | ws/clean | `--base HEAD` without scope | exit 2, stderr `error: --base <REF> requires a files, lines, contextual or baseline scope` + usage, stdout empty | main.rs unit invalid_scope_combinations_are_rejected_before_execution | yes |
| AR-12 | ws/clean | `--staged`, `--include-untracked` alone, `--scope full --staged`, `--scope baseline --include-untracked` | exit 2 each, clap-style error | same unit | no |
| AR-13 | ws/clean (not a git repo) | `--scope files` | exit 2, `scope/base-undetected` "No base branch found: pass --base <REF>." (misleading outside git: passing --base does not help, see AR-14) | none | yes |
| AR-14 | ws/clean (not a git repo) | `--scope files --base HEAD` | exit 2, `scope/base-unavailable` "Git base commit is unavailable." | none | yes |
| AR-15 | ws/clean | `--package nope` | exit 2, `policy/unknown-package` "--package names no workspace member. Members: clean." | precision_controls (unit-level) | yes |
| AR-16 | ws/sf-broken-member | `--package broken-a --package nope` | exit 2, same code; message does not say which name is unknown | none | yes |
| AR-17 | ws/clean | `--package clean` | exit 0, score non-authoritative `rules-not-evaluated` (repo rules `package-scoped`) | precision_controls | yes |
| AR-18 | ws/clean | `inspect ws/clippy-warning` (historical alias) | byte-identical JSON to the root form | local_cli_experience::root_path_default_and_historical_alias_execute_the_same_inspection | yes |

## SG: signals, deadlines, process lifecycle

| ID | Setup | Command | Expected observable | Rust coverage | Verified |
|---|---|---|---|---|---|
| SG-01 | ws/sf-sleeping-build, `RUST_DOCTOR_SLEEPER_PID=<file>` | `rd ... --json`, SIGINT once the pid file exists | exit 130 within 0.05 s, JSON doc, status `failed`, `clippy/interrupted` "The scan was interrupted by a signal.", sleeper pid gone, TMPDIR empty | scan_failures::a_sigint_kills_the_build... | yes |
| SG-02 | same | SIGTERM | identical to SG-01 | none (SIGTERM not exercised) | yes |
| SG-03 | same | SIGINT twice 50 ms apart | exit 130; second signal must `_exit(130)` at once. Racy at e2e level: first signal already finished the run in 50 ms here. Corrected 2026-10-20: a second signal within 500 ms is the same delivery (US-010), so one `clippy/interrupted` report is published and the run exits 130 | scan_failures::a_second_signal_kills_the_build_and_exits_at_once | partial |
| SG-04 | same, linear | SIGINT without `--json` | exit 130, stdout `Scope: full codebase` / `Scan failed: The scan was interrupted by a signal. (clippy/interrupted)` | none | yes |
| SG-05 | ws/bignative (3000 modules, 24 MB, every `clippy::*` rule off in rust-doctor.toml so only native passes run, ~4 s) | SIGINT at 1.5 s | exit 130 in 0.44 s, status `failed`, `execution/interrupted` (stage differs from SG-01) | none | yes |
| SG-06 | same | SIGTERM at 1.5 s | same as SG-05 | none | yes |
| SG-07 | ws/sf-sleeping-build | SIGHUP (terminal closed) | exit 129, NO report, and the whole Cargo process group (cargo-clippy, cargo, build-script) keeps running (BUG-04). Corrected 2026-10-20: fixed by US-010, SIGHUP ends the scan as SIGINT does: exit 130, `clippy/interrupted` report, Cargo's group gone | none | yes |
| SG-08 | git repo copy of sf-sleeping-build with a change, `--scope baseline --base HEAD` | SIGHUP vs SIGINT | SIGHUP: `$TMPDIR/rust-doctor-baseline-<pid>-0` left behind + orphan group. SIGINT: exit 130, TMPDIR empty, no orphan. Corrected 2026-10-20: fixed by US-010, SIGHUP behaves as SIGINT: exit 130, `interrupted`, TMPDIR empty, no orphan | scan_failures (SIGINT only) | yes |
| SG-09 | scan A on sf-sleeping-build holds the build lock (shared target dir) | scan B `ws/clean --max-duration 3` same `CARGO_TARGET_DIR` | B exit 2, `clippy/deadline-exceeded` "... Not linted: clean. Cargo waited on the build directory lock held by another Cargo or rust-analyzer.". Corrected 2026-10-20: B's status is `incomplete`, as every Clippy deadline after the native passes is | unit (clippy::cut_short) | yes |
| SG-10 | PATH dir with a `rustc` wrapper that sleeps 30 s on `--version` | `--max-duration 2` | exit 2 at 2.0 s, status `failed`, `toolchain/deadline-exceeded`, no `sleep` left | scan_failures::a_version_probe_that_sleeps_ends_at_the_run_deadline | yes |
| SG-11 | same wrapper | no deadline | exit 2 at 10.0 s, `toolchain/probe-timeout` "rustc did not report a version within 10 s. Install a Rust toolchain ..." | none | yes |
| SG-12 | ws/sf-sleeping-build | SIGKILL | exit 137, orphan Cargo group survives (inherent; document as known limitation, `timeout -s KILL` in CI leaks builds) | none | yes |
| SG-13 | ws/sf-sleeping-build | SIGQUIT | must be spawned without inherited ignore; expect default core/terminate and orphaned group like SG-07. Corrected 2026-10-20: verified with Bun.spawn, SIGQUIT is not handled: the binary dies of SIGQUIT with no report and Cargo's group survives, as under SIGKILL (SG-12) | none | no (harness artifact, see lesson 1) |
| SG-14 | two concurrent `--scope baseline` runs on one workspace | as in Rust test | both complete | scan_failures::two_concurrent_baseline_runs_on_one_workspace_both_complete | no |

## OW: output write failures

| ID | Setup | Command | Expected observable | Rust coverage | Verified |
|---|---|---|---|---|---|
| OW-01 | ws/clean | `rd ... --json \| head -c 10` | binary exit 0 (broken pipe returns the scan status), stderr empty | unit (render broken pipe) | yes |
| OW-02 | ws/clippy-warning `--blocking warning` | same pipe | exit 1 | none | yes |
| OW-03 | ws/clippy-warning `--warning-exit-code 7` | same pipe | exit 7 | none | yes |
| OW-04 | linear variants of OW-01..03 | `\| head -c 10` | same codes, stderr carries only progress lines | none | yes |
| OW-05 | ws/clean | `>&-` (stdout closed) | exit 0, nothing written, no error: Rust reopens closed std fds on `/dev/null`, the report is silently lost | none | yes |
| OW-06 | ws/clean | `> /dev/full` JSON | exit 2, stderr `Failed to write report: could not serialize report: No space left on device (os error 28)` | none | yes |
| OW-07 | ws/clean | `> /dev/full` linear | exit 2, stderr progress lines then `Failed to write report: could not write report: No space left on device (os error 28)` | none | yes |
| OW-08 | failed scan (not-cargo) | `> /dev/full` | exit 2, same message | none | yes |

## TC: toolchain problems

| ID | Setup | Command | Expected observable | Rust coverage | Verified |
|---|---|---|---|---|---|
| TC-01 | `PATH=/usr/bin:/bin` (no cargo anywhere) | `rd ws/clean --yes --json` | exit 2, `execution/cargo-unavailable` "Cargo could not be started: No such file or directory (os error 2)" raised by `cargo metadata`, so the probe's rustup remedy never appears (BUG-12) | unit (execution/tests.rs) | yes |
| TC-02 | PATH dir holding only a `cargo` symlink to a real toolchain binary | same | exit 2, `execution/rustc-unavailable` "rustc could not be started: ... Install a Rust toolchain from https://rustup.rs and put `rustc` on PATH."; `toolchain.cargo/rustc/clippy` all null ("three or none") | unit | yes |
| TC-03 | PATH dir with cargo+rustc+rustdoc symlinks, no cargo-clippy | same | exit 2, `execution/clippy-unavailable` "Clippy could not report a version (exit status: 101): error: no such command: `clippy`. Clippy is required: install the component with `rustup component add clippy`, or add `components: clippy` ..." Corrected 2026-10-20: Cargo also finds `cargo-clippy` under `$CARGO_HOME/bin`, so the row needs a scratch `CARGO_HOME` to withhold it. | execution/tests.rs (unit), render/tests.rs | yes |
| TC-04 | scratch `RUSTUP_HOME` with a linked toolchain lacking clippy (`settings.toml` default `noclippy`) | same | exit 2, `clippy-unavailable` "... error: 'cargo-clippy' is not installed for the custom toolchain 'noclippy'.. Clippy is required ..." (double period, BUG-11) | none | yes |
| TC-05 | crate with `rust-version = "1.99"` on rustc 1.98 | same | exit 1, status `incomplete`, `execution/build-finished-missing` + `execution/clippy-exit` "... rustc 1.98.0 is not supported by the following package: msrv@0.1.0 requires rustc 1.99"; package score 100 "Great" non-authoritative with no `unlinted` (BUG-05) Corrected 2026-10-20: since US-009 the package is `unlinted: true` with `score: null`, `packages-unlinted` names it, and `audit.score` is null. | none | yes |
| TC-06 | ws/clippy-warning + `rust-toolchain.toml` `channel = "1.95.0"` (installed) | same | exit 0, `toolchain.rustc` = "rustc 1.95.0 (59807616e 2026-04-14)" (probe honors the directory pin), no catalogued lint `not_evaluated` Corrected 2026-10-20: the e2e row pins any installed toolchain other than the default (1.95.0 here), since CI installs only its pin and the runner's stable. | none | yes |
| TC-07 | `rust-toolchain.toml` pinning uninstalled `1.80.0`, `RUSTUP_AUTO_INSTALL=0` | same | exit 2, `metadata/cargo-metadata` "... toolchain '1.80.0-x86_64-unknown-linux-gnu' is not installed help: run `rustup toolchain install` to install it" | none | yes |
| TC-08 | same pin, rustup auto-install default, scratch `RUSTUP_HOME`, under `unshare -rn` | same | exit 2, message shows rustup TRIED TO DOWNLOAD ("syncing channel updates ... Network is unreachable") and leaks the absolute `RUSTUP_HOME/tmp/...` path (BUG-02, BUG-09). Without the namespace this would install a toolchain Corrected 2026-10-20: since US-004 the path reads `$RUSTUP_HOME/tmp/...`; rustup still tries the download (its own, documented beside `CARGO_NET_OFFLINE`), and `RUSTUP_AUTO_INSTALL=0` stops it trying (TC-07). | none | yes |
| TC-09 | catalogued lint missing from an older clippy (`clippy-driver -W help`) | pin a toolchain older than a catalogued lint | that rule `not_evaluated`, `-A unknown_lints` only when the list is unavailable; score reason Corrected 2026-10-20: proven with a fake `clippy-driver` ahead of PATH whose `-W help` omits `clippy::todo` (rule `not_evaluated: "unknown-to-toolchain"`, left out of the command), and one that cannot list (`toolchain/lint-list-unavailable`, `unknown_lints` allowed). | unit (lint_list) | no (no installed toolchain lacks a catalogued lint) |
| TC-10 | `RUSTFLAGS=-Dwarnings` on ws/clippy-warning | `--json` | exit 0, `toolchain.removed_lint_flags: ["-D warnings"]`, warning stays warning | scan_failures::a_caller_exporting_deny_warnings_keeps_the_warning_a_warning | yes |
| TC-11 | `CARGO_ENCODED_RUSTFLAGS=-Dwarnings` | same | identical to TC-10 | none | yes |
| TC-12 | `RUSTFLAGS=-Fclippy::unwrap_used` / `--cap-lints=allow` | same | exit 0, removed `-F clippy::unwrap_used` / `--cap-lints allow`, finding still reported as warning | unit (rustflags) | yes |
| TC-13 | `RUSTFLAGS=-Ctarget-cpu=native` | same | `removed_lint_flags: []`, flags untouched | scan_failures::rustflags_without_a_lint_level_are_left_alone | yes |
| TC-14 | `CARGO_BUILD_RUSTFLAGS=-Dwarnings` | same | exit 0, `removed_lint_flags: []` (cannot strip, `-W warnings` appended) | scan_failures::a_target_table_or_cargo_build_rustflags_cannot_deny_warnings | yes |
| TC-15 | `.cargo/config.toml` `[build] rustflags = ["-D","warnings"]` | same | stripped, complete | scan_failures::the_workspace_configuration_cannot_deny_warnings_either | no |

## WS: workspace shapes and filesystem conditions

| ID | Setup | Command | Expected observable | Rust coverage | Verified |
|---|---|---|---|---|---|
| WS-01 | ws/sf-bad-manifest | `--json` | exit 2, `metadata/cargo-metadata` quoting Cargo's TOML error with `Cargo.toml:8:20` (relative) | scan_failures::a_manifest_cargo_metadata_refuses_publishes_its_cause | yes |
| WS-02 | ws/sf-bad-lockfile | `--json` | exit 1, `dependencies/lockfile-invalid` + `execution/clippy-exit` "failed to parse lock file at: ./Cargo.lock ..." | scan_failures::a_lockfile_cargo_cannot_parse_publishes_cargos_own_sentence | yes |
| WS-03 | ws/sf-broken-member | `--json` | exit 1, `packages-unlinted` "broken-a, dependent-c", healthy-b linted (clippy::todo), score 94 | scan_failures::a_broken_member_leaves_the_others_linted_and_is_named | yes |
| WS-04 | crate whose `build.rs` writes a marker then `exit(3)` | `--json` | exit 1, marker written (trust boundary: build.rs runs), `clippy-exit` "failed to run custom build command ... `$CARGO_TARGET_DIR/debug/build/.../build-script-build` (exit status: 3)" (target dir scrubbed) | none | yes |
| WS-05 | virtual workspace with `members = []` | `--json` | exit 1, `clippy-exit` "manifest path `.` contains no package: The manifest is virtual, and the workspace has no members." | none | yes |
| WS-06 | manifest with `[lib] path = "src/lib.rs"` but no file (no Rust files) | `--json` | exit 1, `audit.score: null`, `source/read-failed` 'Source path "src/lib.rs" could not be resolved.' + clippy-exit | local_cli_experience (no-rust mode) | yes |
| WS-07 | `tests/fixtures/projects/virtual-workspace` + sibling `shared` | `--json` | exit 1, `source/path-outside-workspace` 'Source path "lib.rs" resolves outside the workspace.' | product_proof::virtual_workspace_retains_external_members_without_exposing_their_manifests | yes |
| WS-08 | `[workspace] members = ["../gone"]` (missing sibling) | `--json` and linear | exit 2, `metadata/cargo-metadata`, message contains the ABSOLUTE path of `../gone/Cargo.toml` in both outputs (BUG-02) Corrected 2026-10-20: since US-004 the message carries no absolute path. A member whose name extends the root's (`../gone-member` of `gone`) is mangled into `.-member/Cargo.toml` (failures/BUG-16). | none | yes |
| WS-09 | two crates `amb/a`, `amb/b`, scan `amb` | `--json` | exit 2, `discovery/ambiguous-workspace` "Several Cargo workspaces under this path: a, b. Pass one of them." | source_kernel_cli (ambiguous) | yes |
| WS-10 | one crate below the path (`ws2/other place ü/`) | `--json` | exit 0, JSON byte-identical to scanning the crate directly; stderr (even with `--json`) `No Cargo workspace at this path: scanning the one in other place ü/.` | none | yes |
| WS-11 | scan `ws/clippy-warning/src` (subdir) and `ws/clippy-warning/Cargo.toml` | `--json` | byte-identical to the root scan | none | yes |
| WS-12 | symlink `links/warn-link -> ws/clippy-warning` | `--json` | byte-identical | none | yes |
| WS-13 | workspace dir with a space and non-ASCII (`other place ü`), absolute and relative invocation | `--json` | byte-identical to the plain copy | none | yes |
| WS-14 | workspace dir name with byte `\xff` | `--json` | exit 2, `cargo-metadata` "invalid UTF-8 was detected in one or more arguments ..." (Cargo's limit) | none | yes |
| WS-15 | `src/big.rs` 9,000,045 bytes (> 8 MiB `file-bytes`) | `--json` | exit 1, `source/limit-exceeded` 'Source limit "file-bytes" exceeded (maximum 8388608).', Clippy findings still published | unit (source_kernel limits) | yes |
| WS-16 | `src/generated.rs` opening with `// @generated ... DO NOT EDIT.` holding an `unwrap` | `--json` | exit 0, only `src/lib.rs` finding, `scan.excluded_generated: 1` | none at binary level | yes |
| WS-17 | 3000-module crate (bignative) | `--json --timings` | completes; one `duplicate_function_body` diagnostic with 119,999 `related` entries makes a 12.8 MB single-line JSON (no bound on `related`, OBS-1) Corrected 2026-10-20: 3000 `mod` lines in one file trip the per-file `alias-bindings` limit (1024), so the e2e crate nests them 60 x 50; `related` still lists every duplicate. | none | yes |
| WS-18 | path dependency outside the workspace with a compile error | `--json` | exit 1, `cargo::path_dependency_outside_workspace`, E0308 of the dep with `path: null` (no absolute path), `app` named unlinted | none | yes |
| WS-19 | crate with uncached crates.io dep, empty CARGO_HOME, `unshare -rn` | `--json` | exit 1 after ~10.8 s of Cargo retries, `clippy-exit` "spurious network error ... index.crates.io:443 ... failed to get `itoa`"; package score 100 non-authoritative, no `unlinted` (BUG-05, BUG-09) Corrected 2026-10-20: under `CARGO_NET_OFFLINE=true` Cargo fails at once ("no matching package named `itoa` found"); since US-009 the package is unlinted with no score. | none | yes |
| WS-20 | `tests/fixtures/cargo-health/offline-registry` (vendored, `net.offline = true`) | `--json` under `unshare -rn` | complete, deps resolved from vendor, identical to the networked run | advisory_database (uses it) | partial (complete/exit 0 observed without the namespace in AD-01..07) |
| WS-21 | lib crate without Cargo.lock | `--json`, diff workspace before/after | exit 0 but `Cargo.lock` CREATED in the workspace (BUG-03) Corrected 2026-10-20: since US-005 the scan removes the lockfile it created; the tree fingerprint is unchanged. | none | yes |
| WS-22 | bin crate without Cargo.lock, scanned twice | `--json` x2 | run 1 score 94 with `cargo::missing_lockfile`; run 2 score 99 without it: the scan's own side effect changes its next verdict (BUG-03) Corrected 2026-10-20: since US-005 both scans carry `missing_lockfile` and agree once `timings` is removed. | none | yes |
| WS-23 | read-only workspace (`chmod -R a-w`) with lockfile | `--json` | exit 0, complete (nothing needs writing) | none | yes |
| WS-24 | read-only workspace without lockfile | `--json` | exit 1, `clippy-exit` "failed to write ./Cargo.lock ... Permission denied (os error 13)" Corrected 2026-10-20: still Cargo's refusal when the tree is read-only from the start; a lockfile Cargo wrote but the scan cannot remove (build script making the root 0555) is named by one stderr line (US-005). | none | yes |
| WS-25 | read-only `CARGO_TARGET_DIR` | `--json` | exit 1, `clippy-exit` "failed to create directory `$CARGO_TARGET_DIR/debug` ... Permission denied" (scrubbed) Corrected 2026-10-20: the sibling Cargo creates beside the target directory is scrubbed as `$CARGO_TARGET_DIR<suffix>` (failures/BUG-16). | none | yes |
| WS-26 | `CARGO_TARGET_DIR` unset | `--json` | `target/` created inside the workspace | none | yes |
| WS-27 | `CARGO_TARGET_DIR=reltarget` (relative) invoked from another cwd | `--json` | target created at `<workspace>/reltarget`, not `<cwd>/reltarget` (cwd is the workspace root for Cargo) | none | yes |
| WS-28 | `CARGO_BUILD_TARGET_DIR=<abs>` | `--json` | build lands there, exit 0 | none | yes |
| WS-29 | `rust-doctor.toml` `[rules` | `--json` | exit 2, `configuration/config-invalid` "Invalid rust-doctor.toml at line 1, column 7." | configuration_kernel | yes |
| WS-30 | `rust-doctor.toml` mode 000 | `--json` | exit 2, `configuration/config-unreadable` "Could not read rust-doctor.toml." | none | yes |
| WS-31 | `src/hidden.rs` mode 000 referenced by `mod hidden;` | `--json` | exit 1, `source/read-failed` 'Source path "src/hidden.rs" could not be opened.' + compile failure | none | yes |
| WS-32 | `.cargo/config.toml` mode 000 | `--json` | exit 2, `cargo-metadata` "could not load Cargo configuration ... `./.cargo/config.toml` ... Permission denied" | execution/tests.rs (cargo-config-unreadable, unit) | yes |
| WS-33 | git repo, git removed from PATH | `--json` | exit 1, `repo/git-unavailable` "Repository hygiene skipped: git was not available.", gate not evaluated | repo_hygiene unit an_absent_git_degrades_to_a_recorded_error | yes |
| WS-34 | NOT a git repo, git removed from PATH | `--json` | exit 1 as WS-33, although the same workspace with git on PATH is complete/exit 0 (BUG-08) | none | yes |
| WS-35 | not a git repo, git present | `--json` | exit 0, complete, repo rules silent | repo_hygiene unit a_workspace_outside_git_yields_no_finding_and_no_error | yes |
| WS-36 | `TMPDIR` read-only or missing, `--scope baseline --base HEAD`, `RUST_DOCTOR_NO_CACHE=1` | `--json` | exit 2, `baseline/baseline-temp-unavailable` "Git baseline temporary storage is unavailable." | none | yes |
| WS-37 | `TMPDIR` read-only, full scan | `--json` | exit 0 (full scan never writes TMPDIR) | none | yes |
| WS-38 | `HOME` unset, `CARGO_HOME` unset | `--json` | exit 0 (Cargo falls back to passwd home); advisory discovery finds nothing because `locate` needs HOME or CARGO_HOME | none | yes |

## AD: advisory database

Workspace: copy of `tests/fixtures/cargo-health/offline-registry`; database: `tests/fixtures/advisory-db` (newest advisory 2020-01-20).

| ID | Setup | Command | Expected observable | Rust coverage | Verified |
|---|---|---|---|---|---|
| AD-01 | empty CARGO_HOME | `--json` | exit 0, `scan.advisory_db` absent, both `cargo::vulnerable_dependency` / `unmaintained_dependency` `not_evaluated: "advisory-db-missing"`, score authoritative; linear "No local RustSec database: run cargo audit once to create one." | advisory_database::without_a_database_both_rules_are_not_evaluated_and_the_score_keeps_its_authority | yes |
| AD-02 | `--advisory-db <fixture>` | `--json` | `advisory_db: {source: explicit, date: 2020-01-20, advisories: 7, skipped: 1}`, 3 advisory findings; path never published | advisory_database::a_named_database_reports_the_affected_package_at_its_lockfile_entry | yes |
| AD-03 | `--advisory-db /dev/null` | `--json` | exit 0, silently identical to AD-01 (BUG-07). Corrected 2026-10-20: `/dev/null` is the documented off switch (US-006); the report equals AD-01's | none | yes |
| AD-04 | `--advisory-db <typo path>` | `--json` and linear | exit 0, identical to AD-01, linear tells the user to "run cargo audit once" although they named a database (BUG-07). Corrected 2026-10-20: fixed by US-006, exit 2 before any work, stdout empty, stderr `rust-doctor: --advisory-db: <path> does not exist.` | none | yes |
| AD-05 | `$CARGO_HOME/advisory-db` copy | `--json` | `source: cargo-audit` | advisory_database::discovery_reads_what_cargo_audit_or_cargo_deny_left_under_cargo_home | yes |
| AD-06 | `$CARGO_HOME/advisory-dbs/github.com-abc` copy | `--json` | `source: cargo-deny` | same | yes |
| AD-07 | `CARGO_HOME=` (empty) and `$HOME/.cargo/advisory-db` | `--json` | falls back to HOME, `source: cargo-audit` | none | yes |
| AD-08 | stale database (2020) | linear | "The advisory database is 2465 days old." (value depends on today: assert the shape `is \d+ days old`) | advisory_database::a_stale_database_is_read_and_its_age_is_stated | yes |
| AD-09 | malformed advisory file in `crates/x/` | `--json` | counted in `skipped`, no error | unit (advisories) | no |
| AD-10 | database present, scan under `unshare -rn` | `--json` | identical to networked run (the db is never fetched) | none | no (implied by NW-01) |

## EN: environment and terminal profile

pty runs use `bin/pty_run.py` (stdin, stdout, stderr on one pty, `TIOCSWINSZ` sets columns); "TUI" was identified by output size (about 22 KB of redraws vs about 3 KB linear) after sending `q`.

| ID | Setup | Command | Expected observable | Rust coverage | Verified |
|---|---|---|---|---|---|
| EN-01 | pty 100 cols | `rd ws/clippy-warning --yes` | linear report, SGR colors present (150 sequences), no animation, stderr progress rewritten with `\r\x1b[2K` (4) | local_cli_experience::yes_under_a_terminal_prints_no_animation_frames | yes |
| EN-02 | pty + `NO_COLOR=1` and `NO_COLOR=` (empty) | same | zero SGR sequences with `NO_COLOR=1`, color kept with an empty `NO_COLOR`. Corrected 2026-10-20: no-color.org counts only a non-empty value, and US-014 made the binary follow it (tui/BUG-11) | unit terminal_profile | yes |
| EN-03 | pty + `TERM=dumb` | same | zero SGR, width 80, no progress rewriting (0 erase sequences) | unit | yes |
| EN-04 | pty 60 / 30 / 200 cols | same | non-URL lines wrapped at 60 / clamped at 40 (MIN_WIDTH) / up to 200; URL lines never cut | unit (render width) | yes |
| EN-05 | pty with 0 columns + `COLUMNS=70` | same | wraps at 70 | unit | yes |
| EN-06 | stdout piped + `COLUMNS=120` | same | width 80 (COLUMNS ignored off a terminal) | unit | no |
| EN-07 | pty, no `--yes` | `rd ws/clippy-warning`, send `q` | interactive report, exit = scan status (0) | local_cli_experience::q_escape_and_ctrl_c_all_leave_the_report_with_the_scan_exit_code | yes |
| EN-08 | same with `--blocking warning` | send `q` | exit 1 from the TUI | same | yes |
| EN-09 | pty + `CI=true` / `CI=no` | no `--yes` | linear (any value but "", "0", "false" counts as CI) | unit ci_is_off | yes |
| EN-10 | pty + `CI=false` / `CI=FALSE` / `CI=0` / `CI=` | no `--yes` | TUI | unit | yes |
| EN-11 | pty + each of the 26 `NON_INTERACTIVE_MARKERS` set to `1` (CLAUDECODE ... DRONE, incl. GIT_DIR, GIT_INDEX_FILE) | no `--yes` | linear for all 26 | local_cli_experience::an_agent_marker_keeps_the_linear_report_on_a_terminal (one marker) | yes |
| EN-12 | pty + `CLAUDECODE=` (empty) | no `--yes` | TUI (empty marker ignored) | unit | yes |
| EN-13 | `CLAUDECODE`, `CODEX_SANDBOX`, `CURSOR_AGENT` | linear | stderr ends with `rust-doctor: no rust-doctor skill is installed for claude; run \`rust-doctor skill install --agent claude\`.` (codex/cursor analogues); the other 23 markers print no notice | agent_skill_notice::an_agent_hears_once_that_its_skill_is_missing_stale_or_nothing_when_current | yes |
| EN-14 | `CLAUDECODE=1` | `--json` | stderr empty, JSON identical | same | yes |
| EN-15 | pty + `TERM=dumb`, no `--yes` | | linear, no color | unit | yes |
| EN-16 | pty, `--verbose`, no `--yes` | | linear but ANIMATED: 175 KB / 13,532 SGR sequences of score-block frames (`animate` ignores `--verbose`); with `--yes` no frames | local_cli_experience::verbose_keeps_the_linear_report_on_a_terminal | yes |
| EN-17 | pty, failed scan (not-cargo), no `--yes` | | linear failure, exit 2, no TUI | none | yes |
| EN-18 | stdin `/dev/null`, stdout piped, no `--yes` | | linear, never waits for input | local_cli_experience::redirected_ci_and_yes_runs_never_offer_or_launch_handoff | yes |
| EN-19 | off a terminal | linear | stderr: one line per phase `Running native passes...`, `Compiling dependencies...`, `Linting <pkg> (1/1)` | scan_failures::off_a_terminal_each_phase_prints_one_line | yes |
| EN-20 | `--json` | | no progress on stderr | scan_failures::a_json_run_prints_no_progress | yes |
| EN-21 | `GIT_DIR=/nonexistent` exported by caller (hook context) | `--json` on a git workspace | git calls ignore it (env removed), repo pass works | git unit (GIT_ENVIRONMENT_OVERRIDES) | partial (linear run exit 0) |
| EN-22 | stdout tty, stderr piped | no `--yes` | interactive report still opens | local_cli_experience::the_report_uses_stdin_and_stdout_when_stderr_is_redirected | no |

## OC: output contracts

| ID | Setup | Command | Expected observable | Rust coverage | Verified |
|---|---|---|---|---|---|
| OC-01 | every JSON run in this catalog (success and failure) | `--json` | stdout one object, exactly one `\n` (at the end), `schema_version: 23` | local_cli_experience::json_is_one_clean_v8_document... | yes |
| OC-02 | success | `--json` | stderr empty; exceptions: WS-10 discovery line | scan_failures::a_successful_run_publishes_nothing_from_stderr | yes |
| OC-03 | failure before scan | `--json` | stderr empty, report on stdout, exit 2 | product_proof::failed_json_report_is_valid_and_returns_two | yes |
| OC-04 | clap/argument errors | | stdout EMPTY (no JSON document), stderr carries the error | none | yes |
| OC-05 | same workspace twice | `--json` | byte-identical (priv probe 30,437 B twice) | policy_gate_product_proof (deterministic) | yes |
| OC-06 | relative `.` vs absolute path, moved workspace, symlink | `--json` | byte-identical | none | yes |
| OC-07 | `--timings` | `--json` | `timings` object with keys probes, metadata, lint_list, clippy_dependencies, clippy_linting, walk, source_kernel, structure, cargo_health, repo, total (ms); absent without the flag | performance.rs (ignored) | yes |
| OC-08 | `--json --verbose` | | byte-identical to `--json` (no timings) | none | yes |
| OC-09 | `--verbose` linear | | adds `Phases: probes N ms, ... total N ms` and every group/location | none | yes |
| OC-10 | linear complete report | | section order: Scope, Scanned, findings, rule, totals, Categories, Configuration, advisory line, (narrowing, delta), Gate, scan errors, advisories, score block, notes, Share/Docs/GitHub | render unit | yes |
| OC-11 | linear failed report | | exactly `Scope: ...` and `Scan failed: <message> (<stage>/<code>)`, no score block | render unit | yes |
| OC-12 | human vs JSON score | | `N / 100` equals `audit.score.value` (65, 94, 100, 92 seen). Label: JSON keeps `Great` when non-authoritative while human says `Core partial`; `Share:` URL only when authoritative | none | yes |
| OC-13 | `report markdown <json>` | | renders without republishing secrets or suggestions (see PV-07) | ci_and_report_markdown | yes |
| OC-14 | 12.8 MB report (WS-17) | `--json` | still one line + `\n` | none | yes |
| OC-15 | failed reports | `--json` | `audit.score: null` for discovery/policy/toolchain/metadata failures, but `{value: 100, label: Great, authoritative: false}` for `interrupted` (BUG-06) | none | yes |

## PV: privacy and secret redaction

Probe (`bin/priv.sh`): HOME `<scratch>/home-SECRETUSER/u-SECRETUSER-q7`, workspace `$HOME/My Projects/ws ünï-zz` (git, author SECRETUSER), `CARGO_HOME` and `CARGO_TARGET_DIR` and `TMPDIR` OUTSIDE HOME, env `USER/LOGNAME=SECRETUSER`, `SECRET_TOKEN_ENV`, `GITHUB_TOKEN=ghp_ENV...`, `AWS_SECRET_ACCESS_KEY`, `DATABASE_URL`, `RUSTFLAGS="-D warnings --cfg planted_rustflag_zz"`. Planted: AKIA key, ghp_ token, `sk-` key inside an `expect` message, `unwrap_or("fallbacksecret...")`, `.env`, `config/db.toml` postgres URL with password, `keys/server.pem` PEM body, CI workflow `run:` with a ghp_ token.

| ID | Setup | Command | Expected observable | Rust coverage | Verified |
|---|---|---|---|---|---|
| PV-01 | probe | `--json`, `--verbose`, default linear, `report markdown` | none of: scratch path, `/home/arthur`, `/tmp/`, SECRETUSER, secretuser, `My Projects`, `ünï`, any PLANTED value, AKIAQWERTY, ghp_ tokens (source, CI, env), sk-live, dbpass, MIIEvQ (PEM body), env values, `planted_rustflag`, CARGO_HOME/target/TMPDIR names. All 0 hits | policy_gate_product_proof::assert_private, product_proof::repository_json_has_exact_top_level_contract_without_absolute_private_paths | yes |
| PV-02 | probe | `--verbose` | frames mask with 4-char prefix: `"AKIA<redacted>"`, `"ghp_<redacted>"`, `sk-l<redacted>`, `dbpa<redacted>@db...`, PEM `----<redacted>` + every body line `<redacted>`, fallback `"fall<redacted>"`; the AKIA line is also masked inside the frame of the neighboring `unwrap_used` finding | code_frame unit tests (062bbf6) | yes |
| PV-03 | probe | `--json` | 14 findings: hardcoded_credential x7, tracked_secret_file x2 (.env, keys/server.pem), secret_env_fallback, unwrap_used, expect_used, unnecessary_to_owned x2; messages name the pattern only. Corrected 2026-10-20: the one probe of the suite also holds PV-05's credential passed through `.to_string()` and a plain-words control beside it | repo_hygiene tests | yes |
| PV-04 | probe | `--json` | `toolchain.removed_lint_flags: ["-D warnings"]`; the non-lint RUSTFLAGS token never appears | none | yes |
| PV-05 | crate with `takes(&"xoxb-<planted body>".to_string())` (git repo) | `--json` and `--verbose` | the `clippy::unnecessary_to_owned` finding is kept with no `suggestion`, the linear report prints no `Replace with:` for it, and `hardcoded_credential` reports the same line; the plain-words control keeps its suggestion. Corrected 2026-10-20: US-001 withholds a suggestion that would carry a credential (BUG-01 fixed) | none | yes |
| PV-06 | `command.arg("-Hghp_<36 chars> x")` | `--verbose` | token printed in clear in the frame: no word boundary before `ghp_`, so neither detection nor masking match (design boundary, document) | unit | yes |
| PV-07 | PV-05 JSON | `report markdown` | markdown does not carry the suggestion (0 hits) | none | yes |
| PV-08 | workspace outside HOME referencing missing `../gone` | `--json` | the message names `<external>/gone/Cargo.toml`, never the absolute path. Corrected 2026-10-20: US-004 scrubs paths outside the workspace and HOME (BUG-02 fixed) | none | yes |
| PV-09 | `RUSTUP_HOME` outside HOME + rustup error | `--json` | the message names `$RUSTUP_HOME/tmp/...`, never the path. Corrected 2026-10-20: US-004 (BUG-02 fixed); run inside a network namespace so rustup's install attempt fails at once | none | yes |
| PV-10 | target dir outside HOME, build script failure | `--json` | path scrubbed to `$CARGO_TARGET_DIR` | unit (cargo_stderr) | yes |
| PV-11 | `--rule <value with a secret>=bogus` | | value never echoed (stderr generic, JSON "Invalid rule selector.") | none | yes (AR-06/07) |
| PV-12 | interactive handoff prompt contents | | the prompt `Copy prompt` prints with no clipboard on PATH carries no private path, username, environment value or planted secret. Corrected 2026-10-20: automated in privacy-redaction.test.ts through the interactive report on a terminal | local_cli_experience handoff tests | no |

## NW: network

| ID | Setup | Command | Expected observable | Rust coverage | Verified |
|---|---|---|---|---|---|
| NW-01 | ws/clean, ws/clippy-warning | normal vs `unshare -rn` vs `bwrap --unshare-net` | byte-identical JSON, same exit | none (Rust tests only set `CARGO_NET_OFFLINE`) | yes |
| NW-02 | ws/clippy-warning | `strace -f -e trace=socket,connect,sendto` | zero socket syscalls in the whole tree (binary, cargo, rustc, clippy-driver, git) | none | yes |
| NW-03 | uncached crates.io dependency | `unshare -rn` | with `CARGO_NET_OFFLINE=true` (the documented switch) the scan ends at once: exit 1, `incomplete`, score null, `packages-unlinted`, Cargo's offline refusal quoted, no lockfile left; the same inside a namespace. Corrected 2026-10-20: US-013 documents `CARGO_NET_OFFLINE`, US-009 drops the score of an unlinted package (BUG-09, BUG-05 fixed) | none | yes |
| NW-04 | pinned uninstalled toolchain | `unshare -rn` | with `RUSTUP_AUTO_INSTALL=0` rustup refuses the uninstalled toolchain (`'1.80.0-...' is not installed`) without syncing a channel; without it rustup tries a download, which PV-09 shows failing inside the namespace. Corrected 2026-10-20: the off switch is the contract the suite holds | none | yes |
| NW-05 | git-lfs smudge filter with `--scope baseline` | `unshare -rn` + strace | the git-lfs filter never runs in any scope, although a control `git cat-file --filters` runs it: every git call switches the LFS filter off and sets `GIT_LFS_SKIP_SMUDGE`. Corrected 2026-10-20: US-013 keeps git filters off the network (BUG-09 fixed); proven with the recording `git-lfs` fake | none | no (git-lfs not installed) |

## FS: filesystem side effects (fake HOME, TMPDIR, CARGO_HOME snapshotted before/after)

| ID | Setup | Command | Expected observable | Rust coverage | Verified |
|---|---|---|---|---|---|
| FS-01 | git workspace with lockfile, full scan | snapshot diff | HOME unchanged, TMPDIR unchanged, workspace unchanged, CARGO_HOME gains only Cargo's `.global-cache`, `.package-cache`, `.package-cache-mutate` | policy_gate_product_proof (workspace non-mutation) | yes |
| FS-02 | same, `--scope baseline --base HEAD` | snapshot diff + strace | snapshot `$TMPDIR/rust-doctor-baseline-<pid>-<n>` (mode 0700, `tree/`, `target/`, `index`) created and removed; side cache at `$CARGO_TARGET_DIR/rust-doctor/cache/base-sides.json`, base build at `$CARGO_TARGET_DIR/rust-doctor/baseline/` | scan_failures (snapshot on SIGINT) | yes |
| FS-03 | rerun FS-02 (cache hit) vs `RUST_DOCTOR_NO_CACHE=1` | | JSON byte-identical | side_cache unit | yes |
| FS-04 | workspace without lockfile | | no `Cargo.lock` is left, the workspace fingerprint is unchanged and two scans agree. Corrected 2026-10-20: US-005 removes the lockfile a scan created (BUG-03 fixed) | none | yes |
| FS-05 | `CARGO_TARGET_DIR` unset | | `target/` in the workspace | none | yes |
| FS-06 | SIGHUP during baseline | | exit 130, the snapshot removed, the build scripts' process group gone. Corrected 2026-10-20: US-010 handles SIGHUP (BUG-04 fixed) | none | yes |

## PG: poisoned repository configuration (modeled on pf `poisoned-repository.test.ts`)

`.gitattributes` `*.rs filter=planted diff=planted`; `.git/config` sets `core.fsmonitor`, `filter.planted.clean/smudge`, `diff.planted.textconv`, `core.pager`, `diff.external`; hooks `post-checkout`, `post-index-change`, `reference-transaction`, `pre-commit`; each program appends its name to a log. Control: plain `git status` runs fsmonitor and a hook.

| ID | Setup | Command | Expected observable | Rust coverage | Verified |
|---|---|---|---|---|---|
| PG-01 | poisoned repo with a modified file | full scan `--json` | log empty: no repository program runs | git unit (fsmonitor/pager flags) | yes |
| PG-02 | same | `--scope files --base HEAD` | log: `clean` only. Corrected 2026-10-20: how often it runs follows the files and git calls of the scope (twice on `mini` today), so the row names which programs run | none | yes |
| PG-03 | same | `--scope lines --base HEAD` | log: `clean` only (three times on `mini` today). Corrected 2026-10-20: counts are not the contract, the set of programs is | none | yes |
| PG-04 | same | `--scope baseline --base HEAD`, no cache | log: `clean`, `smudge` and the `post-index-change` hook only (4, 2 and 1 on `mini` today). Corrected 2026-10-20: counts are not the contract, the set of programs is | none | yes |
| PG-05 | same | `--scope baseline --base HEAD --staged` | log: `smudge` and the `post-index-change` hook only (4 and 1 on `mini` today). Corrected 2026-10-20: counts are not the contract, the set of programs is | none | yes |
| PG-06 | all scopes | | fsmonitor, textconv, pager, external diff never run | git unit | yes |

---

## Bugs and questionable behaviors (minimal repros)

BUG-01 (privacy, high). A Clippy suggestion republishes a credential the frame masks. `src/lib.rs`: `fn takes(v: &str) -> usize { v.len() } pub fn owned() -> usize { takes(&"xoxb-<planted body>".to_string()) }` in a git repo; `rust-doctor . --yes --json` gives `"suggestion":{"replacement":"\"xoxb-<planted body>\""...}`, and `--verbose` prints `Replace with: "xoxb-<planted body>"` directly above a frame showing `xoxb<redacted>`, in the same report where `hardcoded_credential`'s help says "the matched value is never republished by the report". Masking lives only in `presentation/code_frame.rs`; the suggestion path (`report/normalize.rs` select_suggestion, render "Replace with") bypasses `credential_ranges`. The TUI detail pane likely shows it too (not checked).

BUG-02 (privacy, medium). Absolute paths outside the workspace root and outside HOME pass through Cargo/rustup stderr excerpts. `mkdir -p /tmp/x/ws && printf '[workspace]\nmembers = ["../gone"]\n' > /tmp/x/ws/Cargo.toml && rust-doctor /tmp/x/ws --yes --json`: the message holds `/tmp/x/gone/Cargo.toml`. Same for a `RUSTUP_HOME` outside HOME (`/usr/local/rustup` in the official Docker images). `sanitize_text`/`cargo_stderr::excerpt` scrub root, target dir, CARGO_HOME and HOME only. GitLab-style `/builds/...` checkouts are outside HOME.

BUG-03 (correctness, medium). The scan writes `Cargo.lock` into a workspace that has none (Cargo's `clippy` resolves and writes it), so the workspace is mutated and two consecutive scans of a bin crate disagree: 94 with `cargo::missing_lockfile`, then 99. A read-only workspace without a lockfile becomes `incomplete` instead.

BUG-04 (lifecycle, medium). SIGHUP (closing the terminal tab) is not handled: exit 129, no report, and because every child leads its own process group, `cargo-clippy`, `cargo` and build scripts keep running, and a baseline snapshot stays in TMPDIR. SIGINT/SIGTERM are clean. Repro: SG-07/SG-08.

BUG-05 (scoring, medium). When Cargo fails before emitting `build-finished` (MSRV mismatch, unreachable registry, unparsable lockfile), `members.unlinted` is skipped (`build_finished == None`), so the never-linted package gets `score {value:100,label:"Great"}` with no `unlinted: true` and no `packages-unlinted` error. Compare a compile error: package `score: null`, `unlinted: true`. Repro: crate with `rust-version = "1.99"` on 1.98.

BUG-06 (contract, low-medium). An `interrupted` report is `status: failed` yet carries `audit.score = {value: 100, label: "Great", authoritative: false}`; every other failed report has `score: null`. A consumer reading `.audit.score.value` sees 100 for a Ctrl-C'd scan.

BUG-07 (UX/correctness, low-medium). `--advisory-db` naming an unreadable path (`/dev/null`, typo) silently degrades to "no database": exit 0, score authoritative, and the linear report says "No local RustSec database: run cargo audit once to create one." The vulnerability check is off with no signal the explicit flag failed.

BUG-08 (CI, medium, arguably design). Git missing from PATH makes every scan `incomplete` (exit 1, gate not evaluated), even for a workspace outside any git repository, which with git present is `complete`/exit 0. A slim CI image without git can never pass the gate.

BUG-09 (promise mismatch, medium). "The tool never reaches the network" holds for the binary (0 socket syscalls) but not for the scan it runs: Cargo fetches uncached dependencies (no `--offline`/`--frozen`), rustup auto-installs a pinned uninstalled toolchain, and scoped scans run git filters (git-lfs smudge fetches). Either pass `--offline` + `RUSTUP_AUTO_INSTALL=0` or narrow the documented promise.

BUG-10 (cosmetic). `--rule nonsense` and `--category x` print `error: expected KEY=LEVEL with LEVEL one of: off, warn, error` without a trailing newline and without the usage/`--help` hint every other clap error carries (`clap::Error::raw` is never formatted).

BUG-11 (cosmetic). `clippy-unavailable` doubles the period when rustup's first line ends with one: `... for the custom toolchain 'noclippy'.. Clippy is required`. Rustup's standard "is not installed for the toolchain '...'." ends with a period too.

BUG-12 (cosmetic). With no cargo on PATH the failure comes from `cargo metadata` (`execution/cargo-unavailable` "Cargo could not be started: ...") without the "Install a Rust toolchain from https://rustup.rs" remedy the probe would add.

OBS-1. `related[]` is unbounded: 120,000 identical function bodies yield one diagnostic with 119,999 related locations, 12.8 MB of JSON.

OBS-2. A workspace that does not compile shows `100 / 100 Core partial` under a smiling face. It is labeled partial, but a reader skimming the block sees a perfect score.

OBS-3. `--json` stderr is not empty when discovery descends (WS-10); intentional per `main.rs`, but the contract test must whitelist that one line.

OBS-4. `--verbose` on a terminal without `--yes` animates the score block (175 KB of frames).

---

## Proposed split into bun e2e files (`tests/e2e/`)

| File | Scenarios | Notes |
|---|---|---|
| `e2e-helpers.ts` | | scrubbed env builder, per-test scratch (`mkdtemp`), fixture copier without `target/`, `runDoctor()` returning `{code, stdout, stderr, json}`, `withNetworkNamespace()` (unshare/bwrap skipIf), python pty helper wrapper |
| `exit-codes.test.ts` | EX-01..16, AR-01..18 | table-driven; the precedence matrix in one `test.each` |
| `signals-and-deadlines.test.ts` | SG-01..14 | sleeper pid file as the sync point; assert the process group is gone with `kill -0 -<pgid>`; SG-07/08 as `test.failing` until BUG-04 is fixed |
| `output-streams.test.ts` | OW-01..08, OC-01..15 | `/dev/full`, closed stdout, `head -c` via a pipe the test closes early |
| `toolchain-preflight.test.ts` | TC-01..15 | fake PATH dirs of symlinks into `~/.rustup/toolchains/<v>/bin`; scratch `RUSTUP_HOME` with a linked toolchain for TC-04/TC-08 |
| `workspace-shapes.test.ts` | WS-01..38 | permission cases need a non-root uid (skip under root) |
| `advisory-database.test.ts` | AD-01..10 | |
| `terminal-profile.test.ts` | EN-01..22 | pty helper; the 26 markers as `test.each` |
| `privacy-redaction.test.ts` | PV-01..12 | one planted probe, grep every output mode; PV-05 `test.failing` until BUG-01 |
| `offline-and-side-effects.test.ts` | NW-01..05, FS-01..06 | strace optional (skipIf) |
| `poisoned-repository.test.ts` | PG-01..06 | control run first (plain git must hit the log), then each scope |

Total: 185 scenarios (EX 16, AR 18, SG 14, OW 8, TC 15, WS 38, AD 10, EN 22, OC 15, PV 12, NW 5, FS 6, PG 6): 169 verified on the binary, 4 partially, 12 derived from source only.
