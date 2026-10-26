[PRD]
# PRD: Security Audit Run-1 Remediation

## Changelog

| Version | Date | Author | Summary |
|---------|------|--------|---------|
| 1.0 | 2026-10-25 | Arthur Jean | Initial draft from security audit run-1 (source ref `de4c6a6`, findings unchanged at `903afbe`) |

## Problem Statement

The run-1 security audit of rust-doctor covered 28 units under the standard profile. It confirmed 13 vulnerabilities (4 medium, 9 low) and left 5 leads that are grounded in the source but depend on hosted facts the audit could not observe. The records live in `~/security-audit-skill/rust-doctor/run-1/` (`REPORT.md`, `FINDINGS-DETAIL.md`, `NEEDS-VALIDATION.md`, `findings.json`).

1. The trust boundary in AGENTS.md is false for scans that compile nothing. With every Clippy rule off, a scan still starts programs the scanned tree selects:
   - git filter drivers run on every scope and snapshot call (medium);
   - the cargo, clippy and rustc version probes and `cargo metadata` run through a workspace `[alias] clippy` or a `rust-toolchain.toml` path toolchain (medium);
   - `rules explain` and `ci install --dry-run` run a workspace-selected cargo through `cargo_workspace_root` (low);
   - scan-time git binds to a git directory fabricated from tracked files (low).
2. Writes and terminal output follow what the workspace commits:
   - the baseline probe and the base-side staging file are opened with `O_TRUNC` through committed symlinks at PID-named paths (medium);
   - the installers anchor confinement on the toplevel git reports, and the tree's own git config can move it (low);
   - the installers print repository-derived names without terminal sanitization (low);
   - `why --report` pastes an unvalidated rule id into the `#[expect]` it tells the reader to write (low).
3. Reports leak what they promise to mask:
   - a private key body written on its header line survives masking in code frames and in Cargo stderr excerpts (low);
   - path externalization stops at a space or an apostrophe, and ignores Windows verbatim, UNC and `USERPROFILE` paths (low);
   - a non-RustSec database that cargo-deny cloned silently becomes the advisory authority (low).
4. The supply chain that ships the binary trusts mutable inputs:
   - the 23 `uses:` of release.yml resolve tags or branches, and `npm@^11.5.1` runs with OIDC publish authority (medium; npm drift was observed between 0.7.0 and 0.8.0);
   - the npm launcher executes a same-named native package from any ancestor `node_modules` (low).
5. Five leads each rest on a hosted fact:
   - the corpus and release builds share a cache scope;
   - the crates verification build holds the job's OIDC identity;
   - snapshot roots sit under the shared `/tmp`;
   - the PR comment token lives in the job that compiles;
   - the agent hook compiles under the host's identity.

   Each has a source fix that removes the dependency on that fact.

**Why now:** 0.8.0 shipped at `903afbe` with every finding still live. The Action, the generated workflow and the agent hooks put the scan into CI and agent loops, where a contributor or a prompt-injected agent writes the tree being scanned. The audit's reproductions are fresh, so each fix can turn one of them into a regression test before the code drifts.

## Overview

This PRD ships in two releases:

- **Release 1** (EP-001 to EP-004, 15 stories, P0) closes the 13 confirmed findings.
- **Release 2** (EP-005, 6 stories, P1) applies the source fix for each of the 5 leads, plus the run-1 hardening notes that close a lead: `save-if`, `cache-bin: false`, the absolute launcher path and the job-level comment permission.

Every story ships a regression test built from the audit's own reproduction. That test fails on `903afbe` and passes after the fix.

A scan with no active Clippy rule makes "only Clippy compiles" true in four ways:

- It starts no toolchain version probe.
- It runs `cargo metadata` under a `RUSTUP_TOOLCHAIN` pin that only accepts a toolchain name. This reuses the `accepted` filter at `src/execution/baseline.rs:290`, and it keeps the manifest directory as the working directory so `build.target-dir` keeps its meaning.
- Its git calls neutralize every filter driver the effective configuration names. The overrides go through `GIT_CONFIG_COUNT` pairs, which git treats as protected command-scope configuration.
- Its git calls set `safe.bareRepository=explicit`.

The non-scanning commands resolve the workspace root from manifests alone.

The writes and the reports follow four decisions:

- **One write module.** `src/workspace_write.rs` moves into the library, so the baseline and side-cache writes share the installers' symlink and `create_new` invariant.
- **Masking.** It covers a private key from its header to its footer.
- **Path externalization.** It closes quoted paths at their delimiter.
- **Advisory discovery.** It accepts only a RustSec remote.

On the release side:

- `release.yml` is pinned by commit and exact version and builds cold.
- The crates verification build moves to a job that holds no OIDC permission.
- The PR comment moves to a new composite action, `comment/action.yml`. It runs from a second job that holds `pull-requests: write` and runs no pull request code, and the generated workflow gets the same two-job layout. This is a breaking change for Action consumers.

`hook run --no-clippy` gives a compile-free agent hook as an opt-in, and the installers say what authority the default hook carries. The report shape does not change, so `SCHEMA_VERSION` stays 23: the toolchain versions are already `Option<String>` in `src/report.rs:508-518`.

## Goals

| Goal | Month-1 Target | Month-6 Target |
|------|---------------|----------------|
| Run-1 confirmed findings still reproducible | 0 of 13 (Release 1 merged) | 0 of 13, held by regression tests |
| Run-1 leads whose source fix is unapplied | 0 of 5 (Release 2 merged) | 0 of 5, and a run-2 audit returns none of the 18 fingerprints as confirmed or needs_validation |
| Tree-selected programs started by a Clippy-off scan of the EP-001 fixture | 0 marker files (from at least 5) | 0, enforced by an integration test and the corpus fixture |
| Unpinned `uses:` in `.github/workflows/release.yml` | 0 of 23 | 0, enforced by a contract test |

## Target Users

### Developer scanning a cloned or agent-written workspace
- **Role:** Rust developer or vibecoder who runs `rust-doctor . --yes`, a Clippy-off scope, `rules explain` or `setup` on code they did not write line by line, often produced by an agent.
- **Behaviors:** Uses the CLI and the TUI locally, installs the pre-commit hook and the agent hooks through `setup`, reads `why` output and pastes the directive it suggests.
- **Pain points:** Believes, because the docs say so, that Clippy-off scans and non-scanning commands execute nothing. Today, any of these run code:
  - a committed `.gitattributes` together with a driver in `.git/config`;
  - a `rust-toolchain.toml` path toolchain;
  - a cargo alias.

  The installers can also write outside the named repository, and their output can carry escape sequences.
- **Current workaround:** None. The documentation states that only Clippy compiles.
- **Success looks like:** A Clippy-off scan, `rules explain`, `ci install --dry-run` and every installer start no program the tree selects, and marker fixtures prove it.

### Maintainer cutting a release
- **Role:** The sole maintainer. Tagging `v<version>` publishes six npm packages and one crate through OIDC trusted publishing.
- **Behaviors:** Tags the release, and the release skill watches npm, crates.io and the GitHub Release.
- **Pain points:**
  - The publishing jobs resolve 23 actions and the npm CLI by mutable ref.
  - Release builds warm-start from a cache scope that corpus runs write.
  - The crates dry-run compiles third-party build scripts inside the job that holds `id-token: write`.
- **Current workaround:** Trust in upstream maintainers and in GitHub's cache ref scoping.
- **Success looks like:** Every program in a publishing job is bound by a commit or an exact version, release builds start cold, and no job holding `id-token: write` compiles anything.

### Repository owner using the Action or the generated workflow
- **Role:** Maintainer of a Rust repository that runs `arthjean/rust-doctor@<ref>`, or the output of `rust-doctor ci install`, on pull requests.
- **Behaviors:** Grants `pull-requests: write` for the sticky comment and relies on Swatinem/rust-cache.
- **Pain points:**
  - The token that edits PR comments lives in the job that runs contributor build scripts.
  - Pull request runs save caches.
  - A restored `~/.cargo/bin` can shadow the pinned launcher.
- **Current workaround:** None documented.
- **Success looks like:** The compiling job holds only `contents: read`, caches save only from the default branch, and the launcher runs by absolute path.

### Report and advisory consumer
- **Role:** Anyone reading `--json`, `report markdown` or the PR comment, including reviewers of a public repository.
- **Behaviors:** Reads the report artifacts and comments, and relies on the masking and on the advisory verdicts.
- **Pain points:**
  - A private key body, the layout of directories outside the workspace and a Windows profile name can be published.
  - A non-RustSec database can silently decide the `vulnerable_dependency` verdicts.
- **Current workaround:** Review artifacts by hand before sharing them.
- **Success looks like:**
  - private key bodies are masked;
  - outside paths are published as `<external>/<last two components>`;
  - advisory verdicts come from RustSec or are reported as missing.

## Research Findings

Key findings that informed this PRD:

### Competitive Context
- **rust-analyzer:** Its security documentation states that it "assumes that all code is trusted". It lists `rust-toolchain.toml`, procedural macros and build scripts as execution vectors, and leaves containment to the editor's restricted mode ([rust-analyzer security](https://rust-analyzer.github.io/book/security.html)). rust-doctor differs by promising a compile-free tier, and this PRD makes that promise true.
- **git:** It refuses repository config owned by another user (the `safe.directory` fix for CVE-2022-24765, [GitHub blog](https://github.blog/engineering/git-security-vulnerability-announced/)). It honors `safe.bareRepository` only in protected configuration, meaning the system, global and command scopes; the command scope includes `GIT_CONFIG_COUNT` pairs (git-config, Context7 `/git/htmldocs`; [safe.adoc](https://code.googlesource.com/git/+/HEAD/Documentation/config/safe.adoc)). `explicit` becomes the default in Git 3.0.
- **crates.io and npm trusted publishing:** Both mint a short-lived token from `id-token: write` ([Rust forge](https://forge.rust-lang.org/infra/docs/trusted-publishing.html), [npm docs](https://docs.npmjs.com/trusted-publishers)). Since January 2026, crates.io owners can enforce trusted publishing ([Rust blog](https://blog.rust-lang.org/2026/01/21/crates-io-development-update)). npm trusted publishing needs CLI 11.5.1 or later.
- **GitHub Actions:** Since 2025-08-15, the allowed-actions policy can require full commit SHAs ([GitHub changelog](https://github.blog/changelog/2025-08-15-github-actions-policy-now-supports-blocking-and-sha-pinning-actions/)).
- **Market gap:** No Rust health tool states and enforces a tier that inspects an untrusted tree without running anything the tree selects. Once these fixes land, rust-doctor's four native producers hold that tier.

### Best Practices Applied
- **Neutralize in protected scope.** Repository-selected programs are neutralized in protected configuration rather than through a fixed list of known names. Environment pairs override configuration files, and `-c` overrides the pairs (git-config, Context7 `/git/htmldocs`).
- **Pin by name.** `RUSTUP_TOOLCHAIN` outranks directory overrides and `rust-toolchain.toml` (rustup overrides, Context7 `/rust-lang/rustup`). The pin accepts only toolchain names, never paths.
- **No neutral directory under shared `/tmp`.** Cargo reads `.cargo/config.toml` from every ancestor of the working directory (Cargo configuration, Context7 `/websites/doc_rust-lang_cargo`), so such a directory is not neutral.
- **Exclusive creation.** `OpenOptions::create_new` (`O_CREAT|O_EXCL`) refuses a planted final symlink and never truncates.
- **Pins and permissions.** Actions are pinned to a 40-hex commit with the ref they were read from in a comment. `id-token: write` is granted only to the job that exchanges it, since permissions are per job and every unlisted one becomes `none` (GitHub Actions, Context7 `/websites/github_en_actions`).
- **rust-cache inputs.** `save-if` makes a job restore-only, and `cache-bin` defaults to caching `$CARGO_HOME/bin` (Context7 `/swatinem/rust-cache`).

*Full research sources are kept in the audit run directory and the links above.*

## Assumptions & Constraints

### Assumptions (to validate)
- **A1:** `cargo metadata --no-deps` runs no program named by the workspace's `.cargo/config.toml` (`build.rustc`, `build.rustc-wrapper`, `[alias]`, `[env]`). Audit check h01 observed this on cargo 1.98.0; US-001 validates it on 1.97.1.
- **A2:** With `RUSTUP_TOOLCHAIN` set to a name, the rustup proxy ignores a workspace `rust-toolchain.toml` `path` key, which matches the documented precedence. Running `rustup show active-toolchain` in such a workspace starts nothing from the path toolchain. US-001 validates both.
- **A3:** Setting `filter.<name>.clean`, `filter.<name>.smudge` and `filter.<name>.process` to the empty string, and `filter.<name>.required` to `false`, through environment pairs stops every driver git would run for `diff`, `diff-index` and `checkout-index`. The audit observed this for `process` in check3, and blanking all four keys is the existing LFS pattern. US-002's fixture matrix validates it.
- **A4:** `GIT_CONFIG_COUNT` pairs require git 2.31 or later. The web research could not confirm the version, and the git docs describe the variables without one. US-002 detects a git that ignores the pairs with a canary.
- **A5:** Action consumers will accept the sticky comment moving to a second job in the next 0.x minor release. The number of consumers is unknown.
- **A6:** Agent hosts may run Stop hooks outside the agent's sandbox. The lead is unvalidated; an opt-in compile-free mode is useful either way.
- **A7:** Linux developer hosts expose either a private `XDG_RUNTIME_DIR` or a home directory that neither its group nor other users can write.

### Hard Constraints
- **Dependencies:** No new dependency. Existing ones stay pinned exactly, and `Cargo.lock` stays committed.
- **Toolchain:** Edition 2024, rustc 1.95 or later, CI toolchain 1.97.1.
- **Trust boundary:** No network, no telemetry, and `--json` reports stay workspace-relative (AGENTS.md).
- **No panics:** Production code carries no `unwrap`, `expect`, `panic!` or `dbg!` (`tests/score_credibility_packs.rs`).
- **Size bounds:** Every file stays under 1000 lines and every `impl` block under 500 (the fourteen `the_X_holds_the_size_bound` tests). Current sizes: `src/execution.rs` 935, `src/report.rs` 948, `src/git_scope.rs` 890, `src/repo_hygiene.rs` 855.
- **Report shape:** Unchanged, so `SCHEMA_VERSION` stays 23. A story that changes the shape bumps it, and the frozen v7 archive keeps projecting.
- **Catalog:** Stays at 142 rules. A help-text edit regenerates `tests/corpus.json` per `docs/catalog-and-corpus.md`.
- **Test conventions:**
  - Tests that run cargo or the binary set `CARGO_TARGET_DIR` through `support::scan_target`.
  - e2e tests name a row in `tasks/e2e-catalog/`, run in sealed copies with `--advisory-db /dev/null`, and build credentials at runtime.
  - Open defects are `test.failing` rows mirrored in `tests/e2e/known-bugs.json`.
- **Documentation:** Edits stay within the word budgets and pass the reference checks (`docs/doc-gates.md`).
- **Package manager:** `bun` under `npm/rust-doctor/` and `tests/e2e/`, never `npm` or `pnpm`.

## Quality Gates

These commands must pass for every user story:
- `cargo clippy --all-targets --no-deps --locked -- -D warnings` - lint, must be clean
- `cargo check --locked --all-targets` - every target compiles
- `cargo test --locked` - unit, integration and contract tests, size bounds included
- `bash scripts/verify-doc-budgets.sh` - word ceilings on AGENTS.md and the documents it links
- `bash scripts/verify-doc-refs.sh` - every path the documents cite resolves
- `cd npm/rust-doctor && bun test tests` - npm launcher tests
- `cd npm/rust-doctor && bun run smoke:packed` - packed launcher smoke test
- `cargo build --release --locked` - release binary the e2e suite drives
- `cd tests/e2e && bun install --frozen-lockfile && bun test` - end-to-end suite against the release build
- `cd tests/e2e && bun run typecheck` - e2e TypeScript type check

Workflow stories (US-014, US-016, US-017, US-019, US-020) add one owner-observed check: a `workflow_dispatch` or pull request run on GitHub after merge completes green.

## Epics & User Stories

**Release 1** (P0): EP-001 to EP-004 close the 13 confirmed findings.

**Release 2** (P1): EP-005 closes the 5 leads. It starts once US-004 and US-005 are DONE, because US-021 depends on both.

### EP-001: Scan-time programs the workspace cannot choose

This epic makes "only Clippy compiles" true. A scan with no active Clippy rule, and every non-scanning command, starts no program the scanned tree selects. It closes two medium findings (git filter drivers; toolchain probes and `cargo metadata`) and two low ones (`cargo_workspace_root`; the embedded tracked repository).

**Definition of Done:** US-001 to US-006 are DONE, and one fixture yields 0 marker files in 8 runs: the files, lines, contextual and baseline scans, each also with `--staged`, all with every Clippy rule off, plus `rules explain` and `ci install --dry-run`. That fixture carries:
- a custom git filter driver, defined three ways: directly, through `include.path`, and with `=` in its name;
- a `.cargo/config.toml` `[alias] clippy`;
- a `rust-toolchain.toml` with `path = "/proc/self/cwd/tc"`, plus marker `cargo` and `rustc` binaries;
- a tracked embedded git-dir layout.

#### US-001: Validate assumption: a name-only toolchain pin closes cargo metadata without moving its working directory
**Description:** As the maintainer, I want A1 and A2 confirmed on toolchain 1.97.1 before US-005 builds on them, so that the metadata fix does not rest on a single observation from cargo 1.98.0.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given a fixture whose `.cargo/config.toml` sets `build.rustc`, `build.rustc-wrapper`, `[alias] metadata` and `[env]` to marker-writing programs, when `cargo metadata --no-deps --format-version 1 --manifest-path <fixture>/Cargo.toml` runs under toolchain 1.97.1 with its working directory set to the fixture, then no marker file is written.
- [ ] Given a fixture whose `rust-toolchain.toml` sets `path = "/proc/self/cwd/tc"` and carries an executable `tc/bin/cargo` marker, when the same command runs with `RUSTUP_TOOLCHAIN` set to an installed toolchain name, then no marker is written.
- [ ] Given the same fixture with `RUSTUP_TOOLCHAIN` unset, when the same command runs, then the marker is written. This control proves the fixture is live.
- [ ] Given the same fixture, when `rustup show active-toolchain` runs with its working directory set to the fixture, then no marker is written, and `accepted` (`src/execution/baseline.rs:290`) refuses the first field of the answer.
- [ ] The checks are committed as a Linux-only integration test. It prints its reason and skips when rustup is not on `PATH`, and it runs its child processes with `RUSTUP_TOOLCHAIN` removed except where a check sets it.
- [ ] Unhappy path: given any check writes a marker, the failing check is recorded in `docs/subsystems/execution.md` before US-005 starts. US-005 then uses the fallback working directory, a directory whose every ancestor is owned by the user or root and is not writable by group or others (never `TMPDIR`), and states this in its implementation notes.

#### US-002: Neutralize every configured git filter driver on scope and snapshot calls
**Description:** As a developer scanning a cloned repository, I want rust-doctor's git calls to run no filter driver the repository configures, so that a scope or snapshot scan with Clippy off executes nothing the tree chose.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given a scan that needs git scope or a snapshot, when its first scope or snapshot call is prepared, then the driver names are enumerated once with `git config -z --name-only --get-regexp '^filter\.'`. The enumeration uses the same `-C` or `--git-dir` as the later calls, reads at most 64 KiB, and takes each name as the text between the first and the last dot of its key.
- [ ] Every git call built through `git_arguments` and `view_arguments` (`src/git.rs:110-198`) sets four keys for each enumerated name and for `lfs`: `filter.<name>.clean`, `filter.<name>.smudge` and `filter.<name>.process` to the empty string, and `filter.<name>.required` to `false`.
- [ ] Those four keys are passed as `GIT_CONFIG_COUNT`/`GIT_CONFIG_KEY_<n>`/`GIT_CONFIG_VALUE_<n>` pairs, never as `-c`. `git_command` sets them after it removes the inherited `GIT_CONFIG_COUNT` (`src/git.rs:47`).
- [ ] Given a fixture whose `.gitattributes` routes `*.rs` to three drivers (one defined in `.git/config`, one through `include.path`, one whose name contains `=`), each writing a marker from its `clean`, `smudge` and `process` commands, when the files, lines, contextual and baseline scopes and each `--staged` variant scan it with every Clippy rule off, then no marker file exists.
- [ ] For that same fixture, each scan's report equals the report of the same scan on the fixture without the drivers.
- [ ] The enumeration call also carries the canary pair `rust-doctor.canary=1` and matches it.
- [ ] Unhappy path: given a git that does not return the canary because it ignores the pairs, when the enumeration finds a driver other than `lfs`, then the scope or snapshot stage fails the way an unreadable repository fails today, with a message naming git 2.31, and no diff or checkout-index call runs.
- [ ] Unhappy path: given the enumeration exits non-zero or exceeds 64 KiB, when a scope resolves, then the stage fails closed with the same error kind and no further git call runs.
- [ ] `git_command_fixes_cwd_stdio_and_hostile_environment` (`src/git/tests.rs:57`) asserts the pairs, and asserts that an inherited `GIT_CONFIG_COUNT` never survives.
- [ ] The module documentation of `src/git.rs` and `docs/subsystems/git-and-scope.md` state that every configured driver is neutralized, not only LFS, and why `-c` is not used: `-c filter.p=q.process=` is misparsed.

#### US-003: Refuse embedded git-dir layouts at discovery and in scan-time git
**Description:** As a developer scanning a repository that tracks a git-dir layout, I want discovery and every git call to ignore embedded repositories, so that the repo pass judges the files the clone itself tracks.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `git_arguments` adds `-c safe.bareRepository=explicit` to every call, and the snapshot calls that pass `--git-dir` explicitly keep working: the existing baseline and staged tests stay green.
- [ ] `src/scan_target/discovery.rs` refuses a candidate directory that holds a `HEAD` file plus `objects/` and `refs/` directories, the way it already refuses a symlinked candidate.
- [ ] Given a crate directory that tracks `HEAD`, `objects/`, `refs/` and an `index` naming `planted.env`, and a clone that tracks its own `.env`, when a full scan runs, then the repo pass reports the clone's `.env` and never `planted.env`.
- [ ] Given the same fixture with `core.bare=false` and `core.worktree` set in the embedded config, when the same scan runs, then its report equals the previous one.
- [ ] Unhappy path: given a workspace root that is only an embedded layout with no clone above it, when a scan runs, then the repo pass falls back to its no-repository walk and the report carries no git-derived fact from the embedded layout.

#### US-004: Start no toolchain probe when no Clippy rule is active
**Description:** As a developer running a Clippy-off scan, I want rust-doctor to skip the cargo, clippy and rustc version probes, so that neither a workspace `[alias] clippy` nor a toolchain file can run through them.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `resolve_toolchain` (`src/execution.rs:707`) starts the three version probes only when the plan's lint-list flag (`src/execution.rs:415`, `:448`) is set. Otherwise the report's `toolchain.rustc`, `toolchain.cargo` and `toolchain.clippy` are `null`.
- [ ] Given a Clippy-off plan on a complete scan, then the report's `authoritative` flag and score equal those of the same scan at `903afbe`, and `SCHEMA_VERSION` stays 23.
- [ ] Given a Clippy-off baseline scan run twice, then the second run reuses the base side the first one stored.
- [ ] The side-cache key (`src/execution/side_cache.rs:243-251`) hashes absent versions as empty fields and still hashes the `RUSTUP_TOOLCHAIN` pin.
- [ ] Given a Clippy-on plan, then the three probes still run, and a failing probe still stops the run (`src/execution.rs:747-751`).
- [ ] When the versions are `null`, the linear, TUI and Markdown renderers print no toolchain version line and no placeholder.
- [ ] `docs/subsystems/execution.md` says a null version means "not probed".
- [ ] Unhappy path: given a workspace `.cargo/config.toml` whose `[alias] clippy` runs a marker-writing program, when a Clippy-off scan runs, then no marker is written.

#### US-005: Pin cargo metadata to a name-only toolchain on Clippy-off plans
**Description:** As a developer running a Clippy-off scan, I want `cargo metadata` to run under a toolchain the workspace cannot pick by path, so that a `rust-toolchain.toml` path toolchain never runs.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-001, US-004

**Acceptance Criteria:**
- [ ] Given a plan with no active Clippy rule, when the scan resolves its target, then `metadata_command` (`src/scan_target.rs:247`) sets `RUSTUP_TOOLCHAIN` to the first value `accepted` passes:
  - the inherited `RUSTUP_TOOLCHAIN`;
  - otherwise, the first field of `rustup show active-toolchain` run in the manifest directory.
- [ ] That resolution is the one `src/execution/baseline.rs:240` uses, moved to where both callers share it.
- [ ] Given rustup is not on `PATH` (the spawn reports not found), then metadata runs without a pin, as it does today.
- [ ] Given a plan with an active Clippy rule, then metadata keeps today's environment, since that scan compiles the workspace by design.
- [ ] Unhappy path: given a `rust-toolchain.toml` with `path = "/proc/self/cwd/tc"` and executable `tc/bin/cargo` and `tc/bin/rustc` markers, when a Clippy-off scan runs with `RUSTUP_TOOLCHAIN` removed from its environment, then no marker is written. The scan exits 2 with the message "the workspace selects its toolchain by path; set RUSTUP_TOOLCHAIN to a toolchain name to scan it without Clippy".
- [ ] The beta-bin fixture of `native_detectors_never_compile_or_execute_corpus_build_code` (`tests/corpus_precision.rs:363`) gains the clippy alias, the path toolchain and their markers. Its scan runs with `RUSTUP_TOOLCHAIN` removed from the child environment (`tests/support/corpus.rs:926-933`) and asserts that no marker exists.
- [ ] An integration test replays the same fixture under `cargo test` without the corpus clone cache.
- [ ] These four places state the invariant the fixed scan holds, "a Clippy-off scan starts no program the scanned tree selects", without claiming more:
  - the `trust_boundary` rationale and `selection_criterion` in `tests/corpus.json`;
  - `.github/workflows/corpus.yml:32`;
  - `tests/corpus_precision.rs:1517`;
  - `docs/agent-population-precision-2026-08.md`.

#### US-006: Resolve the workspace root from manifests for rules explain and ci install
**Description:** As a developer running `rules explain RULE PATH` or `ci install` on a cloned tree, I want the workspace root found by reading manifests, so that these commands start no toolchain process.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] A process-free resolver keeps `discover_manifest` (`src/scan_target.rs:100-144`). It then walks up from that manifest's directory to the nearest `Cargo.toml` declaring a `[workspace]` table, stopping at the same `.git` boundary, and otherwise returns the manifest's own directory.
- [ ] `rules explain` (`src/explain.rs:149`) and `ci install` (`src/ci.rs:244`) use that resolver, which also covers the TUI's add-workflow action. `why` without `--report` (`src/why_command.rs:86`) keeps the cargo-backed resolution, because it scans that root right after.
- [ ] The documentation at `src/rules_command.rs:9` describes the resolver the command now uses.
- [ ] Given a fixture with `rust-toolchain.toml` `path = "/proc/self/cwd/tc"` and an executable `tc/bin/cargo` marker, when `rules explain clippy::todo <fixture>` and `ci install <fixture> --branch main --dry-run` run, then no marker is written.
- [ ] For the same fixture, the effective level and the workflow's workspace placement equal those printed for the fixture without the toolchain file.
- [ ] Given the tests/support fake-cargo `PATH` harness, when both commands run, then no cargo process is started.
- [ ] Given a member crate path inside a workspace, then both commands resolve the workspace root, the directory holding `[workspace]`.
- [ ] Unhappy path: given a path with no manifest at or above it and no single workspace below it, then both commands fail with today's "no Cargo workspace" error kind.

---

### EP-002: Writes and terminal output the workspace cannot steer

This epic stops committed symlinks, git configuration and file names from redirecting rust-doctor's writes or its terminal output. It closes the baseline symlink finding (medium) and three lows: the installer confinement root, the installer print sites, and the `why` directive.

**Definition of Done:** US-007 to US-010 are DONE, and four checks hold:
- no write follows a planted symlink at any of the seven paths: the PID probe, the staging file, the four cache components, and `.cargo-husky/hooks`;
- the three installer confinement fixtures exit non-zero with an unchanged neighbor tree fingerprint;
- the installer and `why` outputs hold no control character other than `\n` on the hostile fixtures;
- a hostile rule id yields no directive.

#### US-007: Refuse symlinked cache components and create probe and staging files exclusively
**Description:** As a developer scanning a workspace with a committed `target/` tree, I want the baseline probe and the base-side staging file never written through a symlink, so that a scan cannot truncate a file outside its cache.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `src/workspace_write.rs` moves from the binary crate (`src/main.rs:19`) into the library. The installers, `persistent_target` (`src/baseline.rs:161-167`) and `side_cache::write` (`src/execution/side_cache.rs:188-195`) all use its `symlinked_component` and `create_new`.
- [ ] Given `<target>/rust-doctor`, `<target>/rust-doctor/baseline`, `<target>/rust-doctor/staged` or `<target>/rust-doctor/cache` is a symlink, when a baseline or staged scan runs, then nothing is created or truncated through it.
- [ ] In that case, `persistent_target` returns `None` and the scan falls back to the snapshot's private 0700 target, and the staging write is skipped.
- [ ] Given symlinks planted at the PID-named probe path and at `base-sides.<pid>.partial`, both pointing at an outside file, when in-process unit tests call `persistent_target` and `side_cache::write` with `std::process::id()`, then the outside file's bytes and modification time are unchanged.
- [ ] Given no symlink, when a baseline scan runs twice, then the first run stores the base side and the second reuses it.
- [ ] Unhappy path: given the staging file already exists (a stale partial whose PID was reused), when the write runs, then it is refused, the scan completes, and a later run stores the side.

#### US-008: Anchor installer confinement to the filesystem's .git entry
**Description:** As a developer running `setup`, `hook install git` or `ci install` on a cloned tree, I want confinement anchored to the `.git` entry on disk, so that the tree's own git configuration cannot move where the installer writes.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Before any write, `install_git` (`src/hook.rs:172-205`), `setup` and the ci placement (`src/ci.rs:241-254`) require that the nearest ancestor of the canonical workspace holding a `.git` directory or gitfile equals the canonical toplevel git reports.
- [ ] Unhappy path: given a workspace outside the reported toplevel, then the command exits non-zero with a message naming both paths. This replaces the `unwrap_or_default` fallback to `.` at `src/hook.rs:178-182`.
- [ ] Given each of three fixtures, when `hook install git`, `setup` and `ci install --branch main` run, then each command exits non-zero and the neighbor tree's `treeFingerprint` is unchanged. The fixtures are:
  - (a) a `.git/config` whose relative `core.worktree` and `core.hooksPath` point at a neighbor;
  - (b) a tracked embedded bare repository;
  - (c) a `core.worktree` with surplus `..` segments.
- [ ] The cases above are e2e rows added to `tasks/e2e-catalog/subcommands.md`.
- [ ] Given a linked worktree (a gitfile) or a submodule, when the same commands run, then they install as they do today.

#### US-009: Sanitize every repository-derived fragment the installers print
**Description:** As a developer running the installers on a cloned tree, I want every repository-derived name and line passed through terminal sanitization before printing, so that committed names cannot drive my terminal.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] These fragments pass through `rust_doctor::terminal_text::sanitize` before they are printed:
  - the hook directory and symlink names and the existing hook's command line (`src/hook.rs`);
  - the workflow file name in `CiError::Installed`'s `Display` (`src/ci.rs`);
  - the serialized previous and updated agent hook objects (`src/hook/agent.rs`).
- [ ] Unhappy path: given a workspace-relative path holding a control character, when `pre_commit_command` would embed it, then the installer exits non-zero before writing.
- [ ] Given a git fixture with these four hostile inputs, when `setup --dry-run`, `hook install git --update --dry-run`, `ci install --branch main --dry-run` and `hook install claude --update --dry-run` run on a clone, then stdout and stderr hold no character for which `char::is_control()` is true other than `\n`. The inputs are:
  - an ESC-named directory inside the tree behind `.cargo-husky/hooks`;
  - a marker hook whose line holds ESC;
  - an ESC-named workflow mentioning the Action;
  - C1 escapes in `.claude/settings.local.json`.

#### US-010: Emit a why directive only for a well-formed lint path
**Description:** As a developer reading `why --report` output from a saved report, I want a suppression directive only for a well-formed rule id, so that a crafted report cannot hand me Rust to paste into my crate.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `directive_for` (`src/explain/why.rs:226`) returns a directive only when the rule id is one or more `::`-separated, non-empty segments of ASCII letters, digits and `_`, the charset `src/render/markdown.rs:314` accepts. Otherwise `WhyHit.directive` is `None` and no directive line is printed.
- [ ] Unhappy path: given a saved report whose `code` is `clippy::todo)]\nfn x(){}//`, when `why --report <file> <path:line>` runs, then the output holds no `#[expect` line.
- [ ] Given `clippy::todo`, the output prints `#[expect(clippy::todo, reason = "<why this site is not a defect>")]`.
- [ ] Given a native `rust_doctor::source::<rule>` id on a `.rs` path, the output prints the `// rust-doctor: allow(...)` comment.
- [ ] Given a saved report whose suppression rules or path hold ESC or C1 characters, then the `why` output holds no control character other than `\n`.

---

### EP-003: Report privacy and advisory authority

This epic makes the published report mask what it detects and judge dependencies against RustSec only. It closes three lows: the private key mask, the external paths, and the non-RustSec advisory database.

**Definition of Done:** US-011 to US-013 are DONE, and three checks hold:
- no regression case in this epic publishes a key body character;
- every path case publishes at most its last two components after `<external>/`;
- a non-RustSec clone never becomes `cargo-deny` source.

#### US-011: Mask a private key from its header to its footer, and mask Cargo stderr before the excerpt cuts it
**Description:** As a report consumer, I want a private key masked across its whole body, including a body on the header's line and a key a build script echoed, so that neither code frames, report errors, Markdown summaries nor PR comments publish it.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] In `src/repo_hygiene/credentials.rs`, the private key range runs from the header to its footer, or to the end of the line when the body sits on the header's line. `mask_credentials` carries an open key body across lines the way the code frame does.
- [ ] `cargo_stderr::excerpt` (`src/cargo_stderr.rs:52`) runs `mask_credentials` on the scrubbed stderr, line by line, before lines are filtered, cut (`:145`) and joined (`:159`).
- [ ] `key_continues` replaces `opens_private_key` in `credentials.rs`, in the re-export at `src/repo_hygiene.rs:39-40` and at its call site in `src/presentation/code_frame.rs:447`. No unused function or re-export remains.
- [ ] Given a one-line PEM escaped with `\n` and a three-line PEM, then `mask_credentials` output holds no body character.
- [ ] Given a stderr whose 1020-byte suffix starts on a body line, then `excerpt` output holds no body character.
- [ ] Given a code frame of a one-line PEM, then it holds no body character and leaves the line below unmasked.
- [ ] Unhappy path: given an e2e fixture whose `build.rs` prints a dummy PEM built at runtime and exits 1, then neither `errors[].message` nor `report markdown` holds a body character.

#### US-012: Externalize quoted, spaced and Windows-rooted paths whole
**Description:** As a report consumer, I want every absolute path outside the workspace reduced to `<external>/` plus its last two components, so that reports do not publish outside directory layouts or Windows profile names.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] A path opened right after a backtick or a double quote ends at its closing delimiter.
- [ ] A path opened by an apostrophe ends at the first apostrophe that no letter or digit follows, so `Jane's Drive` stays inside it.
- [ ] An unquoted path extends across whitespace while the next word continues it with a separator and does not open a new absolute path.
- [ ] `absolute_path_at` (`src/report/sanitize.rs:151-176`) recognizes `\\?\X:\`, `\\?\UNC\` and `\\server\share` roots.
- [ ] `home_paths` (`src/report/sanitize.rs:110`) and `cargo_stderr::excerpt` (`src/cargo_stderr.rs:57`) fall back to `USERPROFILE` when `HOME` is unset.
- [ ] Regression tests cover:
  - a backticked path holding a space, and one holding an apostrophe;
  - a rustup-style single-quoted path holding an apostrophe;
  - a verbatim path and a UNC path;
  - an unquoted `C:\Users\First Last\x`;
  - an unquoted path followed by a relative one.
- [ ] Each covered case publishes at most its last two components after `<external>/`.
- [ ] The outputs pinned by `src/report/tests/normalization.rs:522-571` and the `<external>/lib/libfoo.so` shape at `tests/scan_failures.rs:823` are unchanged.
- [ ] The new code draws no `clippy::manual_pattern_char_comparison` warning.
- [ ] Unhappy path: given a quote that never closes, then the path runs to the end of its line and still publishes no more than its last two components.

#### US-013: Bind advisory database discovery to the RustSec remote
**Description:** As a developer relying on `vulnerable_dependency` verdicts, I want discovery to accept only a RustSec clone, so that another database cargo-deny cloned never silently decides them.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `AdvisoryDatabase::discover` (`src/cargo_health/advisories.rs:89-111`) reads each candidate's `.git/config` as text, starting no process. Both the cargo-audit and the cargo-deny locations accept only a remote equal to `https://github.com/rustsec/advisory-db`, compared case-insensitively and tolerating a trailing `.git` or `/`.
- [ ] Qualifying cargo-deny clones are ordered by the modification time of `.git/FETCH_HEAD`, or of `.git/HEAD` when there is no `FETCH_HEAD`.
- [ ] Unhappy path: given only a non-RustSec clone, then discovery returns `None` and both advisory rules publish `advisory-db-missing`.
- [ ] Given the fixture database under `advisory-dbs` with a `.git/config` naming the RustSec remote, plus a newer directory whose `.git/config` names `https://example.invalid/advisory-db` and holds one unrelated advisory, then the report shows source `cargo-deny`, 7 advisories and two `vulnerable_dependency` findings (`tests/advisory_database.rs`).
- [ ] `--advisory-db` still loads any database the user names explicitly.
- [ ] The existing cargo-audit and cargo-deny discovery tests carry a remote file.
- [ ] The `advisories.rs` module documentation, `src/report.rs:531` and `docs/subsystems/execution.md` state that discovery accepts only the RustSec remote.

---

### EP-004: Release and launcher supply chain

This epic binds the programs that publish and launch rust-doctor by content. It closes the release.yml mutable refs finding (medium) and the npm launcher resolution finding (low).

**Definition of Done:** US-014 and US-015 are DONE. The release.yml contract test passes with 23 of 23 `uses:` pinned and exact npm and Node versions. The launcher refuses an ancestor-only native package.

#### US-014: Pin every release.yml action to a commit and the release's npm and Node exactly, under a contract test
**Description:** As the maintainer, I want every program in a publishing or building release job bound by commit or exact version, so that an upstream ref move cannot reach a publication without review here.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Every non-local `uses:` in `.github/workflows/release.yml` names a 40-hex commit followed by `# <ref>`, the ref the commit was read from.
- [ ] `npm install -g npm@^11.5.1` (`release.yml:242`) becomes `npm install -g npm@11.21.1`.
- [ ] The publish job's `node-version` (`release.yml:233`) becomes `"22.13.0"`, matching the launcher job (`release.yml:196`).
- [ ] A test beside `every_checkout_drops_its_persisted_credentials` (`tests/docs_contract.rs:436`) fails when `release.yml` holds:
  - a non-local `uses:` without a 40-hex ref and a ref comment;
  - an `npm@` spec that is not an exact `x.y.z`;
  - a `node-version` that is not an exact `x.y.z`.
- [ ] Unhappy path: the test's predicate rejects these in-memory samples: `actions/checkout@v5`, `dtolnay/rust-toolchain@master`, a 39-hex ref, a pinned ref with no comment, and `npm@^11.5.1`.
- [ ] `docs/ci-workflows.md` states the pin rule for `release.yml` and how a pin is updated.

#### US-015: Confine the npm launcher's native package to its own install tree
**Description:** As a user installing rust-doctor from npm, I want the launcher to execute only the native binary installed with it, so that a same-named package elsewhere on the module path is never run.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `resolveNativeBinary` (`npm/rust-doctor/lib/launcher.js`) builds exactly two candidate roots instead of calling Node's resolver: `<wrapperRoot>/node_modules/<package>`, and `<dirname(wrapperRoot)>/<package>` when that parent is a `node_modules` directory.
- [ ] `binaryPath` derives from the root whose manifest was verified.
- [ ] Given a pnpm `.pnpm/<wrapper>/node_modules` sibling layout, or the npm and bun flat layouts, then the native binary resolves.
- [ ] Unhappy path: given an ancestor `node_modules/@rustdoctor/linux-x64` above a wrapper with no native package, both inside the test's own mkdtemp root, then the launcher fails with the existing "is not installed" message.
- [ ] Given a sibling with a manifest but no binary, above an ancestor that has a binary, then the launcher fails with the existing "is incomplete" message.
- [ ] `bun run smoke:packed` keeps passing.

---

### EP-005: Close the five needs-validation leads

This epic applies the source fix for each run-1 lead, plus the hardening notes that close a lead. After it, none of the five depends on a hosted fact for its safety.

**Definition of Done:** US-016 to US-021 are DONE, and six checks hold:
- no release job restores a cache;
- no job holding `id-token: write` compiles anything;
- snapshot parents pass the owner check;
- the Action and the generated workflow save caches only from the default branch and run the launcher by absolute path;
- no job holding `pull-requests: write` runs pull request code;
- `hook run --no-clippy` runs no build script.

#### US-016: Verify the crate in a job that holds no OIDC permission
**Description:** As the maintainer, I want the crates verification build moved out of the job that exchanges the OIDC token, so that compiled build scripts and procedural macros never share that job's identity.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** Blocked by US-014

**Acceptance Criteria:**
- [ ] A new job, `crates-verify`, holds `permissions: contents: read` and no `id-token`. It runs a checkout with `persist-credentials: false`, the pinned toolchain and `cargo publish --locked --dry-run`. The `crates` job needs it.
- [ ] The `crates` job keeps only these steps: checkout, toolchain, version resolution, `rust-lang/crates-io-auth-action`, and `cargo publish --locked --no-verify`. It has no rust-cache step.
- [ ] Where an OIDC permission is meant, these say "job", not "step":
  - the comments at `release.yml:373-378`;
  - `docs/publishing.md:64-67`;
  - the `secret_in_build_step` help at `src/policy/catalog/native.rs:198`.
- [ ] `tests/corpus.json` is regenerated when its catalog snapshot carries that help text.
- [ ] A contract test fails when any `release.yml` job granting `id-token: write` contains a rust-cache step or any cargo command other than `cargo metadata --no-deps` and `cargo publish --locked --no-verify`.
- [ ] Unhappy path: given a run that is not a tag, `crates-verify` still runs the dry run and `crates` publishes nothing.

#### US-017: Build release artifacts without a restored cache
**Description:** As the maintainer, I want release builds to start cold, so that no entry written by a corpus or CI run on the default branch reaches a shipped binary.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** Blocked by US-014

**Acceptance Criteria:**
- [ ] The `build`, `build-windows`, `launcher` and `crates` jobs of `release.yml` have no `Swatinem/rust-cache` step.
- [ ] A contract test fails when `release.yml` holds any rust-cache step.
- [ ] The statement that every corpus input is pinned (`.github/workflows/corpus.yml:35` and the `tests/corpus.json` trust record) names the lockless subjects whose dependency graph resolves at replay.
- [ ] Unhappy path: given a `workflow_dispatch` run on the default branch after merge, then each job finishes within its existing `timeout-minutes`: build 60, build-windows 60, launcher 30, crates 30. Owner-observed.

#### US-018: Create baseline and staged snapshots under a per-user private parent
**Description:** As a developer on a shared Unix host, I want snapshot roots created under a parent no other user can write, so that a Cargo config, a rustup toolchain file or a manifest planted by another user in `/tmp` never applies to my scan.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-007

**Acceptance Criteria:**
- [ ] `create_temp_root` (`src/baseline.rs:386`) takes the first candidate whose canonical path and every ancestor are owned by the user or root and are not writable by group or others. The candidates, in order:
  - `env::temp_dir()`;
  - `$XDG_RUNTIME_DIR`, on Unix;
  - `$XDG_CACHE_HOME/rust-doctor` or `$HOME/.cache/rust-doctor`, created 0700.
- [ ] On Windows, `env::temp_dir()` is taken as it is today.
- [ ] A candidate inside the repository being scanned is skipped instead of failing the run.
- [ ] `sweep::dead_snapshots` runs on the parent actually chosen.
- [ ] `discover_manifest` stops at the snapshot root when it starts inside a snapshot.
- [ ] Unit tests drive the chooser with injected candidates:
  - an owner-only directory is accepted;
  - a 1777 directory is refused;
  - a 0775 group-writable directory is refused;
  - a symlinked ancestor is resolved before the check.
- [ ] Unhappy path: given `/tmp` at mode 1777 and no other candidate passing, when a baseline or `--staged` scan runs, then nothing is created under `/tmp` and the scan reports incomplete, with the `temp_unavailable` error kind and a message naming the owner check.

#### US-019: Restore no executables and save no pull request cache in the Action and the generated workflow, and run the launcher by absolute path
**Description:** As a repository owner using the Action or the generated workflow, I want pull request runs never to save a cache and the pinned launcher never to be shadowed, so that a build script from one pull request cannot plant a binary or a target entry for a later run.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] The rust-cache step of `action.yml` (`:89`) and the generated one (`src/ci.rs:470-472`) set `save-if: ${{ github.ref == format('refs/heads/{0}', github.event.repository.default_branch) }}` and `cache-bin: false`, and keep `cache-on-failure: true`.
- [ ] When `version` is not `preinstalled`, `action.yml` resolves the launcher once, after `npm install -g`, as `"$(npm prefix -g)/bin/rust-doctor"`, and invokes that path at `:146` and `:160`.
- [ ] The generated workflow resolves the launcher the same way in its scan and Markdown steps.
- [ ] Given `version: preinstalled`, then the Action keeps its `PATH` lookup.
- [ ] Given a planted `~/.cargo/bin/rust-doctor` earlier on `PATH`, when the Action runs a pinned version, then the npm-installed launcher runs. An `action-replay` e2e row asserts the version line.
- [ ] `src/ci/tests.rs:607-609`, `tests/e2e/ci-install.test.ts:146` and the `action-static` tests assert both inputs and the absolute invocation.
- [ ] Unhappy path: given a `pull_request` event, the static check shows the `save-if` expression evaluates false, so the cache step restores and never saves.

#### US-020: Post the sticky comment from a job that runs no pull request code
**Description:** As a repository owner, I want the PR comment posted by a job that holds `pull-requests: write` and never runs the scanned code, so that a contributor's build script cannot use that token.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** Blocked by US-019

**Acceptance Criteria:**
- [ ] A new composite action, `comment/action.yml`, downloads the `rust-doctor-report` artifact with `actions/download-artifact` pinned to a commit and a `# <ref>` comment. It then runs the comment script with `GH_TOKEN`. It checks out nothing, installs nothing, and takes a `comment-on-clean` input that defaults to `true`.
- [ ] `action.yml` stops posting: its comment step and its `comment` and `comment-on-clean` inputs are removed.
- [ ] `action.yml` uploads `report.json`, `summary.md` and the clean flag in the `rust-doctor-report` artifact.
- [ ] `action.yml`'s documentation tells the caller to grant the scan job `contents: read` only.
- [ ] `ci install --comment` generates two jobs:
  - `inspect`, under a workflow-level `permissions: contents: read` and nothing else;
  - `comment`, with `needs: inspect`, `if: always() && github.event_name == 'pull_request'`, a job-level `permissions: pull-requests: write`, no checkout, the pinned download step and the inline `COMMENT_SCRIPT`.
- [ ] The test that holds `action.yml`'s script equal to `COMMENT_SCRIPT` now holds `comment/action.yml` to it.
- [ ] Given the `inspect` job fails its gate, then the `comment` job still posts the summary.
- [ ] Given no artifact, the `comment` job prints the existing notice and exits 0.
- [ ] Given a fork's read-only token, the `comment` job prints the existing warning and exits 0.
- [ ] The README and `docs/ci-workflows.md` show the two-job layout, and the next release note carries the migration for consumers of the removed inputs.
- [ ] Unhappy path: a contract test fails when, in the generated workflow or the documented layout, a job holding `pull-requests: write` checks out pull request code or runs rust-doctor.

#### US-021: Offer a compile-free agent hook and state the compile authority at install
**Description:** As a developer installing an agent end-of-turn hook, I want a mode that compiles nothing and a clear statement of what the default mode runs, so that I choose knowingly whether agent-written build code runs under my identity every turn.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-004, US-005

**Acceptance Criteria:**
- [ ] `hook run <agent> --no-clippy` plans every `clippy::*` rule off, whatever `rust-doctor.toml` says. Given a fixture with a `build.rs` marker, the run writes no marker and starts no `cargo clippy` process.
- [ ] `hook install claude --no-clippy` and `hook install cursor --no-clippy` write the hook command with `--no-clippy`, and `--update` sets or drops the flag as the new invocation says.
- [ ] Installing an agent hook without `--no-clippy`, directly or through `setup`, prints one statement:
  - the hook runs `cargo clippy`, build scripts and procedural macros of the tree the agent wrote;
  - it does so at the end of every turn, with the user's authority;
  - `--no-clippy` is the compile-free alternative.
- [ ] `why --help` and `hook run --help` carry `TRUST_WARNING` (`src/main.rs:48`), as the root help and the inspect help already do.
- [ ] `skills/rust-doctor/SKILL.md` and `README.md` state the same authority and the flag.
- [ ] `tests/skill_contract.rs` passes against the new `--help`.
- [ ] Unhappy path: given a `rust-doctor.toml` that sets a Clippy rule to `deny`, when `hook run claude --no-clippy` runs, then the report holds no Clippy finding and no build runs.

---

## Functional Requirements

- FR-01: A scan whose plan has no active Clippy rule must start no program the scanned tree selects, whether through git configuration, Cargo configuration or a rustup toolchain file.
- FR-02: Every git call rust-doctor makes during a scan must neutralize every filter driver the effective configuration names, and must set `safe.bareRepository=explicit`.
- FR-03: When rust-doctor cannot neutralize the filter drivers (the enumeration failed, or git ignores the environment pairs), it must fail the git-scoped stage closed, not run the call unneutralized.
- FR-04: `rules explain` and `ci install` must resolve the workspace root without starting a toolchain process.
- FR-05: The system must NOT create or truncate a file through a symlinked component under the target directory, or through a symlink at the final path.
- FR-06: The installers must refuse to write when the nearest `.git` entry on disk does not match git's reported toplevel, and must refuse a workspace outside that toplevel.
- FR-07: Every string derived from the repository or a saved report that rust-doctor prints to a terminal must pass through `terminal_text::sanitize`.
- FR-08: `why` must publish a suppression directive only for a well-formed lint path.
- FR-09: Reports must mask a detected private key across its whole body, in code frames, report errors, Markdown summaries and PR comments.
- FR-10: Reports must reduce every absolute path outside the workspace to `<external>/` plus its last two components, whether it is quoted, contains spaces, or has a Windows root.
- FR-11: Advisory discovery must accept only a clone whose remote is RustSec's, and must report the database as missing otherwise.
- FR-12: Every `uses:` of `release.yml` must name a 40-hex commit, the npm CLI and Node must be exact versions, no release job may restore a cache, and no job holding `id-token: write` may compile.
- FR-13: The npm launcher must execute only a native binary inside its own install tree.
- FR-14: In the Action and the generated workflow, only the default branch may save a cache, `~/.cargo/bin` may not be cached, the launcher must run by absolute path, and only a job that runs no pull request code may hold `pull-requests: write`.
- FR-15: Baseline and staged snapshot roots must live under a parent that only the scanning user or root can write.
- FR-16: `hook run --no-clippy` must compile nothing, and installing an agent hook without the flag must state the compile authority it grants.

## Non-Functional Requirements

- **Performance:** A Clippy-off full scan of this repository starts exactly 3 fewer processes than at `903afbe` (no version probes) and 1 more git process (the driver enumeration). Its median wall-clock over 5 runs grows by at most 5% against `903afbe`. Each release job finishes within its existing `timeout-minutes`: build 60, build-windows 60, launcher 30, crates 30.
- **Security:**
  - 0 of the 18 run-1 fingerprints reproduce with their recorded reproduction;
  - the EP-001 fixture leaves 0 marker files across its 8 runs;
  - 23 of 23 `release.yml` `uses:` are pinned to 40-hex commits;
  - 0 jobs holding `id-token: write` restore a cache or compile;
  - 0 jobs holding `pull-requests: write` run pull request code.
- **Privacy:** 0 key body characters and 0 outside path components beyond the last two appear in the outputs of the US-011 and US-012 regression cases.
- **Reliability:**
  - every new e2e row passes 3 consecutive runs of its file;
  - every fail-closed path (US-002 canary, US-005 path toolchain, US-018 no candidate) exits with a documented code and a message of at most 2 lines;
  - 0 existing tests are deleted to make a story pass.
- **Compatibility:** `SCHEMA_VERSION` stays 23, and 0 report fields change type. Linux and macOS behavior is covered by tests, and Windows path forms by unit tests. The breaking change is limited to 2 removed Action inputs (`comment`, `comment-on-clean`).
- **Accessibility:** The TUI's keyboard map changes in 0 bindings. The only rendered change is the toolchain version line, which is omitted when the versions are null.

## Edge Cases & Error States

| # | Scenario | Trigger | Expected Behavior | User Message |
|---|----------|---------|-------------------|--------------|
| 1 | No filter configured | Repository whose config names no `filter.*` key | Only the `lfs` pairs are set; output identical to `903afbe` | None |
| 2 | Old git ignores env pairs | git without `GIT_CONFIG_COUNT` support and a non-LFS driver | Scope or snapshot stage fails closed; no diff or checkout-index runs | "git 2.31 or later is required to scan a repository that configures filter drivers" |
| 3 | Driver name boundary | Driver named `p=q`, `a.b`, or enumeration over 64 KiB | Names with `=` and `.` neutralized; over-bound output fails closed | Same as row 2 for the over-bound case |
| 4 | Path toolchain on a Clippy-off scan | `rust-toolchain.toml` `path` key and no inherited `RUSTUP_TOOLCHAIN` | Metadata refused before it runs; exit 2 | "the workspace selects its toolchain by path; set RUSTUP_TOOLCHAIN to a toolchain name to scan it without Clippy" |
| 5 | rustup absent | Distro cargo, no `rustup` on `PATH` | Metadata runs unpinned, as today | None |
| 6 | Symlinked cache component | Committed `target/rust-doctor/cache -> /elsewhere` | No write through it; base side not stored; scan completes | None (`--verbose` notes the refused cache) |
| 7 | Concurrent or stale staging file | `base-sides.<pid>.partial` exists (reused PID, crashed run) | `create_new` refuses; scan completes; a later run stores the side | None |
| 8 | Linked worktree or submodule | Workspace whose `.git` is a gitfile | Installer confinement passes; installs as today | None |
| 9 | Workspace outside reported toplevel | Hostile `core.worktree` | Installer exits non-zero, writes nothing | "refusing to install: the workspace <a> is not inside the repository <b>" |
| 10 | RustSec remote spelling | `https://github.com/RustSec/advisory-db.git/` | Accepted (case-insensitive, `.git` and `/` tolerated) | None |
| 11 | Unclosed quote in a path | `` unclosed `/a/b/Jane Doe/c and more `` | Path runs to end of line; publishes at most `<external>/` plus 2 components | None |
| 12 | Yarn PnP install | No `node_modules` layout | Launcher refuses | Existing "is not installed" message |
| 13 | Preinstalled Action version | `version: preinstalled` | `PATH` lookup kept | None |
| 14 | Comment job without artifact | Scan step crashed before upload | Notice printed, exit 0 | "rust-doctor wrote no report, so no comment was posted." |
| 15 | No private snapshot parent | `/tmp` 1777, no `XDG_RUNTIME_DIR`, `$HOME` group-writable | Baseline or staged scan incomplete, nothing created | "no private directory for the snapshot: <candidate> is writable by other users" |
| 16 | Agent re-enables Clippy | `rust-doctor.toml` sets a Clippy rule to `deny` under `hook run --no-clippy` | Flag wins; no build | None |
| 17 | Schedule event in the Action | `schedule` payload without `repository.default_branch` | `save-if` false; restore only | None |

## Risks & Mitigations

| # | Risk | Probability | Impact | Mitigation |
|---|------|------------|--------|------------|
| 1 | `src/execution.rs` (935 lines) or `src/report.rs` (948) crosses the 1000-line bound | High | Low | Move toolchain resolution and the shared name-only pin into `src/execution/toolchain.rs`; the size-bound tests catch a breach |
| 2 | A consumer of toolchain strings (TUI, Markdown, e2e, website) breaks on null versions | Med | Med | Search every reader of `toolchain.*` before US-004 merges; the schema type is already nullable |
| 3 | The environment-pair neutralization misses a git version or a driver form | Low | High | Canary fail-closed (US-002) and a fixture matrix across 8 scope variants |
| 4 | US-001 disproves A1 or A2 | Low | Med | Documented fallback: owner-only working directory, decided before US-005 starts |
| 5 | Action consumers lose PR comments after the breaking move | Med | Med | Release note migration snippet, README two-job example, version bump to the next minor |
| 6 | SHA pins go stale with no automated updater | Med | Low | `docs/ci-workflows.md` documents the update procedure; Dependabot is an open question |
| 7 | Cold release builds exceed a job timeout | Low | Med | Owner-observed `workflow_dispatch` run before the next tag; raise `timeout-minutes` only with a measured duration |
| 8 | Word budgets reject the new subsystem reasoning | Med | Low | Write reasoning once in the owning subsystem document and trim restatements |
| 9 | The name-only pin refuses a legitimate path toolchain on Clippy-off scans | Low | Low | Actionable message; an inherited `RUSTUP_TOOLCHAIN` name is honored |

## Non-Goals

What this version explicitly does NOT include:

- **Run-1 hardening notes that close no lead** (REPORT.md section 6). These include:
  - bidi and format characters in `terminal_text`;
  - Markdown `GH-<n>`, bare SHA and `$` escapes;
  - the bound on the summary size;
  - single-pass substitution in `scan_workflow`;
  - the advisory id charset;
  - paired secrets in code frames;
  - PGP key detection;
  - the four-character prefix `masked()` keeps;
  - raw suppression text in `--json`;
  - symlinked workflow reads in `existing_install`;
  - the symlink root in `path_argument`;
  - porcelain `git diff` rewriting the index;
  - relative `PATH` entries in `handoff`;
  - a digest binding the launcher to its native binaries;
  - `--ignore-scripts`.

  None is a vulnerability. They go to a hardening PRD after the run-2 audit.
- **Pinning ci.yml, dogfood.yml and corpus.yml** (50 unpinned `uses:`). They hold no publish authority, and after US-017 they feed no cache into a publishing build.
- **Restricting the environment of the Clippy child in `hook run`.** It would break build scripts that read toolchain variables. The compile-free mode is the boundary instead.
- **Owner-observed hosted validations of the five leads** (the NEEDS-VALIDATION.md plans). The source fixes remove the dependency on those facts.
- **Resolving the repository from the requested path with `--git-dir`/`--work-tree`** for working-tree calls. `safe.bareRepository` and the discovery refusal close that finding.
- **Repository and registry settings:** enabling GitHub's require-SHA-pinning policy and crates.io's trusted-publishing-only mode are owner actions outside the code.
- **Cutting the release and writing its note.** That belongs to the `release` skill.
- **Unquoted Windows paths whose first component holds a space** (`C:\Program Files (x86)\...`). The audit accepted this residual.

## Files NOT to Modify

- `src/policy/catalog.rs` and `src/policy/catalog/correctness*`: no rule is added or removed, and the catalog stays at 142. The only catalog edit is the `secret_in_build_step` help text in `src/policy/catalog/native.rs:198` (US-016).
- `src/audit/`: the score model is out of scope.
- `SCHEMA_VERSION` in `src/report.rs:44` and the frozen v7 archive: the report shape does not change.
- The measured blocks of `tests/corpus.json` (adjudication, `position_proof`, per-rule sites): only the trust text in US-005 and US-017 changes, plus a regeneration when US-016's help text reaches the catalog snapshot.
- The `version` fields in `Cargo.toml`, `npm/rust-doctor/package.json`, `bun.lock` and `action.yml`, and everything under `.github/releases/`: these are release-only.
- Existing rows in `tasks/e2e-catalog/*.md` and `tests/e2e/known-bugs.json`: append only.

## Technical Considerations

Framed as questions for engineering input, not mandates:

- **Architecture:** Where does toolchain resolution live once `src/execution.rs` nears 1000 lines? Recommended: `src/execution/toolchain.rs`, holding `resolve_toolchain`, `version_command` and the name-only pin moved out of `src/execution/baseline.rs`. Engineering to confirm the seam.
- **Data model:** Should the internal `Toolchain` fields become `Option<String>`, mirroring `ToolchainReport` (`src/report.rs:508-518`), or should Clippy-off plans carry a distinct unprobed variant? Recommended: `Option` fields, with the cache key hashing `None` as an empty field. Trade-off: the variant makes "not probed" explicit at the cost of a new type.
- **Git:** Should the driver enumeration be cached once per scan and per `--git-dir`, or rerun for each call? Recommended: once per scan and per git directory, stored on the scan's git context.
- **Module placement:** Should `workspace_write` move into the library as a public module (precedent: `terminal_text`), or should the library get a `pub(crate)` copy? Recommended: move it, so one implementation holds the invariant.
- **API design:** Should the comment be a separate composite action (`comment/action.yml`) or a `mode` input on the main action? Recommended: the separate action, because a job running it never installs a toolchain or rust-doctor. Backward compatible: no. Rollback: consumers pin the previous tag.
- **Dependencies:** No new crate or package. Which `actions/download-artifact` commit pairs with the pinned `upload-artifact` v4.6.2? Engineering to pick it and record the ref.
- **rustup auto-install:** Should the `rustup show active-toolchain` call set `RUSTUP_AUTO_INSTALL=0` so that a workspace-named, uninstalled toolchain is never downloaded? The variable's semantics are unverified; check the rustup documentation before relying on it.
- **Migration:** Does the snapshot parent order `temp_dir`, then `XDG_RUNTIME_DIR`, then the cache home pass on macOS, where `TMPDIR` is already per user? Engineering to confirm on a macOS host or in CI.

## Success Metrics

| Metric | Baseline (current) | Target | Timeframe | How Measured |
|--------|-------------------|--------|-----------|-------------|
| Run-1 confirmed findings reproducible | 13 | 0 | Month-1 | Regression tests per story, plus a replay of FINDINGS-DETAIL.md reproductions on the release binary |
| Run-1 leads with source fix unapplied | 5 | 0 | Month-1 | EP-005 `DONE` in the status file |
| Marker files written by the EP-001 fixture across its 8 runs | at least 5 (filter clean, filter smudge, clippy alias, path cargo, path rustc) | 0 | Month-1 | Integration test marker count |
| Unpinned `uses:` in release.yml | 23 | 0 | Month-1 | Contract test in `tests/docs_contract.rs` |
| Jobs holding `id-token: write` that compile or restore a cache | 1 (crates) | 0 | Month-1 | Contract test from US-016 |
| Run-2 audit recurrences of run-1 fingerprints | N/A (new) | 0 confirmed, 0 needs_validation | Month-6 | `/security-audit` run-2 using run-1 as its prior run |

## Open Questions

- Should `--no-clippy` become the default for agent hooks? Arthur, before Release 2 ships. It depends on how agent hosts sandbox Stop hooks (lead 5).
- Is a Windows runner supported by the Action? This decides the launcher's absolute path layout (`<prefix>/rust-doctor.cmd`). Arthur, before US-019 starts.
- Should Dependabot's `github-actions` ecosystem update the release.yml pins? Arthur, after US-014.
- Should the repository enable GitHub's require-SHA-pinning policy and crates.io's trusted-publishing-only mode? Arthur, after Release 1.
- Does the Action break justify 0.9.0 rather than 0.8.1? Arthur, at release time.
[/PRD]
