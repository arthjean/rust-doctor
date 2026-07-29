[PRD]
# PRD: React Doctor Product Parity for Rust Doctor

## Changelog

| Version | Date | Author | Summary |
|---------|------|--------|---------|
| 1.3 | 2026-07-22 | Arthur Jean | Reopen parity after adversarial implementation audit; define applicable parity, intentional Rust divergences, public API and handoff gaps, and evidence-based closure gates |
| 1.2 | 2026-07-22 | Arthur Jean | Raise the supported Rust baseline and compatibility gates from MSRV 1.85 to MSRV 1.97 |
| 1.1 | 2026-07-22 | Arthur Jean | Align observability and sharing with React Doctor's stateless product loop while keeping Rust Doctor opt-in and local-first |
| 1.0 | 2026-07-22 | Arthur Jean | Initial research-informed product parity program |

## Problem Statement

Rust Doctor already aggregates Clippy, custom `syn` rules, Cargo security tools, scoring, MCP, npm distribution and a GitHub Action. It does not yet provide the coherent product contract, daily workflows or rule-quality feedback loop that make React Doctor useful beyond its raw detector count.

1. Rule behavior is not governed by one trustworthy source. `rules_config` declares severity, activation and thresholds, but the scan pipeline does not apply those controls. Rule metadata is split between custom rule implementations, the Clippy registry, MCP documentation and README prose.
2. Machine consumers receive a direct serialization of `ScanResult` with no schema version, stable diagnostic identity, normalized source range or explicit distinction between a complete scan and partial coverage.
3. Current diff mode identifies changed files, runs the main lint passes over the project, then filters diagnostics afterward. It cannot express staged content, line ranges, introduced findings, fixed findings or a degraded baseline.
4. There is no pinned external corpus, adversarial mutation harness, diagnostic delta audit or repeatable performance baseline. New rules therefore depend on local fixtures and maintainer intuition for false-positive control.
5. The CLI, Action and editor surfaces expose only part of the mature workflow demonstrated by React Doctor: rule discovery and mutation, `why` explanations, baseline-only CI feedback, managed CI installation, stable PR reporting, LSP diagnostics and editor distribution are absent or incomplete.
6. Distribution metadata and integration behavior can drift independently. For example, the crate is currently version 0.2.0 while checked-in npm package templates remain at 0.1.12, and release smoke coverage does not prove that every published package launches the same binary.

**Why now:** Rust Doctor is still pre-1.0, so its machine contract can be designed before external consumers depend on accidental serialization details. React Doctor has demonstrated that an integrated detector, agent, CI and evaluation loop can become a category-defining developer tool. AI-assisted implementation has reduced the cost of building the surface area; diagnostic truth, compatibility and distribution now determine whether that speed creates leverage or maintenance debt.

## Overview

This program brings Rust Doctor to behavioral product parity with React Doctor while using Rust-native mechanisms. Rustc and Clippy remain the type-aware adapters, `syn` remains the stable local AST adapter, Cargo metadata remains the project graph, and specialized Cargo tools remain external adapters. A canonical rule catalog and diagnostic model become the shared seam used by configuration, reporting, terminal output, MCP, SARIF, CI and editors.

The program is intentionally delivered as three releases because 38 independently implementable stories exceed a safe single-release PRD. Release 1 establishes trustworthy semantics and the quality machine. Release 2 exposes those semantics through daily CLI, library, agent, CI, distribution and editor workflows. Release 3 expands Rust-specific detection and public product surfaces only after the corpus can measure regressions.

| Release | Included epics | Exit gate |
|---------|-----------------|-----------|
| R1: Trustworthy Core | EP-001, EP-002, EP-003 | Versioned report, explicit completeness, correct scopes and baseline, 100-repository corpus, regression gates |
| R2: Daily Workflows | EP-004, EP-005 | CLI rule management, agent installation, managed CI, PR reporting, release smoke tests, LSP and two editor adapters |
| R3: Rust-Native Moat | EP-006 | Evidence-ranked rule pipeline, two validated rule tranches, framework packs, generated public docs, opt-in observability and stateless sharing |

## Normative Parity Contract

"Total parity" means behavioral product parity for Rust codebases, not identical implementation, rule names or numeric scores. Every externally observable React Doctor capability in the reference snapshot must be classified as one of:

1. **Applicable:** Rust Doctor provides the equivalent user outcome and an executable parity fixture.
2. **Intentional Rust divergence:** Rust Doctor deliberately uses a stricter or Rust-native contract, documents the difference and tests it end to end.
3. **Not applicable:** The capability depends on React, browser or JavaScript semantics with no useful Rust analogue.

The comparison reference is React Doctor commit `3d7ea66c3f45fa55828559fce5cc38e879b9907a`. The initial Rust implementation audit used commit `dc82dade892b7dbe94a9c076052b167c03b81275` plus the local MSRV 1.97 update. Final certification must record clean immutable SHAs for both repositories.

| Surface | Classification | Normative Rust Doctor contract |
|---------|----------------|--------------------------------|
| Rule catalog, configuration and consumer surfaces | Applicable | One catalog and one surface classifier govern terminal, score, gate, PR comment, SARIF, MCP and LSP. Rust test, bench, example, build-script, generated and macro contexts replace React test, story and design contexts. Explicit inclusion wins over exclusion. |
| Programmatic scan API | Applicable | A versioned crate API supports one or many projects, per-project overrides, cancellation, partial project errors, cache invalidation and Report V1 conversion. |
| Report, diagnostic identity and structured errors | Applicable | Report construction success, scan outcome, completeness, score authority and quality-gate result are independent fields. Every machine surface validates the same schema. |
| Score formula | Intentional Rust divergence | Rust Doctor keeps its local five-dimension weighted score and unique-rule counting. It does not copy React Doctor's remote flat formula or labels. Score population is filtered by the canonical score surface before scope presentation filters; partial required analysis makes it non-authoritative. |
| Workspace aggregate score | Intentional Rust divergence | Rust Doctor scores aggregate canonical diagnostics with existing Rust weights and also exposes every package score. It does not use React Doctor's worst-project score as the aggregate. |
| Invalid configuration | Intentional Rust divergence | Rust Doctor fails closed with a typed setup error. It does not coerce, drop or silently ignore invalid policy as React Doctor sometimes does. |
| Git scope failure | Intentional Rust divergence | Invalid refs and unreadable indexes fail closed. Only an allowlisted, proven shallow-history, LFS, base-build or materialization failure may degrade to files scope, with requested and resolved bases preserved separately. |
| Full, files, changed, lines, staged and baseline workflows | Applicable | Rust-native scopes preserve React Doctor's user outcomes, including current-worktree changed-file behavior, committed introduced-only comparison, line-to-file degradation and exact staged content. |
| Completeness and exit codes | Intentional Rust divergence | Rust Doctor retains structured exit codes 0 through 4 and `--require-complete`. One normative truth table governs CLI, MCP and Action. Optional unavailable checks remain explicit but do not invalidate required-core completeness. |
| GitHub Action defaults | Intentional Rust divergence | The official Rust Action may remain stricter than React Doctor by enabling required completeness. PR, push and non-PR gating behavior must be explicit and fixture-backed. |
| LSP and editor adapters | Applicable with Rust-native analyzers | Open-document scans use the canonical file-local Rust rule engine. Failed or unparsable refreshes preserve last-known-good diagnostics, overlay proven new findings and mark degraded state instead of silently clearing truth. |
| Agent diagnostic dump and handoff | Applicable | Rust Doctor writes a bounded structured dump and can hand the highest-priority groups to a detected agent or clipboard without changing scan results. |
| Telemetry and share links | Intentional Rust divergence | Zero network is the default. Telemetry is explicit opt-in and sharing is explicit, stateless and source-free. |
| React design command and dynamic JavaScript plugins | Not applicable in R1 through R3 | No synthetic Rust equivalent is required. Curated Cargo adapters and Rust framework packs remain in scope. |
| `why`, transactional config mutation, MCP, SARIF and remediation plans | Rust-native extension | These may exceed React Doctor, but they must still consume the canonical contract and cannot compensate for a missing applicable surface. |

Parity is certified only when every Applicable row has an automated fixture, every divergence has a regression test and rationale, every release gate below passes, and the status tracker cites the exact evidence. Presence of code, a green unit suite or a high self-scan score alone is not completion.

## Goals

| Goal | Month-1 Target | Month-6 Target |
|------|---------------|----------------|
| Rule configuration execution | 100% of declared rule, category, tag, path and surface controls covered by integration fixtures | 0 confirmed configuration no-ops across the pinned corpus |
| Machine report contract | Report V1 emitted and validated on 100% of JSON, MCP and Action smoke paths | No incompatible change within V1 across all published 0.x releases |
| Scope correctness | Full, files, lines, staged and baseline modes pass all repository fixtures | At least 99.5% correct new/fixed classification across 5,000 labeled diagnostic pairs |
| Honest completeness | 100% of reports expose planned, analyzed, skipped, failed and timed-out work | 0 corpus run interpreted as clean when any required check is incomplete |
| Rule robustness | 1,000 deterministic mutations per custom rule with 0 uncaught panics | 10,000 mutations per custom rule with 0 uncaught panics |
| External evaluation | 100 pinned repositories and at least 250 project roots | 1,000 pinned repositories and at least 2,500 project roots |
| False-positive control | Less than or equal to 2% confirmed false positives in reviewed new-diagnostic samples | Less than or equal to 1% confirmed false positives per promoted rule |
| Distribution integrity | 5 of 5 release targets pass packed install and `--version` smoke tests | 100% of releases publish matching crate, npm and GitHub artifact versions |
| Editor responsiveness | LSP P95 under 500 ms after debounce for a 10,000-line open file | LSP P95 under 300 ms on the same benchmark with 0 stale diagnostics after close |

## Target Users

### Individual Rust Maintainer

- **Role:** Maintains one or more Rust crates, often with AI coding agents and limited review time.
- **Behaviors:** Runs Clippy locally, uses Cargo point tools selectively, and asks an agent to fix findings.
- **Pain points:** Tool output is fragmented, project configuration is hard to calibrate, and existing issues obscure regressions introduced by the current change.
- **Current workaround:** Runs several Cargo commands, manually deduplicates findings, and uses broad allow attributes or comments when a rule is noisy.
- **Success looks like:** One command reports only relevant new problems, explains each decision and produces an actionable agent handoff without hiding incomplete analysis.

### Rust Team and CI Reviewer

- **Role:** Owns pull-request quality gates across a workspace or multiple repositories.
- **Behaviors:** Reviews GitHub annotations and comments, maintains workflow YAML, and needs predictable exit codes.
- **Pain points:** Full-project debt makes new gates unusable, shallow clones break comparisons, comments duplicate, and missing tools can look like a passing scan.
- **Current workaround:** Maintains separate Clippy, audit, deny and SARIF jobs with custom shell glue.
- **Success looks like:** A sticky PR report shows introduced and fixed findings, incomplete states fail safely, and action upgrades preserve local workflow changes.

### AI Agent and Editor User

- **Role:** Consumes diagnostics through MCP, Codex or Claude skills, VS Code-compatible editors or Zed.
- **Behaviors:** Expects structured diagnostics, stable rule documentation, source ranges and safe machine-applicable fixes.
- **Pain points:** Unversioned output and ambiguous rule metadata force brittle parsing; full scans are too slow for edit-time feedback.
- **Current workaround:** Invokes the CLI manually and reparses terminal or JSON output.
- **Success looks like:** The same canonical finding appears consistently in CLI, MCP, SARIF and LSP, with a deterministic identity and a clear explanation of activation, confidence and remediation.

## Research Findings

Key findings that informed this PRD:

### Competitive Context

- [React Doctor](https://github.com/millionco/react-doctor) combines deterministic scanning, agent installation, baseline pull-request reporting, configurable rule surfaces, LSP/editor packages, fuzzing and corpus evaluation. Its integrated feedback loop is the primary parity reference.
- [Clippy](https://doc.rust-lang.org/clippy/) provides compiler-aware Rust diagnostics and machine-applicable suggestions. It is authoritative for type-dependent findings but does not provide a unified health contract, baseline workflow or multi-tool product surface.
- [Cargo external tools](https://doc.rust-lang.org/cargo/reference/external-tools.html) expose stable, versioned project metadata and NDJSON compiler messages. Cargo explicitly recommends passing metadata format versions and parsing future fields defensively.
- `cargo-audit`, `cargo-deny`, `cargo-geiger`, `cargo-machete` and `cargo-semver-checks` are strong specialist adapters. Their fragmentation creates the orchestration opportunity Rust Doctor serves.
- [GitHub SARIF support](https://docs.github.com/en/enterprise-cloud%40latest/code-security/reference/code-scanning/sarif-support-for-code-scanning) requires consistent rule IDs, file paths and partial fingerprints to prevent duplicate alerts across scans.
- [Language Server Protocol](https://microsoft.github.io/language-server-protocol/) provides one standardized server surface reusable across multiple editors.
- **Market gap:** Rust has excellent point analyzers but no widely adopted local-first product that combines calibrated Rust health, introduced-only review, agent guidance, editor feedback and corpus-proven rule quality behind one contract.

### Best Practices Applied

- Keep the internal diagnostic model separate from a versioned wire report. Rustc JSON is additive and consumers must tolerate unknown fields and enum values.
- Treat scan completeness as data. Empty diagnostics are not evidence of health when planned files or checks failed.
- Compare head and base findings as multisets using rule identity and normalized source evidence. Use conservative fallbacks and surface degradation.
- Scope file-local AST work before execution. Package-global compiler work may still compile a crate and must report that cost honestly.
- Reuse one rule catalog across runtime activation, documentation, configuration validation and integration surfaces.
- Evaluate every promoted rule against deterministic fixtures, adversarial mutations and pinned public repositories before making it blocking.
- Isolate untrusted corpus builds from credentials, the host home directory and network access.

*Primary research sources are linked above. Codebase evidence was taken from the current Rust Doctor and React Doctor repositories on 2026-07-22.*

## Assumptions & Constraints

### Assumptions (to validate)

- Stable Clippy JSON plus `syn` can cover the first three releases without a nightly rustc-private dependency.
- Source-evidence matching can classify moved Rust diagnostics with at least 99.5% accuracy on the labeled baseline set.
- A 100-repository initial corpus is large enough to expose major false-positive and performance regressions before expanding to 1,000.
- A single Cargo package with feature-gated binaries remains navigable through R2; a Cargo workspace split is not required for LSP or evaluation tooling.
- Users accept package-level compilation for compiler-aware findings when Rust Doctor clearly distinguishes execution scope from reporting scope.
- Explicit opt-in is required for telemetry and public score sharing; local scans and local share-URL construction remain fully functional without a network.

### Hard Constraints

- Rust 2024 and MSRV 1.97 remain supported.
- Production code contains no `unsafe` blocks and uses typed `thiserror` errors. `anyhow` and `Box<dyn Error>` are not introduced in library code.
- Score dimensions and weights remain unchanged unless Arthur explicitly approves a separate scoring decision.
- Diagnostics remain on stderr and score or machine output remains on stdout.
- Custom rules remain protected by `catch_unwind`.
- The existing OS-thread, scoped-thread and Rayon parallelism invariant is preserved unless a benchmark-backed replacement proves no deadlock regression.
- Missing optional tools degrade explicitly through structured check states. Required checks never fail open.
- MCP remains read-only, restricted under `$HOME`, cancellable and offline by default. New MCP tools or weaker security require separate explicit approval.
- New module visibility defaults to `pub(crate)`. Public visibility changes require explicit approval.
- JavaScript and TypeScript tooling uses Bun and `bunx`.
- The completed best-practices PRD is historical state and is not rewritten by this program.

## Quality Gates

These commands must pass for every user story:

- `cargo fmt --check` - formatting
- `cargo +1.97 check --all-targets` - declared MSRV compatibility
- `cargo check --all-targets --all-features` - all Rust targets, evaluation binaries and LSP type-check
- `cargo build` - default build with MCP
- `cargo build --no-default-features` - CLI and library build without MCP
- `cargo clippy --all-targets --all-features -- -W clippy::all -W clippy::pedantic -W clippy::nursery -D warnings` - strict lint gate
- `cargo test --all-features` - unit, integration, snapshot, evaluation and LSP tests
- `cargo audit` - no unacknowledged vulnerable dependency
- `cargo deny check` - advisories, bans, sources and licenses satisfy checked policy

R1 additionally requires protected corpus, diagnostic-delta and benchmark jobs with approved checked-in inputs. R2 additionally requires shell-level Action tests, packed crate/npm/archive smokes, a real LSP JSON-RPC lifecycle, and automated Bun build, typecheck, test and packaging checks for each JavaScript editor surface. Every Report V1 instance must pass a standards-compliant Draft 2020-12 validator, not a top-level-key proxy. CI actions must be pinned to immutable commit SHAs. Browser-based verification is not authorized by this PRD and requires an explicit implementation-time request.

## Implementation Audit and Reopening Criteria

The 2026-07-22 adversarial audit reopens the program. The previous `DONE` state was based mainly on implementation presence and local tests; it did not prove the acceptance criteria below. A story can return to `DONE` only when its status record links an executable test or immutable artifact proving every criterion.

| Priority | Stories | Proven implementation gap | Required closure evidence |
|----------|---------|---------------------------|---------------------------|
| P0 | US-003, US-004, US-007, US-009 | MCP accepts diff input but performs a full scan and can label the result baseline without a baseline comparison. | Shared CLI/MCP scope executor; fixture proving `mode=baseline` requires two traced scans, a baseline payload and a completed baseline check. |
| P0 | US-004, US-010 | Report completeness includes file coverage, while `--require-complete` checks only required adapter states and can exit 0 for an incomplete report. | One `compute_completeness` result consumed unchanged by Report V1, baseline, CLI, MCP, Action, telemetry and sharing; stale-file and zero-required-check fixtures. |
| P0 | US-003, US-010, US-012, US-031 | Parse failures, unreadable files and custom-rule panics can become empty successful results and `Completed` checks. | Per-check work-unit receipts; structured failed units for parse/read/panic/oversize; no panic path can remain authoritative. |
| P0 | US-006, US-007, US-010 | Full AST scan currently ignores valid Rust surfaces outside `src/` while file accounting can mark them analyzed; files scope can find more than full. | Matrix over `src`, tests, benches, examples, build scripts, custom Cargo targets, generated files and macros proving full is a semantic superset. |
| P0 | US-004, US-008, US-011 | Manifest-only staged changes can return `nothing_to_scan` without dependency checks. | Planning model that counts source, manifest, lockfile and project/workspace work units; manifest-only staged and changed fixtures. |
| P0 | US-002, US-008 | Staged source can be analyzed with divergent worktree configuration and ignore policy. | Indexed source, manifest, lockfile, config and ignore provenance or a deliberate config-drift refusal, all fingerprinted in Report V1. |
| P0 | US-003, US-011, US-024 | Diagnostics lack explicit Cargo owner and canonical file context; project-level findings can be copied to unrelated packages and nested-project GitHub paths are wrong. | Explicit `package_id | workspace | unowned` owner plus source-surface context and repository-relative rendering fixtures. |
| P0 | US-001, US-003 | Unknown rustc and Clippy diagnostics can fall into the generic external/style namespace. | Adapter-provenance namespace fallback for rustc, Clippy, RustSec and cargo-deny with identity-stability tests. |
| P0 | US-002, US-005, US-024 | `pr-comment` visibility is not enforced by Action reporting and presentation flags mutate canonical report diagnostics. | Consumer-by-surface conformance matrix; immutable Report V1 after construction. |
| P0 | US-002, US-017 | Invalid `fail_on`, glob limits and catalog initialization may warn and continue despite the fail-closed contract. | Exhaustive policy-input error matrix with no silent gate disablement or truncation. |
| P0 | US-007, US-009 | Baseline degrades invalid user refs and may replace the requested ref with HEAD. | Typed invalid-ref failure plus explicit allowlist for degradable failures and separate requested/resolved base fields. |
| P0 | US-007, US-011 | Inter-package renames select only the new owner and project-level ownership is inferred from missing spans. | Separate execution/reporting path sets and old/new package fixtures. |
| P0 | US-010, US-013 | External adapter non-zero exits, probes and offline behavior can still be recorded as completed. | Timeout/signal/exit-code contract for every subprocess and probe; offline adapter allowlist; failure-injection fixtures. |
| P0 | US-005, US-016, US-023 | Report smoke and Action validate only top-level fields or a schema-version prefix. | Full Draft 2020-12 validation for success, partial, failure, nothing-to-scan and Action paths, including a renderer-failure fallback report. |
| P0 | US-012 | Mutation tests prove mainly no unwind; they do not prove liveness, fire coverage or semantic non-broadening. | Versioned limitation IDs, parseable mutation quota, metamorphic positive/negative oracles, subprocess isolation and reproducible minimized failures. |
| P0 | US-013, US-014 | Evaluation does not force candidate rules on, normalize severity or disable local config and suppressions. | Versioned evaluation profile fingerprint shared by baseline and candidate, with all promoted rules observed or an explicit failure. |
| P0 | US-013 | Prepared nested Cargo roots are counted but not necessarily scanned; commit checking does not prove a clean pinned tree. | Exact equality of expected, attempted and reported roots; one-owner coverage; clean index/worktree, submodule and tree-digest verification. |
| P0 | US-014 | Promotions can be omitted, labels are weakly identified, changed severities can evade gating and no protected delta job runs. | Catalog-diff-derived promotions, stratified hash sample with full occurrence identity, per-root completeness comparison and required protected CI job. |
| P1 | US-015 | Benchmark baseline is optional and lacks binary/host identity; cache keys omit implementation identity and the size cap is applied after unbounded load/insertion. | Approved mandatory gate baseline, binary and host fingerprints, minimum repetitions, bounded load/eviction and cache-cycle RSS assertions. |
| P0 | US-010, US-016 | MCP cancellation smoke accepts a normal response, read-only-output smoke does not exercise a truly unwritable destination, and the normal all-features test suite launches enough concurrent self-scans to time out Clippy/semver checks and fail two MCP tests although each passes alone. | Deadline-observed cancellation, a platform-safe unwritable-output test and bounded/isolated MCP fixtures that pass under the repository's normal test concurrency. |
| P0 | US-020 | `why` and rule workflows do not consistently use parent project discovery, namespace fallback, required-check filtering or actual framework/test context. | End-to-end invocation from root and nested directories for local, dynamic and framework rules with optional checks unavailable. |
| P0 | US-022, US-023, US-024 | CI PR installation is not transactionally reversible; API fallback can pass absolute paths to a relative-only CLI and report all findings as introduced; shell reporting is not control-character safe. | Failure-injected transaction tests and real shell E2E over shallow PRs, nested roots, fallback API, tabs/newlines and degraded baseline. |
| P0 | US-025 | Release tests native binaries before archiving, do not execute extracted archives and package the crate with `--no-verify`. | Extract-install-run smoke for every final archive/package and verified `.crate` contents before publication. |
| P0 | US-026, US-027, US-028 | LSP timeout may still wait for blocked work; default adapters can require a nonexistent config; framework gates lose version/features/target; protocol compatibility is advertised but not negotiated. | Real JSON-RPC and packaged-adapter E2E covering absent config, hard cancellation, last-known-good overlay, framework gates and protocol-major rejection. |
| P0 | US-031, US-032, US-033 | New rules infer target/test context from paths, framework target gates are incomplete and promoted rules remain default-off without corpus evidence. | Cargo-target-aware context, serialized gate reasons and successful protected promotion gates before default enablement. |
| P1 | US-034 | Public site generation, link validity, accessibility and example replay have no checked local evidence in this repository. | Immutable website build artifact tied to catalog hash and required external-repository checks. |
| P0 | US-035, US-036 | Telemetry and sharing collapse partial completeness to score authority and omit several suppression classes. | Exact Report V1 completeness reuse, all suppression sources counted and loopback tests proving the payload allowlist and zero default network. |
| P0 | All | CI has no MSRV, corpus, delta, performance, editor or real Action lanes; several actions use mutable tags; on Rust 1.97 the strict Clippy gate is red with 21 diagnostics, while `cargo audit` and `cargo deny check` are also red. | All Quality Gates above green on the candidate SHA with no skipped required job. |

## Epics & User Stories

The 38 stories are explicitly phased. R2 cannot be certified until the R1 exit gate passes. R3 cannot promote new blocking rules until the R1 corpus and R2 distribution smoke gates pass.

### EP-001: Canonical Rule and Diagnostic Contract

Create one deep internal module for rule knowledge and one explicit diagnostic/report contract shared by every consumer.

**Definition of Done:** Every emitted finding resolves through the canonical catalog or a documented external namespace fallback; configured behavior is executed; Report V1, its schema and compatibility fixtures are the only machine-output contract.

#### US-001: Define the canonical rule catalog

**Description:** As a maintainer, I want one catalog for all rule metadata so that runtime behavior, documentation and integrations cannot drift.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** None

**Acceptance Criteria:**

- [ ] A single internal `RuleDescriptor` model represents canonical ID, provider, category, default severity, tags, analyzer kind, confidence, default activation, applicable frameworks, documentation URL, supported threshold and fix capability.
- [ ] All 34 registered custom AST rules, all 74 explicit Clippy mappings and every project/dependency descriptor resolve through the catalog; tests derive expected counts from registration instead of a stale hand-written total.
- [ ] External diagnostics resolve through exact descriptors or a documented namespace fallback for dynamic codes such as RustSec advisory IDs.
- [ ] README rule counts and MCP rule listings are generated or asserted against catalog data.
- [ ] Given duplicate canonical IDs or aliases, catalog construction fails deterministically in tests instead of accepting the last entry.
- [ ] Adding a custom rule requires registration in one implementation-owned location and no hand-edited parallel documentation table.

#### US-002: Execute typed rule configuration

**Description:** As a Rust maintainer, I want rule, category, tag, path and surface controls to affect actual diagnostics so that project policy is trustworthy.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-001

**Acceptance Criteria:**

- [ ] Severity uses a typed `off | info | warning | error` value and activation is resolved before local rules execute when possible.
- [ ] Precedence is deterministic and documented: path override, exact rule, category, tag, catalog default.
- [ ] Visibility surfaces support terminal, score, CI failure, PR comment, SARIF and MCP without changing rule activation.
- [ ] One source-context classifier covers library, binary, test, bench, example, build script, generated source and macro expansion; default score/gate exclusions and explicit include-over-exclude precedence are documented and fixture-backed.
- [ ] Existing `rules_config`, `ignore.rules` and `ignore.enable` syntax is accepted for one minor release with a deprecation message and equivalent behavior.
- [ ] Thresholds are accepted only for descriptors declaring a supported numeric range.
- [ ] Given an unknown rule, invalid severity, unsupported threshold, malformed glob, exceeded policy limit, conflicting legacy/canonical key or catalog initialization failure, configuration returns a typed error and does not silently fall back, truncate or disable a gate.
- [ ] Security rules suppressed by project configuration remain visible in verbose audit metadata without re-enabling them.

#### US-003: Introduce canonical diagnostics and stable identities

**Description:** As an integration author, I want one rich diagnostic model with deterministic identities so that findings remain consistent across CLI, CI, SARIF, MCP and editors.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-001

**Acceptance Criteria:**

- [ ] Canonical diagnostics include provider, rule, title, category, severity, message, help, URL, tags, analysis kind, confidence, source-surface context and explicit `package_id | workspace | unowned` ownership.
- [ ] Primary spans carry normalized project-relative path, start and end line, column and byte offsets; related locations and macro expansion context are retained when available.
- [ ] Fixes preserve rustc applicability and may share a stable fix-group ID when one edit resolves multiple findings.
- [ ] A deterministic site ID and path-independent baseline evidence key are produced with a documented SHA-256 input contract.
- [ ] Repeated scans of unchanged content produce byte-identical sorted diagnostic identities on Linux, macOS and Windows path formats.
- [ ] Given a compiler diagnostic with no local primary span, the model emits an explicit project-level location instead of inventing a source line.
- [ ] Project-level location means absence of a source span, not workspace-global ownership; an adapter must mark workspace-global findings explicitly.
- [ ] Namespace fallback receives adapter provenance and preserves stable `rustc`, `clippy`, `rustsec` and `cargo-deny` provider/category identity before an exact descriptor exists.
- [ ] Unknown additive rustc JSON fields or diagnostic levels do not crash parsing and are preserved or mapped to an explicit unknown state.

#### US-004: Emit versioned Report V1

**Description:** As a machine consumer, I want an explicit report schema so that I can distinguish health, failure and partial coverage safely.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-003

**Acceptance Criteria:**

- [ ] Report V1 includes `schema_version`, tool version, report-construction success, scan outcome, requested root, resolved root, execution mode, reporting scope, completeness, score authority, gate result, projects, flattened diagnostics, summary, elapsed milliseconds and structured error.
- [ ] Report-construction success, scan outcome, completeness, score authority and gate result are independent axes and no consumer infers one from another.
- [ ] Each project includes Cargo package ID, package root, targets, framework capabilities, planned files, analyzed files, check states, skipped reasons, completeness and score.
- [ ] Modes are serialized as `full`, `files`, `lines`, `staged` or `baseline`.
- [ ] Empty diagnostics with failed, timed-out or cancelled required work never serialize as a clean complete report.
- [ ] Expected discovery, configuration and scan failures still produce schema-valid Report V1 when JSON mode is active.
- [ ] Given a non-Rust directory or a scope with zero eligible files, the report returns an explicit `nothing_to_scan` outcome and a null score rather than 100.
- [ ] `nothing_to_scan` requires zero applicable source, manifest, lockfile, package and workspace work units after discovery and scope planning; it cannot be inferred from an empty Rust file list alone.
- [ ] `requested_root` preserves the user input root after normalization and never silently becomes the discovered Cargo root.

#### US-005: Publish schema, compatibility and renderer contracts

**Description:** As a downstream consumer, I want checked and documented report compatibility so that upgrades do not break CI or editor integrations.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-004

**Acceptance Criteria:**

- [ ] A checked-in Draft 2020-12 JSON Schema for Report V1 is generated from the wire types and validated with a standards-compliant validator against every report snapshot and built-CLI artifact.
- [ ] Top-level legacy fields consumed by the current Action remain available through the V1 migration release or receive an explicit machine-readable replacement.
- [ ] `--json-compact` and `--json-out <path>` produce the same data as `--json` with only formatting or destination differences.
- [ ] Terminal, JSON, SARIF and MCP rendering consume the canonical diagnostic and report models without mutating serialized values after creation.
- [ ] Structured JSON errors include typed kind, redacted message, causal chain and exit classification; if normal rendering fails, a minimal schema-valid failure report is emitted when stdout remains writable.
- [ ] Given an unwritable `--json-out` path, stdout remains empty, the error is actionable and no partial report file remains.
- [ ] A migration document defines additive V1 evolution and requires a new schema version for removed fields or changed semantics.

---

### EP-002: Scope, Baseline and Workspace Semantics

Make scan scope a first-class execution input, compare head and base conservatively, and expose exactly what ran for every Cargo member.

**Definition of Done:** Full, files, lines, staged and baseline modes have end-to-end fixtures covering worktrees, indexes, shallow clones, renames and workspaces; every report exposes complete execution coverage.

#### US-006: Validate the Rust scan-scope execution plan

**Description:** As a maintainer, I want measured scope constraints before implementation so that Rust Doctor does not promise file-level compiler speedups Cargo cannot provide.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**

- [ ] A design record classifies every current pass as file-local, package-global, workspace-global or network-dependent.
- [ ] Benchmarks compare full, package-selected and report-filtered Clippy execution on at least five pinned projects covering a single crate, virtual workspace, proc macro, build script and 20-member workspace.
- [ ] The record selects an index materialization strategy for staged compiler scans and documents cleanup and target-directory isolation.
- [ ] Execution scope and reporting scope are named separately in the report contract.
- [ ] Given a pass that cannot be narrowed safely, the decision record requires full package execution or an explicit skipped state and forbids claiming an early-scan speedup.
- [ ] Findings and measurements are reproducible from a checked-in command and fixture manifest.

#### US-007: Implement a unified Git scope engine

**Description:** As a user, I want explicit scan scopes so that local rules perform only relevant work and global passes behave predictably.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-006

**Acceptance Criteria:**

- [ ] `--scope full|files|changed|lines`, `--base <ref>` and `--include-untracked` resolve to one typed internal scope model.
- [ ] CLI, public crate API and MCP invoke the same scope executor; Report mode is derived from an execution receipt and never from requested flags alone.
- [ ] `changed` against a committed comparison base reports introduced-only diagnostics; `changed` for current uncommitted changes reports and gates all findings in changed files because no historical worktree snapshot exists.
- [ ] `lines` filters presentation to intersecting changed lines after score calculation and degrades explicitly to files when ranges cannot be computed.
- [ ] Legacy `--diff [base]` remains functional for one minor release as a warned alias for changed scope.
- [ ] Git changed-path parsing uses NUL-delimited output and preserves Unicode, spaces, tabs and newline-containing filenames.
- [ ] Untracked files are included only when requested and still respect Git ignore rules plus Rust Doctor ignore configuration.
- [ ] File-local AST passes receive the selected file set before reading or parsing; project-global pass policy follows US-006.
- [ ] Added, modified, renamed and deleted paths are represented without suffix-based path ambiguity.
- [ ] Given an invalid ref, non-Git directory or unreadable index, scope resolution returns a typed actionable error and does not silently run a full scan; only the allowlisted baseline failures in US-009 may degrade.

#### US-008: Scan line ranges and the exact staged snapshot

**Description:** As a contributor, I want line and staged scans to analyze the code I am committing so that pre-commit feedback matches the index rather than the working tree.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-007

**Acceptance Criteria:**

- [ ] Line scope parses zero-context Git hunks and retains diagnostics whose full primary span intersects an added or modified line.
- [ ] Staged scope reads or materializes index content, including when the working-tree file differs from the staged blob.
- [ ] Staged provenance covers Rust sources, manifests, lockfile, Rust Doctor configuration and ignore policy. If safe indexed configuration cannot be applied, configuration drift causes a typed refusal instead of mixing index and worktree policy.
- [ ] Compiler-aware staged checks use an isolated temporary project and target directory selected by US-006.
- [ ] Project-level diagnostics with no span are excluded from line scope and retained in staged or files scope only when their owning manifest or package is affected.
- [ ] Temporary staged materializations are removed after success, failure, timeout and cancellation.
- [ ] Given an unresolved index conflict, missing blob or unsafe materialization path, the scan fails with a staged-snapshot error instead of inspecting working-tree content.
- [ ] Empty staged Rust changes return `nothing_to_scan` with a complete report and null score.

#### US-009: Report introduced and fixed diagnostics against a baseline

**Description:** As a pull-request reviewer, I want only introduced findings plus a fixed count so that existing debt does not block incremental adoption.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-003, US-007

**Acceptance Criteria:**

- [ ] Baseline mode scans head and resolved merge-base with the same Rust Doctor binary and canonical rule contract without checking out over the user's worktree.
- [ ] The base scan uses configuration and applicable source files from the resolved base commit, runs only comparison-relevant analyzers, disables score-only/dead-work passes and records its configuration fingerprint.
- [ ] Matching uses a multiset of provider, rule, normalized message and diagnosed source evidence, with same-file strict matches before cross-file matches.
- [ ] The report includes base commit, new count, fixed count, base total and cross-file match count.
- [ ] Fixtures cover pure rename, copy plus edit, file deletion, whitespace-only change, moved function, repeated identical findings and case-only path change.
- [ ] Ambiguous evidence remains new rather than being matched optimistically.
- [ ] Any incomplete head or base coverage discards the diagnostic delta; untracked head sources excluded from the base comparison force `fixed_count=0`.
- [ ] Given shallow history, unreadable Git LFS content, base compilation failure or index conflict, baseline mode degrades to files scope with `baseline_degraded=true` and a reason visible in JSON, terminal and PR output.
- [ ] Invalid syntax, injection or typo in a requested ref never degrades; Report V1 retains `requested_base` separately from `resolved_base`.
- [ ] A degraded baseline cannot satisfy a configured `require_complete` CI gate.

#### US-010: Track completeness, budgets and cancellation

**Description:** As a CI operator, I want every planned check and file accounted for so that timeouts or missing analysis cannot look healthy.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-004, US-007

**Acceptance Criteria:**

- [ ] Every check has a structured state: planned, running, completed, skipped, failed, timed_out or cancelled, plus a machine-readable reason when not completed.
- [ ] Reports distinguish required checks from unavailable optional adapters and expose whether the score covers every required dimension.
- [ ] One normative `compute_completeness` function governs Report V1, baseline eligibility, CLI exit, MCP, Action, telemetry and share payloads; consumers may not reconstruct a weaker predicate.
- [ ] `--max-duration <seconds>` applies one wall-clock budget across workspace members and stops launching work after the deadline.
- [ ] `--require-complete` fails the quality gate on incomplete required work; the official Action enables it by default.
- [ ] Cancellation terminates active subprocess groups and removes temporary materializations within 2 seconds on supported platforms.
- [ ] Given a timeout after partial diagnostics, the report retains proven findings, marks completeness false and never labels the score authoritative.
- [ ] Planned and analyzed file counts remain accurate when files disappear or change during a scan.
- [ ] Every file-local check returns planned, completed and failed work-unit receipts; parse/read errors, oversized files and caught rule panics cannot be counted as analyzed or completed.
- [ ] A report with zero required checks is complete only when discovery produced zero applicable required work, never by vacuous truth.

#### US-011: Produce per-package and aggregate workspace reports

**Description:** As a workspace maintainer, I want separate package results and an aggregate view so that one unhealthy member does not obscure ownership.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-004, US-007, US-010

**Acceptance Criteria:**

- [ ] `--project` accepts package names, package-relative paths, comma-separated selections and `*` for every discovered workspace member.
- [ ] Virtual workspaces, root packages, default members, excluded members and nested path dependencies follow Cargo metadata semantics.
- [ ] Changed files map to owning packages before pass execution; root manifests and lockfiles trigger every affected global check.
- [ ] Inter-package renames schedule package-global work for both old and new owners while file-local reporting uses only existing selected paths.
- [ ] Each package receives diagnostics, check states, file coverage, elapsed time and score; the aggregate preserves existing score weights.
- [ ] The aggregate score is computed from deduplicated aggregate diagnostics using the existing Rust dimension weights, while every package score remains independently visible; this intentional difference from React Doctor's worst-project aggregate is tested.
- [ ] Diagnostics from overlapping roots are deduplicated by canonical identity without dropping distinct repeated occurrences.
- [ ] Given an unknown or ambiguous package selector, the CLI returns a typed error with valid candidate names and performs no scan.
- [ ] Workspace scheduling preserves the documented OS-thread and Rayon deadlock invariant and bounds active package scans to available parallelism.

---

### EP-003: Evaluation and Regression Infrastructure

Build the evidence system that decides whether rules, scopes, reports and performance changes are safe to ship.

**Definition of Done:** Every custom rule has conformance and mutation coverage; a sandboxed 100-repository corpus produces schema-valid NDJSON; diagnostic and performance deltas have blocking thresholds; built-package smoke tests cover all supported surfaces.

#### US-012: Add rule conformance fixtures and adversarial mutation tests

**Description:** As a rule author, I want positive, negative and mutated fixtures so that syntactic edge cases cannot crash or silently broaden a rule.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-001, US-002

**Acceptance Criteria:**

- [ ] Every custom rule has at least two positive fixtures, four negative fixtures and one fixture for each documented limitation or test-code exception.
- [ ] Every documented limitation has a stable catalog ID and the set of limitation fixtures equals the set declared by the descriptor.
- [ ] A deterministic seeded mutator runs at least 1,000 syntax-preserving or syntax-breaking mutations per custom rule.
- [ ] The harness enforces a minimum number of distinct parseable mutations, a liveness fixture and metamorphic positive/negative oracles so a rule that fires on every mutation cannot pass.
- [ ] Mutation failures print the seed and minimized input needed to reproduce the failure.
- [ ] Registry tests verify every Clippy mapping, alias, category and default severity without invoking the network.
- [ ] Given malformed Rust, truncated UTF-8, deep nesting or an unknown attribute, subprocess isolation distinguishes parse skip, caught panic, abort and timeout; no rule failure escapes the harness and every failure retains its seed/input.
- [ ] A rule cannot be enabled by default until its conformance manifest is complete.

#### US-013: Run a pinned OSS repository corpus in sandboxes

**Description:** As a maintainer, I want repeatable scans of real Rust projects so that rule quality is measured outside handcrafted fixtures.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-004, US-010, US-011

**Acceptance Criteria:**

- [ ] The initial manifest contains 100 public repositories and at least 250 Cargo project roots pinned to full commit hashes.
- [ ] A versioned evaluation profile forces every candidate rule active at one normalized severity, disables inline suppressions and repository Rust Doctor configuration, fixes adapter/toolchain policy and contributes its SHA-256 to every record.
- [ ] Fetching occurs in a separate preparation phase; evaluated scans run without network, host credentials or writable access outside their sandbox.
- [ ] Results are NDJSON records containing corpus schema version, repository, commit, package roots, tool revision, completeness, diagnostic counts, per-rule counts, durations and failure chains.
- [ ] Records contain canonical `expected_roots`, `attempted_roots` and `reported_roots`; each discovered Cargo root is covered exactly once directly or by its owning workspace, and inequality fails the run.
- [ ] Immediately before scanning, the runner proves a clean index/worktree, expected submodule state and tree digest bound to the prepared manifest, not only `rev-parse HEAD`.
- [ ] Default concurrency is bounded to the lower of 8 or available processors, with configurable per-repository and global budgets.
- [ ] Failed or incomplete projects retry at most twice with lower concurrency and remain non-successful if still incomplete.
- [ ] Given a repository with a malicious build script, symlink escape or oversized output, the sandbox contains effects and records a structured failure.
- [ ] All sandboxes and build artifacts are removed after success, failure and interruption.

#### US-014: Gate diagnostic deltas and false-positive regressions

**Description:** As a maintainer, I want candidate scans compared with an approved baseline so that broad detector changes require explicit evidence.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-009, US-013

**Acceptance Criteria:**

- [ ] The delta report compares complete candidate and baseline records by repository, package, rule, evidence identity and count.
- [ ] Baseline and candidate must contain the same canonical root set and evaluation-profile fingerprint; any root-level completeness degradation blocks comparison rather than disappearing from the denominator.
- [ ] It reports introduced, removed and changed diagnostics, completeness changes, runtime deltas and the top affected repositories per rule.
- [ ] For each rule, introductions, severity/category changes or absolute count growth affecting more than 0.5% of complete corpus roots block promotion unless an updated reviewed baseline is supplied.
- [ ] An incomplete-root increase greater than 0.2 percentage points blocks promotion.
- [ ] Promoted rules are derived from the catalog diff and cannot be omitted by a CLI flag; all new diagnostics or a deterministic hash-stratified sample of 100, whichever is smaller, can be labeled true positive, false positive or uncertain.
- [ ] Label identity includes repository, Cargo root, rule, site and evidence fingerprint; approval is tied to a protected CI artifact or required CODEOWNERS review rather than free-form self-attestation.
- [ ] A rule with more than 2% confirmed false positives in the labeled sample cannot become default-enabled.
- [ ] Given a missing, schema-incompatible or unpinned baseline, the audit exits non-zero and does not emit a favorable comparison.
- [ ] Corpus, delta and performance jobs are required CI checks for promotion; absent, skipped or empty evidence fails closed.

#### US-015: Establish performance benchmarks and bounded caches

**Description:** As a user, I want scans to remain predictably bounded as capabilities grow so that richer analysis does not make daily workflows unusable.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** Blocked by US-007, US-010, US-013

**Acceptance Criteria:**

- [ ] Benchmarks cover cold and warm full, files, lines and baseline scans on fixed small, medium, large and 20-member workspace fixtures.
- [ ] Results include wall time, CPU time, peak RSS, files per second, cache hit rate and per-pass time.
- [ ] Candidate median or P95 regressions greater than 10% and the documented minimum absolute duration against the approved benchmark baseline fail the performance gate.
- [ ] Gate mode requires an approved baseline with identical fixture/source fingerprint, diagnostic hash, host class, toolchain and exact repetition matrix; baseline and candidate binary hashes are recorded and artifact-bound independently because the compared binaries may differ; one repetition is insufficient.
- [ ] Cache keys include binary or rule-implementation digest, tool version, schema version, rule-set fingerprint, resolved configuration, target triple, compiler version and content hashes.
- [ ] Project scan caches use atomic writes, tolerate corruption by recomputing, reject oversized input before full deserialization and evict incrementally before exceeding a 512 MB default LRU cap.
- [ ] Given changed configuration, compiler version, rule metadata or file content, stale cache entries are never returned as hits.
- [ ] A 100,000-line custom-rule scan stays below 512 MB peak RSS on the benchmark host.

#### US-016: Add built-artifact end-to-end smoke tests

**Description:** As a release owner, I want packaged surfaces tested as users invoke them so that source-level tests cannot hide distribution failures.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** Blocked by US-005, US-009, US-011

**Acceptance Criteria:**

- [ ] A built release binary is exercised for terminal, score, JSON, SARIF, baseline and malformed-project paths.
- [ ] Every JSON fixture is validated by a standards-compliant Draft 2020-12 validator against Report V1 and every SARIF fixture contains stable rule IDs and fingerprints.
- [ ] MCP initialize, list-tools, every scope, scan and cancellation paths run against the built default-feature binary; cancellation must be observed within the deadline and a normal late response is a failure.
- [ ] The no-default-features build launches CLI scans without linking MCP-only dependencies.
- [ ] Packed npm wrapper installation, final compressed archives and the verified `.crate` each launch or build their exact packaged contents and report the same version as Cargo metadata.
- [ ] Given missing optional tools, corrupt cache, invalid configuration or a genuinely non-writable output directory, each surface returns the documented structured outcome.
- [ ] The smoke harness is reusable in Linux, macOS and Windows CI matrices without changing fixture expectations.

---

### EP-004: CLI, Configuration and Agent Workflows

Expose the trustworthy core through discoverable commands, reversible configuration edits and idempotent agent installation.

**Definition of Done:** Users can inspect and configure rules, explain a finding at a source location, scan every supported scope and install or remove agent integrations without manual file surgery.

#### US-017: Complete the scan CLI contract

**Description:** As a CLI user, I want a coherent command and flag surface so that local, scripted and CI invocations share the same semantics.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** Blocked by US-005, US-007, US-010, US-011

**Acceptance Criteria:**

- [ ] The default command remains a scan and supports explicit scope, base, staged, untracked, project, category, warning visibility, parallelism, duration, completeness and blocking controls.
- [ ] Users can explicitly enable or disable adapter groups such as compiler lint, custom AST, supply chain, quality and network-dependent checks without creating hidden completeness gaps.
- [ ] JSON, compact JSON, JSON file, SARIF, score-only, verbose, color and no-color modes have documented conflicts and stdout/stderr routing.
- [ ] `--output-dir` and diagnostic dump behavior are deterministic; closed stdout or stderr pipes terminate cleanly without a panic or corrupt machine output.
- [ ] `--no-respect-inline-disables` reports diagnostics hidden by Rust Doctor suppression directives without modifying source files.
- [ ] `version` reports Rust Doctor, rustc, Cargo, target triple and operating-system information without invoking a project build.
- [ ] Legacy `--diff` and `--fail-on` aliases warn once and preserve their argument consumption for one minor release.
- [ ] Help output leads with tested examples and documents one normative truth table mapping report outcome, completeness, blocking level and output mode to exit codes 0 through 4.
- [ ] Given an unknown flag, removed flag or incompatible output combination, Clap returns a stable usage error and no scan subprocess starts.

#### US-018: Add rule list and explanation commands

**Description:** As a user, I want to discover effective rules and their rationale so that I can calibrate policy without reading source code.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-001, US-002

**Acceptance Criteria:**

- [ ] `rules list` shows canonical ID, effective severity, category, tags, analyzer, confidence, activation and configuration source.
- [ ] Rule commands use the same upward project/config discovery boundary as scanning, including invocation from nested source directories.
- [ ] List filters support category, tag, framework, analyzer, configured-only and JSON output.
- [ ] `rules explain <rule>` shows rationale, evidence model, limitations, fix guidance, effective configuration and official external documentation links.
- [ ] Clippy and dynamic external rule families resolve through catalog namespace fallbacks.
- [ ] Given an unknown rule or filter value, the command exits with a typed error and suggests up to five nearest valid values.
- [ ] Listing and explanation perform no project build or network request.

#### US-019: Add atomic rule configuration mutation commands

**Description:** As a maintainer, I want reversible commands for changing rule policy so that configuration stays valid and reviewable.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** Blocked by US-002, US-018

**Acceptance Criteria:**

- [ ] `rules set`, `enable`, `disable`, `category`, `ignore-tag` and `unignore-tag` write canonical policy to `rust-doctor.toml`.
- [ ] `--dry-run` prints the exact proposed diff and performs no write.
- [ ] Writes use an atomic temporary file and preserve unrelated TOML comments, ordering and unknown future sections.
- [ ] Legacy `rules_config` entries migrate to the canonical schema only after a successful parse and retain equivalent behavior.
- [ ] The command detects if the file changed after reading and refuses to overwrite concurrent edits.
- [ ] Given an unknown rule, invalid level, unsupported threshold, malformed TOML or read-only directory, no file content changes and the error includes a recovery action.
- [ ] Mutation commands never edit `Cargo.toml` metadata automatically.

#### US-020: Explain findings and suppressions with `why`

**Description:** As a developer, I want to explain a source location so that I understand why a rule fired, did not fire or was suppressed.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-003, US-007, US-018

**Acceptance Criteria:**

- [ ] `why <file:line[:column]>` reuses normal project discovery, configuration and scoped analysis.
- [ ] It shows every intersecting finding, the matched evidence, analyzer confidence, severity resolution chain and applicable fix.
- [ ] It explains Rust Doctor inline suppression, config suppression, test-context downgrade, framework gating and scope exclusion.
- [ ] Multiple findings at one location are ordered deterministically and are individually addressable by canonical ID.
- [ ] Given an incomplete required pass, the explanation names unavailable evidence and never states that a rule is clean; unavailable optional checks do not falsely make unrelated rule evidence inconclusive.
- [ ] Dynamic external namespaces and framework packs use the same fallback and version/feature/target gate reasons as normal scanning.
- [ ] Given an invalid path, out-of-root path, zero line, unreadable file or location outside the file, the command returns a typed error without scanning unrelated roots.

#### US-021: Install and remove agent skills and hooks

**Description:** As an agent user, I want one idempotent installer so that Rust Doctor is available in my coding workflow without manual configuration.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** Blocked by US-008, US-017, US-020

**Acceptance Criteria:**

- [ ] `install`, with `setup` as an alias, detects Claude Code, Cursor, Codex, OpenCode and Windsurf using a data-driven agent registry.
- [ ] Interactive, `--yes` and `--dry-run` modes can install the skill, existing MCP configuration and optional staged pre-commit or native agent hooks.
- [ ] Package-manager integration may add a version-pinned development dependency and script only after an explicit preview; non-JavaScript Rust projects remain package-manager independent.
- [ ] Generated hooks invoke staged scope, have configurable blocking level and use a namespace marker instead of replacing unrelated hook content.
- [ ] Repeated installation is idempotent and `uninstall` removes only Rust Doctor-owned files or marked blocks.
- [ ] Existing files are backed up before the first mutation and restored if a multi-file installation fails.
- [ ] Given no supported agent, an unwritable destination or an existing conflicting namespace, the installer reports exact paths and performs no partial installation.
- [ ] No installer path escapes the project root or the detected agent-owned configuration directory.

#### US-037: Expose a versioned programmatic scan API

**Description:** As a Rust tool author, I want a stable crate API equivalent to React Doctor's programmatic API so that I can embed scans without reconstructing CLI internals.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-004, US-007, US-010, US-011

**Acceptance Criteria:**

- [ ] The public crate API scans one project or a bounded batch and returns Report V1 without terminal rendering, process exit or implicit network access.
- [ ] Callers can supply typed global and per-project configuration overrides, scope, deadline, cancellation and adapter policy without mutating process-global state.
- [ ] Batch results preserve input order, isolate project failures, expose a deterministic aggregate and never discard successful project reports because one project failed.
- [ ] Public cache invalidation and report conversion do not expose internal mutable cache representation.
- [ ] The API documents thread safety, cancellation, side effects, semver compatibility and typed error behavior.
- [ ] CLI and MCP are thin consumers of the same public orchestration seam or pass an executable equivalence suite against it.

#### US-038: Produce diagnostic dumps and agent handoffs

**Description:** As an agent-assisted maintainer, I want a bounded diagnostic handoff so that the scan becomes executable work instead of terminal-only output.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-005, US-017, US-021

**Acceptance Criteria:**

- [ ] Every interactive scan with findings writes `diagnostics.json` plus deterministic per-rule text groups to a temporary directory or explicit `--output-dir` without changing stdout/stderr contracts.
- [ ] The handoff payload contains at most the three highest-priority rule groups inline and references the complete dump for the remaining findings.
- [ ] Interactive users may select a detected agent or clipboard target; CI, JSON, score, non-TTY and non-interactive runs never prompt.
- [ ] Remembered target preference contains no project identifier and can be reset; declining a handoff has no effect on future scan correctness.
- [ ] Paths, source excerpts and secrets are bounded and redacted according to the canonical diagnostic policy.
- [ ] Failure to write or deliver a handoff never changes the already-computed scan report or gate exit and returns an actionable secondary warning.

---

### EP-005: CI, Distribution and Editor Surfaces

Bring baseline semantics to team review, prove every published artifact and provide low-latency diagnostics in VS Code-compatible editors and Zed.

**Definition of Done:** Managed CI produces stable introduced-only feedback, all five binary targets pass packed smoke tests, LSP behavior is protocol-correct, and both editor adapters launch a compatible server.

#### US-022: Manage CI installation, configuration and upgrades

**Description:** As a team maintainer, I want Rust Doctor to manage its CI scaffold so that workflow setup and upgrades do not require copying YAML from documentation.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** Blocked by US-017, US-021

**Acceptance Criteria:**

- [ ] `ci install` creates a least-privilege GitHub Actions workflow with configurable scope, blocking, comment, review-comment and commit-status settings.
- [ ] `ci config` reads and changes only Rust Doctor-owned workflow fields; `ci upgrade` updates the action major and required permissions.
- [ ] GitHub Actions is fully supported and GitLab CI receives a documented gate-only scaffold.
- [ ] The GitLab scaffold selects baseline scope only for merge requests with a non-empty base SHA and uses a valid documented non-merge-request scope otherwise.
- [ ] `--dry-run` shows the workflow diff; an explicit `--pr` path may create a branch and pull request only after local validation succeeds.
- [ ] `--pr` is transactionally failure-injected: push or pull-request creation failure restores the original branch and local files, and reports any remote branch that could not be removed.
- [ ] Workflow ownership uses stable markers and preserves unrelated jobs, comments and formatting outside the managed block.
- [ ] Given an unclean conflicting file, missing Git repository, unavailable provider CLI, invalid permission request or authentication failure, no remote mutation occurs and local work is preserved.
- [ ] Generated PR text uses non-closing issue references and contains no agent attribution.

#### US-023: Upgrade the GitHub Action scan and cache contract

**Description:** As a CI user, I want cached, correct baseline scans on pull requests so that feedback is incremental and repeatable.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** Blocked by US-009, US-011, US-016, US-017

**Acceptance Criteria:**

- [ ] Action inputs cover directory, project, scope, blocking, require-complete, comment, review comments, commit status, SARIF and version.
- [ ] Pull-request changed paths are resolved locally with the base SHA, with a paginated GitHub API fallback only when history is unreachable.
- [ ] API fallback preserves repository-relative NUL-safe paths, passes them through the CLI's supported changed-files transport and cannot relabel a file-filter scan as introduced-only baseline output.
- [ ] Changes to Rust source, `Cargo.toml`, `Cargo.lock`, Rust Doctor configuration or build scripts select all affected package checks; irrelevant-only changes report skipped.
- [ ] A skipped irrelevant run still emits a schema-valid `nothing_to_scan` or documented skipped Report V1 outcome shared with the Rust enum; shell code cannot invent an out-of-schema outcome.
- [ ] Prebuilt tool installation and scan caches are keyed by resolved Rust Doctor version, OS, architecture, compiler, configuration and ruleset fingerprints.
- [ ] Cache restoration cannot turn a stale result into a hit because every entry revalidates its internal schema and content fingerprint.
- [ ] Subdirectory checkouts, absolute directories, nested repositories, forks and detached HEAD have integration fixtures.
- [ ] Given shallow history or denied pull-request API access, the Action exposes degraded scope and never silently reports introduced-only results.

#### US-024: Publish sticky PR summaries, inline findings and stable SARIF

**Description:** As a reviewer, I want one concise PR summary plus precise changed-line findings so that Rust Doctor participates in normal code review.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** Blocked by US-003, US-005, US-023

**Acceptance Criteria:**

- [ ] A hidden stable marker updates one existing PR summary comment instead of creating duplicates.
- [ ] The summary reports completeness, score, introduced, fixed, error and warning counts, affected packages and top rule groups.
- [ ] Inline review comments are limited to diagnostics intersecting changed lines, deduplicated by canonical ID and capped at 50 per run with overflow summarized.
- [ ] Changed-line matching accepts added lines and valid RIGHT-side context lines, publishes the new review before retiring the old one, and preserves prior inline truth when the replacement report failed or is incomplete.
- [ ] Commit status distinguishes passed, blocked, incomplete and skipped outcomes and links to the workflow run.
- [ ] SARIF uses canonical rule IDs, normalized paths, source ranges and `partialFingerprints` derived from the diagnostic identity contract.
- [ ] Markdown, control characters, rule messages and file paths including tabs and newlines are transported without line-oriented shell parsing and escaped before GitHub API submission.
- [ ] Paths from nested Cargo roots are rebased to the checkout root before SARIF or GitHub API submission.
- [ ] Given missing comment, review, status or security permissions, each reporting channel skips independently with a warning while the scan and configured quality gate remain authoritative.

#### US-025: Make release and package publication reproducible

**Description:** As a release owner, I want one version source and packed installation tests so that crates.io, npm and GitHub assets cannot drift.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** Blocked by US-016, US-023, US-024

**Acceptance Criteria:**

- [ ] Cargo package version is the release source of truth and all npm manifests are generated or validated from it before packaging.
- [ ] Release builds cover Linux x64, Linux arm64, macOS x64, macOS arm64 and Windows x64.
- [ ] Every archive has a SHA-256 checksum and the release includes one machine-readable checksum manifest.
- [ ] Each platform package and the main npm wrapper are packed and installed in an isolated directory with Bun before publication; `rust-doctor --version` must match the tag.
- [ ] Final compressed archives are extracted and their contained binaries executed after checksums are produced; the crates.io package is built and tested from the generated `.crate` without `--no-verify` bypass.
- [ ] Publication is idempotent after partial failure: already-published immutable artifacts are verified and skipped, mismatched artifacts abort.
- [ ] Action-major upgrade automation proposes a draft change after a successful release without auto-closing issues.
- [ ] Given a tag/version mismatch, missing archive, failed smoke test or registry inconsistency, publication stops before the dependent artifact is published.

#### US-026: Implement a feature-gated Rust Doctor language server

**Description:** As an editor user, I want low-latency custom-rule diagnostics so that issues appear while I edit without running global Cargo tools on every keystroke.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** Blocked by US-001, US-003, US-010, US-012

**Acceptance Criteria:**

- [ ] A feature-gated LSP 3.18 server supports initialize, shutdown, text synchronization, diagnostics, hover rule metadata and code actions for safe fixes or suppressions, and negotiates a Rust Doctor protocol major independently from binary semver.
- [ ] Open-document analysis runs the canonical file-local custom rules with normal config discovery and Cargo framework/version/feature/target context, with a 300 ms debounce and cancellation of superseded work.
- [ ] Source offsets convert correctly between Rust UTF-8 byte spans and LSP UTF-16 positions for multi-byte and multiline text.
- [ ] Optional on-save project analysis has an explicit budget and never invokes network-dependent adapters.
- [ ] Deadline expiration returns without awaiting blocked analysis; subprocess and blocking-task cleanup completes within the declared cancellation budget.
- [ ] Diagnostic `data` contains canonical ID and rule identity so code actions cannot apply to stale findings.
- [ ] P95 diagnostic publication is under 500 ms after debounce for a 10,000-line file on the benchmark host.
- [ ] Given temporarily invalid syntax, rapid edits or degraded analysis, the server preserves last-known-good diagnostics, overlays only proven current findings and exposes degraded state; close clears diagnostics, disconnect performs no write and no path panics.

#### US-027: Ship a VS Code-compatible editor adapter

**Description:** As a VS Code or Cursor user, I want an extension that manages the Rust Doctor language server so that setup requires no manual command configuration.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-026

**Acceptance Criteria:**

- [ ] The Bun-managed extension launches a bundled or explicitly configured Rust Doctor binary and negotiates a compatible protocol version.
- [ ] An absent `rust-doctor.toml` uses defaults rather than disabling diagnostics, while an explicitly configured missing path is an actionable error.
- [ ] Settings control enablement, debounce, on-save project checks, configuration path and trace logging.
- [ ] Commands expose scan workspace, explain selected diagnostic and open rule documentation.
- [ ] The extension never sends telemetry unless the separate observability opt-in is enabled.
- [ ] Package smoke tests cover VS Code and Cursor-compatible manifests plus Linux, macOS and Windows binary resolution.
- [ ] Packaged extension E2E launches the real server, rejects incompatible protocol majors and proves diagnostics with no project config.
- [ ] Given a missing, incompatible or non-executable binary, the extension disables diagnostics and shows one actionable error without restart loops.

#### US-028: Ship a Zed editor adapter

**Description:** As a Zed user, I want Rust Doctor diagnostics through the same server so that editor behavior stays consistent across my workflow.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-026

**Acceptance Criteria:**

- [ ] A Zed extension manifest launches the compatible Rust Doctor language server for Rust buffers and workspace roots.
- [ ] The default adapter omits a config-path argument when no config exists and negotiates the same protocol-major contract as the VS Code adapter.
- [ ] Settings map to the shared LSP configuration without inventing Zed-only rule semantics.
- [ ] Rule links, diagnostic severity, ranges and safe code actions match the VS Code adapter fixtures.
- [ ] The adapter resolves platform binaries from documented locations and reports its selected binary in diagnostics logs.
- [ ] Given an unsupported architecture, missing binary or protocol mismatch, the extension reports one actionable failure and does not repeatedly spawn the server.
- [ ] Packaging validation contains no hard-coded developer paths or unpublished local dependencies.
- [ ] Packaged adapter E2E launches the real server, rejects incompatible protocol majors and proves diagnostics with no project config.

---

### EP-006: Rust-Native Rule Moat and Public Product Loop

Use corpus evidence to expand beyond orchestration, then expose trustworthy rule knowledge, optional observability and shareable results publicly.

**Definition of Done:** Rule candidates are evidence-ranked, the selected stable analysis strategy is documented, at least two six-rule tranches and three framework capability packs pass corpus gates, public docs derive from the catalog, observability is an explicit opt-in, and public sharing requires an explicit stateless action.

#### US-029: Build an evidence-ranked Rust rule mining backlog

**Description:** As a product maintainer, I want rule candidates grounded in real Rust failures so that expansion targets costly problems rather than syntactic novelty.

**Priority:** P2
**Size:** L (5 pts)
**Dependencies:** Blocked by US-012, US-013, US-014

**Acceptance Criteria:**

- [ ] The backlog contains at least 100 candidates sourced from corpus findings, Clippy gaps, Rust API guidelines, RustSec patterns and high-signal project postmortems.
- [ ] Every candidate records user impact, positive and negative examples, existing-tool overlap, required analyzer, confidence ceiling, framework or version gates and false-positive risks.
- [ ] Candidates duplicating an enabled Clippy or compiler lint are rejected or explicitly justify additional behavior.
- [ ] A deterministic scoring rubric ranks impact, prevalence, detectability and expected precision and selects the top 20 for validation.
- [ ] Each accepted source is linked and each rejected candidate keeps a reason so mining work is not repeated.
- [ ] Given thin evidence, ambiguous semantics or a required unstable compiler hook, the candidate remains experimental and cannot enter a default-enabled tranche.

#### US-030: Validate the compiler-aware extension strategy

**Description:** As a maintainer, I want an evidence-backed decision on deeper type-aware rules so that Rust Doctor gains precision without accidental nightly lock-in.

**Priority:** P2
**Size:** L (5 pts)
**Dependencies:** Blocked by US-029

**Acceptance Criteria:**

- [ ] Five top-ranked candidates are prototyped or modeled against stable Clippy configuration, Cargo/rustc JSON, Dylint and a rustc-driver approach.
- [ ] The comparison measures supported Rust versions, build coupling, runtime, binary size, distribution complexity, unsafe requirements and diagnostic precision.
- [ ] Stable Clippy plus `syn` remains the default unless another approach supports MSRV 1.97, production `forbid(unsafe_code)` and all five release targets.
- [ ] Every normative document, generated scaffold, example and compatibility fixture uses MSRV 1.97; rule-specific historical version ranges remain only when they describe actual rule applicability.
- [ ] Any experimental backend is feature-gated and cannot affect default scans, scores or Report V1 semantics.
- [ ] The decision record includes a removal path and maintenance owner for every new backend dependency.
- [ ] Given no approach meeting the constraints, the story records a no-go decision and returns candidates to the backlog instead of weakening project invariants.

#### US-031: Ship the first reliability and concurrency rule tranche

**Description:** As a Rust maintainer, I want high-signal reliability and concurrency diagnostics so that expensive production failures are caught before review.

**Priority:** P2
**Size:** L (5 pts)
**Dependencies:** Blocked by US-001, US-002, US-012, US-013, US-029

**Acceptance Criteria:**

- [ ] The six highest-ranked eligible reliability, correctness, async or concurrency candidates from US-029 are implemented through the least complex validated analyzer.
- [ ] Every rule has catalog metadata, configuration, documentation, positive, negative, test-context, macro and malformed-syntax fixtures.
- [ ] No rule duplicates an enabled Clippy diagnostic on the same canonical evidence.
- [ ] Each rule passes mutation testing and the corpus delta gate with at most 2% confirmed false positives before default activation.
- [ ] Rules below the confidence threshold ship opt-in and are excluded from score and CI failure surfaces by default.
- [ ] Given a rule panic or unavailable analyzer, the scan records the failed rule and continues other rules without marking analysis complete.
- [ ] Rule execution returns structured per-rule receipts, so `catch_unwind` containment cannot turn a panic into an empty successful `Completed` pass.

#### US-032: Ship the first API, performance and security rule tranche

**Description:** As a library maintainer, I want high-signal public API, performance and security diagnostics so that regressions with broad blast radius are prioritized.

**Priority:** P2
**Size:** L (5 pts)
**Dependencies:** Blocked by US-001, US-002, US-012, US-013, US-029

**Acceptance Criteria:**

- [ ] The six highest-ranked eligible API design, allocation, unsafe-boundary or security candidates from US-029 are implemented.
- [ ] Rules distinguish library, binary, test, example, benchmark and build-script contexts where impact differs.
- [ ] Public API and semver findings use Cargo target kind, explicit target path, visibility and cfg evidence rather than filename or directory-name assumptions.
- [ ] Each rule passes mutation testing and the corpus delta gate with at most 2% confirmed false positives before default activation.
- [ ] Findings that require human security judgment are tagged heuristic and excluded from automatic fixes.
- [ ] Given macro-generated, external-crate or unresolved type evidence, the rule abstains or emits opt-in informational output rather than asserting a confirmed defect.

#### US-033: Add version-aware framework capability packs

**Description:** As a framework user, I want rules activated by actual dependency capabilities so that irrelevant or version-incompatible advice never appears.

**Priority:** P2
**Size:** L (5 pts)
**Dependencies:** Blocked by US-001, US-002, US-011, US-012, US-013, US-029

**Acceptance Criteria:**

- [ ] A capability model derives framework name, version, enabled Cargo features and target context from Cargo metadata.
- [ ] Target-specific dependencies are activated only when their cfg expression matches the analyzed target, not merely because a cfg string is present.
- [ ] The three highest-prevalence framework gaps not already covered are implemented as independently gated packs.
- [ ] Every framework rule declares supported semver ranges and required features in catalog metadata.
- [ ] Workspace packages with different dependency versions receive different active rule sets without cross-package leakage.
- [ ] Each pack passes its fixtures, mutations and corpus delta gate before default activation.
- [ ] Given an unknown version, renamed dependency, optional disabled feature, target mismatch or ambiguous re-export, the pack abstains and serializes its gating reason in Report V1, `why` and verbose metadata.

#### US-034: Generate the public documentation and rule catalog

**Description:** As a prospective user, I want accurate public documentation generated from the product contract so that installation and findings are understandable before adoption.

**Priority:** P2
**Size:** L (5 pts)
**Dependencies:** Blocked by US-001, US-005, US-018, US-025

**Acceptance Criteria:**

- [ ] The public site documents installation, CLI, agents, CI, Report V1, scoring limits, privacy, suppression, every rule and every analysis limitation.
- [ ] Rule pages and machine-readable catalog data are generated from `RuleDescriptor` and fail the build on undocumented descriptors.
- [ ] Documentation includes copyable full, staged, baseline, JSON, SARIF, MCP and CI examples verified by smoke fixtures.
- [ ] Static pages meet automated WCAG 2.2 AA checks and ship less than 200 KB of JavaScript on rule pages.
- [ ] Release notes and versioned report migration pages link to the matching published version.
- [ ] Given a broken internal link, stale generated catalog, missing rule limitation or invalid command example, the documentation build fails.
- [ ] The independent website build records the Rust Doctor binary SHA, catalog hash and command-replay artifact; CI verifies that external artifact rather than treating an absent local website as success.
- [ ] The website remains an independent Bun-managed surface and does not convert the Cargo package into a workspace.

#### US-035: Add privacy-safe opt-in observability

**Description:** As a maintainer, I want aggregate usage and failure signals with explicit consent so that product decisions use evidence without exposing source code.

**Priority:** P2
**Size:** L (5 pts)
**Dependencies:** Blocked by US-004, US-010, US-017, US-023, US-026

**Acceptance Criteria:**

- [ ] Local CLI, MCP, LSP and Action runs perform zero observability network requests by default.
- [ ] Opt-in events are schema-versioned and limited to tool version, platform, invocation surface, duration bucket, completeness, aggregate counts, pass states and suppression counts.
- [ ] Completeness is copied directly from Report V1 and suppression counts include inline, rule, category, tag, path and security-policy suppression sources.
- [ ] Raw paths, repository names, source text, diagnostic messages, Git remotes, environment values and command arguments are prohibited by schema tests.
- [ ] `--no-telemetry`, `RUST_DOCTOR_TELEMETRY=0` and offline mode override every stored opt-in.
- [ ] Crash reports scrub home and workspace prefixes before transmission and contain a local event ID users can quote.
- [ ] Given endpoint failure, timeout, invalid response or revoked consent, scanning behavior, output and exit code remain unchanged and queued events are discarded.
- [ ] Loopback and denied-network tests capture every request and prove both the prohibited-field schema and zero requests without explicit consent.
- [ ] The client persists only endpoint consent, retains no event queue after a delivery attempt and creates no cross-project persistent identifier.

#### US-036: Publish explicitly shared health reports

**Description:** As a project maintainer, I want an optional shareable health summary so that I can communicate progress without publishing source or sensitive diagnostics.

**Priority:** P2
**Size:** L (5 pts)
**Dependencies:** Blocked by US-004, US-005, US-025, US-034, US-035

**Acceptance Criteria:**

- [ ] `--share` requires explicit invocation, constructs the URL locally and performs no report upload.
- [ ] The default payload contains tool version, score, dimension scores, aggregate severity/category counts, completeness and skipped-check names, with no repository or diagnostic content.
- [ ] Completeness and score authority are copied from Report V1 without reconstructing or collapsing partial states.
- [ ] A versioned, percent-encoded query string contains the complete shared payload; no database, opaque identifier, expiry, deletion token or report backend is required.
- [ ] Generated URLs are capped at 8 KiB and skipped-check values are bounded canonical check names rather than reasons or arbitrary text.
- [ ] A real CLI integration test covers percent encoding, maximum cardinalities and an oversized payload without leaking a partial URL.
- [ ] The public page displays completeness, score authority and heuristic limitations as prominently as the score, and states that stateless values are self-reported and editable.
- [ ] Given nothing to scan, a machine-output conflict, invalid payload or an oversized URL, no share URL is emitted and the already-rendered local scan result remains available.
- [ ] Rust Doctor's own release workflow may publish a self-scan only through the same explicit sanitized path.

## Functional Requirements

- FR-01: One canonical catalog must govern rule metadata, aliases and documentation.
- FR-02: Rule, category, tag, path and surface configuration must execute with deterministic precedence.
- FR-03: Every finding must have a canonical provider/rule identity and a stable normalized source model.
- FR-04: Every machine scan must emit or map to versioned Report V1.
- FR-05: JSON, SARIF, MCP, Action and LSP must consume the same canonical diagnostic semantics.
- FR-06: Scan scopes must include full, files, changed, lines, staged and baseline behavior.
- FR-07: Staged scans must inspect index content, not divergent working-tree content.
- FR-08: Baseline reports must expose introduced, fixed and degraded-comparison states.
- FR-09: Every report must account for planned files, analyzed files and every check state.
- FR-10: Workspace reports must preserve package ownership and existing score weights.
- FR-11: Every custom rule must pass conformance and adversarial mutation coverage.
- FR-12: Corpus records must be pinned, schema-valid, sandboxed and comparable.
- FR-13: Diagnostic, completeness and performance regressions must have blocking thresholds.
- FR-14: CLI users must be able to list, explain and atomically configure rules.
- FR-15: `why` must explain findings, suppressions, gating and missing evidence at a source location.
- FR-16: Agent and hook installation must be idempotent, reversible and namespace-scoped.
- FR-17: CI installation and upgrades must preserve unrelated workflow content.
- FR-18: The official Action must prefer introduced-only PR feedback and fail safely on incomplete analysis.
- FR-19: PR comments, review comments, statuses and SARIF must deduplicate through stable identities.
- FR-20: Every published binary and npm wrapper must pass packed cross-platform smoke tests.
- FR-21: LSP must provide cancellable file-local diagnostics without network-dependent passes.
- FR-22: VS Code-compatible and Zed adapters must launch the same versioned LSP contract.
- FR-23: Rule mining must rank evidence and reject overlap before implementation.
- FR-24: Compiler-aware expansion must preserve stable Rust, MSRV and production safety by default.
- FR-25: New rule tranches and framework packs must pass corpus false-positive gates.
- FR-26: Public rule documentation must derive from the canonical catalog.
- FR-27: Observability and report sharing must be disabled by default and exclude source-identifying data.
- FR-28: A versioned public crate API must expose single-project and bounded batch scans with typed overrides, cancellation and partial results.
- FR-29: Interactive scans must support a bounded diagnostic dump and agent handoff without changing machine output or gate semantics.
- FR-30: Every React Doctor reference behavior must be classified as Applicable, Intentional Rust divergence or Not applicable, with executable evidence for the first two classes.

## Non-Functional Requirements

- **Performance:** Custom-rule files scope touching at most 10% of project files completes in at most 25% of the full custom-rule phase P95 on the same benchmark fixture.
- **Performance:** LSP diagnostics publish within 500 ms P95 after a 300 ms debounce for a 10,000-line file in R2 and within 300 ms P95 by Month 6.
- **Performance:** Candidate median and P95 scan regressions greater than 10% against the approved fixture baseline block release.
- **Performance:** A 100,000-line custom-rule scan uses less than 512 MB peak RSS on the benchmark host.
- **Performance:** The project cache defaults to a 512 MB maximum and performs LRU eviction before exceeding 563 MB (10% transient allowance).
- **Reliability:** 100% of machine reports expose completeness and check states; zero incomplete required scans serialize as clean.
- **Reliability:** Cancellation terminates supported child process groups and removes temporary scan roots within 2 seconds.
- **Reliability:** Each custom rule survives 1,000 mutations in R1 and 10,000 by Month 6 with zero uncaught panics.
- **Reliability:** One completeness computation and one surface classifier are reused by every consumer without post-construction report mutation.
- **Compatibility:** CI validates MSRV 1.97 and current stable Rust on Linux; release smoke covers five declared OS/architecture targets.
- **Compatibility:** Public crate API and Rust Doctor LSP protocol majors have explicit compatibility fixtures independent from CLI binary semver.
- **Compatibility:** Report V1 removes or changes semantics for zero existing fields; incompatible changes require a new schema version.
- **Security:** Corpus scan sandboxes receive zero host secrets, zero host-home write access and zero scan-phase network access.
- **Security:** Observability schemas allow zero raw paths, repository names, source text, diagnostic messages, Git remotes or environment values.
- **Security:** GitHub Actions are pinned to immutable commit SHAs before R2 release.
- **Evaluation integrity:** Candidate and baseline corpus, delta and benchmark runs use identical protected profiles, roots, toolchain/host class and immutable input fingerprints.
- **Scalability:** Workspace scheduling supports 200 members with active root scans bounded to available processors and no deadlock.
- **Scalability:** The evaluation runner supports 1,000 repositories and 2,500 project roots within a configurable 6-hour global budget by Month 6.
- **Accessibility:** Public documentation passes automated WCAG 2.2 AA checks with zero critical violations.

## Edge Cases & Error States

| # | Scenario | Trigger | Expected Behavior | User Message |
|---|----------|---------|-------------------|--------------|
| 1 | No eligible Rust files | Empty project or irrelevant diff | Complete `nothing_to_scan` report with null score | "No Rust source files matched this scope." |
| 2 | Invalid configuration | Unknown rule, level, threshold or malformed TOML | Typed setup error, no scan or file mutation | "Invalid Rust Doctor configuration: {reason}." |
| 3 | Missing optional adapter | cargo-geiger or another optional tool absent | Explicit skipped check; required-core completeness preserved | "{tool} was not scheduled because it is unavailable." |
| 4 | Missing required adapter | User or CI marks a tool required | Incomplete report and gate failure | "Required check {tool} could not run." |
| 5 | Compiler failure | Project does not compile | Preserve compiler diagnostics, mark Clippy incomplete | "Compiler analysis did not complete." |
| 6 | Deadline or cancellation | Global budget expires or client cancels | Stop new work, kill process groups, keep proven findings | "Scan stopped before all required checks completed." |
| 7 | Shallow baseline | Base commit cannot be resolved | Degrade to files scope and expose reason | "Baseline unavailable; reporting all findings in changed files." |
| 8 | Rename, copy or deletion | Git path topology changes | Conservative multiset evidence matching | N/A |
| 9 | Divergent staged file | Index and working tree differ | Analyze index snapshot only | "Scanning staged content." |
| 10 | Index conflict | Unmerged stages exist | Abort staged scan without worktree fallback | "Resolve Git index conflicts before a staged scan." |
| 11 | Unusual filename | Unicode, tab, newline or case-only path | Preserve NUL-delimited identity and normalized output | N/A |
| 12 | Macro or external span | Primary rustc span is not local | Use local call site or project-level diagnostic | "No local source span is available." |
| 13 | Virtual or 200-member workspace | Large or rootless Cargo graph | Bounded package scheduling and per-package results | N/A |
| 14 | File changes during scan | Editor or generator rewrites source | Mark affected file stale or re-read once within budget | "Source changed during analysis; result is incomplete." |
| 15 | Corrupt or future cache | Invalid JSON, schema or fingerprint | Ignore entry, recompute, atomically replace | "Ignoring incompatible scan cache entry." |
| 16 | GitHub fork permissions | Comment or status token lacks rights | Skip only denied channels, preserve scan gate | "GitHub reporting permission unavailable." |
| 17 | GitHub rate limit | Inline comments exceed API budget | Cap at 50 and summarize overflow | "Additional findings are available in the full report." |
| 18 | Malicious corpus project | Build script, symlink or output abuse | Sandbox containment and structured failure | "Evaluation sandbox rejected unsafe project behavior." |
| 19 | Invalid unsaved editor syntax | Partial code during typing | Preserve last-known-good diagnostics, overlay proven current findings and mark degraded | "Analysis is degraded until the document parses." |
| 20 | Release partial publication | Registry fails after some immutable artifacts publish | Verify existing artifacts and resume idempotently | "Release paused after partial publication; no artifact was overwritten." |
| 21 | Observability outage | Opted-in endpoint times out | Drop event without affecting scan | No user-facing message unless debug logging is enabled |
| 22 | Share rejection | Nothing to scan, oversized URL or invalid sanitized payload | Emit no share URL and retain local output | "Share URL was not created: {reason}." |
| 23 | Custom rule panic | A contained rule unwinds | Continue independent rules, record the failed rule and make completeness false | "Rule {rule} failed; remaining diagnostics are partial." |
| 24 | Manifest-only scope | Only Cargo/config/build metadata changed | Schedule applicable package/workspace checks; never infer nothing-to-scan from Rust files alone | N/A |
| 25 | Staged config drift | Index and worktree policies differ | Use a complete indexed policy or refuse the mixed snapshot | "Staged configuration differs from the worktree." |
| 26 | Inter-package rename | Source moves between workspace members | Execute global checks for old and new owners and report against existing head paths | N/A |
| 27 | Corpus root mismatch | Prepared, attempted and reported root sets differ | Fail the corpus run before delta calculation | "Evaluation did not cover every pinned Cargo root." |
| 28 | LSP protocol mismatch | Adapter and server protocol majors differ | Reject startup once with an actionable compatibility message | "Rust Doctor editor protocol is incompatible." |
| 29 | Closed output pipe | Downstream process exits early | Stop rendering cleanly without panic or corrupt JSON | N/A |

## Risks & Mitigations

| # | Risk | Probability | Impact | Mitigation |
|---|------|------------|--------|------------|
| 1 | Thirty-eight stories create coordination drift | High | High | Three release gates, dependency-ordered epics, one consolidated validation pass per epic and mandatory review before the next release |
| 2 | Baseline evidence falsely matches or duplicates moved findings | Medium | High | Multiset matching, same-file strict priority, conservative ambiguity, labeled rename/move fixtures and 99.5% target |
| 3 | New heuristic rules damage trust through false positives | High | High | Opt-in default, rule conformance, mutation harness, corpus delta gate and less than or equal to 2% promotion threshold |
| 4 | Untrusted repositories execute build scripts during evaluation | High | High | Credential-free disposable sandbox, scan-phase network denial, output caps, timeouts and no host-home writes |
| 5 | Compiler or Cargo JSON evolves | Medium | Medium | Stable documented interfaces, explicit metadata version, additive parsing and unknown enum states |
| 6 | LSP duplicates rust-analyzer diagnostics or consumes excessive CPU | Medium | Medium | File-local Rust Doctor rules by default, debounce, cancellation, optional on-save project checks and performance gate |
| 7 | CI comments or SARIF create duplicate findings | Medium | Medium | Canonical IDs, stable hidden comment marker, normalized paths, partial fingerprints and repeated-run fixtures |
| 8 | Multi-registry release fails halfway | Medium | High | Generated version contract, pre-publish packed smokes, checksums, immutable artifact verification and idempotent resume |
| 9 | Telemetry harms Rust community trust | Medium | High | Zero network by default, explicit opt-in, prohibited-field schema tests, no durable event queue and no persistent cross-project ID |
| 10 | Single crate becomes shallow or hard to navigate | Medium | Medium | Deep internal modules, feature-gated binaries, public-surface review and a workspace split only after measured interface pressure |

## Non-Goals

Explicit boundaries for this program:

- Hosted remote source-code scanning. All analysis remains local or inside user-controlled CI and evaluation sandboxes.
- Dynamic third-party rule plugins in R1 through R3. Catalog adapters are internal and curated.
- Changing score dimensions, weights or labels. That requires a separate explicitly approved scoring decision.
- Making nightly rustc-private or Dylint infrastructure part of the default install unless US-030 proves every hard constraint.
- Splitting the Cargo package into a workspace. Feature-gated binaries and internal modules are sufficient for this plan.
- Automatically applying heuristic or `MaybeIncorrect` fixes. Only machine-applicable edits with validated source identity may run automatically.
- Reproducing React-specific design, accessibility or framework rules that have no Rust analogue.
- Browser automation during implementation unless Arthur explicitly authorizes it in that task.

## Files NOT to Modify

- `tasks/prd-best-practices.md` - completed historical PRD with a separate scope
- `tasks/prd-best-practices-status.json` - completed historical status tracker
- `src/output/score.rs` - score weights are outside this PRD and require explicit approval
- `tests/snapshots/*.snap` - snapshots must be changed through the normal Insta review workflow, not edited manually
- `Cargo.lock` - may change through Cargo dependency resolution but must not be hand-edited
- `rust-doctor-video/` - promotional video project is unrelated to product parity
- `assets/demo.gif`, `assets/demo.mp4` and `assets/social-card.png` - existing marketing assets are not implementation surfaces for this PRD

## Technical Considerations

- **Architecture:** Should the catalog expose one enum-backed registry with namespace fallbacks or separate registries behind an interface? Recommended: one external catalog interface with internal adapters so callers learn one model. Engineering must confirm initialization and compile-time validation.
- **Diagnostic identity:** Should stable hashes use SHA-256 through a small dependency or another deterministic hash? Recommended: SHA-256 because the contract is portable and aligns with existing ecosystem fingerprints. Engineering must confirm binary-size impact.
- **Report compatibility:** Should Report V1 be a DTO separate from `ScanResult` or make `ScanResult` itself the wire type? Recommended: a separate wire DTO so internal scan evolution cannot change external JSON accidentally.
- **Configuration editing:** Should mutation commands use `toml_edit` or a narrower writer? Recommended: `toml_edit` for comment preservation and atomic replacement, subject to MSRV and dependency review.
- **Baseline materialization:** Should base and staged compiler scans use temporary worktrees, Cargo package reconstruction or blob-level virtual files? Recommended: isolated temporary trees because Cargo and build scripts require filesystem context. US-006 must validate containment and performance.
- **Compiler-aware rules:** Should Rust Doctor adopt Dylint or rustc-driver internals? Recommended: keep stable Clippy and `syn` as defaults; US-030 may authorize only a feature-gated backend that satisfies every platform and safety constraint.
- **LSP implementation:** Should the server use a protocol crate or a higher-level async framework? Recommended: the smallest maintained crate that supports cancellation, UTF-16 positions and LSP 3.18 without forcing MCP dependencies into no-default-feature builds.
- **Action implementation:** Should GitHub reporting remain shell plus GitHub Script or move into a Rust subcommand? Recommended: keep the Action orchestration thin and derive every decision from Report V1; engineering should compare testability and startup cost.
- **Website location:** Decision: public documentation, generated rule pages and stateless share reports live in the independent `rust-doctor-web` repository. This repository owns the canonical CLI catalog; the website snapshots it through `rust-doctor rules list --json` and gates drift with an explicit `RUST_DOCTOR_BIN` input.
- **Observability provider:** Should opt-in events use a managed provider or a minimal first-party endpoint? Decision: keep the client provider-neutral and require the user to consent to an explicit HTTPS endpoint; Rust Doctor bundles no collector.
- **Migration:** Existing `--diff`, `--fail-on` and `rules_config` behavior needs one minor-release compatibility window. Rollback is removal of new consumers while retaining legacy fields and aliases until V1 adoption is measured.

## Success Metrics

| Metric | Baseline (current) | Target | Timeframe | How Measured |
|--------|-------------------|--------|-----------|-------------|
| Declared per-rule controls applied | Partially implemented; fail-closed and consumer-surface gaps remain | 100% of canonical controls | R1 release | Configuration and consumer-surface integration matrix |
| Versioned report schemas | Report V1 exists; current tests validate only structural proxies | 1 active V1 schema with 100% standards-compliant instance validation | R1 release | Built CLI JSON and Action smoke suite |
| Explicit user scan modes | All CLI modes exist; MCP, manifest-only, staged-policy and degradation semantics diverge | Full, files, lines, staged and baseline equivalent across CLI, API and MCP | R1 release | Git repository and cross-surface integration fixtures |
| Baseline classification accuracy | N/A | At least 99.5% across 5,000 labeled pairs | Month 6 | Delta fixture and corpus label set |
| Reports exposing required completeness | Fields exist; gate, telemetry and sharing recompute weaker predicates | 100% through one normative computation | R1 release | Schema validation and incomplete-path tests |
| External evaluation repositories | Manifest exists; exact Cargo-root execution and protected result artifacts are unproven | 100 in R1, 1,000 by Month 6 | R1 and Month 6 | Pinned corpus root-set equality and CI artifact |
| Uncaught custom-rule mutation panics | 1,000 mutations per rule run, without semantic broadening oracle | 0 failures and required liveness/fire-coverage | R1 release | Seeded isolated mutation harness |
| Confirmed false positives for promoted rules | Not measured | Less than or equal to 2% in R1, 1% by Month 6 | R1 and Month 6 | Reviewed diagnostic samples |
| Duplicate PR comments or SARIF alerts | Not measured | 0 across 100 repeated-run fixtures | R2 release | Action and SARIF replay tests |
| Packed platform install success | 5 native binaries smoke before archive; final archive and `.crate` contents unproven | 5 of 5 final artifacts | R2 release | Extracted release matrix smoke jobs |
| Editor diagnostic latency | LSP exists; hard cancellation and packaged-adapter E2E are absent | P95 under 500 ms after debounce | R2 release | Fixed 10,000-line LSP benchmark and protocol E2E |
| Public rule pages generated from catalog | 0 | 100% of descriptors | R3 release | Documentation build completeness gate |
| Default network requests per local scan | 0 | 0 | Every release | Network-denial integration test |
| Public programmatic scan API | Low-level internal orchestration only | Single and batch Report V1 API with partial-error and cancellation fixtures | R2 release | External-crate API compatibility suite |
| Agent diagnostic handoff | Skill/setup exists; no React-equivalent post-scan dump and handoff | Deterministic dump and bounded handoff on every eligible interactive scan | R2 release | CLI PTY and non-interactive fixtures |
| Applicable React Doctor behaviors classified | Ad hoc comparison | 100% classified and executable | Final parity certification | Versioned parity matrix tied to both repository SHAs |

## Open Questions

- **Compiler-aware backend:** Owner: maintainer. Decision point: completion of US-030 before US-031. Default if unresolved: stable Clippy plus `syn` only.
- **Website repository location:** Resolved in US-034. Publish to the existing `rust-doctor-web` homepage repository without moving Cargo code or creating a nested website project here.
- **Observability provider:** Resolved in US-035. The client stores explicit consent for an operator-selected HTTPS endpoint and bundles no provider.
- **Corpus execution capacity:** Owner: maintainer. Decision point: expansion beyond 100 repositories. Default if unresolved: nightly GitHub Actions sample plus manually triggered full local run.
- **Public score retention:** Resolved in US-036. Reports are never stored by Rust Doctor; the complete self-reported summary lives in the share URL.
[/PRD]
