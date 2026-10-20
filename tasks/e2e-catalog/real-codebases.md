# rust-doctor e2e on real codebases: findings and plan

Date: 2026-10-20. Binary: `/home/arthur/dev/rust-doctor/target/release/rust-doctor` 0.7.0, built 08:36 from HEAD `b5389f7` (clean tree). Host: AMD Ryzen 7 7800X3D, 16 threads, 30 GB, Fedora (kernel 7.2). Every scan ran with `RUSTUP_TOOLCHAIN=1.97.1` (the CI pin; the machine default is stable 1.98.0), `--yes`, and a `CARGO_TARGET_DIR` under this directory. Nothing in the repository or the corpus clone cache was written (checked with `git ls-files -m -o` and `find -newer <binary>` after the runs).

Everything below was measured unless marked "estimate" or "unverified".

## 0. Defects and drift found (with evidence)

| # | Kind | Finding | Evidence |
|---|---|---|---|
| B1 | Bug | **Baseline scope reports every `rust_doctor::repo::*` finding as introduced.** The base side is a snapshot under `$TMPDIR` that is not a git repository, so the repo pass (`git ls-files`) abstains there by design (`src/repo_hygiene.rs:1-10`), and every repo finding of the head side has no partner. `--scope baseline --base $(git rev-parse HEAD)` on a clean clone of this repo: `delta.summary = {introduced: 1, pre_existing: 173, fixed: 0}`, the introduced one being `rust_doctor::repo::hardcoded_credential tasks/prd-react-doctor-gap-closure.md` (`base_diagnostics` 173 vs `current_diagnostics` 174). | `out/self/baseline-same.json` |
| B1a | Consequence | Through the Action, a no-op PR (head == base) gets `introduced=1`, `clean=false`, so `comment-on-clean: false` still posts a comment; with `blocking: warning` the PR fails on a file it never touched. `dogfood.yml` runs exactly this on every PR of this repo. | `action-proto/case4` (POST issued on a no-op PR), `action-proto/case1` (2 introduced, 1 of them the untouched credential) |
| B1b | Test gap | `tests/precision_controls.rs:250-260` runs a baseline against HEAD with a comment acknowledging "The base side is a snapshot outside git" but asserts only `fixed == 0` and `pre_existing == 3`, never `introduced == 0`, and its fixture has no repo-rule finding. No test pairs a repo rule with baseline scope. | grep over `tests/` |
| B2 | Bug | **Ctrl-C through the npm launcher takes the binary's hard-exit path.** A terminal Ctrl-C sends SIGINT to the whole foreground process group: the native child gets it from the terminal, then `launchNative` forwards a second SIGINT (`lib/launcher.js` `forward`). `src/execution/interrupt.rs` treats a second signal as "kill groups and `_exit(130)` at once", which skips the `interrupted` report and every `Drop`. Reproduced with `setsid` + `kill -INT -- -PGID` and a fake blocking `cargo`: | `scripts/interrupt.sh`, `out/int-*.json` |
| | | direct binary, group SIGINT: exit 130, 24000-byte JSON, `status: failed`, `errors: [interrupted]`, TMPDIR clean | |
| | | launcher, SIGINT to launcher pid only (what `smoke-packed.mjs` `proveInterruption` does): same as direct | |
| | | launcher, group SIGINT (terminal Ctrl-C): exit 130, **0 bytes on stdout**, and in baseline scope **`rust-doctor-baseline-329951-0` left in `$TMPDIR`** | `tmp/launcher-group-baseline-1792480194502589132/` |
| B3 | Bug | **A scan of a workspace without a committed `Cargo.lock` mutates it and is not idempotent.** `cargo clippy --workspace --no-deps --keep-going` (no `--locked`, `src/execution/clippy.rs:49`) resolves and writes `Cargo.lock` into the scanned tree; the next scan's cargo-health pass reads that lockfile (dev-dependencies included). Fresh materializations, first vs second scan: `smol` 100 then 85 (12 `duplicate_major_versions`), `serde_json` 88 then 85, `log` 90 then 87. A native-only scan (every Clippy rule off) writes nothing and is stable. The generated lockfile depends on the local registry index, so the second result is also machine- and date-dependent. | `scripts/lockprobe.sh`, `out/lp-serde_json-*.json`, `out/{smol,log,serde_json}/{cold,warm}.json` |
| B3a | Consequence | **"No network" is false for lockless workspaces.** Inside `unshare -rn` a fresh `smol` scan is `incomplete` (exit 1, `clippy-exit`: "spurious network error ... Failed to connect to index.cr[ates.io]"); with `CARGO_NET_OFFLINE=true` it completes. `hexyl` (committed lockfile) and this repo complete offline with no variable. README.md:12 says "Everything runs locally: no network". The binary itself is offline; the Cargo it drives is not. | `scripts/netns-subject.sh`, `out/smol-netns-*.json` |
| B3b | Cache pollution | The corpus clone cache already holds an **ignored `Cargo.lock` in 8 repos** (anyhow, async-channel, bytes, claudes-c-compiler, log, serde_json, smol, thiserror), all dated 2026-09-26 16:11: an earlier in-place scan or cargo run. The harness is immune (it materializes the pinned tree with `read-tree` + `checkout-index`), but any e2e that scans the cache in place gets different numbers from the record. | `git -C <repo> ls-files --others --ignored --exclude-standard` |
| D1 | Doc drift | "temporary worktree" while the code snapshots `git ls-tree` output into `env::temp_dir()` (`src/baseline.rs:162-200`, `create_temp_root_in(.., &env::temp_dir())` at :385). Three places: `.github/workflows/dogfood.yml:8`, `.github/workflows/ci.yml:101`, and **`src/ci.rs:451`, which is template text `rust-doctor ci install` writes into users' workflows**. `action.yml` does not repeat it ("These scopes need the base's history"). `docs/subsystems/delta.md:15` is correct ("the snapshot's temporary root"). | grep |
| D2 | Doc drift | Catalog help of `rust_doctor::repo::hardcoded_credential` says "the closed list judged here is AKIA, ghp_, github_pat_, sk-, xoxb- and BEGIN PRIVATE KEY blocks" (`src/policy/catalog/native.rs:174`), but the detector also fires on database URL passwords (`src/repo_hygiene/credentials.rs:99`, `database_passwords`), which is the very finding this repo's self-scan carries. | self-scan help vs message |
| D3 | Likely FP | That self-scan finding is an acceptance-criterion example in a tracked PRD: `postgres://app:<redacted>@db.internal.example.net/prod` (`tasks/prd-react-doctor-gap-closure.md:697`). The host is an RFC 2606 example domain; `PLACEHOLDER_WORDS` contains `example` but `placeholder_password` only inspects the password, not the host. It caps the self-scan at 65 (P1 ceiling, `applied_ceiling: 65`, security 37). | `out/self/tree-cold.json` |
| D4 | Doc drift | `.github/workflows/ci.yml:33`: "37 of the 62 catalogued rules are Clippy lints". Now 104 of 142. | `rules list --json` |
| D5 | Doc drift | `.github/workflows/corpus.yml:11,32,76` speak of "compiling eighteen repositories" and "three-hour runs". The 8 agent repositories are scanned with every Clippy rule off and compile nothing (`trust_boundary.native_detectors_compile_corpus_code: false`), and scanning all 18 cold took about 37 s here (CI wall time unverified, but nowhere near three hours). | timings below |
| D6 | Stale baseline | `tests/fixtures/performance/baseline.json` was measured under `rustc 1.98.0`; the benchmark compares only when the toolchain matches, so under the project pin (1.97.1) it never compares anything. | file header vs `host.toolchain` |
| D7 | Ambiguity | AGENTS.md: files under 1000 lines "tests included". The self-scan reports 5 integration-test files over 1000 lines (`tests/corpus_precision.rs` 1885, `tests/support/corpus.rs` 1726, `tests/corpus_agreement.rs` 1094, `tests/structure_kernel.rs` 1080, `tests/corpus_statistics.rs` 1014). Fine if "module" means `src/` modules, but the sentence reads wider. Nothing under `src/` is flagged, so the documented gate holds. | self-scan |

Not a bug but worth a line in `--help`: `--base HEAD~1` is refused with "Invalid Git base selector." by a deliberate closed grammar (`src/git_scope.rs:179-195`); the help says only `--base <REF>`.

## 1. The corpus

### Where it lives and how CI seeds it

- Local clone cache: `~/.cache/rust-doctor/corpus/<name>` (334 MB, 18 repos, each a detached checkout at the pinned commit; all 18 `HEAD`s match `tests/corpus.json`). Artifacts: `~/.cache/rust-doctor/corpus-artifacts` (plus `-before`, `-ep006` leftovers). No env var is set in the shell; `tests/performance.rs` defaults to that path when `RUST_DOCTOR_CORPUS_DIR` is unset, the corpus tests require both `RUST_DOCTOR_CORPUS_DIR` and `RUST_DOCTOR_CORPUS_ARTIFACTS` (one alone is a hard failure, neither is a printed skip: `tests/support/corpus/reproduction.rs`).
- CI (`.github/workflows/corpus.yml`): `actions/cache/restore` of `$RUNNER_TEMP/corpus-cache` keyed `corpus-clones-${hashFiles('tests/corpus.json')}` with prefix restore `corpus-clones-`; a PR with no cache match fails and asks for `gh workflow run corpus.yml`; materialization is `git init` + `fetch --depth 1 origin <commit>` + `checkout --detach FETCH_HEAD` per repo from `jq` over `manifest.repositories + agent_population.repositories`; save only on a non-exact hit. Triggers: `workflow_dispatch` and PRs touching the record, harness or the five producers and `src/audit/`.
- The harness never scans the cache: `materialise` (`tests/support/corpus.rs:962`) does `read-tree <commit>` into a private `GIT_INDEX_FILE` and `checkout-index --all --prefix=<artifacts>/work/<name>/`, then scans `inspect <work> --json --yes --advisory-db /dev/null` with `RUST_DOCTOR_STRUCTURE_TIME_BUDGET_SECS=600` and a fresh `CARGO_TARGET_DIR` per repo. The agent population adds `--rule clippy::<id>=off` for all 104 Clippy rules (212 arguments, `agent_population.scan_arguments`).

### Pinned subjects

| Subject | Pop. | Tag / commit | Files (tracked) | .rs files | .rs LOC | Prod. lines (record) | Members | Lockfile tracked |
|---|---|---|---|---|---|---|---|---|
| anyhow | healthy | 1.0.104 `1dbe186` | 54 | 37 | 5,857 | 4,113 | 1 | no |
| async-channel | healthy | v2.5.0 `35a63c4` | 12 | 3 | 2,296 | 1,388 | 1 | no |
| bytes | healthy | v1.12.1 `76c0fbb` | 46 | 34 | 13,750 | 9,881 | 1 | no |
| fd | healthy | v10.4.2 `7027d45` | 55 | 23 | 7,780 | 4,676 | 1 (bin) | yes |
| hexyl | healthy | v0.16.0 `abc20a3` | 22 | 7 | 2,821 | 1,854 | 1 (bin+lib) | yes |
| log | healthy | 0.4.33 `f405739` | 23 | 13 | 6,409 | 5,745 | 1 | no |
| ripgrep | healthy | 15.2.0 `e89fff8` | 222 | 100 | 53,025 | 45,490 | 10 (workspace) | yes |
| serde_json | healthy | v1.0.145 `efa66e3` | 91 | 70 | 23,077 | 18,301 | 1 | no |
| smol | healthy | v2.0.2 `a1e6421` | 37 | 23 | 1,743 | 129 | 1 (async) | no |
| thiserror | healthy | 2.0.19 `e13a785` | 118 | 69 | 5,286 | 3,096 | 3 (proc-macro) | no |
| claudes-c-compiler | agent | `6f1b99a` | 447 | 351 | 186,696 | 186,696 | 1 | no |
| ostendo | agent | `0284e33` | 138 | 60 | 16,663 | 16,184 | 1 | yes |
| artifexprocal | agent | `2be33f7` | 289 | 137 | 14,015 | 10,664 | 11 (workspace) | no |
| mdr | agent | `26e9250` | 36 | 16 | 5,233 | 5,031 | 1 | no |
| ralph-os | agent | `cd6dac9` | 62 | 39 | 11,749 | 11,653 | 1 | yes |
| vibesql | agent | `4d9b33d` | 2,971 | 1,719 | 698,611 | 413,822 | 15 (workspace) | yes |
| syncvibe | agent | `ff10443` | 86 | 68 | 14,289 | 13,170 | 2 (workspace) | yes |
| coda | agent | `cf32fbe` | 63 | 31 | 41,615 | 40,599 | 1 | yes |

(The "Lockfile tracked" column is `git ls-tree HEAD Cargo.lock`; agent members counted with `cargo metadata --no-deps --offline` on the scratch copies.)

### Measured scans (all 18, release binary, toolchain 1.97.1)

Materialized exactly as the harness does, into `work/<name>`, then three scans on one target dir: cold (empty target), warm, warm2. Times are the report's own `timings.total` in ms.

| Subject | Cold ms | Warm ms | Score run/record | Distinct run/record | Status | cold == warm | warm == warm2 |
|---|---|---|---|---|---|---|---|
| anyhow | 1,011 | 81 | 73/73 | 107/107 | complete | yes | yes |
| async-channel | 1,174 | 74 | 97/97 | 13/13 | complete | yes | yes |
| bytes | 517 | 90 | 82/82 | 143/143 | complete | yes | yes |
| fd | 15,259 | 106 | 87/87 | 60/60 | complete | yes | yes |
| hexyl | 2,299 | 85 | 93/93 | 29/29 | complete | yes | yes |
| log | 1,425 | 76 | 90/90 | 35/35 | complete | **no (87)** | yes |
| ripgrep | 2,604 | 186 | 90/90 | 499/499 | complete | yes | yes |
| serde_json | 2,514 | 102 | 88/88 | 125/125 | complete | **no (85)** | yes |
| smol | 4,387 | 114 | 100/100 | 2/2 | complete | **no (85)** | yes |
| thiserror | 1,695 | 80 | 85/85 | 76/76 | complete | yes | yes |
| claudes-c-compiler | 1,014 | 2,201 | 82/82 | 1201/1201 | complete | yes | yes |
| ostendo | 103 | 107 | 79/79 | 96/96 | complete | yes | yes |
| artifexprocal | 102 | 105 | 79/79 | 75/75 | complete | yes | yes |
| mdr | 56 | 53 | 88/88 | 36/36 | complete | yes | yes |
| ralph-os | 75 | 72 | 89/89 | 38/38 | complete | yes | yes |
| vibesql | 2,526 | 2,736 | 58/58 | 3663/3663 | complete | yes | yes |
| syncvibe | 98 | 95 | 80/80 | 62/62 | complete | yes | yes |
| coda | 240 | 237 | 88/88 | 198/198 | complete | yes | yes |

- Every cold scan reproduces the record (score, label, distinct, occurrences, production lines, status, exit 0, authoritative), schema 23, toolchain rustc 1.97.1.
- Sum of cold scans: healthy 32.9 s (fd alone 15.3 s, mostly `clippy_dependencies`), agent 4.2 s; about 37 s for the whole corpus plus about 1 s of materialization. Warm reruns of healthy subjects are 75 to 190 ms. Agent scans are native-only, so cold equals warm; vibesql (414k production lines, 699k .rs LOC) is the native stress subject at 2.5 to 4.1 s (walk 1 to 2.2 s, structure 1.4 to 1.7 s). Target dirs: hexyl 80 MB, ripgrep 129 MB, serde_json 24 MB; agent 0.
- Determinism: report minus `timings` is byte-identical across runs except B3 (the three lockless libraries whose second scan sees the lockfile the first one wrote). Raw files differ only by `timings`.

### Can an e2e tier reuse the cache offline? Yes, under four conditions

1. **Materialize, never scan in place** (B3b): reuse `read-tree` + `checkout-index` into scratch, one fresh copy per scan that must match the record.
2. **Offline Cargo**: a lockless subject needs `CARGO_NET_OFFLINE=true` and a `CARGO_HOME` that already holds its dependencies (B3a). In CI, warm `~/.cargo/registry` with a `cargo fetch` in a **throwaway copy** (fetching in the copy to be scanned writes `Cargo.lock` first and changes the result away from the record).
3. **Restore-only** the same `corpus-clones-*` cache key as `corpus.yml`; never save from the e2e job, so the two workflows cannot race on one key.
4. **Same scan arguments as the record** (`--advisory-db /dev/null`, the 600 s structure budget, the 212 agent arguments), else the record is not the oracle.

What the e2e corpus tier should add over `corpus_precision` (which already asserts strict equality in-process): the same equality through the shipped CLI and through the launcher, a determinism rerun that flags B3, non-mutation of the scanned copy, network isolation, and the performance envelope (cold and warm medians, as `tests/performance.rs` does, but per subject class). `findings_digest` is blake3 and Bun has no blake3; compare the sorted `(rule, path, line, column, occurrences)` tuples instead, or skip the digest since the Rust test owns it.

## 2. Self-scan as a real codebase

Measured on the working tree and on a `git clone --local --no-hardlinks` of HEAD: cold 5.8 s, warm 0.42 s, clone cold 5.9 s, inside `unshare -rn` 5.5 s. The tree, warm, clone and network-less reports are byte-identical once `timings` is removed. `status: complete`, `errors: []`, authoritative, schema 23, `toolchain.removed_lint_flags: []`, gate passed (blocking error, 0 blocking), 174 diagnostics, score 65 (capped by D3).

Invariants an e2e test can assert without freezing a defect (the self-scan is a gate, `docs/testing.md`):

1. Exit 0, `status == "complete"`, `complete == true`, `errors == []`, `audit.score.authoritative == true`, `gate.status == "passed"` at the default blocking level (an error-level finding is a regression, not a frozen fact). Corrected 2026-10-20: zero P1 means no P0 or P1 finding in production context, `worst_tier` neither P0 nor P1 and `applied_ceiling` null (US-012); a P1 rule may fire in test code (`dynamic_shell_command` under `tests/`), which the score model does not count.
2. `schema_version` equals `SCHEMA_VERSION` parsed from `src/report.rs` (as `smoke-packed.mjs` does), `toolchain.rust_doctor` equals `--version` equals `Cargo.toml` version equals `npm/rust-doctor/package.json` equals `action.yml` `inputs.version.default`.
3. `toolchain.rustc` and `toolchain.clippy` match the pinned toolchain; `toolchain.removed_lint_flags == []` (every catalogued lint exists in the pinned Clippy).
4. Determinism: cold, warm and a third run are identical minus `timings` (valid here because this repo commits its lockfile; see B3 for why the test must say so).
5. Location invariance: a `git clone --local` of HEAD in another directory scans identically to the clean working tree (no dependency on ignored files, `target/`, or the absolute path).
6. Privacy: the JSON contains neither the repository root, the clone root, `$HOME`, `$TMPDIR`, nor any value from the environment; every `diagnostics[].path` is relative and is either `git ls-files`-tracked or `Cargo.lock`.
7. Catalog closure: every `diagnostics[].code` with a non-null `category` is an id from `rules list --json`; `rules list --json` has as many entries as `tests/corpus.json` `catalog`, unique ids, and only the five prefixes; `rules explain <id>` exits 0 for every id and 2 for an unknown one.
8. Human/JSON parity on one scan pair: `Scanned N files` equals `audit.source_files`; `All X occurrences across Y findings` equals `summary.occurrences.total` and `summary.total`; each category line equals `audit.categories`; the boxed score and label equal `audit.score`; the "Capped at" line appears iff `applied_ceiling` is non-null; the share URL's `s, m, w, i, f, l` equal score, model, warning and info occurrences, source files, production lines; `--verbose` lists every diagnostic. Corrected 2026-10-20: a rule whose sites are all in test code is collapsed into an `Unscored:` line rather than listed by location.
9. Renderer parity: `rust-doctor report markdown <saved.json>` renders the same score, counts and delta as the live run, and runs with no `cargo` on `PATH` (proves it does not scan).
10. The crate passes its own rules through the CLI: zero `clippy::unwrap_used`, `expect_used`, `panic`, `dbg_macro`, `todo`, `unimplemented` in production context, and zero `oversized_unit` or `complex_function` under `src/` (both hold today: the 18 `oversized_unit` and 4 `complex_function` sites are all under `tests/`).
11. Non-mutation: the repository state hash (`HEAD`, tree, index bytes, `ls-files --stage`, `status --porcelain=v2`, working tree file hashes, forbidden outputs) is identical before and after, as `smoke-packed.mjs` `repositoryState()` computes it.
12. Network isolation (Linux): the scan inside `unshare -rn` produces the same report.
13. Scopes on a clean tree: `--scope files --base HEAD` and `--scope lines --base HEAD` publish zero diagnostics and `status: complete`; `--scope baseline --base <HEAD sha>` publishes `introduced == 0 && fixed == 0` (**fails today: B1**); a scratch clone with one seeded `unwrap()` commit reports exactly that finding as introduced.
14. `--package rust-doctor` equals the full scan (single member). Corrected 2026-10-20: except that the five `rust_doctor::repo::*` rules are `package-scoped` under `--package` and the score loses its authoritative flag.
15. Deadline: `--max-duration 1` on a cold target exits 2, `status: failed` (corrected 2026-10-20: or `incomplete` when the deadline lands during Clippy, as EX-09 shows), an error `deadline-exceeded`, no surviving `cargo` process, no `rust-doctor-baseline-*` left in `$TMPDIR`.
16. Interrupt (direct binary): group SIGINT during Clippy exits 130 with a parseable report carrying `interrupted`, no orphan, no leftover snapshot (passes today).
17. Stderr hygiene: with a sandboxed `HOME` and `CLAUDECODE`, `CODEX_SANDBOX`, `CURSOR_AGENT` unset, `--json` stderr is empty (in an agent shell it prints the skill nag, so the suite must sandbox this).
18. `--timings`: the phase set is the documented one and `total` is at least the largest phase.
19. `--advisory-db` pinned to `/dev/null` or a fixture: without it the human report says "The advisory database is 80 days old", which is time-dependent.

What must not be asserted: the score value, label, counts, any specific rule firing on this repo.

## 3. npm launcher

### What `smoke:packed` already proves (`npm/rust-doctor/scripts/smoke-packed.mjs`)

Wrapper and linux-x64 tarball inventories; offline `bun install --ignore-scripts --backend=copyfile` (registry and proxies pointed at `127.0.0.1:9`) and offline `npm install -g --prefix <scratch> --offline`; npm's native resolution stays inside the prefix; a PATH-guard `rust-doctor` proves the launcher never falls back to `PATH`; `--version`; human section order, count, one-decimal duration, score, direct-binary exit parity; one parseable JSON at the shipped schema with no ANSI, prompt or absolute path, `stderr == ""`; five identical normalized fixture runs; the Codex TTY handoff (through `cargo test --test local_cli_experience tty::packed_wrapper_tty_handoff_contract_when_requested`); a SIGINT to the **wrapper pid** during a fake `cargo` exits 130 with no orphan; repository state unchanged after completed and interrupted runs.

Cost and side effects: `packLocal()` is called without `binaryPath`, so it always runs `cargo build --release` into a temp target, and `proveHandoff` compiles the `local_cli_experience` test crate; the script writes `target/local-cli-dogfood.json` into the repository. That is why it was not run here (repository write forbidden). Offline feasibility by construction: yes, given a warm `~/.cargo/registry` (cargo build `--locked`, `CARGO_NET_OFFLINE=true` for the test crate) and local `bun` and `npm` (bun 1.4.2 and npm 10.9.8 here; CI pins bun 1.3.14 and Node 22.13.0).

### What the e2e suite should add

1. **Terminal Ctrl-C** (B2): SIGINT to the process group, and the real thing through tmux `send-keys C-c`; assert a parseable `interrupted` report on stdout and no `rust-doctor-baseline-*` in `$TMPDIR`. Fails today. Also SIGTERM to the group, SIGINT twice (the documented hard exit), and SIGKILL of the launcher (child must not outlive it as an orphan scan, worth deciding).
2. **Byte parity, launcher vs binary**, for stdout, stderr and exit code across 0, 1 (`--blocking warning`), 2 (`--bogus`), the warning code (`--warning-exit-code 42` gave 42 through the launcher here), 130, `--help`, `rules list --json`, `rules explain`. Measured: stdout, stderr and exit code identical for `--version` and `--bogus` (exit 2); exit codes 1 (`--blocking warning`) and 42 (`--warning-exit-code 42`) propagated through the launcher.
3. **Resolution failures on the installed layout**, not only with injected `resolve` (the unit tests cover the logic): delete the native `package.json` (measured: "Native package @rustdoctor/linux-x64 is not installed...", exit 1), version mismatch, wrong `os`/`cpu`, binary without `+x`, native package missing entirely with `--omit=optional`; each message bounded to one line under 1024 bytes and exit 1.
4. **`npx`-style invocation** of the locally packed tarball with an offline cache (`npm exec --offline --package <tarball>` after seeding the npm cache with both tarballs), and `bunx` equivalents, since the registry install path is what users run.
5. **TTY passthrough via tmux**: the interactive report opens through the launcher when stdin and stdout are terminals, `q` exits 0, terminal size reaches the child (resize the tmux pane), and a non-TTY stdout falls back to the linear report.
6. **Real-codebase scan through the launcher**: one corpus subject (hexyl, 2.3 s cold) through the installed wrapper equals the direct binary byte for byte.
7. **Environment and cwd forwarding**: `CARGO_TARGET_DIR`, `RUSTUP_TOOLCHAIN`, relative `PATH` argument resolved against the caller's cwd.
8. Reuse the release artifact: call `packLocal({ binaryPath })` so the e2e job never rebuilds.

## 4. GitHub Action

### Logic outside the binary (`action.yml`)

1. Toolchain and rust-cache steps (`uses:`, pinned to SHAs; cache saved on failure).
2. Install: `npm install -g "rust-doctor@$VERSION"` unless `version: preinstalled`.
3. Scan (bash): report path under `$RUNNER_TEMP/rust-doctor/report.json` written to `GITHUB_OUTPUT` first; scope defaulting (`baseline` when `github.base_ref` is set, else `full`); base defaulting to `origin/$PULL_REQUEST_BASE` for non-full scopes on a PR; **shallow-clone repair** (`git fetch --no-tags --unshallow origin +refs/heads/<base>:refs/remotes/origin/<base> || true` when `rev-parse --is-shallow-repository` is true); argument assembly with every input through `env` (no `${{ }}` in the script); `ARGS` split with `read -r -a`; exit code captured with `|| status=$?` and written as `exit-code`.
4. Summarize (`if: always()`): `rust-doctor report markdown` into the step summary or a fallback sentence; `clean` computed by jq from `.status`, `.scope.mode`, `.delta.summary.introduced`, `.diagnostics | length`, coerced to `true`/`false`.
5. Upload artifact (`if: always()`, `if-no-files-found: ignore`).
6. Sticky comment (PR and `comment: true`): marker `<!-- rust-doctor -->`, `gh api --paginate` list, `jq -s` selection of a `github-actions[bot]` comment with the marker, skip on clean when `comment-on-clean: false` and no comment exists, PATCH or POST, any failure downgraded to a `::warning::` (fork token).
7. Outputs: jq reads only numbers and booleans (`score`, `authoritative`, `introduced`, `fixed`), falling back to `authoritative=false` when the report is unreadable.
8. Exit policy: `code=${EXIT_CODE:-2}`; 1 or the warning code becomes 0 with a `::notice::` on any event other than `pull_request`; 2 and 130 fail every event.

### Local testing: feasible without `act` (not installed)

Prototype at `action-proto/run-action.ts` (about 60 lines): parses `action.yml` with `Bun.YAML`, evaluates the handful of expressions it uses (`inputs.*`, `github.*`, `steps.<id>.outputs.*`), runs each `run:` with `bash --noprofile --norc -eo pipefail` (GitHub's `shell: bash`), fake `RUNNER_TEMP`, `GITHUB_OUTPUT` per step, `GITHUB_STEP_SUMMARY`, a recording `gh` stub serving canned comments, and the release binary as `rust-doctor` (the `preinstalled` path). The origin is a local bare clone, so the unshallow fetch runs over `file://` with no network.

| Case | Result |
|---|---|
| PR, depth-1 checkout with `origin/main` deleted, `blocking: warning`, no comment yet | unshallow fetch repaired history; scan exit 1; outputs `score=65 authoritative=true introduced=2 fixed=0`; POST of the sticky comment; final step exits 1; 11.7 s wall with a cold target |
| push, full scope, `blocking: warning` | scan exit 1; `introduced`/`fixed` empty; final step 0 with the notice |
| PR no-op, existing sticky comment | PATCH `issues/comments/42` |
| PR no-op, no comment, `comment-on-clean: false` | **POST anyway** (B1a) |
| canned comments that are not JSON | comment step exits 5 (jq under `set -e`) instead of the documented warning; edge case, `gh` normally returns JSON |

An e2e file for the Action is this prototype plus a case matrix: event (push, pull_request, workflow_dispatch) times exit code (0, 1, 2, 130, warning code, missing) for the exit policy; shallow and full checkouts; base input set or empty; `working-directory` in a subdirectory; `args` carrying `$(touch pwned)` and `;` (must stay literal); an unreadable report (outputs fall back, summary sentence, no comment); comment API failure (warning, step green); outputs injection attempt (a report whose strings contain `\nscore=1`). Plus static checks: every `uses:` pinned to a 40-hex SHA, no `${{` inside any `run:`, `inputs.version.default` equals the crate version, the `ci install` template agrees with `action.yml` (and drops the "worktree" wording, D1).

## 5. CI integration

### Proposed topology

```
ci.yml (push, pull_request)
  build-release        cargo build --release --locked (1.97.1), upload artifact rust-doctor-linux-x64
  e2e (3 shards)       needs build-release; download, chmod +x; toolchain 1.97.1 + clippy;
                       setup-bun 1.3.14; setup-node 22.13.0; tmux (apt if missing);
                       bun install --frozen-lockfile in tests/e2e; ci-shards.ts | ci-run-files.sh
  packed               unchanged, or folded into an e2e shard via packLocal({ binaryPath })
e2e-corpus.yml (schedule weekly, workflow_dispatch, PR paths: the corpus.yml filter + tests/e2e/corpus*)
  e2e-corpus           needs the same release artifact (or builds it); restore-only corpus-clones-*;
                       rust-cache registry; cargo fetch in throwaway copies; run with
                       CARGO_NET_OFFLINE=true inside a no-network namespace
```

Copy pf's three pieces as is: `ci-shards.ts` (greedy longest-first on checked-in weights, refuses missing and stale entries), `ci-shard-weights.json`, `ci-run-files.sh` (one Bun process and one tmux server per file, one retry with a `::warning::` on pass-on-retry). Environment per file: `CARGO_TARGET_DIR` under `$RUNNER_TEMP/e2e-targets/<file>` (cached with `actions/cache` keyed on `Cargo.lock` + `rust-toolchain` pin + file name, so the self-scan cold cost is paid once per cache generation), sandboxed `HOME`, agent markers unset, `RUSTUP_TOOLCHAIN=1.97.1`, `GIT_CONFIG_GLOBAL=/dev/null` (the gpgsign trap noted in `tasks/prd-core-v3-score-recalibration-status.json`).

Network isolation on `ubuntu-24.04`: unprivileged `unshare -rn` is likely blocked by AppArmor (`kernel.apparmor_restrict_unprivileged_userns=1`); unverified here. Either `sudo sysctl -w kernel.apparmor_restrict_unprivileged_userns=0` in the job or `sudo unshare -n -- sudo -u "$USER" -E ...`.

### Files and initial weights (seconds, estimate on a 4-vCPU runner = local x 3 to 4 for compile-bound phases)

| File | Covers | Weight |
|---|---|---|
| `self-scan.test.ts` | invariants 1 to 7, 10, 14, 17, 18 | 30 |
| `self-scan-parity.test.ts` | 8, 9 | 5 |
| `self-scan-isolation.test.ts` | 5 (clone), 11, 12 | 25 |
| `scopes-real.test.ts` | 13 (B1), seeded baseline, files and lines | 35 |
| `deadline-and-interrupt.test.ts` | 15, 16, snapshot cleanup | 15 |
| `rules-catalog.test.ts` | 7, `rules explain` for 142 ids | 5 |
| `launcher-packed.test.ts` | pack with `binaryPath`, offline bun and npm, parity matrix, npx-style | 25 |
| `launcher-failures.test.ts` | resolution failures on the installed layout | 5 |
| `launcher-signals.test.ts` | single, group (B2), double, SIGTERM | 15 |
| `launcher-tty.test.ts` | tmux TUI open, q, resize, C-c | 20 |
| `action-replay.test.ts` | case matrix above | 45 |
| `action-static.test.ts` | SHA pins, no `${{` in run, version sync, `ci install` template | 2 |
| `real-subject.test.ts` | hexyl and ripgrep (lockfile, 1 and 10 members) cold vs record, launcher parity | 30 |
| `ci-shards.test.ts` | the planner | 2 |
| `corpus-replay.test.ts` (corpus job only) | 18 subjects vs record, rerun, non-mutation | n/a |

Three shards land near 85 to 90 weight-seconds each.

### Estimated durations

| Job | Local measurement | Runner estimate |
|---|---|---|
| build-release | (not timed; rebuilt incrementally here) | 4 to 6 min cold, about 1 to 1.5 min with rust-cache (estimate) |
| e2e shard | self-scan cold 5.8 s, warm 0.42 s; Action PR case 11.7 s cold; launcher startup about 80 to 115 ms (`--version`: node launcher vs 1 ms direct) | 1 min setup + 1.5 to 2 min tests, so 3 min per shard and about 7 to 9 min critical path after build with 3 shards (estimate) |
| e2e-corpus | all 18 cold 37 s, warm 2 to 3 s, materialization about 1 s, cache 334 MB | cache restore 20 to 40 s, toolchain 30 s, cold scans 2 to 3 min, rerun 15 s, so 4 to 6 min (estimate) |

The current `packed` job builds its own release binary and compiles `local_cli_experience`; feeding it the `build-release` artifact through `packLocal({ binaryPath })` removes one release build per CI run.

## 6. Reproduction kit (in this directory)

- `scripts/measure.sh <healthy|agent> <subject>`: harness-identical materialization plus cold, warm, warm2 scans; `out/timings.tsv`, `out/<subject>/*.json`.
- `scripts/lockprobe.sh <subject>`: B3 (native vs full, first vs second scan).
- `scripts/netns-subject.sh <subject> <0|1>`: B3a (network-less scan, with or without `CARGO_NET_OFFLINE`).
- `scripts/selfscan.sh`: tree cold/warm, human, verbose, clone, `rules list`, repository untouched check; `out/self/`.
- `scripts/interrupt.sh <direct|launcher-single|launcher-group> <full|baseline>` with `fakebin/cargo` (passes everything through except the lint run, which blocks) and `launcher-pkg/` (the wrapper plus a hand-staged `@rustdoctor/linux-x64`): B2.
- `action-proto/run-action.ts`, `action-proto/bin/gh` (stub), `action-proto/origin.git` (local bare origin), `action-proto/ws-shallow`, `action-proto/ws-noop`, `action-proto/case*/runner/` (outputs, summaries, `gh.log`).
