# Gap audit: rust-doctor vs react-doctor

rust-doctor is the Rust transposition of react-doctor (`/home/arthur/dev/react-doctor`,
commit `83466a8fa`). Each zone below pairs the react-doctor paths with their rust-doctor
counterpart and is audited by its own subagent. A side is checked only after its
subagent's evidence was re-verified (every cited `file:line` quote re-read, every
absence claim re-searched). React-only machinery is out of scope.

Paths are relative to each repository root. `RD` is react-doctor, `RS` is rust-doctor.

## Zones

### Z1. Rule catalog breadth
- [x] RD: `packages/oxlint-plugin-react-doctor/src/plugin/rules/**`, `docs/{rule-candidates-backlog,rule-retirements,*-rule-research,three-r3f-grading-rubric-coverage}.md`
- [x] RS: `src/policy/catalog.rs`, `src/policy/catalog/**`, `src/policy/rejected.json`, `src/policy/coverage.rs`, `tests/fixtures/**`, `tests/rule_evidence.json`

### Z2. Rule-authoring infrastructure and semantic analysis
- [x] RD: `packages/oxlint-plugin-react-doctor/src/plugin/{utils,semantic,liveness,constants}/**`, `src/test-utils/**`, `src/types.ts`, `tests/**`, `docs/{HOW_TO_WRITE_A_RULE}.md`
- [x] RS: `src/source_text.rs`, `src/source_kernel.rs`, `src/source_kernel/**`, `src/structure/normalize.rs`, `tests/{generic_source_kernel,source_kernel_cli,source_kernel_product_proof,rule_scaling_kernel,rule_scaling_surfaces}.rs`, `tests/support/rule_scaling.rs`

### Z3. Diagnostic pipeline, configuration, suppression, score, report schema
- [x] RD: `packages/core/src/*.ts` (orchestration, pipeline, suppression, severity, config, score, JSON report), `packages/core/src/{services,types}/**`, `docs/{json-report}.md`
- [x] RS: `src/lib.rs`, `src/policy.rs`, `src/policy/{noise,catalog/validate}.rs`, `src/audit.rs`, `src/audit/**`, `src/configuration.rs`, `src/report.rs`, `src/report/**`, `src/internal_error.rs`, `src/structure/suppression.rs`, `tests/{audit_contract,policy_kernel,policy_gate_product_proof,configuration_kernel,configuration_oracle,persistent_configuration_product_proof,kernel_contract,score_credibility_kernel,score_credibility_packs,suppression_audit,profile_hardening}.rs`

### Z4. Project-level analysis and native checks
- [x] RD: `packages/core/src/{project-info,project-analysis,checks,react-cleanup}/**`, `packages/core/src/check-*.ts`, `packages/core/src/services/{dead-code,maintainability,supply-chain,project,node-resolver}.ts`
- [x] RS: `src/cargo_health.rs`, `src/cargo_health/**`, `src/structure.rs`, `src/structure/{duplication,hotspots,manifest}.rs`, `src/repo_hygiene.rs`, `tests/{cargo_health_product_proof,dependency_truth,repo_hygiene,structure_kernel}.rs`

### Z5. Execution engine, performance, caching, concurrency
- [x] RD: `packages/core/src/runners/**`, `packages/core/src/{run-oxlint,*-worker,start-*-worker,materialize-source-tree,git-prefetch}.ts`, `packages/core/src/utils/**`, `packages/react-doctor/src/cli/utils/scan-result-cache*.ts`, `scripts/performance/**`
- [x] RS: `src/execution.rs`, `src/execution/{clippy,messages}.rs`, `src/bounded_read.rs`, `src/scan_target.rs`, `src/workspace_path.rs`, `src/source_kernel/walk.rs`, `src/structure/benchmark.rs`, `tests/presentation_nfr.rs`

### Z6. Git scope, diff, staged files, baseline, delta
- [x] RD: `packages/core/src/{get-diff-files,parse-changed-line-ranges,compute-diagnostic-delta,parse-gitattributes-linguist}.ts`, `packages/core/src/services/{git,staged-files}.ts`, `packages/react-doctor/src/cli/utils/{*staged*,*baseline*,*git-hook*,collect-affected-files,detect-default-branch,filter-diagnostics-by-changed-lines,diagnostic-intersects-line-ranges,read-changed-files-from,project-manifest-changed,resolve-scope,resolve-project-diff-include-paths}.ts`
- [x] RS: `src/git.rs`, `src/git/**`, `src/git_scope.rs`, `src/git_scope/**`, `src/baseline.rs`, `src/execution/baseline.rs`, `src/delta.rs`, `src/delta/**`, `tests/{git_scope_kernel,git_scope_oracle,git_change_scope_product_proof,baseline_kernel,baseline_oracle}.rs`, `tests/baseline_kernel/**`

### Z7. CLI surface, CI gating, install and agent handoff
- [x] RD: `packages/react-doctor/src/{index,inspect,inspect-options,inspect-runtime,instrument}.ts`, `src/cli/{index,start-scan-preamble}.ts`, `src/cli/commands/**`, `src/cli/runtime-scan/**`, `src/cli/utils/**` not claimed by Z5, Z6 or Z8
- [x] RS: `src/main.rs`, `src/handoff.rs`, `src/handoff/**`, `src/skill.rs`, `src/permutations.rs`, `tests/{local_cli_experience,policy_cli,product_proof,skill_contract}.rs`

### Z8. Terminal output, TUI and rendering
- [x] RD: `packages/react-doctor/src/cli/ink/**`, `src/cli/utils/{render-*,print-*,format-*,build-code-frame,highlight-*,*hyperlink*,spinner,terminal-symbols,doctor-face,wrap-indented-text,build-footer-*,diagnostic-grouping,select-report-diagnostics}.ts`, `packages/core/src/{highlighter,summarize-diagnostics}.ts`
- [x] RS: `src/tui.rs`, `src/tui/**`, `src/render.rs`, `src/render/**`, `src/presentation.rs`, `src/presentation/**`, `src/terminal_text.rs`, `src/score_block.rs`

### Z9. Distribution and integration surfaces
- [x] RD: `packages/api/**`, `packages/eslint-plugin-react-doctor/**`, `packages/react-doctor/package.json`, `action.yml`, `.github/workflows/{publish,publish-any-commit,action-version-bump,react-doctor,terminal-recording}.yml`, `.cursor-plugin/**`, `skills/**`, `.changeset/**`, root `scripts/*`
- [x] RS: `npm/rust-doctor/**`, `skills/rust-doctor/**`, `.github/workflows/release.yml`, `.github/releases/**`, `docs/{publishing,agent-skill}.md`, `Cargo.toml`, `README.md`

### Z10. Validation, calibration and rule lifecycle
- [x] RD: `packages/fuzz/**`, `packages/evals/**`, `scripts/{fn-mining,delta-audit}/**`, `.agents/skills/{benchmark-fp-fn-audit,fuzz,rde-eval,rule-research,rule-validate,rule-writing,run-parity}/**`, `.github/workflows/delta-audit.yml`
- [x] RS: `tests/corpus*.rs`, `tests/corpus.json`, `tests/{rule_admission,rule_scaling_precision,precision_harness,protocol_corpus}.rs`, `tests/support/corpus/**`, `.claude/skills/**`, `docs/catalog-and-corpus.md`, `docs/*-2026-08.md`, `.github/workflows/{corpus,dogfood}.yml`, `tasks/**`

### Z11. Contributor process, docs and CI
- [x] RD: `AGENTS.md`, `CLAUDE.md`, `docs/archive/**`, `.agents/skills/{deslop,find-similar-functions,product-thinking,ship,writing-guidelines,react-doctor}/**`, `.github/workflows/{ci,code-quality}.yml`, `.vite-hooks/**`, `.claude/**`, root configs
- [x] RS: `AGENTS.md`, `CLAUDE.md`, `docs/{doc-gates,testing,ci-workflows}.md`, `docs/subsystems/**`, `docs/doc-budgets.json`, `scripts/**`, `.github/workflows/ci.yml`, `tests/{docs_contract,out_of_line_test_modules}.rs`, `tests/support/mod.rs`, `src/{test_clock,test_scratch}.rs`, `clippy.toml`

## Verification log

| Zone | Report | Citations checked | Absence claims re-searched | Rejected or downgraded |
|---|---|---|---|---|
| Z2 | 9 gaps (1 high, 6 medium, 2 low) | 62/62 exact | 8/8 zero hits | none; semantic spot-check of aliases.rs, detectors.rs test attribute, Cargo.toml parser dep |
| Z4 | 9 gaps (1 high, 7 medium, 1 low) | 50/50 exact | 11/11 zero hits | none; Z4-09 overlaps Z6 (gitattributes), merged at synthesis |
| Z6 | 10 gaps (2 high, 6 medium, 2 low; Z6-10 unconfirmed) | 71/71 exact | 10 patterns: 3 with hits, all matching the stated explanation (catalog help text, repo_hygiene comments, workflow template comment) | none; Z6-08 duplicates Z4-09, Z6-04 overlaps Z5 |
| Z3 | 12 gaps (3 high, 6 medium, 3 low) | 80/80 exact | 14 patterns: 3 with hits, all matching the stated explanation | none; semantic check of normalize.rs keeping uncatalogued warnings (Z3-03) and SKILL.md "no suppression comment" (Z3-02); Z3-06 merged with Z4-09/Z6-08 |
| Z5 | 9 gaps (3 high, 5 medium, 1 low) | 65/65 exact | 9 patterns: 2 with hits, both matching the stated explanation | Z5-01 nuanced: Cargo replays cached warnings on the current side, the cold cost is the baseline temp target; Z5-07 Clippy 1.83 date unconfirmed; Z5-06 -Dwarnings effect inferred |
| Z10 | 7 gaps (2 high, 5 medium) | 64/64 exact | 10 patterns: 2 with hits, both matching the stated explanation | none; corpus counts recomputed from tests/corpus.json (precision 36 unobserved / 22 measured / 4 incomplete, agent side 48 unobserved) |
| Z8 | 11 gaps (1 high, 5 medium, 5 low; Z8-06 unconfirmed) | 74/74 exact | 9 patterns, zero hits | none; render.rs top-group-only and main.rs COLUMNS-only width re-read |
| Z7 | 9 gaps (4 high, 4 medium, 1 low) | 91/91 exact | 12 patterns: 2 with hits, both matching the stated explanation | none; main.rs interactions_allowed and skill.rs AlreadyPresent re-read; Z7-03 overlaps Z9-04, Z7-08 overlaps Z8-05 |
| Z9 | 8 gaps (2 high, 5 medium, 1 low) | 70/70 exact | 8 patterns, zero hits | Z9-02 re-read in src/tui/workflow.rs (push runs --scope full and exits with the scan status); the README contradiction is softened, the README sentence is about PRs; Z9-04 merged with Z7-03 |
| Z11 | 11 gaps (1 high, 5 medium, 5 low) | 70/70 exact | 20 patterns: 1 with hits, matching the stated explanation | Z11-01 and Z11-02 are documented deferrals in docs/ci-workflows.md:10-17 ("a port to do"), kept as gaps but labeled as known |
| Z1 | 14 gaps (5 high, 7 medium, 2 low) | 102/102 exact | 23 patterns: 1 with hits, matching the stated explanation | Z1-01 reproduced on toolchain 1.97.1 (clippy-driver on a scratch file: -A clippy::all silences deny-by-default eq_op, plain clippy errors, -W restores it as a warning); Z1-06 merged with Z4-04, Z1-11 with Z10-06, Z1-05 with Z3-03 |

## Synthesis

All 22 zone sides are checked. The subagents reported 109 gaps, 98 once cross-zone
duplicates are merged (Z3-06/Z4-09/Z6-08, Z7-03/Z9-04, Z1-05/Z3-03, Z1-06/Z4-04,
Z1-11/Z10-06, Z3-08/Z7-07, Z7-01/Z9-02, Z7-06/Z8-11, Z7-08/Z8-05, Z5-08/Z11-10).
799 citations were re-read against their exact line and 138 absence regexes re-run.
Two gaps stay unconfirmed (Z6-10, Z8-06). The zone reports are JSON files in the
session scratchpad (`zones/Z*.json`).
