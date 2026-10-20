# Scope and git-state e2e scenarios for rust-doctor

Measured on 2026-10-20 against `target/release/rust-doctor` built from `b5389f7`, git 2.56.0, cargo/clippy 1.98.0 (rustup default; CI pins 1.97.1), Linux, 16 cores, with another agent's cargo build running concurrently (timings are noisy upper bounds). Every scenario marked `yes` was run from the scripts beside this file (`s_*.sh`, helpers in `lib.sh` and `one.sh`), raw outputs are in `out/`, and the repositories are in `repos/`. Nothing under `/home/arthur/dev/rust-doctor` was written; the only access was read-only (`git clone --no-hardlinks` from it, `git status --porcelain` before and after, unchanged).

## Conventions every scenario uses

Invocation: `rust-doctor <path> --yes --json <args>` plus a second, human run for the scope line. Environment: `CLAUDECODE`, `CODEX_SANDBOX`, `CURSOR_AGENT` unset (otherwise a skill notice lands on stderr), `LC_ALL=C` (git speaks French on this host), `GIT_CONFIG_NOSYSTEM=1`, `GIT_AUTHOR_*`/`GIT_COMMITTER_*` fixed to `e2e <e2e@example.invalid>`, commit dates fixed to `2026-01-0<n>T00:00:00Z`, `CARGO_NET_OFFLINE=true` (production cargo calls do not pass `--offline`), a private `CARGO_TARGET_DIR` per scenario (or per template when warm reuse is the point), a private `TMPDIR` per scenario so snapshot cleanup is observable. Git is driven with `-c init.defaultBranch=main -c commit.gpgsign=false -c core.hooksPath=/dev/null` and `-c protocol.file.allow=always` for submodules.

Fingerprint ("repo state"): sha256 of every file and symlink under the repository including `.git/`, plus the sorted path list, plus `stat %Y %s` of `.git/index`, excluding `target/`. Computed without running git. `same` means identical before and after the scan.

### Templates

- `mini`: `Cargo.toml` (`[package] name="mini" version="0.1.0" edition="2024" publish=false` and an empty `[workspace]`), a v4 `Cargo.lock`, `.gitignore` = `/target`, `src/lib.rs` = `pub mod creds;` + `old_debt() -> u8 { todo!() }` (todo at line 4) + `untouched() -> u8 { 1 }`, `src/creds.rs` returning the AWS-shaped literal `"AKIA" + "Q3VJ7RNYH2KXW4PZ"` on line 2 (build it at runtime, as `tests/repo_hygiene.rs::fake_aws_key` does, so the suite's own sources never trip `hardcoded_credential`). One commit `base` on `main`. With the fixed identity and date the base SHA is deterministic: `af441c9b9f96faa3f098b61e401b7475f31e56d7`, which lets tests assert `comparison_base` literally.
- `NEWFN`: append `\npub fn new_debt() -> u8 {\n    todo!()\n}\n` to a file; in `src/lib.rs` the new `todo!()` lands on line 12.
- `nested`: repo root holding `README.md`, `top.rs` and `crates/app/` = `mini` without its own `.git`.
- `conc`: `tests/fixtures/score-credibility/packs/concurrency/{Cargo.toml,Cargo.lock,src/lib.rs,src/negatives.rs}` with the `ready(*value).await` line rewritten to `*value` in the base commit (same as `tests/contextual_scope.rs`).
- `rd`: `git clone --no-hardlinks /home/arthur/dev/rust-doctor` (HEAD `b5389f7`, `origin/HEAD` -> `origin/main`, tags `v0.1.0`..`v0.7.0`).

### Observables

JSON: `status`, `complete`, `gate.{status,blocking,blocking_diagnostics}`, `scope.{mode,execution_scope,comparison_base,files,base_ref,staged,untracked_unreported}` (the whole block is `null` when scope resolution failed), `delta.summary.{introduced,pre_existing,fixed,cross_file_matches,rule_count_matches}`, `delta.introduced[]` ids joined to `diagnostics[].{code,path,span.line_start}`, `errors[].{stage,code}`. `base_side_cache` is never serialized; it shows only in the human `--verbose` output as `Base side cache: hit|miss|disabled`. Human: the first `Scope:` line, the `N untracked Rust file(s) ... --include-untracked` note, `No staged Rust change to judge.`, `Delta: +I introduced; =P pre-existing; -F fixed; C cross-file matches.`, `Fixed: path:line:col ...`. Exit codes: 0 pass, 1 gate failed or incomplete, 2 failed or invalid arguments, 3..125 `--warning-exit-code`, 130 SIGINT/SIGTERM.

Scope line formats (from `src/render/scope.rs`): `Scope: full codebase`; `Scope: {changed|staged} files (N selected, base <REF> at <12hex>)`; `Scope: {changed|staged} lines (N files, base ...)`; `Scope: changed lines and the functions holding them (N files, base ...)`; `Scope: [staged ]baseline comparison (base ...)`. `<REF> at` appears only when the base was detected (`base_ref` non-null); a named `--base` prints `base <12hex>`.

## Confirmed bugs (minimal repros)

**BUG-A. `--scope baseline` reports every repository-pass finding as introduced, and never as fixed.** The base side is scanned from a snapshot written under `$TMPDIR/rust-doctor-baseline-<pid>-<n>/tree` (`src/baseline.rs:162-250`), which has no `.git`; `repo_hygiene::inspect` enumerates through `git ls-files` and treats "not a repository" as an observed empty result (`src/repo_hygiene.rs:1-10`, wired at `src/execution.rs:559`). The base side therefore holds zero `rust_doctor::repo::*` findings and the side cache stores that. Repro (E01): `mini`, then `git switch -c f` with no change at all; `CARGO_TARGET_DIR=<t> rust-doctor <repo> --yes --json --scope baseline --blocking warning` exits 1 with `gate {"blocking":"warning","status":"failed","blocking_diagnostics":1}`, `delta.summary {"introduced":1,"pre_existing":1,"fixed":0,...}`, and the introduced id is `rust_doctor::repo::hardcoded_credential` at `src/creds.rs:2` on a branch whose `git diff main` is empty. Without `--blocking warning` (bugA-cold, branch adds one `todo!()`): exit 0, `introduced: 2` = `hardcoded_credential@src/creds.rs:2` + `clippy::todo@src/lib.rs:12`, `pre_existing: 1`, and the human report leads with `Top warning: Hardcoded credential`. Mirror (E04): deleting `src/creds.rs` on the branch gives `fixed: 0`. Reproduces on rust-doctor itself (R03): a no-op branch of the `rd` clone reports `introduced: 1`, `rust_doctor::repo::hardcoded_credential@tasks/prd-react-doctor-gap-closure.md`. All five repo rules are affected (also seen: `tracked_secret_file`, S01f). `--package` masks it because the repo pass does not run under `--package` (E10).

**BUG-D. Every `--staged` scan drops the repository pass, including the installed pre-commit hook (`--staged --scope lines`).** The staged tree is the same kind of git-less snapshot (`src/baseline/staged.rs`), so a staged AWS key in `src/leak.rs` and a force-added `.env` holding a `ghp_` token pass with exit 0 and zero repo findings under `--scope files --staged`, `--scope lines --staged` and `--scope baseline --staged` (S01, S01b, S01c), while the same working-tree scans report `hardcoded_credential=2, tracked_secret_file=1` (S01d, S01e) and the report stays `complete`. The hook that exists to catch a commit is blind to the commit-time secret rules.

**BUG-C. Working-tree `files`, `lines` and `contextual` scopes rewrite `.git/index`.** `git_scope.rs` runs porcelain `git diff` (`diff_operation`), which refreshes the index's stat data and writes it back under `index.lock` despite `GIT_OPTIONAL_LOCKS=0`; the delta pass already switched to `git diff-index` for exactly this reason (`docs/subsystems/delta.md`). Repro (`s_index2.sh`): `mini`, wait 1.1 s, `touch src/lib.rs`, `rust-doctor <repo> --yes --json --scope files` -> sha256 of `.git/index` changes. With a racily clean index (`git add` then rewrite the file) the index mtime changes on every run, first and second (`s_index.sh`, D04, D14). `--staged`, `baseline` and full scopes leave it untouched. Bare `GIT_OPTIONAL_LOCKS=0 git diff --name-only HEAD` reproduces it; `git diff-index` does not.

**BUG-B. A scope that fails to resolve is printed as `Scope: full codebase`.** `render_scope` falls back to that line when `report.scope` is `None`, so `--scope files --base HEAD~1` prints `Scope: full codebase` then `Scan failed: Invalid Git base selector. (scope/invalid-base)`, exit 2. Same line for `base-undetected`, `base-unavailable`, `merge-base-unavailable`, `merge-base-ambiguous`, `shallow-clone`, `diff-too-large`, `index-conflicted`, `index-unavailable` (C04, C08b, C09, C10, C11, C13*, C14*, F01, F03). JSON carries `scope: null`, so neither output says which scope was asked for.

**BUG-E. Any submodule anywhere in the base tree refuses `--scope baseline`.** `parse_inventory` rejects `160000 commit` entries (`baseline-entry-invalid`, exit 2, E06), while `--staged` skips gitlinks (`staged.rs` accepts `160000`). Files and lines scopes work in the same repository (E06b).

**BUG-F. A relative symlink with a `..` component is refused even when it stays inside the repository.** `src/s.rs -> ../shared/s.rs` makes `--scope baseline` fail with `baseline/baseline-entry-invalid` (E07) and `--staged` fail with `scope/staged-materialization-failed` (E07c); a same-directory symlink passes (E07d). `validate_materialized_symlinks` requires every component to be `Normal` before checking containment, so the containment check never decides.

**BUG-G. An edit made through a tracked symlinked module is dropped by `files` and `lines`.** `src/alias.rs -> impls/real.rs`; adding a `todo!()` to `real.rs`: git lists `src/impls/real.rs`, rustc reports the finding at `src/alias.rs`, so the files and lines scopes keep nothing (H07, H07b, `todo paths: []`) while baseline reports it introduced at `src/alias.rs` (H07c).

Lesser findings, all verified: a cold base side cut by `--max-duration` reports only `baseline/baseline-scan-incomplete`, never the deadline (L03); a staged scan whose `TMPDIR` is inside the repository fails as `baseline/baseline-temp-unavailable` although no baseline was requested (H01b); a depth-1 single-branch clone (the `actions/checkout` default) gets `base-undetected` rather than the `shallow-clone` hint, because only `base-unavailable`/`merge-base-unavailable` are diagnosed (C08); SIGKILL leaves `$TMPDIR/rust-doctor-baseline-<pid>-0` behind and no later run sweeps it (K4); an untracked module declared in a committed `lib.rs` makes the base side fail to compile, so `--scope baseline --base HEAD` exits 2 with `baseline-scan-incomplete` (D05e, correct but a likely agent trap).

## Catalog

Columns: ID, setup recipe, arguments after `<path> --yes --json`, expected observable, existing Rust coverage, verified, timing (single JSON run; `mini` unless stated; `c` cold target, `w` warm). Abbreviations: `S{m,cb,ref,f,st,uu}` = scope `mode`, `comparison_base` (7 hex), `base_ref`, `files`, `staged`, `untracked_unreported`; `D[i,p,f,x,r]` = delta summary introduced, pre_existing, fixed, cross_file_matches, rule_count_matches; `E[...]` = `errors` as `stage/code`; `L:` = human scope line.

### A. Argument validation (clap level: exit 2, empty stdout even with `--json`, one `error:` line on stderr, no git or cargo process)

| ID | Setup | Args | Expected | Rust coverage | Ver. | Time |
|---|---|---|---|---|---|---|
| A01 | mini | `--base main` | exit 2, stderr `error: --base <REF> requires a files, lines, contextual or baseline scope` | main.rs `invalid_scope_combinations_are_rejected_before_execution`; git_scope_kernel `clap_rejects_invalid_scope_combinations_without_a_report_or_inspection` | yes | 0.00 |
| A02 | mini | `--scope full --base main` | same as A01 | same | yes | 0.00 |
| A03 | mini | `--staged` | exit 2, `error: --staged cannot judge the full codebase: pass --scope files, lines, contextual or baseline` | same; agent_loop `staged_is_refused_with_the_full_scope` | yes | 0.00 |
| A04 | mini | `--scope full --staged` | same as A03 | same | yes | 0.00 |
| A05 | mini | `--include-untracked` | exit 2, `error: --include-untracked requires a files, lines or contextual scope` | same | yes | 0.00 |
| A06 | mini | `--scope baseline --include-untracked` | same as A05 | same | yes | 0.00 |
| A07 | mini | `--scope lines --staged --include-untracked` | exit 2, `error: --include-untracked cannot be combined with --staged: the index holds no untracked file` | same | yes | 0.00 |
| A08 | mini | `--scope files --staged --include-untracked` | same as A07 | none for files | yes | 0.00 |
| A09 | mini | `--scope nope` | exit 2, `error: invalid value 'nope' for '--scope <SCOPE>'` | none | yes | 0.00 |
| A10 | mini | `--scope` (no value) | exit 2, `a value is required for '--scope <SCOPE>'` | none | yes | 0.00 |
| A11 | mini | `--base` (no value) | exit 2, `a value is required for '--base <REF>'` | none | yes | 0.00 |
| A12 | mini | `--scope full` | exit 0, `S{full,null,null,null,false,null}`, L: `Scope: full codebase` | git_scope_kernel full cases | yes | 0.12c/0.08w |
| A13 | mini | `--scope full --base main --staged` | exit 2, the `--base` message wins (first match) | none | yes | 0.00 |
| A14 | mini | `--scope contextual --include-untracked --staged` | exit 2, A07 message | none | no | 0.00 |

### A2. Base selector grammar (`--scope files --base=<b>`). Refused selectors fail before discovery: exit 2, `status:"failed"`, `E[scope/invalid-base]`, message `Invalid Git base selector.`, `scope:null`, `project:null`, L: `Scope: full codebase` (BUG-B), the selector never echoed

| ID | Base | Expected | Rust coverage | Ver. | Time |
|---|---|---|---|---|---|
| A20 | `HEAD~1` | invalid-base | git_scope/tests `closed_base_grammar_accepts_only_named_selectors_and_full_oids` | yes | 0.00 |
| A21 | `HEAD^` | invalid-base | same | yes | 0.00 |
| A22 | `main^{commit}` | invalid-base | same | yes | 0.00 |
| A23 | `-x` (as `--base=-x`) | invalid-base | same | yes | 0.00 |
| A24 | `--` (as `--base=--`) | invalid-base | none | yes | 0.00 |
| A25 | `a..b` | invalid-base | same | yes | 0.00 |
| A26 | `a//b` | invalid-base | same | yes | 0.00 |
| A27 | `.hidden` | invalid-base | same | yes | 0.00 |
| A28 | `x.lock` | invalid-base | same | yes | 0.00 |
| A29 | `x.` | invalid-base | none | yes | 0.00 |
| A30 | `@` | invalid-base | none | yes | 0.00 |
| A31 | `main@{1}` | invalid-base | none | yes | 0.00 |
| A32 | `café` (non-ASCII) | invalid-base | same | yes | 0.00 |
| A33 | `a b` | invalid-base | none | yes | 0.00 |
| A34 | `a:b` | invalid-base | none | yes | 0.00 |
| A35 | `refs/heads/` (trailing slash) | invalid-base | none | yes | 0.00 |
| A36 | 256 x `x` | invalid-base | same | yes | 0.00 |
| A37 | empty (`--base=`) | invalid-base | none | yes | 0.00 |
| A38 | `HEAD~1` on a nonexistent path | invalid-base, `project:null`: refused before discovery | git_scope_kernel `invalid_api_base_stops_before_discovery_without_disclosing_input` | yes | 0.00 |
| A39 | `HEAD~1` with `--package nope` | invalid-base wins over `policy/unknown-package` (E10d) | none | yes | 0.00 |
| A40 | `refs/heads/main` | accepted: exit 0, `cb=af441c9`, `ref:null` | none | yes | 0.10 |
| A41 | `HEAD` | accepted, `cb` = HEAD | same | yes | 0.10 |
| A42 | `main` | accepted | same | yes | 0.10 |
| A43 | 7-hex abbreviation | accepted (passes as a ref-like token, git resolves it) | none | yes | 0.10 |
| A44 | 40-hex full SHA-1 | accepted | same | yes | 0.10 |
| A45 | 39-hex abbreviation | accepted (not the 40/64 hex path, still a valid token) | none | yes | 0.10 |
| A46 | `v1.0` absent | grammar accepts, `E[scope/base-unavailable]`, exit 2 | none | yes | 0.02 |
| A47 | `MAIN` (case) | `E[scope/base-unavailable]` on a case-sensitive FS | none | yes | 0.02 |
| A48 | `origin/main` without remote | `E[scope/base-unavailable]` (= C09) | git_scope_kernel `missing_base_is_closed_and_never_reaches_tool_execution` | yes | 0.02 |
| A49 | 64-hex SHA-256 in a sha256 repo | accepted (L01b) | grammar unit only | yes | 0.09 |

Workaround to document for agents: `--base "$(git rev-parse HEAD~1)"`.

### C. Base resolution and repository shapes (`--scope files` unless stated)

| ID | Setup | Args | Expected | Rust coverage | Ver. | Time |
|---|---|---|---|---|---|---|
| C01 | mini on `main`, clean | `--scope files` | exit 0, `S{files,af441c9,"HEAD",[],false,0}`, L: `Scope: changed files (0 selected, base HEAD at af441c9b9f96)` | agent_loop `on_the_default_branch_the_base_is_head`; git_scope/tests same name | yes | 0.15 |
| C01b | C01 + unstaged edit of `src/lib.rs` | `--scope files` | `f:["src/lib.rs"]`, `ref:"HEAD"`, todo=2 | same | yes | 0.11 |
| C02 | mini, `switch -c feature`, commit NEWFN | `--scope files` | `ref:"main"`, `cb=af441c9`, `f:["src/lib.rs"]`, L: `... base main at af441c9b9f96)` | git_scope/tests `a_missing_base_is_the_first_default_branch_that_resolves` | yes | 0.13 |
| C02b | C02 | `--scope files --base main` | same files, `ref:null`, L: `... base af441c9b9f96)` | git_scope_kernel | yes | 0.09 |
| C02c | C02 | `--scope files --base HEAD` | `f:[]`, `cb` = branch tip | none | yes | 0.09 |
| C02d | C02 | `--scope lines` | `S{lines,...,"main",["src/lib.rs"]}`, todo=1 (line 12 only) | git_scope/tests lines | yes | 0.10 |
| C02e | C02 | `--scope contextual` | mode contextual, L: `Scope: changed lines and the functions holding them (1 files, base main at ...)` | contextual_scope (staged only) | yes | 0.09 |
| C02f | C02 (branch commit adds `pub fn f() {}`, no finding) | `--scope baseline` | `S{baseline,af441c9,"main",null,false,null}`, L: `Scope: baseline comparison (base main at af441c9b9f96)`; today `D[1,1,0,0,0]` with the credential introduced (BUG-A); expected `D[0,2,0,0,0]` | baseline_kernel | yes | 0.21 |
| C03 | mini with branch renamed `master`, on master | `--scope files` | `ref:"HEAD"` | agent_loop `every_changed_scope_resolves_its_base_on_a_master_repository` | yes | 0.11 |
| C03b | C03 + feature branch with a commit | `--scope files` | `ref:"master"` | same | yes | 0.11 |
| C04 | mini with branch renamed `trunk` | `--scope files` | exit 2, `E[scope/base-undetected]` (`No base branch found: pass --base <REF>.`), `scope:null`, L: `Scope: full codebase` (BUG-B) | agent_loop `no_base_to_find_fails_with_base_undetected` | yes | 0.02 |
| C04b | C04 | `--scope baseline` | same as C04 | none | yes | 0.02 |
| C04c | C04 | `--scope files --staged` | exit 0, `ref:"HEAD"`, `st:true`, `uu:null`: staged never detects a branch | none | yes | 0.14 |
| C04d | C04 | `--scope files --base trunk` | exit 0, `ref:null` | none | yes | 0.10 |
| C05 | origin repo with `develop` checked out; `git clone` (origin/HEAD -> origin/develop) | `--scope files` | on develop: `ref:"HEAD"`, `cb` = develop tip | none | yes | 0.11 |
| C05b | C05 + `switch -c topic` + commit | `--scope files` | `ref:"origin/develop"` (origin/HEAD wins over origin/main) | git_scope/tests (mocked) | yes | 0.11 |
| C05c | C05b + `git remote set-head origin main` | `--scope files` | `ref:"origin/main"`, `cb=af441c9` | none | yes | 0.09 |
| C05d | C05b + `git remote set-head origin -d` | `--scope files` | falls back to `origin/main` | none | yes | 0.09 |
| C05e | C05b | `--scope files --base origin/develop` | `ref:null`, `cb` = develop tip | none | yes | 0.09 |
| C05f | origin/HEAD pointing at a branch whose name fails the grammar (e.g. `feat@x`) | `--scope files` | candidate skipped, next candidate used | none | no | |
| C06 | mini + commit, `checkout --detach HEAD` (detached at main tip) | `--scope files` | `ref:"main"` (symbolic-ref fails, so the HEAD shortcut never applies), `f:[]` | none | yes | 0.11 |
| C06b | C06 + commit while detached | `--scope files` | `ref:"main"`, `f:["src/lib.rs"]` | none | yes | 0.11 |
| C06c | C06b | `--scope baseline` | `ref:"main"`, `D[1,1,0,0,0]`, introduced = credential only (BUG-A; expected `D[0,1,0,0,0]` since the commit adds no finding) | none | yes | 0.18 |
| C08 | origin with main(4 commits)+feature; `clone --depth 1 --branch feature` (refs: feature, origin/feature only) | `--scope files` | exit 2, `E[scope/base-undetected]` (not `shallow-clone`, see lesser findings) | none | yes | 0.02 |
| C08b | C08 | `--scope files --base origin/main` | exit 2, `E[scope/shallow-clone]` (`This clone is shallow: fetch with fetch-depth: 0, or pass --base to a fetched ref.`) | agent_loop `a_shallow_clone_names_the_missing_fetch`; git_scope/tests `a_base_missing_from_a_shallow_clone_says_so` | yes | 0.01 |
| C08c | C08 | `--scope files --base HEAD` | exit 0, `f:[]` | none | yes | 0.11 |
| C08d | C08 | `--scope baseline --base HEAD` | exit 0, `D[1,2,0,0,0]` (credential introduced, BUG-A) | none | yes | 0.17 |
| C08e | `clone --depth 1 --no-single-branch --branch feature` (both tips, no common ancestor) | `--scope files` | detects `origin/main` via origin/HEAD, merge base missing -> `E[scope/shallow-clone]` | git_scope/tests `a_merge_base_missing_from_a_shallow_clone_says_so` | yes | 0.02 |
| C08f | C08e | `--scope baseline` | `E[scope/shallow-clone]` | none | yes | 0.02 |
| C08g | `--depth 3 --no-single-branch` (enough history) | `--scope files` | exit 0, `ref:"origin/main"`, `f:["src/lib.rs"]` | none | yes | 0.10 |
| C09 | C06 repo (no remote) | `--scope files --base origin/main` | exit 2, `E[scope/base-unavailable]` | git_scope_kernel `missing_base_is_closed_...` | yes | 0.02 |
| C09b | same | `--base nosuch` | `E[scope/base-unavailable]` | same | yes | 0.01 |
| C09c | same | `--scope baseline --base nosuch` | `E[scope/base-unavailable]` | none | yes | 0.01 |
| C10 | criss-cross: branches `a` and `main` each merge the other's earlier tip, then each gets one more commit (2 merge bases) | `--scope files --base main` (on `a`) | exit 2, `E[scope/merge-base-ambiguous]` | git_scope/tests `missing_and_ambiguous_merge_bases_fail_before_diff` (mock) | yes | 0.02 |
| C10b | C10 | `--scope baseline` | `E[scope/merge-base-ambiguous]` | none | yes | 0.02 |
| C10c | `main` = `--no-ff` merge commit of `side`; branch `after` adds a todo fn | `--scope files --base main` | exit 0, `cb` = the merge commit, `f:["src/lib.rs"]` | none | yes | 0.10 |
| C10d | C10c | `--base main~1` | invalid-base | covered by A20 | yes | 0.00 |
| C11 | orphan branch `other` with unrelated history | `--scope files --base other` | exit 2, `E[scope/merge-base-unavailable]` (not shallow) | git_scope/tests mock | yes | 0.02 |
| C12 | branch `ahead` one commit past main; on main | `--scope files --base ahead` | exit 0, `cb` = main tip (merge base = HEAD), `f:[]` | none | yes | 0.10 |
| C13 | `git init` + files, no commit, no index | `--scope files` | `E[scope/base-undetected]` | none | yes | 0.02 |
| C13b | C13 | `--scope files --base HEAD` | `E[scope/base-unavailable]` | none | yes | 0.02 |
| C13c | C13 (no `.git/index` yet) | `--scope files --staged` | `E[scope/index-unavailable]` | agent_loop `an_unusable_index_fails_the_staged_scan` (other cause) | yes | 0.01 |
| C13d | C13 + `git add -A` | `--scope files --staged` | `E[scope/base-unavailable]` (HEAD unborn) | none | yes | 0.02 |
| C13e | C13d | `--scope baseline` | `E[scope/base-undetected]` | none | yes | 0.02 |
| C13f | C13d | (full) | exit 0, `hardcoded_credential=1` (index-tracked files count as tracked) | none | yes | 0.11 |
| C14 | mini files copied, no `.git` anywhere above | `--scope files` | `E[scope/base-undetected]` | none | yes | 0.02 |
| C14b | C14 | `--scope lines --base HEAD` | `E[scope/base-unavailable]` | none | yes | 0.02 |
| C14c | C14 | `--scope files --staged` | `E[scope/index-unavailable]` | none | yes | 0.01 |
| C14d | C14 | `--scope baseline --base main` | `E[scope/base-unavailable]` | none | yes | 0.01 |
| C14e | C14 | (full) | exit 0, no repo finding (outside git is an observed fact) | repo_hygiene | yes | 0.10 |
| C15 | mini, annotated tag `v1.0` on base, then 2 commits (edit lib.rs, add src/extra.rs) | `--scope files --base v1.0` | exit 0, tag peeled, `cb=af441c9`, `f:["src/extra.rs","src/lib.rs"]` | none | yes | 0.11 |
| C15b | C15 | `--scope files --base <sha of v1.0^{commit}>` | same as C15 (commit range) | git_scope_kernel | yes | 0.09 |
| C15c | C15 | `--scope lines --base v1.0` | `f` both files, ranges only | none | yes | 0.09 |
| H02 | mini | `--scope files --base <tree oid>` | `E[scope/base-unavailable]` (`^{commit}` refuses a tree) | none | yes | 0.02 |
| H02b | mini | `--base <blob oid>` | `E[scope/base-unavailable]` | none | yes | 0.01 |
| H03 | mini, `git switch --orphan fresh` (worktree emptied) | `--scope files` | `E[discovery/no-manifest]` | none | yes | 0.00 |
| L01 | mini in `git init --object-format=sha256`, feature commit | `--scope files` | exit 0, `ref:"main"`, 64-hex `comparison_base` | none | yes | 0.14 |
| L01c | L01 | `--scope baseline` | exit 0, `D[2,1,0,0,0]` (BUG-A inflates by 1) | none | yes | 0.20 |
| L01d | L01 | `--scope files --staged` | exit 0, `ref:"HEAD"` | none | yes | 0.14 |

### D. Working-tree states (files, lines, contextual)

| ID | Setup | Args | Expected | Rust coverage | Ver. | Time |
|---|---|---|---|---|---|---|
| D01 | mini + NEWFN unstaged | `--scope files` | `f:["src/lib.rs"]`, todo at lib.rs:4 and :12 | git_change_scope_product_proof | yes | 0.11 |
| D01l | D01 | `--scope lines` | todo only at :12 | git_scope/tests `a_lines_scope_keeps_...` | yes | 0.09 |
| D01c | D01 | `--scope contextual` | todo only at :12 (`clippy::todo` is line reach) | contextual_scope (staged) | yes | 0.09 |
| D01s | D01 | `--scope files --staged` | `f:[]`, `st:true`, `uu:null`, L: `Scope: staged files (0 selected, base HEAD at ...)` | agent_loop `nothing_staged_passes_and_says_so` | yes | 0.12 |
| D02 | mini + NEWFN staged (index = worktree) | `--scope lines` | worktree diff includes staged work: todo at :12 | none | yes | 0.11 |
| D02b | D02 | `--scope lines --staged` | `st:true`, todo at :12, L: `Scope: staged lines (1 files, ...)` | agent_loop `a_staged_scan_judges_the_index_and_never_the_working_tree` | yes | 0.12 |
| D04 | NEWFN staged, then worktree restored to HEAD content | `--scope lines` | `f:[]`, nothing reported; `.git/index` rewritten (BUG-C) | none | yes | 0.11 |
| D04b | D04 | `--scope lines --staged` | judges the index: todo at :12 | agent_loop (same) | yes | 0.12 |
| D04c | D04 | `--scope baseline --staged` | `S{baseline,...,"HEAD",st:true}`, `D[1,1,0,0,0]` introduced = todo@lib.rs:12; the credential is absent from both sides (BUG-D), expected `D[1,2,0,0,0]`; L: `Scope: staged baseline comparison (base HEAD at ...)` | none | yes | 0.12 |
| D04d | D04 | `--scope baseline --base HEAD` | worktree side: `D[1,1,0,0,0]`, introduced = credential (BUG-A) | none | yes | 0.09 |
| D05 | mini + committed `pub mod fresh;` + untracked `src/fresh.rs` (todo), untracked `notes.txt`, `*.log` in `.git/info/exclude` + `ignored.log` | `--scope files` | `f:[]`, `uu:1`, todo in fresh.rs not reported; human second line `1 untracked Rust file was compiled but not reported: pass --include-untracked to judge them.` | agent_loop `untracked_files_are_counted_then_included_on_request` | yes | 0.25 |
| D05b | D05 | `--scope files --include-untracked` | `f:["notes.txt","src/fresh.rs"]` (ignored file absent, non-Rust included), `uu:null`, todo at fresh.rs:2 | same | yes | 0.15 |
| D05c | D05 | `--scope lines --include-untracked` | same files, untracked judged whole: todo at fresh.rs:2 | same | yes | 0.10 |
| D05d | D05 | `--scope contextual --include-untracked` | same | none | yes | 0.11 |
| D05e | D05 | `--scope baseline --base HEAD` | exit 2, `E[baseline/baseline-scan-incomplete]` (base declares a module only the worktree has) | none | yes | 0.13 |
| D07 | mini + `gone.rs`/`before.rs` committed; branch `work` deletes `gone.rs` and its `mod` | `--scope files` | `f:["src/lib.rs"]` (deletion filtered by ACMR) | git_scope_oracle | yes | 0.11 |
| D07b | D07 | `--scope lines` | `f:["src/lib.rs"]` (deletion point), no finding | agent_loop `a_lockfile_deletion_alone_keeps_...` | yes | 0.10 |
| D07c | D07 | `--scope baseline` | `D[1,2,1,0,0]`: fixed = todo of gone.rs; introduced = credential (BUG-A; expected `D[0,2,1,0,0]`) | baseline_kernel | yes | 0.18 |
| D08 | D07 + commit `git mv before.rs after.rs` with `#[path]` | `--scope files --base <work~1 sha>` | `f:["src/after.rs","src/lib.rs"]` (`--no-renames` lists the new path) | git_scope_oracle | yes | 0.11 |
| D08b | D08 | `--scope lines --base <sha>` | `f:["src/lib.rs"]` only: a 100% rename has no hunk | none | yes | 0.09 |
| D08c | D08 | `--scope baseline --base <sha>` | `D[1,2,0,1,0]`: cross_file 1 for the moved todo, introduced = credential (BUG-A) | delta_correspondence `a_finding_carried_to_another_file_pairs_...` | yes | 0.17 |
| D09 | D08 + modify one line of after.rs | `--scope lines --base <work~2 sha>` | `f:["src/after.rs","src/lib.rs"]`, no todo (only the edited line) | none | yes | 0.10 |
| D09b | D09 | `--scope baseline --base <sha>` | `D[1,2,0,1,0]` | same | yes | 0.09 |
| D11 | mini + untracked README.md | `--scope files` | `f:[]`, `uu:0` (only `*.rs` counted) | none | yes | 0.11 |
| D13 | `chmod +x src/lib.rs` after a README commit | `--scope files --base <HEAD~1 sha>` | lib.rs listed (mode change is M) | none | yes | 0.09 |
| D13b | D13 | `--scope lines --base HEAD` | `f:[]` (no hunk) | none | yes | 0.09 |
| D14 | staged binary `blob.bin` | `--scope files --base HEAD` | `f:["blob.bin"]`; index rewritten (BUG-C) | none | yes | 0.09 |
| D14b | D14 | `--scope lines --base HEAD` | `f:[]` (binary has no hunk) | none | yes | 0.09 |
| D15 | staged `src/sp ace.rs`, `src/pct%41.rs`, `src/café.rs` via `#[path]` | `--scope files --base HEAD` | `f:["src/café.rs","src/lib.rs","src/pct%2541.rs","src/sp ace.rs"]` (`%` published as `%25`, UTF-8 raw), todo in all four | git_scope_kernel `files_scope_projects_diagnostics_through_the_canonical_path_representation` | yes | 0.14 |
| D15b | D15 | `--scope lines --base HEAD` | todo=3 (old_debt excluded) | none | yes | 0.08 |
| D15c | D15 | `--scope lines --staged` | same as D15b, `st:true` | none | yes | 0.14 |
| D15d | D15 | `--scope baseline --base HEAD` | `D[4,1,0,0,0]` introduced paths `src/café.rs`, `src/creds.rs` (BUG-A), `src/pct%2541.rs`, `src/sp ace.rs` | delta oracle | yes | 0.19 |
| D15e | D15 committed, `git mv pct%41.rs moved%42.rs` staged | `--scope baseline --base HEAD` | `D[1,4,0,1,0]`: the percent-named move pairs cross-file | none | yes | 0.20 |
| D16 | path with a control character or invalid UTF-8 | `--scope files --base HEAD` | `E[scope/git-path-invalid]` | git_scope/tests `all_output_and_path_boundaries_fail_atomically` | no | |
| D17 | committed `body.rs` (`let x; let y; todo!()`), delete `let y` | `--scope lines --base HEAD` | `f:["src/body.rs"]`, no finding (deletion point only) | git_scope/tests | yes | 0.10 |
| D17b | D17 | `--scope contextual --base HEAD` | no finding (todo is line reach, never widened) | git_scope/tests `a_contextual_scope_widens_only_...` | yes | 0.09 |
| D17c | D17 | `--scope files --base HEAD` | todo=1, unused_variables=1 | none | yes | 0.09 |
| D18 | branch `topic`: commit adds `src/c.rs` (todo) + mod; uncommitted NEWFN in `src/creds.rs` | `--scope files` | `f:["src/c.rs","src/creds.rs","src/lib.rs"]` committed range and dirty work together | none | yes | 0.11 |
| D18b | D18 | `--scope lines` | todo at `src/c.rs:2` and `src/creds.rs:6` | none | yes | 0.09 |
| D18c | D18 | `--scope baseline` | `D[3,1,0,0,0]` (2 todo + credential BUG-A) | none | yes | 0.09 |
| D19 | > 10 000 untracked files | `--scope files --include-untracked` | `E[scope/too-many-files]` | git_scope/tests boundaries | no | |
| F03 | staged `src/big.rs` of 40 000 lines (~1.9 MB patch) | `--scope lines --base HEAD` | exit 2, `E[scope/diff-too-large]` (`... use --scope files or baseline.`), L: `Scope: full codebase` (BUG-B) | agent_loop `a_diff_past_its_bound_fails_with_diff_too_large` | yes | 0.03 |
| F03b | F03 | `--scope contextual --base HEAD` | same | none | yes | 0.03 |
| F03c | F03 | `--scope files --base HEAD` | exit 0, `oversized_unit=1` | none | yes | 1.20 |
| F05 | conc, worktree rewrites the guard function to await under the lock | `--scope contextual` | `await_holding_lock=1` (item reach widened to the fn) | contextual_scope (staged only) | yes | 0.28 |
| F05b | F05 | `--scope lines` | `await_holding_lock=0` | same | yes | 0.16 |
| F05c | F05 | `--scope files` | whole file: 6 concurrency findings | none | yes | 0.18 |
| F05d | F05 | `--scope baseline --base HEAD` | `D[1,5,1,0,0]`: await_holding_lock introduced, one finding fixed | none | yes | 0.42 |
| H05 | nested; untracked `outside.rs` at repo root and `crates/app/src/stray.rs` | path `crates/app`, `--scope files` | `uu:1` (only inside the workspace) | none | yes | 0.14 |
| H05b | H05 | `--scope files --include-untracked` | `f:["src/stray.rs"]`, `orphan_module_file=1` | none | yes | 0.09 |
| H07 | `src/alias.rs -> impls/real.rs` committed; worktree adds todo to `real.rs` | `--scope files --base HEAD` | `f:["src/impls/real.rs"]`, no todo kept (BUG-G); expected todo at `src/alias.rs` | none | yes | 0.13 |
| H07b | H07 | `--scope lines --base HEAD` | same drop (BUG-G) | none | yes | 0.09 |
| H07c | H07 | `--scope baseline --base HEAD` | `D[2,1,0,0,0]` introduced `clippy::todo@src/alias.rs` + credential (BUG-A) | none | yes | 0.20 |

### S. Staged scans (index judged, private tree under `$TMPDIR`)

| ID | Setup | Args | Expected | Rust coverage | Ver. | Time |
|---|---|---|---|---|---|---|
| S01 | mini without creds (committed), staged `src/leak.rs` with AWS key + `git add -f .env` with a `ghp_` token | `--scope files --staged` | today: exit 0, `f:[".env","src/leak.rs","src/lib.rs"]`, zero repo findings (BUG-D); expected `hardcoded_credential=2`, `tracked_secret_file=1` as in S01d | none | yes | 0.11 |
| S01b | S01 | `--scope lines --staged` (the pre-commit hook) | today: exit 0, no finding at all (BUG-D) | none | yes | 0.11 |
| S01c | S01 | `--scope baseline --staged` | today: `D[0,1,0,0,0]` (BUG-D) | none | yes | 0.20 |
| S01d | S01 | `--scope files` | `hardcoded_credential=2`, `tracked_secret_file=1`, todo=1 | repo_hygiene (full only) | yes | 0.11 |
| S01e | S01 | `--scope lines` | `hardcoded_credential=2`, `tracked_secret_file=1` | none | yes | 0.09 |
| S01f | S01 | `--scope baseline --base HEAD` | `D[3,1,0,0,0]` | none | yes | 0.09 |
| S01g | S01 | (full) | same findings as S01d | repo_hygiene | yes | 0.08 |
| D11b | mini + staged README.md only | `--scope files --staged` | `f:["README.md"]`; human: `Scope: staged files (1 selected, ...)` then `No staged Rust change to judge.` | agent_loop `nothing_staged_passes_and_says_so` | yes | 0.12 |
| F01 | merge with conflict in `src/lib.rs` (3 unmerged entries) | `--scope lines --staged` | exit 2, `E[scope/index-conflicted]` | agent_loop `an_unusable_index_fails_the_staged_scan`; staged.rs unit | yes | 0.01 |
| F01b | F01 | `--scope files` | exit 1, incomplete (conflict markers do not compile), L: `Scope: changed files (1 selected, base HEAD at ...)` | none | yes | 0.14 |
| F01c | F01 | `--scope baseline --base HEAD` | exit 1, incomplete | none | yes | 0.21 |
| F02 | NEWFN added only to an alternate index | `GIT_INDEX_FILE=<alt>` + `--scope lines --staged` | judges the alternate index: todo at :12 | agent_loop (hook path) | yes | 0.13 |
| F02b | F02 | `--scope lines --staged` (no env) | `f:[]` (default index untouched) | none | yes | 0.12 |
| F02c | F02 | `GIT_INDEX_FILE=<missing>` + `--scope lines --staged` | `E[scope/index-unavailable]` | none | yes | 0.01 |
| F02d | F02 | `GIT_INDEX_FILE=<alt>` + `--scope lines` | env ignored by the worktree scope (removed from every other git call): judges worktree, todo at :12 | none | yes | 0.13 |
| F02e | F02 | `GIT_DIR=/nonexistent` + `--scope lines` | env removed, scan normal | none | yes | 0.09 |
| F02f | relative `GIT_INDEX_FILE=.git/index` with cwd = repo root (hook form) | `--scope lines --staged` | index resolved against cwd | agent_loop | no | |
| I01 | mini + touch; `: > .git/index.lock` | `--scope files --staged` | exit 2, `E[scope/index-unavailable]` | agent_loop | yes | 0.01 |
| I01b | I01 | `--scope files` | exit 0 (porcelain diff skips its index write) | none | yes | 0.10 |
| I01c | I01 | `--scope baseline --base HEAD` | exit 0 | none | yes | 0.10 |
| H01 | `TMPDIR=<repo>/tmpin` (inside the repo) | `--scope baseline --base HEAD` | exit 2, `E[baseline/baseline-temp-unavailable]` | baseline.rs unit | yes | 0.02 |
| H01b | H01 | `--scope lines --staged` | exit 2, `E[baseline/baseline-temp-unavailable]` (stage label says baseline on a staged scan) | none | yes | 0.02 |
| H01c | `TMPDIR=/nonexistent` | `--scope baseline --base HEAD` | `E[baseline/baseline-temp-unavailable]` | none | yes | 0.02 |
| H04 | committed `d.rs`, `r.rs`; staged `git rm d.rs` + `git mv r.rs r2.rs` | `--scope files --staged` | `f:["src/lib.rs","src/r2.rs"]` | none | yes | 0.13 |
| H04b | H04 | `--scope lines --staged` | `f:["src/lib.rs"]` | none | yes | 0.11 |
| H04c | H04 | `--scope baseline --staged` | `D[0,2,1,1,0]` (d.rs todo fixed, r.rs todo paired across the rename) | none | yes | 0.22 |

### E. Baseline comparison specifics

| ID | Setup | Args | Expected | Rust coverage | Ver. | Time |
|---|---|---|---|---|---|---|
| E01 | mini, `switch -c f`, no change | `--scope baseline` | today `D[1,1,0,0,0]` credential introduced (BUG-A); expected `D[0,2,0,0,0]` | baseline_kernel `baseline_runs_two_identical_sides_without_mutation_or_leak` (fixture has no repo finding) | yes | 0.24c |
| E01b | E01 | `--scope baseline --blocking warning` | today exit 1, `gate.status:"failed"`, `blocking_diagnostics:1`; expected exit 0 | none | yes | 0.10 |
| E02a | E01 target fresh | `--scope baseline --verbose` | human `Base side cache: miss`; writes `<target>/rust-doctor/cache/base-sides.json` and `<target>/rust-doctor/baseline/` | baseline_kernel `an_unchanged_base_is_judged_once_and_read_back_from_the_side_cache` | yes | 0.23c |
| E02b | E02a again | `--scope baseline --verbose` | `Base side cache: hit`, same delta, no snapshot created | same | yes | 0.10w |
| E02c | E02a | `RUST_DOCTOR_NO_CACHE=1` + `--verbose` | `Base side cache: disabled`; `base_side_cache` absent from JSON in all three (human lines miss/disabled/hit verified) | none | yes | 0.17 |
| E03 | branch replaces `todo!()` by `0` | `--scope baseline` | `D[1,0,1,0,0]`; human `Fixed: src/lib.rs:4:5 warning [clippy::todo] ...` (introduced 1 = BUG-A) | delta_correspondence | yes | 0.23 |
| E04 | branch deletes `src/creds.rs` | `--scope baseline` | today `D[0,1,0,0,0]`; expected fixed=1 for the credential (BUG-A mirror) | none | yes | 0.24 |
| E05 | branch adds NEWFN | `--scope baseline --blocking warning` | exit 1, `D[2,1,0,0,0]` | baseline_kernel `baseline_gate_counts_only_introduced_diagnostics` | yes | 0.24 |
| E05b | E05 | `--scope baseline` | exit 0 (todo is a warning, default blocking error) | same | yes | 0.18 |
| L02 | L01 | `--scope baseline --warning-exit-code 3` | exit 3 | warning_exit_code | yes | 0.10 |
| E06 | mini + submodule `vendor/sub` committed on main; branch adds NEWFN | `--scope baseline` | exit 2, `E[baseline/baseline-entry-invalid]`, L: `Scope: baseline comparison (base main at ...)` (BUG-E) | baseline.rs unit (gitlink rejected) | yes | 0.06 |
| E06b | E06 | `--scope files` | exit 0, `f:["src/lib.rs"]` | none | yes | 0.14 |
| E06c | E06 | `--scope files --staged` | exit 0 (gitlink skipped) | staged.rs unit | yes | 0.14 |
| E07 | committed `src/s.rs -> ../shared/s.rs`; branch adds NEWFN | `--scope baseline` | exit 2, `E[baseline/baseline-entry-invalid]` (BUG-F) | baseline_kernel `escaping_baseline_symlink_fails_before_baseline_metadata` (escaping case only) | yes | 0.05 |
| E07b | E07 | `--scope files` | exit 0 | none | yes | 0.14 |
| E07c | E07 | `--scope files --staged` | exit 2, `E[scope/staged-materialization-failed]` (BUG-F) | none | yes | 0.02 |
| E07d | committed `src/alias.rs -> lib.rs` | `--scope baseline` | exit 0, `D[2,1,0,0,0]` | baseline.rs unit | yes | 0.21 |
| E07e | committed `abs-link -> /etc/hostname` | `--scope baseline` | exit 2, `E[baseline/baseline-entry-invalid]` (correct refusal) | same | yes | 0.05 |
| E08 | base commit has only README; branch adds the workspace | `--scope baseline` | exit 2, `E[baseline/baseline-scan-incomplete]` | baseline_kernel `invalid_baseline_metadata_fails_closed_before_any_scan` | yes | 0.05 |
| E08b | E08 | `--scope files` | exit 0, all 5 new files listed | none | yes | 0.13 |
| E09 | base commit adds `pub fn broken( {`; branch repairs it | `--scope baseline` | exit 2, `E[baseline/baseline-scan-incomplete]` | baseline_kernel `incomplete_baseline_clippy_stops_before_the_current_clippy_scan` | yes | 0.14 |
| E09b | E09 | `--scope files` | exit 0 | none | yes | 0.13 |
| E10 | branch adds NEWFN | `--scope baseline --package mini` | `D[1,1,0,0,0]` correct (repo pass does not run under `--package`, so BUG-A is masked) | none | yes | 0.24 |
| E10b | E10 | `--scope files --package mini` | exit 0 | none | yes | 0.09 |
| E10c | E10 | `--scope files --package nope` | exit 2, `E[policy/unknown-package]`, L: `Scope: full codebase` | none | yes | 0.01 |
| H08 | committed `one.rs` (todo) moved to `two.rs` with > 50% rewrite (no rename detected) | `--scope baseline --base HEAD` | `D[1,2,0,1,0]` todo paired on evidence across files | delta_correspondence | yes | 0.23 |
| F03d | F03 (patch > 1 MiB) | `--scope baseline --base HEAD` | exit 0, `status:"complete"`, `errors:[{stage:"delta",code:"line-map-unavailable"}]` | delta_correspondence `a_diff_too_large_to_map_still_compares_and_says_so` | yes | 0.27 |
| F04 | mini (fast) | `--scope baseline --base HEAD --max-duration 1` | completes in 0.22 s (deadline unused) | none | yes | 0.22 |
| L03 | rd, fresh target, `RUST_DOCTOR_NO_CACHE=1` | `--scope baseline --max-duration 2` | exit 2 after 2.03 s, `E[baseline/baseline-scan-incomplete]` only (no deadline code), snapshot removed | scan_failures (deadline, full scope) | yes | 2.03 |
| E11 | concurrent: 4 parallel `--scope baseline` on one repo, one shared target, `RUST_DOCTOR_NO_CACHE=1` | `--scope baseline` | all exit 0, identical delta and diagnostic ids, repo unchanged, no snapshot left | baseline_kernel `concurrent_baseline_scans_are_isolated_and_deterministic` | yes | ~0.4 |
| E12 | promisor/partial clone | `--scope baseline` | fails closed without fetching | baseline_kernel `promisor_clone_fails_closed_...` | no | |
| E13 | unresolvable `rustup show active-toolchain` (e.g. `rust-toolchain.toml` naming a missing toolchain) | `--scope baseline` | `E[baseline/baseline-scan-incomplete]` before any scan | baseline_kernel `unresolved_active_toolchain_fails_closed_...` | no | |
| E14 | base > 100 000 entries or a blob > 64 MiB | `--scope baseline` | `E[baseline/baseline-limit-exceeded]` | baseline.rs unit | no | |

### G. Workspace placement

| ID | Setup | Path, args | Expected | Rust coverage | Ver. | Time |
|---|---|---|---|---|---|---|
| G01 | nested; branch edits `crates/app/src/lib.rs`, `README.md`, `top.rs` | `crates/app`, `--scope files` | `f:["src/lib.rs"]` workspace-relative, outside changes excluded | none | yes | 0.14 |
| G01b | G01 | `crates/app`, `--scope lines` | todo=1 | none | yes | 0.09 |
| G01c | G01 | `crates/app`, `--scope baseline` | `D[2,1,0,0,0]` (credential BUG-A + todo), full tree materialized | none | yes | 0.20 |
| G01d | G01 | `crates/app`, `--scope files --staged` | exit 0, `f:[]` | none | yes | 0.14 |
| G01e | G01 | repo root, `--scope files` | discovery walks down: stderr `No Cargo workspace at this path: scanning the one in crates/app/.`, same result as G01 | configuration_kernel | yes | 0.09 |
| G01f | G01 | `crates/app/src`, `--scope files` | discovery walks up to `crates/app` | none | yes | 0.09 |
| G04 | two workspaces `a/`, `b/` in one repo | repo root, `--scope files` | exit 2, `E[discovery/ambiguous-workspace]` | configuration_kernel | yes | 0.00 |
| G04b | G04 | `b`, `--scope files` | exit 0 | none | yes | 0.14 |
| G05 | independent repo `inner/` (mini) inside an outer repo, NEWFN unstaged | `inner`, `--scope files` | uses the inner repo: `cb=af441c9`, `ref:"HEAD"` | none | yes | 0.14 |
| G05b | G05 | `inner`, `--scope baseline --base HEAD` | `D[2,1,0,0,0]` | none | yes | 0.19 |
| G05c | G05 | outer root, full | `E[discovery/no-manifest]` (inner untracked by outer `git ls-files`) | none | yes | 0.00 |
| G06 | outer repo with mini as submodule `sub` (`sub/.git` is a file), NEWFN in sub | `sub`, `--scope files` | exit 0, `cb` = submodule HEAD | none | yes | 0.14 |
| G06b | G06 | `sub`, `--scope baseline --base HEAD` | exit 0, `D[2,1,0,0,0]` | none | yes | 0.19 |
| G06c | G06 | `sub`, `--scope lines --staged` | exit 0 | none | yes | 0.13 |
| G07 | `git worktree add ../wt -b wt`, NEWFN in the worktree | worktree path, `--scope files` | `ref:"main"` | none | yes | 0.14 |
| G07b | G07 | `--scope baseline` | `D[2,1,0,0,0]` | none | yes | 0.19 |
| G07c | G07 | `--scope files --staged` | exit 0 (worktree's own index) | none | yes | 0.14 |
| G08 | repo directory named `g08 dir` | `--scope lines`; `--scope baseline --base HEAD` | exit 0 both | none | yes | 0.14 / 0.19 |
| H06 | symlink `h06-link -> repo`, staged NEWFN | link path, `--scope files`; `--scope lines --staged`; `--scope baseline --base HEAD` | all exit 0, same results as the real path | none | yes | 0.14 / 0.13 / 0.19 |

### K. Integrity, cleanup and signals (cross-cutting; every scenario above also asserts the fingerprint)

| ID | Setup | Action | Expected | Rust coverage | Ver. | Time |
|---|---|---|---|---|---|---|
| N01 | every verified scenario except D04, D14, I-series | fingerprint before/after | identical (refs, objects, HEAD, config, worktree list, index bytes and mtime) | baseline_kernel `repository_state`, git_scope_kernel, product_proof | yes | |
| N02 | mini, `sleep 1.1; touch src/lib.rs` | `--scope files` | today `.git/index` sha256 changes (BUG-C); expected unchanged | none (fixtures never leave a stale stat) | yes | 0.10 |
| N03 | racily clean index (`git add` then rewrite the file) | each of files, lines, contextual, twice | today index mtime changes on each run (BUG-C); baseline, staged, full leave it untouched | none | yes | 0.1 each |
| N04 | private `TMPDIR` | every baseline and staged run | no `rust-doctor-baseline-*` left after exit 0, 1 or 2 | baseline_kernel `temporary_snapshots()` | yes | |
| N05 | default `TMPDIR` (`/tmp`) | `--scope baseline` and `--scope baseline --staged` | `/tmp/rust-doctor-*` count 0 before and after | none | yes | 0.2 |
| N06 | any scope | outputs | no absolute path in `--json` (assert no `$W` substring) | git_change_scope_product_proof `assert_private` | no | |
| K1 | rd, fresh target, `RUST_DOCTOR_NO_CACHE=1`, `--scope baseline` | SIGINT 0.3 s after the snapshot appears | exit 130, `status:"failed"`, `E[execution/interrupted]`, `scope.mode:"baseline"`, snapshot removed, repo unchanged, stop latency 0.10 s | scan_failures (full scope) | yes | 0.10 |
| K2 | same | SIGINT at 2.5 s (base side compiling) | same as K1, stop 0.02 s, no cargo/clippy orphan | same | yes | 0.02 |
| K3 | same | SIGTERM at 2.5 s | same as K2 | same | yes | 0.02 |
| K4 | same | SIGKILL at 2.5 s | exit 137, `rust-doctor-baseline-<pid>-0` left in `TMPDIR`, never swept by later runs | none | yes | 0.00 |
| K5 | rd, `--scope lines --staged` | SIGINT mid-run | exit 130, staged snapshot removed | none | no | |

### R. Real repository (`rd` clone), sizing and dogfood

| ID | Setup | Args | Expected | Ver. | Time cold / warm |
|---|---|---|---|---|---|
| R01 | rd on main | (full) | exit 0, `status: complete`, gate passed, `scope.mode: full`. Corrected 2026-10-20: US-012 rewrote the credential example, so the report holds no `hardcoded_credential`; the test asserts the contract, never a finding of this repository | yes | 5.46 / 0.45 |
| R02 | `switch -c e2e-noop` | `--scope files` | `ref:"origin/main"`, `f:[]`, no diagnostic. The test pins the clone's `origin/main` to the cloned commit, so the detected base is the same whatever branch the working copy is on | yes | 0.47 w |
| R02b | R02 | `--scope lines` | `f:[]` | yes | 0.46 w |
| R03 | R02 | `--scope baseline` | `introduced: 0`, `fixed: 0`, `pre_existing` equal to the full scan's `summary.total`, `base_ref: origin/main`. Corrected 2026-10-20: BUG-A is fixed (US-002) and the counts drift with every commit, so the test compares them with the full scan instead of writing them | yes | 6.51 (10.88 fully cold) / 0.49 (cache hit) |
| R03c | R02 | `RUST_DOCTOR_NO_CACHE=1 --scope baseline --verbose` | same `Delta:` line as the cached run, `Base side cache: disabled` | yes | 3.17 (warm target) |
| R04 | commit a `todo!()` fn in `src/git.rs` | `--scope files` | `f:["src/git.rs"]`, the seeded function carries exactly `clippy::todo` and `dead_code`, every diagnostic in `src/git.rs` | yes | 1.12 / 0.43 |
| R04b | R04 | `--scope lines` | same two findings | yes | 0.44 w |
| R04c | R04 | `--scope contextual` | same | yes | 0.44 w |
| R04d | R04 | `--scope baseline` | `introduced: 2` (the seeded `todo` and `dead_code`), `fixed: 0`. Corrected 2026-10-20: BUG-A is fixed (US-002), so the credential is no longer introduced | yes | 0.45 (cache hit) |
| R04e | R04 | `--scope baseline --base v0.7.0` | exit 0, `comparison_base` = the tag's commit, the seeded `todo` among the introduced, any error at stage `delta` (`line-map-unavailable` when a file moved too much). Corrected 2026-10-20: the counts move with every commit since the tag; needs a full-history checkout | yes | 2.01 |
| R04f | R04 | `--scope files --base v0.7.0` | `scope.files` equals `git diff --name-only --diff-filter=d v0.7.0 HEAD`, `src/git.rs` among them. Corrected 2026-10-20: the count (475 then) grows with every commit | yes | 0.44 |
| R05 | rd, staged edit (a `pub fn` with `todo!()` in `src/progress.rs`) | `--scope lines --staged` | exit 0, `scope.staged: true`, the staged `clippy::todo` reported at its line | yes | 5.39 / 2.64 |
| R05b | R05 | `--scope baseline --staged` | exit 0, `scope.staged: true`, the staged `clippy::todo` the only introduced finding | yes | 11.19 / 3.06 |

Target directory cost on rd: 180 MB (files), 229 MB (staged), 359 MB (baseline), 409 MB (baseline staged). A staged run never gets fully warm (2.6 s) because the snapshot moves every run and the workspace crate rebuilds.

## Timing summary for sizing

`mini` scans: 0.08-0.15 s warm, 0.12-0.25 s cold, baseline 0.22 s cold / 0.10 s warm, any scope failure 0.00-0.03 s. The verified catalog is about 230 scans on `mini`-sized repos (two per scenario, JSON + human), roughly 30-40 s sequential, a few seconds per test file when files run in parallel. The `rd` group costs about 25-35 s cold per fresh target and needs about 1.2 GB of target directories; gate it behind an opt-in variable and reuse one warm target per mode.

## Proposed e2e files

1. `scope-arguments.test.ts`: A, A2 (no cargo, under 1 s).
2. `scope-base-resolution.test.ts`: C, H02, H03, L01.
3. `scope-worktree-states.test.ts`: D, F03, F05, H05, H07.
4. `scope-staged.test.ts`: S, D11b, F01, F02, I01, H01, H04.
5. `scope-baseline-delta.test.ts`: E, L02, L03 (mini part), H08, F03d, every BUG-A expectation.
6. `scope-workspace-placement.test.ts`: G, H06.
7. `scope-repo-integrity.test.ts`: N, K, E11 (fingerprint, `TMPDIR` sweep, signals, concurrency).
8. `scope-real-repo.test.ts`: R, L03, K1-K4 (opt-in, `RUST_DOCTOR_E2E_REAL=1`).

Known-bug rows are written as the correct expectation and marked `test.failing` (bun) with the BUG id, so a fix flips them green.

## Git scenario builder the suite needs

```ts
const repo = await gitScenario("bug-a")          // mkdtemp under the suite root, git init -b main, deterministic config
  .template("mini")                              // or .fixture("tests/fixtures/...") / .nested("crates/app", "mini")
  .commit("base")                                // fixed identity and date: stable SHAs (mini base = af441c9...)
  .branch("feature")
  .append("src/lib.rs", NEW_FN).commit("feature")
  .stage({ write: { "src/leak.rs": leak(credential()) } })
  .unstaged({ "src/lib.rs": restoreFrom("HEAD") })
  .untracked({ "src/fresh.rs": TODO_FN })
  .symlink("src/alias.rs", "impls/real.rs")
  .submodule("vendor/sub", await gitScenario("sub").template("mini").commit("base").build())
  .clone({ depth: 1, branch: "feature", singleBranch: true })   // shallow, set-head, worktree, detach, orphan, criss-cross helpers
  .build();

const run = await rustDoctor(repo.path, ["--scope", "baseline"], {
  target: repo.target("baseline"),   // per scenario, or shared(templateKey) for warm reuse
  env: { GIT_INDEX_FILE: undefined },
});
run.exit; run.json.scope; run.delta(); run.introduced(); // [{code, path, line}]
run.scopeLine;   // from a second, human run, or parsed from --verbose
await expectUntouched(repo, run);   // fingerprint before/after, TMPDIR empty, no orphan process
```

Requirements, each one a trap met while building this catalog: spawn with an explicit environment (PATH, HOME, RUSTUP_HOME, CARGO_HOME only) plus `LC_ALL=C`, `GIT_CONFIG_NOSYSTEM=1`, `GIT_CONFIG_GLOBAL=/dev/null`, `CARGO_NET_OFFLINE=true`, never `CLAUDECODE`/`CODEX_SANDBOX`/`CURSOR_AGENT`; a private `TMPDIR` per test outside the repository (inside it, every baseline and staged run fails); a private `CARGO_TARGET_DIR` per test unless the test is about warmth (rd targets weigh 180-410 MB); build credential literals at runtime; drive git with hooks disabled and `protocol.file.allow=always`; the fingerprint must hash `.git/index` bytes and mtime without spawning git (a `git status` inside the check would refresh the index itself); a `racyIndex()` helper (`git add` then rewrite, or `touch` after a 1.1 s sleep) to expose BUG-C; signal helpers that wait for `rust-doctor-baseline-*` to appear in `TMPDIR` before sending.
