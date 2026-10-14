# react-doctor gap audit, 2026-10-13

What rust-doctor can take from react-doctor, engine to integrations, after a
full re-read of both codebases. Compared: react-doctor `d1feed112` (2026-10-12,
114 commits past the `83466a8fa` of the 2026-09-25 audit) against rust-doctor
`9b8c0ab` plus the uncommitted EP-006 worktree (US-028 to US-033, IN_REVIEW).

Every item below names its zone finding ids, value and effort, one react-doctor
citation and one rust-doctor citation. All 270 cited `path:line` references
resolve at those revisions, and every quote behind them was re-read at its exact
line (see the verification log). Items whose behavior was not reproduced sit
under "To measure before committing", not in the main list. Filters applied
throughout: nothing that reaches the network or emits telemetry, nothing
specific to React or JavaScript, no copied code.

"Prior row" points at the Parity Audit Disposition table of
`tasks/prd-react-doctor-parity.md`: a `Z*-*` row was a "Later" item this audit
re-confirmed, a `US-*` row is a shipped story that falls short of react-doctor.

## Start here

Small, confirmed, high value, in this order:

1. Item 1: detected credentials are printed verbatim in terminal and viewer code
   frames, which breaks the shipped FR-12 requirement.
2. Items 89 to 94: the agent and CI loop. Cursor's aborted turns are resubmitted,
   a default-branch push fails on legacy findings, a failed scan posts "No
   finding.", a shallow PR checkout fails without its remedy, a failing gate
   discards the Cargo cache, and a contributor without the binary cannot commit.
3. Items 47 and 49: `--scope lines`, the mode both hooks run, drops every
   span-less Cargo finding and new copies of existing code.
4. Items 2 to 4: one recall bug (prelude globs) and two false-positive classes
   (numeric SQL interpolation, orphans behind `cfg_attr(path)` whose help says
   to delete a compiled file).
5. Items 69, 81 and 82: no animation under hooks and `--yes`, the real terminal
   width, every scored rule listed by default.
6. Items 108 and 109: harden the variant gate and require line-exact citations
   in adjudication before the next corpus pass.

## Before certifying EP-006

Items 3, 5, 6, 7 and 108 are shortfalls of the uncommitted EP-006 stories
(US-029 blocking helpers, US-030 numeric parameters and builder escape hatches,
shell argv forms, US-033 variant gate). Fixing them before EP-006 is committed
avoids certifying recall gaps the audit has already measured.

## Decisions to challenge before building

- Item 51, count-based delta: removes false "introduced" findings after a rename
  or reformat inside a flagged line, but reverses `docs/subsystems/delta.md` and
  hides a same-file, same-rule swap (one fixed, one introduced).
- Item 52, saved report as baseline: the largest cost cut for `--scope baseline`,
  but it publishes fingerprints, the source revision and a policy digest, so the
  report schema grows for good. Item 39 (base-side cache) gets most of the speed
  with no schema change; measure it first.
- Item 29, opt-in tier: the honest home for the 18 noisy rules, but it moves the
  score calibration and every recorded corpus score.
- Item 66, published JSON schema: needs `schemars` (a new dependency) or a
  hand-kept schema under the doc gates.
- Item 98, inline review comments: needs `pull-requests: write` on the workflow;
  the runner posts, the binary stays offline.

## Engine: precision of shipped rules

1. **Mask detected credentials in terminal and viewer code frames** (Z3-N17; high, S). Privacy bug: the credential span is the literal, and the linear frame (src/render.rs:391) and the viewer print it unredacted; the FR-12 test reads only the JSON. Prior row US-014 (prd-suppression-dependency-hygiene, FR-12). RD `packages/core/src/runners/oxlint/parse-output.ts:178`, RS `src/repo_hygiene.rs:325`.
2. **Stop prelude globs from silencing every crate-rooted path** (Z2-N02; high, S). Recall bug: a glob visible in any scope, `std` included, turns every crate-rooted path Indeterminate before Unbound, so detectors abort. Corpus re-measure. RD `packages/oxlint-plugin-react-doctor/src/plugin/semantic/scope-analysis.ts:859`, RS `src/source_kernel/aliases.rs:118`.
3. **Carry declared parameter types to clear numeric SQL interpolation** (Z2-N04; high, S). FP class: a parameter resolves Indeterminate, so a `u64` in `format!` reads as SQL or shell injection. Targets US-030 (EP-006, IN_REVIEW). Prior row US-030. RD `packages/oxlint-plugin-react-doctor/src/plugin/utils/get-symbol-type-annotation.ts:6`, RS `src/source_kernel/bindings.rs:133`.
4. **Withhold orphan findings where the module walk lost declarations** (Z4-N03; high, S). FP with a destructive help: `#[cfg_attr(unix, path = ..)]` reads as no `path` (src/source_kernel/walk.rs:471) and `cfg_if! { mod x; }` is never walked, so compiled files are told to be deleted. Prior row US-013. RD `packages/core/src/project-analysis/utils/mark-complete-package-graphs.ts:225`, RS `src/structure/manifest.rs:272`.
5. **Follow same-module synchronous helpers called from async code** (Z2-N03; high, M). Recall gap of US-029 (EP-006): a blocking call one same-module sync helper away from `async fn` is missed. Prior row US-029. RD `packages/oxlint-plugin-react-doctor/src/plugin/utils/resolve-exact-local-function.ts:516`, RS `src/source_kernel/detectors/blocking.rs:11`.
6. **Cover the raw-SQL escape hatches of sqlx, diesel and sea-orm** (Z1a-N04; medium, S). US-030 (EP-006) judges the query call's first argument only; `QueryBuilder::push`, `diesel::dsl::sql` and `sea_orm::Statement::from_string` take raw fragments unseen. Prior row US-030. RD `packages/oxlint-plugin-react-doctor/src/plugin/rules/security-scan/raw-sql-injection-risk.ts:33`, RS `src/source_kernel/detectors/sql.rs:59`.
7. **Catch argv-array and Windows shell forms of dynamic shell commands** (Z1a-N05; medium, S). Only `Command::new(<sh literal>).arg("-c").arg(dynamic)` fires; `.args(["-c", x])`, `cmd /C`, `powershell -Command`, `/bin/sh` and a dynamic program are silent. RD `packages/oxlint-plugin-react-doctor/src/plugin/rules/security-scan/command-execution-input-risk.ts:14`, RS `src/source_kernel/detectors.rs:212`.
8. **Require a key body and veto placeholders in credential matching** (Z1a-N03; medium, S). A private-key header alone fires (src/repo_hygiene.rs:366); require a key body and veto placeholders. Prior row Z1-06. RD `packages/oxlint-plugin-react-doctor/src/plugin/rules/security-scan/key-lifecycle-risk.ts:20`, RS `src/repo_hygiene.rs:366`.
9. **Unify test-item classification into one structural reader** (Z2-N08; medium, S). Four readers disagree: an inline `#[cfg(all(test, feature = ..))] mod tests {}` weighs on the score while the out-of-line form does not. Prior row Z2-06. RD `packages/oxlint-plugin-react-doctor/src/plugin/utils/define-rule.ts:125`, RS `src/source_kernel/walk.rs:544`.
10. **Resolve crate-relative paths and re-exports across the module tree** (Z2-N05; medium, L). `use crate::..`/`super::..` stay Indeterminate: `pub use rusqlite::Connection;` re-imported through `crate::db` silences sql_method. Prior row Z2-04. RD `packages/oxlint-plugin-react-doctor/src/plugin/utils/resolve-cross-file-function-export.ts:44`, RS `src/source_kernel/aliases.rs:407`.
11. **Reparse expression-list macros so detectors see the calls inside** (Z2-N06; medium, L). No call inside `tokio::join!`, `vec![..]` or `tracing::info!` is visited; the one reparse, `format!`, misses `std::format!`. Prior row Z2-02. RD `packages/fuzz/corpus/README.md:32`, RS `src/source_kernel/detectors.rs:586`.
12. **Add per-body reachability and dominance for binding and guard questions** (Z2-N07; medium, L). Per-body reachability and dominance: prerequisite for guard-held-across-await and checked-before-use rules. Prior row Z2-03. RD `packages/oxlint-plugin-react-doctor/src/plugin/semantic/control-flow-graph.ts:42`, RS `src/source_kernel/bindings.rs:147`.

## Engine: new native rules

13. **Report reqwest bodies read without an HTTP status check** (Z1a-N01; high, M). reqwest resolves on 4xx/5xx; `.json()`/`.text()` without `error_for_status`/`status()` is RD's fetch rule transposed. RD `packages/oxlint-plugin-react-doctor/src/plugin/rules/correctness/no-fetch-response-used-without-status-check.ts:35`, RS `src/source_kernel/detectors.rs:140`.
14. **Report secret-named env vars that fall back to a literal** (Z1a-N02; high, M). `env::var("API_KEY").unwrap_or("sk-...")`: fails open in production. Name vocabulary needs corpus measurement. RD `packages/oxlint-plugin-react-doctor/src/plugin/rules/security-scan/secret-in-fallback.ts:30`, RS `src/policy/catalog.rs:557`.
15. **Add tokio rules for current-thread block_in_place and nested block_on** (Z1b-N02; high, M). `block_in_place` panics on a current_thread runtime (tokio docs, fetched). The nested `block_on` half is unconfirmed. Depends on the version gate. Prior row Z1-12. RD `packages/oxlint-plugin-react-doctor/src/plugin/rules/server/server-sequential-independent-await.ts:194`, RS `src/source_kernel/detectors/blocking.rs:84`.
16. **Give the kernel a caller-input source before path traversal and SSRF** (Z1a-N07; high, L). Prerequisite: a caller-input source in the kernel; today only constant vs non-constant exists, which makes path traversal and SSRF all noise. Prior row Z1-02. RD `packages/oxlint-plugin-react-doctor/src/plugin/rules/security-scan/command-execution-input-risk.ts:6`, RS `src/source_kernel/bindings.rs:10`.
17. **Add config-call detectors for CORS, auth cookies, JWT and MAC compare** (Z1a-N06; medium, M). Config-call detectors: permissive CORS, auth cookies without Secure/HttpOnly, JWT `insecure_disable_signature_validation`, non-constant-time MAC compare. Today the only config-call detector is disabled_tls_verification. Prior row Z1-02. RD `packages/oxlint-plugin-react-doctor/src/plugin/rules/security-scan/cors-cookie-trust-risk.ts:19`, RS `src/source_kernel/detectors.rs:324`.
18. **Report CI steps that compile the workspace with secrets in scope** (Z1a-N09, Z4-N10; medium, M). CI steps that build the workspace with registry or deploy secrets in env. Will fire on this repo's release.yml until it is split. Prior row Z4-05. RD `packages/oxlint-plugin-react-doctor/src/plugin/rules/security-scan/build-pipeline-secret-boundary.ts:24`, RS `AGENTS.md:32`.
19. **Flag library crates that enable tokio's full feature** (Z1a-N10; medium, S). A library enabling tokio `full` forces every feature on its dependents. Prior row Z1-14. RD `packages/oxlint-plugin-react-doctor/src/plugin/rules/bundle-size/no-full-lodash-import.ts:24`, RS `src/cargo_health.rs:620`.
20. **Report a binary's Cargo.lock that git ignores** (Z4-N13; medium, S). A binary's Cargo.lock that `.gitignore` excludes: `missing_lockfile` only checks the file exists. RD `packages/core/src/checks/expo/check-gitignore.ts:13`, RS `src/cargo_health/resolution.rs:105`.
21. **Judge dev and build dependencies for unused entries** (Z4-N06; medium, M). Unused-dependency judgment covers `[dependencies]` only. Prior row Z4-02. RD `packages/core/src/check-project-analysis.ts:371`, RS `src/cargo_health.rs:186`.
22. **Report patch and source replacement that bypass crates.io** (Z4-N08; medium, M). `[patch]`, `[replace]`, `source` replacement bypassing crates.io go unreported. Prior row Z4-03. RD `packages/core/src/check-pnpm-hardening.ts:159`, RS `src/cargo_health.rs:134`.
23. **Detect cloned blocks inside different functions** (Z4-N11; medium, L). Cloned blocks inside different functions (today only whole items). Precision risk, corpus calibration first. Prior row Z4-06. RD `packages/core/src/react-cleanup/detect-duplicate-jsx-subtrees.ts:246`, RS `src/structure/duplication.rs:61`.
24. **Treat tracked Cargo credentials files as secret files** (Z4-N14; low, S). Tracked `.cargo/credentials(.toml)` belongs on the secret-file list. Prior row Z4-04. RD `packages/oxlint-plugin-react-doctor/src/plugin/rules/security-scan/utils/is-repository-secret-file-path.ts:5`, RS `src/repo_hygiene.rs:221`.
25. **Report unused pub items of internal workspace crates** (Z4-N12; low, L). Unused `pub` items of internal workspace crates. Macro and trait-impl precision risk. Prior row Z4-08. RD `packages/core/src/project-analysis/report/exports.ts:28`, RS `src/source_kernel/aliases.rs:5`.
26. **Report regexes built from unescaped runtime text** (Z1a-N12; low, S). `Regex::new(&format!(..))` without `regex::escape`. RD `packages/oxlint-plugin-react-doctor/src/plugin/rules/correctness/no-unescaped-dynamic-string-in-regexp.ts:913`, RS `src/policy/coverage.rs:281`.
27. **Triage first the queued Clippy lints RD's families map to** (Z1a-N11; medium, S). Clippy queue: triage let_underscore_future, join_absolute_paths, regex_creation_in_loops, suspicious_open_options first (RD families map to them). Prior row Z1-07. RD `packages/oxlint-plugin-react-doctor/src/plugin/rules/correctness/no-floating-then-in-jsx-handler.ts:107`, RS `src/policy/coverage.rs:33`.

## Engine: catalog and policy model

28. **Gate crate packs on each member's resolved dependency version** (Z1b-N01; high, M). Gate crate packs on each member's resolved version (RD `requires`); reports NotEvaluated when unmet. Schema bump 20 to 21. Prior row Z1-12. RD `packages/core/src/project-info/capabilities.ts:87`, RS `src/source_kernel/detectors.rs:668`.
29. **Add an opt-in tier that category overrides cannot switch on** (Z1b-N07; high, M). An opt-in tier that a category override cannot switch on; today all 133 rules default to Warn and 18 noisy ones stay on discounted. Score calibration moves. Prior row Z1-10. RD `rule-retirements.md:3` under `docs/`, RS `src/policy/catalog/tests.rs:265`.
30. **Publish a title and a false-positive check with every rule** (Z1a-N08; high, M). Publish a title and a `not_a_defect_when` check per rule; the `catches` sentence exists only in tests/rule_evidence.json. Grows `rules list --json`. Prior row Z1-13. RD `packages/oxlint-plugin-react-doctor/src/plugin/utils/rule.ts:30`, RS `src/policy/catalog.rs:28`.
31. **Retire and rename rules as resolvable ids, not unknown ones** (Z1b-N06; medium, M). Retired and renamed ids resolve with a lifecycle state instead of `unknown-rule`. Schema bump. Prior row Z1-11. RD `packages/oxlint-plugin-react-doctor/src/plugin/utils/define-retired-rule.ts:11`, RS `src/configuration.rs:205`.
32. **Mark advisories on dev-only crates as test context** (Z4-N07; medium, M). Advisories on dev-only crates scored as test context. Prior row Z4-02. RD `packages/core/src/check-supply-chain.ts:165`, RS `src/cargo_health/advisories.rs:292`.
33. **Type stages and error codes as closed enums** (Z3-N12, Z11-N01; medium, M). Stages and error codes as closed enums; same strings on the wire, about 45 call sites. Prior row Z11-09. RD `packages/core/src/errors.ts:183`, RS `src/internal_error.rs:13`.
34. **Let closed detector lists take project-specific names** (Z3-N08; low, M). Rule options for closed detector lists (project blocking calls, SQL sinks). Schema bump. Prior row Z3-12. RD `packages/core/src/types/config.ts:356`, RS `src/configuration.rs:72`.
35. **Record native rule candidates and rejections like the Clippy queue** (Z1b-N09; medium, S). Record native rule candidates and rejections like the Clippy queue. RD `rule-candidates-backlog.md:8` under `docs/`, RS `src/policy/coverage.rs:221`.
36. **Write a detector authoring guide mapped to the kernel API** (Z2-N12; medium, S). A detector authoring guide mapped to the kernel API, under the doc gates. Prior row Z2-07. RD `HOW_TO_WRITE_A_RULE.md:199` under `docs/`, RS `.claude/skills/rule-admit/SKILL.md:26`.
37. **Let a detector declare several node kinds and required crates** (Z2-N10, Z2-N11; low, S). Detectors declare several node kinds and required crates; one test proves the hand-sized registries complete. Prior row Z2-05. RD `packages/oxlint-plugin-react-doctor/src/plugin/utils/rule-visitors.ts:10`, RS `src/source_kernel/detectors.rs:52`.

## Execution and performance

38. **Overlap Clippy with the native passes and the probes** (Z5-N01; high, M). `ExecutionContext::run` is strictly serial (src/execution.rs:440-530); run the native passes and probes under Clippy's wall time with `thread::scope`, stable sort after. Prior row Z5-04. RD `packages/core/src/run-inspect.ts:407`, RS `src/execution.rs:452`.
39. **Cache the baseline side's findings keyed by its tree** (Z5-N02; high, M). No result cache: every `--scope baseline` rescans the base. Key the base side on its tree and toolchain, store only complete scans. Prior row Z5-01. RD `packages/react-doctor/src/cli/utils/scan-result-cache.ts:456`, RS `src/baseline.rs:145`.
40. **Strip lint levels from every rustflags source Cargo reads** (Z5-N06, Z4-N09; medium, M). Any `[target.*]` rustflags returns empty (src/execution/rustflags.rs:138-160) and only the root `.cargo/config.toml` is read; follow Cargo's full source hierarchy. Prior row US-010. RD `packages/core/src/utils/build-oxlint-child-env.ts:22`, RS `src/execution/rustflags.rs:39`.
41. **Name members the deadline left unlinted and Cargo's lock wait** (Z5-N03; medium, S). The deadline branch (src/execution/clippy.rs:172-215) names no unlinted member and never reports Cargo's lock wait. Prior row US-008. RD `packages/core/src/runners/oxlint/spawn-batches.ts:235`, RS `src/execution/clippy.rs:181`.
42. **Kill Cargo's group and clean snapshots on SIGINT and SIGTERM** (Z5-N04, Z7-N12; medium, M). Kill Cargo's process group and clean snapshots on SIGINT/SIGTERM. `libc` is already in Cargo.lock, so no new crate enters the build. Prior row US-008. RD `packages/react-doctor/src/cli/index.ts:27`, RS `src/execution/process.rs:87`.
43. **Time each phase and gate a whole-binary benchmark** (Z5-N05; medium, M). Phase timings and a whole-binary benchmark gate. Schema bump if timings enter `--json`. Prior row Z5-08, Z11-10. RD `scripts/performance/run-performance.ts:273`, RS `src/structure/benchmark.rs:42`.
44. **Bound probes, cargo metadata and git by the run deadline** (Z5-N07; low, S). The three version probes always get a fresh 10 s, and `cargo metadata` and every git call run unbounded: `--max-duration 5` can take 30 s of probes alone. Prior row US-008. RD `packages/core/src/runners/oxlint/spawn-batches.ts:208`, RS `src/execution.rs:713`.
45. **Bound detector cost by counted work, not by wall clock** (Z2-N14; low, S). Bound detector cost by counted work, deterministic across machines. Prior row Z5-08. RD `packages/oxlint-plugin-react-doctor/src/plugin/rules/design/no-tiny-text.performance.test.ts:50`, RS `src/source_kernel/tests/benchmark.rs:30`.
46. **Run the Clippy child below normal CPU priority** (Z5-N09; low, S). Run the Clippy child at lower CPU priority; watch the 600 s hook timeout. RD `packages/core/src/runners/oxlint/spawn-oxlint.ts:84`, RS `src/execution/process.rs:15`.

## Scope, baseline and delta

47. **Keep span-less manifest findings in a lines scope** (Z6-N01, Z1b-N08, Z4-N02; high, M). `(Some(_), None) => false` (src/git_scope.rs:404-428) drops every span-less Cargo finding under `--scope lines`, the mode both hooks use. Anchor manifest findings at their key. Prior row US-015. RD `packages/react-doctor/src/cli/utils/build-project-scan-plan.ts:58`, RS `src/git_scope.rs:424`.
48. **Add a context-aware lines scope for remotely reported lints** (Z6-N03, Z2-N01; high, M). Contextual lines scope (RD #1965): `await_holding_lock` sits on the guard binding, so an added `.await` under it is dropped by every lines-scoped run. RD `packages/core/src/utils/create-changed-line-matcher.ts:36`, RS `src/git_scope/lines.rs:8`.
49. **Keep a clone family in scope when any member changed** (Z4-N01; high, S). A clone family is filtered on its first member only: a new copy of old code vanishes under `--scope lines` when the old copy sorts first. Prior row US-015. RD `packages/core/src/services/maintainability.ts:82`, RS `src/structure.rs:588`.
50. **Discover nested Cargo workspaces when no manifest is found** (Z4-N04, Z3-N15; high, M). No Cargo.toml above the path is a dead end; descend into the single nested workspace (Tauri `src-tauri/`, `backend/`) or list candidates. Highest-traffic case for `npx rust-doctor`. Prior row Z4-07. RD `packages/core/src/project-info/discover-react-subprojects.ts:60`, RS `src/scan_target.rs:89`.
51. **Gate the delta on per-file, per-rule count increases** (Z6-N02; high, M). STRATEGIC: RD's delta compares counts per (file, rule); RS pairs on evidence hashes, so renaming a variable inside a flagged line reports introduced plus fixed. Reverses docs/subsystems/delta.md; a same-file swap becomes invisible. RD `packages/core/src/compute-diagnostic-delta.ts:96`, RS `src/delta.rs:350`.
52. **Let a saved report replace the baseline compilation** (Z3-N10, Z6-N07; high, L). `--baseline <report.json>` (RD 8ed70df0f): needs published fingerprints, source revision and policy digest. Schema bump, size L. RD `packages/react-doctor/src/cli/index.ts:225`, RS `src/lib.rs:219`.
53. **Degrade a failed base side to a files-scoped report** (Z6-N12; medium, M). A failed base side never scans the current side (src/execution/baseline.rs:1-12); degrade to a files-scoped report. RD `packages/react-doctor/src/inspect.ts:403`, RS `src/execution/baseline.rs:6`.
54. **Pick the introduced duplicate by Git-mapped line distance** (Z6-N08, Z6-N09; medium, S). Choose the introduced duplicate by Git-mapped line distance; allow cross-file matches only on Git renames. RD `packages/core/src/compute-diagnostic-delta.ts:138`, RS `src/delta.rs:519`.
55. **Judge a staged scan under the index's own configuration** (Z6-N04, Z6-N05; medium, S). `--staged` compiles policy from the worktree rust-doctor.toml and .gitattributes, so an unstaged allow silences a commit that lacks it; it also fails on a repository with no commit. Prior row US-016. RD `packages/react-doctor/src/cli/commands/inspect.ts:379`, RS `src/lib.rs:145`.
56. **Stop the manifest ancestor walk at the git root** (Z4-N05; low, S). Stop the manifest ancestor walk at the git root (document the divergence from Cargo). RD `packages/core/src/project-info/find-nearest-ancestor-package-json.ts:25`, RS `src/scan_target.rs:89`.
57. **Accept an explicit changed-file list for the files scope** (Z6-N11; low, S). Accept an explicit changed-file list for the files scope. Schema bump for provenance. Prior row Z6-06. RD `packages/react-doctor/src/cli/index.ts:244`, RS `src/git_scope.rs:486`.

## Suppression, configuration and gates

58. **Exempt context-marked findings from the baseline gate** (Z3-N01; medium, S). The full gate uses `weighs`, the baseline gate only `unscored` (src/report/assembly.rs:698-731): a test-context finding blocks a baseline run it would not block in full. Prior row US-004. RD `packages/react-doctor/src/cli/utils/resolve-scan-exit-code.ts:18`, RS `src/report/assembly.rs:702`.
59. **Anchor native directives to the next statement and stacked chains** (Z3-N02; medium, M). A directive covers only its line and the next (src/report/exclusion.rs:132-141); anchor on the next statement and on stacked method chains. Prior row US-021. RD `packages/core/src/find-stacked-disable-comments.ts:29`, RS `src/report/exclusion.rs:139`.
60. **Report malformed directives and hint near misses on findings** (Z3-N03; medium, S). Malformed directives are silently inert; report them and hint near misses on the finding. Schema bump. RD `packages/core/src/constants.ts:703`, RS `src/directive.rs:169`.
61. **Add per-surface score, gate and comment exclusions** (Z3-N05; medium, M). Per-surface (score, gate, comment) exclusions. Schema bump. Prior row Z3-04. RD `packages/core/src/types/config.ts:88`, RS `src/report.rs:680`.
62. **Discover configuration in Cargo metadata and above the workspace** (Z3-N07; medium, M). Read config from `[workspace.metadata.rust-doctor]` and above the workspace. Schema bump (policy.config_file). Prior row Z3-11. RD `packages/core/src/load-config.ts:22`, RS `src/configuration.rs:17`.
63. **Accept short rule ids and name the nearest catalogued id** (Z3-N06; medium, S). Accept short rule ids and name the nearest catalogued id on a typo. Prior row Z3-10. RD `packages/core/src/rule-key-aliases.ts:8`, RS `src/configuration.rs:204`.
64. **Derive every path-based context from one classifier** (Z3-N13; low, S). Two path classifiers (src/repo_hygiene.rs:244, src/report.rs:659) can disagree; derive every context from one. RD `packages/core/src/is-test-file.ts:8`, RS `src/report.rs:665`.
65. **Emit a rust-doctor.toml schema for editor completion** (Z3-N16; low, S). Emit a rust-doctor.toml schema for taplo and RustRover completion. RD `packages/core/src/utils/define-config.ts:5`, RS `src/configuration.rs:33`.

## Report and schema

66. **Publish the report schema and decode saved reports typed** (Z3-N11; medium, M). Publish the report JSON schema and decode saved reports typed. New dependency (schemars) or a hand-kept schema: decide first. Prior row Z3-09. RD `packages/core/src/schemas.ts:255`, RS `src/report.rs:200`.
67. **Write one file per rule under --output-dir** (Z8-N14, Z7-N13; low, M). One file per rule under `--output-dir`, which the handoff prompt can point to; stays under the target dir unless an explicit directory is given. Prior row Z8-05. RD `packages/react-doctor/src/cli/utils/write-diagnostics-directory.ts:36`, RS `src/main.rs:240`.
68. **Separate and document the library surface from binary support** (Z7-N14, Z9-N16; low, M). docs.rs advertises a library surface nobody stabilized; separate and document it, `#[non_exhaustive]` where it matters. Prior row Z9-06. RD `packages/react-doctor/package.json:51`, RS `src/lib.rs:23`.

## CLI

69. **Stop animating and rewriting for hooks, agents and --yes runs** (Z8-N02; high, S). Animation depends on TTY, NO_COLOR, TERM and CI only (src/main.rs:745-778), never on `--yes` or the agent markers. Prior row US-012. RD `packages/react-doctor/src/cli/utils/onboarding-pacing.ts:46`, RS `src/main.rs:775`.
70. **Add an opt-in exit code for non-blocking warnings** (Z7-N01, Z3-N09; medium, S). Opt-in exit code for a passing run with warnings (RD #1900). RD `packages/react-doctor/src/cli/utils/resolve-scan-exit-code.ts:21`, RS `src/report.rs:850`.
71. **Print the handoff prompt when launch or clipboard fails** (Z7-N03, Z8-N04; medium, S). Print the handoff prompt when the agent launch or clipboard fails. Prior row Z7-08. RD `packages/react-doctor/src/cli/utils/handoff-to-agent.ts:308`, RS `src/handoff.rs:606`.
72. **Add `rules explain <id>` with the effective level and its source** (Z7-N06; medium, S). `rules explain <id>` with the effective level and where it came from. Prior row Z7-05. RD `packages/react-doctor/src/cli/index.ts:465`, RS `src/main.rs:220`.
73. **Add `why <path:line>` built on the report's suppressions** (Z7-N05; medium, M). `why <path:line>` built on the report's suppressions. Prior row Z7-06. RD `packages/react-doctor/src/cli/index.ts:322`, RS `src/report.rs:223`.
74. **Tell an agent when its skill is missing or stale** (Z7-N09; medium, S). Tell an agent on stderr when its installed skill is missing or stale. RD `packages/react-doctor/src/cli/utils/prompt-install-setup.ts:70`, RS `src/skill.rs:161`.
75. **Freeze the command-line surface in a ledger test** (Z11-N06; medium, S). Freeze the CLI surface in a ledger test. Prior row Z11-05. RD `AGENTS.md:18`, RS `tests/skill_contract.rs:74`.
76. **Resolve one tested terminal profile instead of scattered gates** (Z8-N17; medium, M). Resolve one tested TerminalProfile instead of scattered TTY/color/width gates. RD `packages/react-doctor/src/cli/utils/should-use-tui.ts:9`, RS `src/main.rs:717`.
77. **Widen the non-interactive markers to more agents and CI providers** (Z7-N15; low, S). Widen the non-interactive markers to more agents and CI providers. Prior row US-013. RD `packages/react-doctor/src/cli/utils/is-ci-environment.ts:68`, RS `src/main.rs:693`.
78. **Add --color and honor FORCE_COLOR, CLICOLOR_FORCE and empty NO_COLOR** (Z8-N10; low, S). `--color`, FORCE_COLOR, CLICOLOR_FORCE, empty NO_COLOR. Prior row Z8-03. RD `packages/react-doctor/src/cli/utils/apply-color-preference.ts:25`, RS `src/main.rs:763`.
79. **Edit rule levels from the CLI without rewriting user files** (Z7-N07; low, S). Edit rule levels from the CLI, writing only files rust-doctor created. Prior row Z8-11. RD `packages/react-doctor/src/cli/index.ts:475`, RS `src/configuration.rs:17`.
80. **Add `--score` that prints only the number** (Z7-N08, Z8-N16; low, S). `--score` prints only the number; group `rules list` by category on a terminal. Prior row Z7-09. RD `packages/react-doctor/src/cli/index.ts:203`, RS `src/audit.rs:152`.

## Rendering

81. **Measure the live terminal width in the linear report** (Z8-N01; high, S). Width comes from COLUMNS floored at 80 (src/main.rs:559); measure the live terminal. Prior row Z8-04. RD `packages/react-doctor/src/cli/utils/resolve-measure-width.ts:18`, RS `src/main.rs:559`.
82. **List every scored rule in the default linear report** (Z8-N03; high, S). Without `--verbose` only `scored.first()` prints (src/render.rs:204-216): a commit blocked by three rules shows one. Prior row Z8-02. RD `packages/react-doctor/src/cli/utils/print-headless-report.ts:64`, RS `src/render.rs:212`.
83. **Label test, bench, example and build-script sites in both reports** (Z8-N05; medium, S). Label test, bench, example and build-script sites in both reports. Prior row Z8-08. RD `packages/react-doctor/src/cli/utils/print-headless-report.ts:28`, RS `src/presentation.rs:69`.
84. **Show findings arriving on the progress line during the scan** (Z8-N06; medium, M). Show findings arriving on the progress line. `Progress` gains a variant (semver-visible). RD `packages/react-doctor/src/cli/ink/scan-bridge-layers.ts:16`, RS `src/progress.rs:14`.
85. **Repaint the interactive report when the terminal is resized** (Z8-N08; medium, M). Repaint the TUI on resize (polling, no SIGWINCH dependency). Prior row Z8-09. RD `packages/react-doctor/src/cli/ink/hooks/use-stdout-dimensions.ts:22`, RS `src/tui.rs:194`.
86. **Say why a report is empty or unscored, in both reports** (Z8-N13, Z8-N15; low, S). Say why a report is empty or unscored; print a bounded triage preamble when an agent reads it. RD `packages/react-doctor/src/cli/utils/build-empty-report-message.ts:15`, RS `src/tui/screens/menu.rs:151`.
87. **Detect OSC 8 support instead of tying links to color** (Z8-N11; low, S). Detect OSC 8 support instead of tying links to color. Prior row Z8-10. RD `packages/react-doctor/src/cli/utils/supports-hyperlinks.ts:34`, RS `src/tui/canvas.rs:27`.
88. **Highlight Rust tokens and mark multi-line spans in code frames** (Z8-N12; low, M). Rust token highlighting and multi-line span marks in code frames; bidi controls stay escaped. Prior row Z8-07. RD `packages/react-doctor/src/cli/utils/render-code-frame.ts:53`, RS `src/render.rs:463`.

## Agent hooks, GitHub Action and CI integration

89. **Honor Cursor's abort status and loop count in the stop hook** (Z7-N02; high, S). `hook run cursor` reads only Claude's `stop_hook_active` (src/hook/agent.rs:192): an aborted turn is resubmitted up to Cursor's 5 loops (Cursor docs, fetched). Prior row US-019. RD `packages/react-doctor/src/cli/utils/install-agent-hooks.ts:262`, RS `src/hook/agent.rs:192`.
90. **Stop the Action failing default-branch pushes on legacy findings** (Z9-N01; high, S). A push runs `--scope full` at the default error gate and the Action exits with that code (action.yml:95-110, 182-187): legacy findings fail main. Release-note it. Prior row US-026. RD `action.yml:721`, RS `action.yml:100`.
91. **Render the report's errors in the summary and comment** (Z9-N02; high, S). `render_markdown` never reads `errors[]`: a failed scan posts 'No finding.' and overwrites the last sticky comment. Prior row US-025. RD `scripts/render-github-action-comment.mjs:431`, RS `src/render/markdown.rs:94`.
92. **Make the Action survive or explain a shallow pull request checkout** (Z9-N03; high, S). A depth-1 PR checkout fails as `base-unavailable`; the fetch-depth remedy fires only for `merge-base-unavailable` (src/git_scope.rs:640). Prior row US-026. RD `action.yml:155`, RS `action.yml:103`.
93. **Persist Cargo's build in the Action, even when the gate fails** (Z5-N08, Z9-N08; high, S). rust-cache without `cache-on-failure` (src/ci.rs:371) and no cache in action.yml: a failing gate discards the build. Prior row US-026. RD `action.yml:105`, RS `action.yml:68`.
94. **Make the installed hook survive a missing binary and upgrades** (Z6-N06, Z7-N04, Z11-N05, Z7-N11; medium, S). The git hook does `exec rust-doctor` with no PATH guard (src/hook.rs:52-55): every commit fails 127 for a contributor without the binary. Add a refresh on reinstall. Prior row US-017. RD `packages/react-doctor/src/cli/utils/install-git-hook-file.ts:38`, RS `src/hook.rs:55`.
95. **Add comment-on-clean so clean pull requests stay quiet** (Z9-N04; medium, S). `comment-on-clean` so clean PRs stay quiet (RD #1386). Prior row US-027. RD `action.yml:23`, RS `src/ci.rs:435`.
96. **Write ci install's workflow at the git root, scanning the workspace** (Z9-N05; medium, S). `ci install` run from `src-tauri/` or `backend/` writes a nested `.github/workflows/` that GitHub ignores, with no warning. Prior row US-024. RD `packages/react-doctor/src/cli/utils/install-github-workflow.ts:150`, RS `src/ci.rs:51`.
97. **Pin the generated workflow through one Action ref bots can bump** (Z9-N06, Z9-N07; medium, M). Pin the generated workflow through one bumpable Action ref; pin third-party actions by SHA. Prior row US-024. RD `packages/react-doctor/src/cli/utils/install-github-workflow.ts:65`, RS `src/ci.rs:376`.
98. **Anchor introduced findings as inline review comments on changed lines** (Z9-N15; medium, M). Inline review comments on changed lines; needs `pull-requests: write`, the runner posts, the binary stays offline. Prior row US-027. RD `action.yml:27`, RS `action.yml:159`.
99. **Drop persisted credentials from every checkout that runs code** (Z11-N08; low, S). `persist-credentials: false` on every checkout that runs code. Prior row Z11-11. RD `.github/workflows/ci.yml:55`, RS `.github/workflows/dogfood.yml:47`.

## Distribution

100. **Detect installed agents and set everything up in one command** (Z7-N10; medium, M). `setup`: detect installed agents and install skill and hooks in one command. RD `packages/react-doctor/src/cli/index.ts:338`, RS `src/main.rs:206`.
101. **Smoke-test the packed launcher through npm, the path users take** (Z9-N09; medium, S). Smoke the packed launcher through npm, the path users take, not only bun. RD `scripts/smoke-packed-cli-install.ts:162`, RS `npm/rust-doctor/scripts/smoke-packed.mjs:276`.
102. **Ship musl and Windows ARM binaries with libc-aware native packages** (Z9-N10; medium, M). musl and Windows ARM binaries with libc-aware npm packages; three new package names, first publish by token. Prior row Z9-07. RD `pnpm-lock.yaml:1705`, RS `.github/workflows/release.yml:61`.
103. **Ship a read-only audit-then-plan reference that writes executable plans** (Z9-N12, Z9-N13; medium, M). Skill references: a read-only audit-then-plan reference writing under plans/, and `references/explain.md` backed by report evidence. Prior row Z9-05. RD `skills/improve-react/SKILL.md:25`, RS `skills/rust-doctor/SKILL.md:99`.
104. **Hold references/expert-review.md to the catalog it supplements** (Z9-N14; medium, S). `references/expert-review.md` covers 62 rules against 133 in the catalog; gate it. Prior row US-018. RD `skills/improve-react/AUDIT.md:22`, RS `skills/rust-doctor/references/expert-review.md:3`.
105. **Publish a dev dist-tag snapshot from every main push** (Z9-N11; low, M). A `dev` dist-tag snapshot from every main push. Prior row Z9-08. RD `.github/workflows/publish.yml:103`, RS `.github/workflows/release.yml:7`.
106. **Verify the crate tarball on every push** (Z11-N09, Z11-N07; low, S). `cargo package` on every push; the release skill stops before the first publishing push and loses its stale 'Publishing' pointer. Prior row Z11-07. RD `.github/workflows/ci.yml:127`, RS `.github/workflows/release.yml:374`.

## Validation and calibration

107. **Pin each adjudicated native false positive as a replayed seed** (Z10-N01, Z2-N09; high, M). 147 native false-positive verdicts in tests/corpus.json, none pinned as a replayed seed; add an in-memory harness and a seed directory. Prior row Z10-04. RD `packages/fuzz/corpus/README.md:9`, RS `tests/corpus.json:3341`.
108. **Add semantic variant cases with carve-out reasons to variant tables** (Z10-N06, Z2-N15; high, S). Variant gate (US-033, EP-006): add the gap-exposing rewrites and semantic cases with carve-out reasons. Prior row US-033. RD `scripts/fn-mining/README.md:4`, RS `src/source_kernel/detectors.rs:196`.
109. **Require line-exact source citations in every adjudication pass** (Z10-N04; high, S). Adjudication passes carry no line-exact citation; the only check is a non-empty justification. RD `packages/evals/src/utils/validate-sol-citations.ts:18`, RS `.claude/skills/corpus-adjudicate/SKILL.md:32`.
110. **Publish a site-level corpus diff between base and head** (Z10-N02; high, M). On a corpus mismatch the job uploads counts only; publish a site-level base/head diff. Prior row Z10-03. RD `.agents/skills/run-parity/scripts/compare-parity.mjs:182`, RS `tests/corpus_precision.rs:1727`.
111. **Audit what agent commits introduce, through the Stop hook's scope** (Z10-N08; high, L). The agent population is 8 repos scanned whole; audit what the Stop hook actually hands an agent (`--scope lines` on agent commits). Prior row Z10-07. RD `.agents/skills/benchmark-fp-fn-audit/SKILL.md:35`, RS `src/hook/agent.rs:44`.
112. **Calibrate agent judges against human labels written beforehand** (Z10-N05, Z10-N10; medium, M). Provenance: agent 369, unrecorded 98, human 0; calibrate judges on human labels written first, and challenge each proposed FP against the rule's contract. RD `packages/evals/src/audit-classification.ts:11`, RS `.claude/skills/corpus-adjudicate/SKILL.md:245`.
113. **Sample silent applicable units to mine false negatives** (Z10-N03; medium, M). Mine false negatives by sampling silent applicable units. Prior row Z10-02. RD `packages/evals/src/prepare-classification.ts:183`, RS `docs/suppression-precision-2026-08.md:224`.
114. **Fuzz native producers with seeded mutations and crash, slow oracles** (Z10-N07; medium, M). Fuzz native producers with seeded mutations and crash/slow oracles (inline PRNG, test-only catch_unwind). Prior row Z10-05. RD `packages/fuzz/src/fuzz-rule.ts:345`, RS `src/source_kernel/detectors.rs:257`.
115. **Grow a native-only breadth corpus with a measured exclusion list** (Z10-N09; medium, L). A native-only breadth corpus with a measured exclusion list. Prior row Z10-01. RD `packages/evals/README.md:12`, RS `docs/catalog-and-corpus.md:24`.
116. **Make the Windows leg a green cargo test with reasoned ignores** (Z11-N03; medium, M). Make the Windows leg a green `cargo test` with reasoned ignores. Prior row Z11-01. RD `.github/workflows/ci.yml:48`, RS `.github/workflows/ci.yml:181`.
117. **Fail catalog admission of lints newer than the Clippy floor** (Z11-N11; low, S). Fail catalog admission of lints newer than the Clippy 1.95 floor. Prior row Z11-04. RD `.github/workflows/ci.yml:46`, RS `.github/workflows/ci.yml:136`.

## Contributor process

118. **Split two files at the bound, reformat once, then gate rustfmt** (Z11-N02; medium, M). Measured: rustfmt 1.97.1 turns src/main.rs (999 lines) into about 1024, past the bound. Split first, reformat once, then gate rustfmt. Prior row Z11-02. RD `.github/workflows/ci.yml:80`, RS `docs/ci-workflows.md:10`.
119. **Install rust-doctor's own Stop hook on this repository** (Z11-N04; medium, S). Dogfood: install rust-doctor's own Stop hook here (it runs the released binary on PATH). Prior row Z11-03. RD `package.json:14`, RS `src/hook/agent.rs:44`.

## To measure before committing

120. **Materialize snapshots without running checkout filters** (Z6-N10; high, S). Snapshots may run checkout filters (smudge, LFS) of the scanned repo; not reproduced. Prior row Z6-10. RD `packages/react-doctor/src/cli/utils/is-materializable-git-source.ts:1`, RS `src/baseline.rs:239`.
121. **Ship audit mode as a single --force-warn pass** (Z3-N04; medium, M). Audit mode as one `--force-warn` pass; behavior on Clippy tool lints unmeasured. Prior row Z3-07. RD `packages/react-doctor/src/cli/index.ts:280`, RS `src/policy.rs:53`.
122. **Exclude findings in build-script output under the target directory** (Z3-N14; medium, S). Findings in build-script output under the target dir; whether any reach the report is unmeasured. Prior row US-022. RD `packages/core/src/project-info/constants.ts:64`, RS `src/generated.rs:8`.
123. **Read keys during the score reveal instead of echoing them** (Z8-N07, Z8-N09; medium, M). Keys echoed during the score reveal; Windows VT and ASCII fallback (no Windows run). RD `packages/react-doctor/src/cli/ink/components/report-landing.tsx:81`, RS `src/tui.rs:188`.
124. **Add an HTTP pack for reqwest clients and axum routes** (Z1b-N03, Z1b-N04, Z1b-N05, Z1b-N10; medium, L). HTTP and tracing packs, stub crates for needs-crate lints, bevy/ratatui packs: trigger evidence not yet produced. Prior row Z1-12. RD `packages/oxlint-plugin-react-doctor/src/plugin/rules/tanstack-query/query-stable-query-client.ts:46`, RS `src/source_kernel/detectors.rs:196`.
125. **Emit a Cargo-JSON stream for rust-analyzer's check.overrideCommand** (Z2-N13; medium, M). Cargo-JSON stream for rust-analyzer `check.overrideCommand`: whether r-a needs the absolute paths is unknown. Prior row Z9-03. RD `packages/eslint-plugin-react-doctor/src/index.ts:88`, RS `src/execution/messages.rs:1`.
126. **Add a Cursor plugin manifest over the existing skills directory** (Z9-N17, Z11-N10; low, S). Cursor plugin manifest; `.agents/skills` for Codex through a symlink. RD `.cursor-plugin/plugin.json:3`, RS `docs/agent-skill.md:10`.

## Verification log

Both repositories were cut into 12 zones (4,552 react-doctor files, 267
rust-doctor files), one read-only agent per zone, each returning findings with
exact `{repo, path, line, quote}` citations and absence checks written as ERE
greps. Every report was re-verified mechanically before use: each quote matched
at its stated line, each absence grep replayed with its stated hit count, and
coverage diffed against the zone manifest. "Hits as stated" means the grep
returned exactly the hits its finding declared. The one path error is Z9-N17's
check of plugin directories that do not exist, which is the absence it
asserts; that item sits under "To measure".

| Zone | Scope | Citations exact | Absence: zero hit | Absence: hits as stated | Absence: path error |
|---|---|---|---|---|---|
| Z1a | Security and correctness rule breadth | 81/81 | 13 | 1 | 0 |
| Z1b | Framework packs, Clippy admission, rule lifecycle | 116/116 | 12 | 0 | 0 |
| Z2 | Semantic kernel, bindings, detector infrastructure | 112/112 | 18 | 3 | 0 |
| Z3 | Diagnostic pipeline, suppression, config, score, schema | 166/166 | 21 | 3 | 0 |
| Z4 | Project analysis, Cargo health, structure | 135/135 | 17 | 2 | 0 |
| Z5 | Execution, concurrency, caching, signals | 106/106 | 10 | 1 | 0 |
| Z6 | Git scope, staged, baseline, delta | 96/96 | 14 | 0 | 0 |
| Z7 | CLI surface, hooks, handoff | 127/127 | 14 | 1 | 0 |
| Z8 | Terminal output and TUI | 139/139 | 13 | 7 | 0 |
| Z9 | Action, npm, skill, distribution | 133/133 | 21 | 3 | 1 |
| Z10 | Corpus, calibration, validation | 98/98 | 15 | 0 | 0 |
| Z11 | Contributor process and CI | 103/103 | 15 | 2 | 0 |
| Total | | 1,412/1,412 | 183 | 23 | 1 |

Semantic re-reads on top of the mechanical pass, by the auditor, at the cited
lines: the serial pass order of `src/execution.rs`, the deadline branch of
`src/execution/clippy.rs`, the `[target.*]` early return of
`src/execution/rustflags.rs`, `(Some(_), None) => false` in `src/git_scope.rs`
and the staged base, the `exec` hook body, the base-failure path of
`src/execution/baseline.rs`, the glob-before-Unbound order of
`src/source_kernel/aliases.rs`, parameter resolution in
`src/source_kernel/bindings.rs`, the animation and width gates of `src/main.rs`,
`scored.first()` in `src/render.rs`, the push path of `action.yml`, the
Incomplete exit code of `src/report.rs`, both gates of
`src/report/assembly.rs`, the directive reach of `src/report/exclusion.rs`, the
`path_attribute` reader of `src/source_kernel/walk.rs`, the Cursor branch of
`src/hook/agent.rs`, the absent `errors` read in `src/render/markdown.rs`, the
shallow diagnosis in `src/git_scope.rs`, the delta fingerprint of
`src/delta.rs`, and the unredacted credential frame of `src/render.rs`.

Measured or recomputed: `tests/corpus.json` holds 265 false-positive verdicts
(147 on native rules), provenance agent 369, unrecorded 98, human 0, 239 pairs
and 17 open escalations. rustfmt 1.97.1 on a scratch copy of `src/main.rs`
yields about 1,024 lines. The pinned clippy-driver 1.97.1 lists
`invalid_regex`, `let_underscore_lock` and `serde_api_misuse` at deny and no
span-guard lint. `libc` is already a dev-dependency (`Cargo.toml:52`) and in
`Cargo.lock`.

External facts fetched on 2026-10-13: Cursor hooks pass `status` (completed,
aborted, error) and `loop_count` from 0, default `loop_limit` 5
(cursor.com/docs/agent/hooks). tokio `block_in_place` "panics if called from a
current_thread runtime" (docs.rs/tokio). axum 0.8 captures are `{key}` and
`{*key}`, and a segment starting with `:` or `*` panics unless
`without_v07_checks()` is set (docs.rs/axum 0.8.4).

Merged duplicates: 159 zone findings collapse into 126 items. The merge of each
item is visible in its id list.
