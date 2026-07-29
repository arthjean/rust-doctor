[PRD]
# PRD: Diagnostic Trust and Rust-Native Analysis Parity

**Document Status:** Ready for implementation  
**Author:** Arthur Jean  
**Created:** 2026-07-27  
**Last Updated:** 2026-07-27  
**Related Program:** `tasks/prd-react-doctor-parity.md`

## Changelog

| Date | Author | Change |
|---|---|---|
| 2026-07-27 | Arthur Jean | Initial complete PRD |

## Problem

Rust Doctor already has broad product surfaces and a substantial analyzer catalog, but its health score is more confident than its evidence warrants.

The current scanner can report hundreds of findings while returning a near-perfect score because scoring counts unique violated rules rather than occurrence burden, priority, evidence quality, analyzer coverage, or whether a dimension was actually observed. Custom `syn` heuristics default to medium confidence and many remain score-visible without a measured precision and recall contract. External analyzers can be absent, time out, or return partial evidence without the score expressing the resulting uncertainty. Ranking and grouping are not yet governed by one canonical decision model across terminal, JSON, SARIF, MCP, CI, plans, and handoffs.

This creates four product failures:

1. **False certainty:** an unavailable or incomplete analyzer can be indistinguishable from a healthy dimension.
2. **Weak calibration:** a heuristic can affect the score without a labeled opportunity set demonstrating both precision and recall.
3. **Poor prioritization:** severity, confidence, score impact, recurrence, and root cause are not separated cleanly.
4. **Inconsistent decisions:** consumers can order or group the same diagnostics differently.

React Doctor's product advantage is not the React-specific rule catalog alone. It is a coherent decision system: deterministic diagnostics, curated defaults, framework-aware applicability, stable rule identities, understandable fixes, changed-code workflows, and a score users can use as a compact health signal. Product parity for Rust Doctor therefore means equivalent decision quality for Rust codebases, not copying React Doctor's formula or maximizing rule count.

**Why now:** The preceding parity program has delivered the catalog, Report V1, completeness, evaluation, conformance, configuration, and integration foundations needed to enforce a trust contract without rebuilding the product. Continuing to expand rules before calibration would compound noisy defaults, make Score V2 migration harder, and weaken every downstream surface that now consumes the canonical report.

## Overview

This program turns Rust Doctor from a broad finding aggregator into an evidence-aware Rust code health system.

The implementation will:

- establish a versioned truth dataset containing positive opportunities, negative contexts, emitted findings, and missed findings;
- make analyzer authority, trust tier, score eligibility, required evidence, aggregation policy, and calibration version explicit in the rule catalog;
- represent dimension coverage and scan authority in Report V1;
- introduce a versioned Score Core V2 while preserving the five current dimensions and their approved weights;
- requalify every default custom rule and curate compiler, Clippy, and external analyzer behavior;
- normalize external evidence without treating missing tools as clean results;
- enforce promotion and demotion gates using measured precision, recall, and context coverage;
- use one canonical priority and grouping contract across all product surfaces;
- certify the result on fixtures, the pinned public corpus, workspace cases, and degraded execution cases.

This PRD complements `tasks/prd-react-doctor-parity.md`. The earlier program remains the foundation for catalog, configuration, Report V1, completeness, evaluation, conformance, handoff, and integration surfaces. When the two documents disagree on diagnostic trust or workspace headline scoring, this PRD is the newer decision record.

## Normative Trust Contract

The following terms are normative:

- **Analyzer authority:** whether an analyzer produced sufficient evidence for its planned scope in the current scan.
- **Dimension coverage:** the proportion and status of planned evidence sources completed for a health dimension.
- **Trust tier:** the provenance class of a rule: compiler-proven, calibrated heuristic, advisory-backed, or audit-only.
- **Score eligibility:** an explicit catalog property granted only after the rule satisfies its trust-tier evidence contract.
- **Priority:** product urgency independent of presentation severity. Priorities are P0, P1, P2, and P3.
- **Aggregation policy:** how repeated findings from one rule influence ranking and score. Supported policies are root-cause, bounded-occurrence, unique-rule, and audit-only.
- **Abstention:** an analyzer's explicit refusal to emit a diagnostic because required context is unavailable or ambiguous.
- **Authoritative score:** a score whose required analyzers completed with sufficient coverage. A numeric score can be present while authority is false, but consumers must be able to distinguish it.
- **Core score:** the deterministic score produced from built-in, stable analyzers. Optional external adapters can add diagnostics and completeness state but cannot silently change the Core Score based only on local tool availability.

Trust tier, severity, confidence, priority, score eligibility, and category are separate fields. No field is inferred silently from another at report time.

Unknown or dynamically discovered rules default to score-ineligible and unranked until an explicit policy maps them.

## Goals

1. Make the score a defensible summary of observed Rust code health rather than a count of distinct rule identifiers.
2. Prevent incomplete scans and unavailable analyzers from appearing fully healthy.
3. Require measured precision and recall before a heuristic becomes default and score-eligible.
4. Preserve compiler, rustc, Cargo, and Clippy evidence as the highest-authority stable layer.
5. Reduce high-volume false positives by adding context, abstention, aggregation, promotion, and demotion mechanisms.
6. Provide one deterministic priority and root-cause order across terminal, JSON, SARIF, MCP, CI, plans, and handoffs.
7. Preserve local-first, offline-capable, privacy-safe scanning and the current CLI piping contract.
8. Deliver a migration path that does not silently compare legacy and Score Core V2 values.

## Target Users

### Primary: Rust maintainer

Maintains a library, service, CLI, embedded crate, or Cargo workspace. Pain point: hundreds of weakly ranked findings do not answer "what should I fix first?" Current workaround: run Clippy and dependency tools independently, inspect source context manually, and ignore rules that repeatedly misfire.

### Primary: Staff or platform engineer

Uses Rust Doctor across multiple repositories. Pain point: machine-dependent analyzer availability and opaque score changes undermine CI policy. Current workaround: maintain repository-specific lint scripts, SARIF adapters, allowlists, and score baselines.

### Secondary: Coding agent

Consumes Report V1, MCP tools, plans, and handoffs. Pain point: severity alone does not reveal evidence quality or fix safety. Current workaround: re-read source, infer applicability, and rebuild priority from diagnostic text before editing.

### Secondary: Security and dependency owner

Needs RustSec, cargo-deny, unsafe exposure, license, source, and dependency diagnostics to retain their distinct meanings. Pain point: collapsing them into one security severity creates false urgency and duplicate remediation. Current workaround: reopen each upstream tool report and deduplicate advisory or dependency paths manually.

## Research Findings

### Local Rust Doctor audit

- The current score counts unique violated rules per severity and dimension. Repeated occurrences and confidence do not affect the score.
- The self-scan can produce 734 diagnostics, including 8 errors, while returning 98/100. This is a direct calibration contradiction, regardless of whether the eight errors are later proven false positives.
- Eighteen medium-confidence `syn` rules are default-enabled across all six source surfaces without a uniform measured precision and recall qualification.
- High-volume rules such as excessive clone, unsafe dependency exposure, indexing, unwrap, and complexity dominate output and require different aggregation and applicability contracts.
- The catalog, policy precedence, completeness model, Report V1, evaluation corpus, conformance system, promotion workflow, and aggregate telemetry already provide the correct extension points.
- The current evaluation labels emitted findings but does not model positive opportunities and therefore cannot measure recall.
- Compiler and dependency adapters are excluded from the main evaluation corpus.
- The current workspace headline chooses the worst package score, while the earlier parity PRD described an aggregate workspace score. This PRD resolves the conflict in favor of worst-package headline scoring, with aggregate workspace health exposed separately.
- The Clippy adapter creates temporary configuration in the scanned repository. Official Clippy supports configuration isolation through `CLIPPY_CONF_DIR`.
- `cargo-geiger` measures unsafe exposure, not vulnerability. It must not be presented or scored as a confirmed security defect.

### React Doctor product model

React Doctor exposes a deterministic score, curated diagnostics, framework detection, changed-line workflows, monorepo support, rule explanation, configuration, JSON output, hooks, and CI integrations. Its relevant product lesson is consistent decision quality and progressive disclosure, not framework-specific rule equivalence.

Sources:

- [React Doctor documentation](https://www.react.doctor/docs)
- [React Doctor CLI reference](https://www.react.doctor/docs/reference/cli-reference)
- [React Doctor data use](https://www.react.doctor/docs/legal/data-use)

### Rust toolchain and ecosystem evidence

- Cargo and rustc emit stable JSON Lines through `--message-format=json`. Parsers must read line by line and tolerate non-JSON lines because build scripts, proc macros, and tools can write arbitrary output.
- Cargo workspace lint policy can be inherited from `[workspace.lints]` through a member's `[lints] workspace = true`; effective policy cannot be inferred from one manifest table alone.
- Cargo metadata consumers must request an explicit format version.
- Clippy contains more than 800 lints, but `pedantic`, `nursery`, and `restriction` are not suitable as undifferentiated default groups. Defaults require lint-level curation.
- GitHub SARIF consumers rely on stable rule identifiers, locations, severities, precision metadata, and fingerprints.
- SonarQube's quality-gate model reinforces two relevant principles: evaluate new code separately when possible and never equate an unexecuted check with a passed check.

Sources:

- [Clippy lint documentation](https://doc.rust-lang.org/stable/clippy/index.html)
- [Cargo external tools and JSON messages](https://doc.rust-lang.org/cargo/reference/external-tools.html)
- [cargo-deny](https://github.com/EmbarkStudios/cargo-deny)
- [GitHub SARIF support](https://docs.github.com/en/enterprise-cloud@latest/code-security/reference/code-scanning/sarif-files/sarif-support)
- [SonarQube quality gates](https://docs.sonarsource.com/sonarqube-server/quality-standards-administration/managing-quality-gates/introduction-to-quality-gates)

### Compiler-aware architecture boundary

`docs/adr/0002-compiler-aware-rule-strategy.md` remains binding. Stable Clippy, Cargo or rustc JSON, and `syn` are the default analyzer layers. Dylint, `rustc_driver`, MIR, nightly-only analyzers, and unsafe in-process compiler integrations are outside this program. A future deep backend must be feature-gated, absent from the default score, and justified by measured recall or precision that stable layers cannot reach.

## Assumptions & Constraints

1. Rust 2024 and MSRV 1.97 remain mandatory.
2. Rust Doctor remains a single crate with CLI, library, MCP server, npm distribution, and GitHub Action surfaces.
3. The five score dimensions and approved weights remain unchanged:
   - Security: 2.0
   - Reliability: 1.5
   - Maintainability: 1.0
   - Performance: 1.0
   - Dependencies: 1.0
4. Changing these weights requires Arthur's explicit approval and is not authorized by this PRD.
5. Report V1 evolves additively. Existing required fields are not removed or retyped.
6. Existing policy precedence remains CLI over TOML over Cargo metadata over defaults.
7. Diagnostics remain on stderr and the bare `--score` integer remains on stdout.
8. MCP tools remain read-only and retain their current path, timeout, offline, and sanitization hardening.
9. Custom rules remain protected by `catch_unwind`.
10. Missing optional tools degrade completeness and authority; they do not fail the entire scan unless explicitly required by policy.
11. Default scoring must not depend on whether an optional executable happens to be installed.
12. No source code, paths, messages, repository identifiers, or dependency graph is uploaded by telemetry.
13. Existing user modifications and historical task artifacts are preserved.
14. This program improves and requalifies current analyzers before adding new rule families.

## Quality Gates

These gates are defined once and apply to every implementation story:

```bash
cargo fmt --check
cargo +1.97 check --all-targets
cargo check --all-targets --all-features
cargo build
cargo build --no-default-features
cargo clippy --all-targets --all-features -- -W clippy::all -W clippy::pedantic -W clippy::nursery -D warnings
cargo test --all-features
```

For changes touching dependency policy or locked dependencies:

```bash
cargo audit
cargo deny check
```

Snapshot changes must be reviewed through `cargo insta review`. Snapshot files must never be edited manually.

## Epics & User Stories

### EP-001: Trustworthy Health Contract

**Objective:** Establish the evidence, metadata, coverage, invariants, and score model required for a trustworthy health signal.

**Definition of Done:**

- A versioned truth dataset measures positive opportunities, negative contexts, emitted findings, and missed findings.
- Every catalog rule has an explicit trust and aggregation contract.
- Report V1 exposes analyzer authority, dimension coverage, and Score Core version.
- Workspace headline and score invariants are unambiguous and protected by tests.
- Score Core V2 satisfies the calibration and compatibility criteria below without changing dimension weights.

#### US-001: Build the diagnostic truth dataset and baseline

**Priority:** P0  
**Size:** L  
**Dependencies:** None

**User Story:**  
As a scanner maintainer, I want a versioned labeled truth dataset so that rule and score decisions use measured evidence rather than intuition.

**Acceptance Criteria:**

- [ ] Given the ten highest-volume or highest-impact current custom rules, when the seed dataset is inspected, then each rule has at least 20 labeled positive opportunities and 20 labeled negative contexts distributed across applicable crate roles and source surfaces.
- [ ] Given each labeled case, when it is serialized, then it records rule ID, source fixture, opportunity location, expected applicability, expected emission, expected priority, context dimensions, label provenance, and reviewer state.
- [ ] Given the current scanner, when the baseline job runs, then it records per-rule true positives, false positives, false negatives, precision, recall, context coverage, emitted count, score contribution, and scan completeness.
- [ ] Given the current self-scan and pinned public corpus, when the baseline is generated, then its toolchain, corpus revision, configuration hash, rule-catalog hash, and score-model identifier make the result reproducible.
- [ ] Given an unknown, disputed, stale, or unreviewed label, when calibration runs, then that case is excluded from pass statistics and the affected rule is marked evidence-incomplete rather than implicitly passing.
- [ ] Given a fixture that no longer parses on MSRV 1.97, when validation runs, then the dataset job fails with the fixture and reason identified.

#### US-002: Add rule trust, priority, and aggregation metadata

**Priority:** P0  
**Size:** M  
**Dependencies:** Blocked by US-001

**User Story:**  
As a report consumer, I want every rule to declare how much it can be trusted and how it affects decisions so that severity is not used as a proxy for evidence.

**Acceptance Criteria:**

- [ ] Given any built-in catalog rule, when it is listed, then it exposes analyzer provenance, trust tier, priority, score eligibility, required evidence, aggregation policy, calibration version, supported contexts, and known limitations.
- [ ] Given catalog serialization, when rules are emitted through JSON or MCP, then severity, confidence, priority, trust tier, category, and score eligibility remain distinct fields.
- [ ] Given an unknown Clippy lint, dynamically discovered rule, or adapter diagnostic without an explicit mapping, when it is normalized, then it defaults to score-ineligible and unranked without being discarded.
- [ ] Given a score-eligible rule, when catalog validation runs, then missing calibration version, required evidence, or aggregation policy fails validation.
- [ ] Given configuration overrides, when effective policy is resolved, then one canonical catalog remains the source of default trust metadata and configuration changes activation or presentation without fabricating calibration evidence.
- [ ] Given an invalid trust-tier or aggregation value in project configuration, when configuration loads, then the scan reports a typed configuration error and does not silently coerce it.

#### US-003: Represent analyzer authority and dimension coverage

**Priority:** P0  
**Size:** L  
**Dependencies:** Blocked by US-002

**User Story:**  
As a maintainer, I want the report to distinguish healthy, unobserved, partial, and failed dimensions so that missing evidence never appears clean.

**Acceptance Criteria:**

- [ ] Given a scan plan, when Report V1 is produced, then every dimension records planned analyzers, scheduled analyzers, completed analyzers, skipped analyzers, failed analyzers, covered scope, authority state, and machine-readable reasons.
- [ ] Given a dimension with no completed authoritative analyzer, when its dimension score is calculated, then the value is unavailable rather than 100 and the overall score authority is false.
- [ ] Given an optional external adapter that is absent, when the default Core Score is calculated, then its numeric value is unchanged while completeness records the skipped optional evidence.
- [ ] Given an analyzer explicitly required by CLI or project policy, when it is missing, times out, or returns malformed output, then the affected dimension and overall score are non-authoritative and the quality gate can fail according to policy.
- [ ] Given an analyzer that completes only part of a workspace, when coverage is calculated, then completed and uncovered package or target scopes are both reported.
- [ ] Given an older Report V1 consumer, when it reads a report containing the additive coverage fields, then all prior required fields retain their type and meaning.
- [ ] Given an analyzer panic or process failure, when report normalization runs, then it records a failed receipt and never converts the failure into an empty successful result.

#### US-004: Resolve workspace headline and protect score invariants

**Priority:** P0  
**Size:** M  
**Dependencies:** Blocked by US-001, US-003

**User Story:**  
As a workspace owner, I want an unambiguous headline score and stable mathematical invariants so that repository health cannot improve through duplication, omission, or package averaging.

**Acceptance Criteria:**

- [ ] Given a multi-package workspace, when the headline Core Score is selected, then it equals the lowest authoritative package score; aggregate distribution and per-package scores are reported separately.
- [ ] Given the same normalized report, catalog, configuration, and score-model version, when scoring runs repeatedly or with different scan parallelism, then the numeric score, dimension scores, authority, and label are identical.
- [ ] Given one additional score-eligible diagnostic while all other evidence is unchanged, when scoring runs, then no affected package or dimension score increases.
- [ ] Given duplicate diagnostics with the same stable identity and source location, when deduplication and scoring run, then they contribute once.
- [ ] Given a score-ineligible, suppressed, audit-only, or out-of-scope diagnostic, when scoring runs, then it has zero score impact while remaining observable where policy permits.
- [ ] Given one package with a non-authoritative score, when the workspace headline is calculated, then workspace authority is false and the package cannot be hidden by healthy siblings.
- [ ] Given an empty workspace, metadata failure, or workspace containing no scannable target, when scoring runs, then no synthetic 100 is emitted.

#### US-005: Calibrate and implement Score Core V2

**Priority:** P0  
**Size:** L  
**Dependencies:** Blocked by US-001, US-002, US-003, US-004

**User Story:**  
As a Rust Doctor user, I want a versioned score that reflects priority, evidence, bounded recurrence, and coverage so that the health label matches the remediation decision.

**Acceptance Criteria:**

- [ ] Given the truth baseline, when candidate score models are compared, then the selected model is stored as a versioned machine-readable artifact containing exact penalties, thresholds, occurrence bounds, dimension normalization, and calibration evidence.
- [ ] Given Score Core V2, when a diagnostic contributes, then its impact is determined by explicit priority and aggregation policy and never by occurrence count without a configured bound.
- [ ] Given a confirmed P0 score-eligible diagnostic, when the package is scored, then its overall score cannot receive a Good or Great label.
- [ ] Given 100 repeated occurrences of one bounded-occurrence rule, when scoring runs, then their total penalty is no more than twice the first occurrence penalty unless the versioned model explicitly uses root-cause groups.
- [ ] Given an audit-only, uncalibrated, suppressed, or score-ineligible finding, when Score Core V2 runs, then the numeric contribution is exactly zero.
- [ ] Given partial required coverage, when Score Core V2 runs, then it exposes a numeric provisional score and non-authoritative state rather than inflating unobserved dimensions.
- [ ] Given legacy output and Score Core V2 output, when rendered, then Report V1 exposes `score_model_version`, caches include that version, and consumers are not invited to compare values across models without an explicit migration marker.
- [ ] Given `--score`, when the score is authoritative, then stdout remains one bare integer; when policy requires authority and authority is false, then the command exits through the existing quality-gate path without contaminating stdout.
- [ ] Given any candidate model that changes the five dimension weights, when validation runs, then the model is rejected pending explicit approval.

### EP-002: Evidence-Aware Analyzer Defaults

**Objective:** Make built-in and external analyzers context-aware, quiet by default, and honest about unavailable evidence.

**Definition of Done:**

- Custom rules can declare and receive the context needed to decide or abstain.
- Every currently default custom rule is retained, demoted, or disabled using measured evidence.
- Clippy and compiler diagnostics are parsed from stable structured fields without writing into the scanned repository.
- External analyzers preserve their distinct semantics and never report empty success on parser or process failure.

#### US-006: Expand rule context and explicit abstention

**Priority:** P0  
**Size:** L  
**Dependencies:** Blocked by US-002

**User Story:**  
As a rule author, I want reliable package, target, feature, framework, and source context so that heuristics emit only where their assumptions hold.

**Acceptance Criteria:**

- [ ] Given a custom rule execution, when context is available, then it can inspect package ID, target kind, crate role, source surface, edition, declared MSRV, enabled feature profile, detected frameworks, dependency capabilities, cfg profile, and generated or macro-origin status supported by stable evidence.
- [ ] Given a rule with required context, when that context is unavailable or ambiguous, then the rule emits an abstention receipt with a reason instead of a diagnostic.
- [ ] Given test, bench, example, build-script, proc-macro, and generated sources, when a rule declares unsupported contexts, then those sources are excluded before traversal and the exclusion contributes to coverage reporting.
- [ ] Given file-local `syn` evidence, when a rule requires type, trait-resolution, MIR, macro-expansion, or interprocedural knowledge, then catalog validation prevents it from being score-eligible unless a stable authoritative backend supplies that evidence.
- [ ] Given two workspace members with different metadata or features, when the same source pattern is analyzed, then each rule receives member-specific context.
- [ ] Given malformed metadata or unresolved target ownership, when context construction fails, then affected rules abstain and completeness degrades rather than inheriting another package's context.
- [ ] Given a panicking custom rule, when it executes with the expanded context, then `catch_unwind` still produces an isolated analyzer failure without crashing the scan.

#### US-007: Requalify current custom rules

**Priority:** P0  
**Size:** L  
**Dependencies:** Blocked by US-001, US-002, US-006

**User Story:**  
As a Rust maintainer, I want default custom diagnostics to be measured and context-appropriate so that the scanner is quiet enough to trust.

**Acceptance Criteria:**

- [ ] Given every currently default-enabled custom rule, when qualification runs, then it has a recorded decision of score-eligible default, non-scoring default, opt-in, or disabled with evidence and rationale.
- [ ] Given a heuristic seeking default score eligibility, when evaluated on its labeled opportunities, then false-positive rate is at most 2%, recall is at least 80%, it has at least 20 reviewed positive opportunities and 20 reviewed negative contexts, and required-context coverage is at least 90%.
- [ ] Given a rule that misses the precision threshold, when the catalog is generated, then it is demoted to opt-in or disabled and cannot affect score, CI failure, or the default top-priority list.
- [ ] Given a rule that meets precision but lacks enough positive opportunities to establish recall, when qualification runs, then it remains score-ineligible until evidence is sufficient.
- [ ] Given high-volume rules including excessive clone, unsafe dependency, indexing, unwrap, and complexity, when decisions are recorded, then each has an explicit aggregation policy and applicability contract rather than a shared blanket policy.
- [ ] Given a rule retained as non-scoring default, when it emits, then output labels it advisory and does not imply that its presence lowered the score.
- [ ] Given a future catalog change that makes an unqualified rule default and score-eligible, when protected validation runs, then CI fails.
- [ ] Given current findings located in fixtures, generated files, snapshots, or unsupported source surfaces, when requalification fixtures run, then the rule either proves applicability or abstains.

#### US-008: Curate and isolate Clippy and compiler analysis

**Priority:** P0  
**Size:** L  
**Dependencies:** Blocked by US-001, US-003

**User Story:**  
As a project owner, I want Rust Doctor to use compiler-grade evidence without mutating my repository or enabling noisy lint groups indiscriminately.

**Acceptance Criteria:**

- [ ] Given a Clippy scan, when temporary configuration is required, then Rust Doctor uses an isolated temporary directory and `CLIPPY_CONF_DIR`; no `clippy.toml` or other analyzer configuration is created in the scanned repository.
- [ ] Given the default lint profile, when the command is constructed, then correctness, suspicious, performance, and individually approved lints are explicit; pedantic, nursery, and restriction are never enabled as undifferentiated groups.
- [ ] Given workspace lint inheritance through `[workspace.lints]` and `[lints] workspace = true`, when effective policy is recorded, then the owning member's effective lint level and Rust Doctor's own mapping are both preserved.
- [ ] Given rustc or Clippy JSON Lines mixed with non-JSON output, when parsing runs, then valid compiler messages are retained, non-JSON lines are bounded and classified, and structured fields rather than rendered text drive identity, location, severity, applicability, and score mapping.
- [ ] Given a compiler diagnostic with child spans or macro expansion, when normalized, then the primary actionable span is deterministic and expansion provenance is retained when available.
- [ ] Given a compile failure, when Clippy cannot complete, then compiler errors already emitted remain visible and analyzer coverage is partial or failed rather than clean.
- [ ] Given mutually exclusive feature configurations, when the default scan runs, then it analyzes the declared default profile and does not force `--all-features`; additional profiles require explicit policy and are represented separately.
- [ ] Given a target or profile not analyzed, when the report is rendered, then it appears as uncovered scope rather than inferred healthy scope.
- [ ] Given an unsupported future Cargo JSON reason or unknown Clippy lint, when parsing runs, then the scan continues, preserves bounded raw classification metadata, and defaults the rule to score-ineligible.

#### US-009: Normalize external analyzer evidence

**Priority:** P1  
**Size:** L  
**Dependencies:** Blocked by US-002, US-003

**User Story:**  
As a dependency and security owner, I want external tool evidence normalized by meaning so that advisories, policy violations, unsafe exposure, unused dependencies, and semver risk are not conflated.

**Acceptance Criteria:**

- [ ] Given cargo-audit and cargo-deny reports for the same RustSec advisory, when diagnostics are normalized, then stable advisory identity deduplicates the root cause while retaining analyzer provenance.
- [ ] Given cargo-geiger output, when it is normalized, then it is categorized as direct or transitive unsafe exposure, marked audit-only by default, and never described as a confirmed vulnerability.
- [ ] Given cargo-shear or cargo-semver-checks output, when it is normalized, then the adapter records tool version, parser contract version, package scope, evidence source, and score eligibility separately.
- [ ] Given an adapter that supports structured output, when invoked, then Rust Doctor consumes the structured form; text parsing is isolated to adapters without stable structured output and protected by versioned fixtures.
- [ ] Given a missing optional tool, timeout, non-zero exit, malformed document, truncated output, or unsupported version, when the adapter finishes, then it emits a skipped or failed receipt and never an empty successful result.
- [ ] Given an explicitly required external tool that cannot provide authoritative evidence, when the scan completes, then the affected dimension and overall gate are non-authoritative according to policy.
- [ ] Given the same dependency finding through direct and transitive paths, when grouped, then the report retains dependency path evidence without multiplying the root-cause score contribution beyond its aggregation policy.
- [ ] Given adapter output containing absolute paths or unbounded tool text, when errors are reported, then paths are sanitized where required and captured text respects configured output limits.

### EP-003: Evaluation and Feedback Loop

**Objective:** Turn analyzer quality into a continuously measured product invariant instead of a one-time review.

**Definition of Done:**

- Evaluation measures precision, recall, context coverage, and scan completeness.
- Compiler and external adapters have versioned conformance matrices.
- Every default and score-eligible rule is governed by promotion and demotion gates.
- Optional telemetry can reveal rule-level aggregate behavior without collecting repository content.

#### US-010: Measure opportunities, recall, and context coverage

**Priority:** P0  
**Size:** L  
**Dependencies:** Blocked by US-001, US-006

**User Story:**  
As an analyzer maintainer, I want evaluation to include missed opportunities and unsupported contexts so that a quiet rule cannot appear accurate by never firing.

**Acceptance Criteria:**

- [ ] Given the evaluation schema, when a corpus root is labeled, then positive opportunities, negative contexts, emitted findings, suppressed findings, abstentions, and uncovered contexts are representable independently.
- [ ] Given a completed evaluation, when metrics are calculated, then each rule reports precision, recall, false-positive rate, false-negative count, opportunity coverage, required-context coverage, abstention rate, and sample size.
- [ ] Given no labeled positive opportunity for a rule, when evaluation runs, then recall is reported as unavailable and the rule cannot pass score-eligibility gates.
- [ ] Given an emitted finding with no matching opportunity label, when matching runs, then it is classified as unknown or false positive according to reviewer state and never silently counted as true positive.
- [ ] Given a labeled positive opportunity with no emitted finding, when matching runs, then it is counted as a false negative even though no diagnostic exists in the scanner output.
- [ ] Given incomplete corpus roots, when aggregate metrics are computed, then complete and incomplete populations are separated and denominators are reported.
- [ ] Given multiple findings mapped to one opportunity, when metrics are calculated, then opportunity recall counts once and duplicate emissions are reported separately.
- [ ] Given malformed, duplicate, or contradictory labels, when validation runs, then the evaluation fails with stable case identifiers.

#### US-011: Add analyzer conformance matrices

**Priority:** P0  
**Size:** L  
**Dependencies:** Blocked by US-008, US-009, US-010

**User Story:**  
As a release owner, I want versioned compiler and external-tool conformance fixtures so that parser drift cannot silently change diagnostics or score.

**Acceptance Criteria:**

- [ ] Given Cargo metadata, rustc, Clippy, cargo-audit, cargo-deny, cargo-geiger, cargo-shear, and cargo-semver-checks adapters, when conformance is listed, then each has supported tool versions, parser contract version, fixture provenance, expected receipts, and normalized outputs.
- [ ] Given Cargo metadata invocation, when it runs, then it requests an explicit supported format version and handles additive unknown fields without losing known package and target identity.
- [ ] Given a supported adapter fixture, when conformance tests run, then diagnostic identity, package scope, primary location, provenance, authority state, and score eligibility match the approved snapshot.
- [ ] Given malformed JSON, mixed text, truncated output, timeout, non-zero exit, missing executable, and unknown output version fixtures, when conformance runs, then each produces the expected degraded receipt without panic or empty success.
- [ ] Given a new upstream tool version outside the supported matrix, when encountered, then the adapter reports unsupported or best-effort status and cannot become authoritative without approved conformance evidence.
- [ ] Given a parser change that alters stable identities or score eligibility, when protected tests run, then CI fails until the migration is explicitly reviewed.
- [ ] Given hermetic fixture execution without network access, when conformance runs, then results are deterministic on MSRV 1.97 and the current stable toolchain.

#### US-012: Enforce promotion and demotion gates for all rules

**Priority:** P0  
**Size:** L  
**Dependencies:** Blocked by US-007, US-010, US-011

**User Story:**  
As a product owner, I want every default and scored rule continuously requalified so that historical rules do not bypass standards applied to new rules.

**Acceptance Criteria:**

- [ ] Given the complete catalog, when trust validation runs, then every default or score-eligible rule has a current calibration artifact or a compiler, advisory, or policy authority exemption defined by trust tier.
- [ ] Given a calibrated heuristic, when false-positive rate exceeds 2%, recall falls below 80%, required-context coverage falls below 90%, or reviewed samples fall below the minimum, then protected validation fails.
- [ ] Given a rule that fails its gate on two consecutive approved corpus revisions, when the next default catalog is generated, then it is proposed for automatic demotion to non-scoring or opt-in with a machine-readable reason.
- [ ] Given a compiler-proven rule, when its parser or mapping loses conformance, then trust validation blocks authority even if corpus precision remains high.
- [ ] Given an advisory-backed rule, when advisory identity or affected-version evidence is missing, then it cannot retain advisory-backed score eligibility.
- [ ] Given a historical rule present before this PRD, when validation runs, then it receives no grandfathering exception.
- [ ] Given a deliberate threshold exception, when approved, then it is rule-specific, owner-attributed, time-bounded, documented with evidence, and visible in Report V1 metadata.
- [ ] Given missing or uncertain labels that would make a rule appear to pass, when gate metrics are computed, then the uncertainty cannot be counted as success.

#### US-013: Add privacy-safe per-rule aggregate telemetry

**Priority:** P1  
**Size:** M  
**Dependencies:** Blocked by US-002, US-003

**User Story:**  
As a product maintainer, I want optional aggregate rule telemetry so that real-world activation and suppression patterns can guide evaluation without collecting code.

**Acceptance Criteria:**

- [ ] Given telemetry is not explicitly enabled, when any scan runs, then no telemetry network request is attempted.
- [ ] Given telemetry is enabled, when an event is produced, then it contains only schema version, Rust Doctor version, score-model version, anonymous installation cohort, aggregate rule counts, trust tiers, analyzer receipts, completeness bands, and coarse duration bands.
- [ ] Given a telemetry payload, when privacy validation runs, then source text, diagnostic messages, file paths, package names, repository identifiers, dependency names, command lines, environment variables, Git remotes, and exact timestamps are absent.
- [ ] Given rule counts, when serialized, then fired, suppressed, abstained, disabled, and score-contributing counts remain distinct and are bounded to the current catalog.
- [ ] Given an event exceeding 64 KiB, when serialization runs, then it is dropped or compacted according to a deterministic documented policy without delaying the scan.
- [ ] Given network failure, timeout, invalid endpoint, or server rejection, when telemetry delivery runs, then scan diagnostics, score, exit status, and duration remain unaffected.
- [ ] Given an unknown dynamic rule, when telemetry aggregates it, then it is counted in an unknown bucket and its identifier is not uploaded.
- [ ] Given telemetry schema evolution, when an older client emits, then server-side interpretation is versioned and no missing field is inferred as a healthy analyzer state.

### EP-004: Decision-Quality Outputs and Certification

**Objective:** Convert trustworthy evidence into one actionable product decision across every consumer and certify the migration.

**Definition of Done:**

- Report V1 contains canonical priority, root-cause, evidence, limitations, aggregation, and fix guidance.
- Every consumer uses one ordering and grouping contract.
- Automatic fixes are limited to explicitly safe evidence.
- Score Core V2 and diagnostic trust gates are certified on all required execution surfaces.

#### US-014: Add canonical priority, root cause, and fix recipe

**Priority:** P1  
**Size:** L  
**Dependencies:** Blocked by US-002, US-005

**User Story:**  
As a maintainer or coding agent, I want each diagnostic to explain urgency, evidence, root cause, and remediation so that I can act without reverse-engineering the rule.

**Acceptance Criteria:**

- [ ] Given a normalized diagnostic, when Report V1 is emitted, then it includes priority, trust tier, score eligibility, score impact, aggregation policy, root-cause key, evidence summary, limitation summary, and fix-recipe identifier where available.
- [ ] Given a built-in rule, when its documentation is rendered, then it answers why the pattern matters, when it applies, what evidence was used, known false-positive boundaries, and the smallest safe remediation.
- [ ] Given multiple diagnostics caused by one dependency advisory, configuration issue, or code pattern, when root-cause grouping runs, then one canonical group owns priority and bounded score impact while occurrences remain inspectable.
- [ ] Given a diagnostic without a validated fix recipe, when output is rendered, then it provides guidance without claiming machine applicability.
- [ ] Given an unknown external or dynamic rule, when normalized, then it has no fabricated priority, score impact, root cause, or fix recipe.
- [ ] Given a suppressed diagnostic, when included in a diagnostic export that permits suppressed records, then its suppression state and zero score impact are explicit.
- [ ] Given a consumer that only understands prior Report V1 fields, when it reads the additive diagnostic metadata, then existing fields retain their type and behavior.

#### US-015: Unify ordering and migration grouping across consumers

**Priority:** P1  
**Size:** L  
**Dependencies:** Blocked by US-014

**User Story:**  
As a user moving between terminal, CI, SARIF, MCP, plan, and handoff output, I want the same issues prioritized in the same order so that the product gives one answer.

**Acceptance Criteria:**

- [ ] Given one canonical report, when terminal, JSON, SARIF, MCP, CI annotations, plans, and handoffs render it, then all use one shared comparator based on priority, authority, trust tier, root-cause impact, category, rule ID, package, path, and location.
- [ ] Given the same report rendered twice, when order is compared, then the root-cause groups and diagnostics are byte-stable where the format itself is deterministic.
- [ ] Given at least 50 findings across at least 10 files or 5 root-cause groups, when terminal and handoff output render, then they switch to migration grouping and present the highest-impact root causes before individual occurrences.
- [ ] Given fewer findings than the migration threshold, when output renders, then it uses the normal priority list without empty migration sections.
- [ ] Given more findings than a consumer's output limit, when truncation occurs, then omitted counts are reported by priority and category and the top three root-cause groups are never displaced by lower-priority findings.
- [ ] Given equal-priority diagnostics with missing paths or locations, when sorting runs, then stable fallback keys prevent nondeterministic order.
- [ ] Given a SARIF result, when exported, then stable rule ID, fingerprint, priority mapping, precision metadata, primary location, and root-cause correlation survive within supported SARIF fields and properties.
- [ ] Given a consumer-specific override that would reorder score decisions, when validation runs, then it is rejected unless the canonical report itself changes.

#### US-016: Enforce actionable guidance and safe fix eligibility

**Priority:** P1  
**Size:** L  
**Dependencies:** Blocked by US-006, US-014

**User Story:**  
As a maintainer, I want fixes to be explicit about safety and validation so that Rust Doctor never turns uncertain diagnostics into risky edits.

**Acceptance Criteria:**

- [ ] Given a machine-applicable compiler suggestion, when fix eligibility is computed, then applicability, exact span, replacement, file identity, and precondition hash are all required.
- [ ] Given a custom-rule fix, when it is marked machine-applicable, then a dedicated fixture suite proves parse preservation, formatting stability, idempotency, and absence of edits outside the targeted span.
- [ ] Given overlapping fixes, stale source hashes, macro-generated spans, multi-file semantic changes, ambiguous types, or unsupported encodings, when fix planning runs, then the affected fixes are guidance-only.
- [ ] Given an eligible fix batch, when changes are planned, then they are grouped by root cause, ordered deterministically, and can be validated independently before another group is attempted.
- [ ] Given a suggested command or dependency change, when rendered, then it is presented as an explicit user action and never executed by read-only MCP tools.
- [ ] Given a fix that would alter public API, unsafe code, Cargo features, MSRV, dependency policy, or security hardening, when eligibility is evaluated, then it cannot be automatic.
- [ ] Given a fix is applied through an authorized mutable surface, when post-fix validation fails, then the failure is reported with the affected fix group and no subsequent group is represented as validated.
- [ ] Given a diagnostic has no safe remediation, when documentation renders, then it states the decision boundary instead of inventing a generic fix.

#### US-017: Certify and migrate diagnostic trust parity

**Priority:** P0  
**Size:** L  
**Dependencies:** Blocked by US-005, US-012, US-015, US-016

**User Story:**  
As a release owner, I want a reproducible certification and explicit migration so that Score Core V2 can become the product default without silent regressions.

**Acceptance Criteria:**

- [ ] Given the pinned evaluation corpus, when certification runs, then at least 260 Cargo roots complete and corpus incompleteness does not regress by more than 0.2 percentage points from the approved baseline.
- [ ] Given every default score-eligible calibrated heuristic, when certification runs, then false-positive rate is at most 2%, recall is at least 80%, required-context coverage is at least 90%, and sample minima are satisfied.
- [ ] Given every compiler, advisory, and external adapter included in authority, when certification runs, then its supported-version conformance matrix passes.
- [ ] Given self-scan, clean fixtures, known-defect fixtures, partial analyzer fixtures, workspace fixtures, timeout fixtures, and malformed-output fixtures, when reports are compared, then score invariants, authority, dimension coverage, priority order, and stable identities match approved expectations.
- [ ] Given the Score Core V2 release, when users inspect CLI help, documentation, JSON, SARIF properties, MCP output, plans, and handoffs, then the score-model version, authority meaning, migration warning, and non-comparability with legacy scores are documented consistently.
- [ ] Given a pre-V2 cache or baseline, when Rust Doctor loads it, then the model-version mismatch invalidates or separates it and never treats legacy values as V2.
- [ ] Given the current default catalog, when release validation runs, then zero medium-confidence or heuristic rules are score-eligible without a current calibration artifact.
- [ ] Given a missing required analyzer or unavailable dimension, when certification fixtures run, then no output surface displays an authoritative 100 or a healthy dimension by omission.
- [ ] Given any product surface orders diagnostics differently from the canonical comparator, when cross-surface contract tests run, then release validation fails.
- [ ] Given certification evidence is incomplete, stale, or produced with an unapproved corpus, catalog, toolchain, or model hash, when release validation runs, then Score Core V2 cannot be marked certified.

## Functional Requirements

### Trust and catalog

- **FR-001:** The catalog shall define analyzer provenance, trust tier, priority, score eligibility, required evidence, aggregation policy, calibration version, supported contexts, and limitations for every built-in rule.
- **FR-002:** Unknown or dynamic rules shall default to score-ineligible and unranked.
- **FR-003:** Severity, confidence, priority, trust tier, category, and score impact shall remain independent properties.
- **FR-004:** A heuristic shall not become default and score-eligible without approved precision, recall, sample-size, and context-coverage evidence.
- **FR-005:** Historical rules shall be subject to the same qualification gates as new rules.

### Coverage and score

- **FR-006:** Every scan shall produce analyzer receipts containing planned, completed, skipped, failed, partial, and abstained states.
- **FR-007:** Every health dimension shall expose coverage and authority.
- **FR-008:** An unobserved dimension shall not receive a synthetic perfect score.
- **FR-009:** Score Core V2 shall retain the five existing dimensions and weights.
- **FR-010:** Score Core V2 shall use explicit priority and bounded aggregation policies.
- **FR-011:** Optional analyzer availability alone shall not change the Core Score.
- **FR-012:** The workspace headline shall be the lowest authoritative package score, with aggregate workspace health reported separately.
- **FR-013:** Reports, caches, baselines, and telemetry shall identify the score-model version.

### Analyzer execution

- **FR-014:** Stable compiler, Cargo, and Clippy structured evidence shall outrank file-local heuristic inference.
- **FR-015:** Rules requiring unavailable semantic evidence shall abstain or remain score-ineligible.
- **FR-016:** Clippy configuration shall be isolated outside the scanned repository.
- **FR-017:** Cargo and rustc JSON shall be parsed line by line and tolerate bounded non-JSON output.
- **FR-018:** External adapter failure shall never normalize to empty success.
- **FR-019:** Unsafe exposure, vulnerability, dependency policy, unused dependency, and semver compatibility shall remain distinct diagnostic meanings.

### Evaluation and outputs

- **FR-020:** Evaluation shall model positive opportunities and missed findings in addition to emitted diagnostics.
- **FR-021:** Precision, recall, false-positive rate, false-negative count, context coverage, abstention, and completeness shall be measurable per rule.
- **FR-022:** Compiler and external adapters shall have versioned conformance fixtures.
- **FR-023:** Terminal, JSON, SARIF, MCP, CI, plans, and handoffs shall share one canonical priority and grouping contract.
- **FR-024:** Diagnostics shall expose root-cause, evidence, limitations, aggregation, and fix eligibility metadata.
- **FR-025:** Automatic fixes shall require explicit machine-applicability evidence and shall never be exposed through read-only MCP mutation.
- **FR-026:** Telemetry shall remain opt-in and aggregate-only.

## Non-Functional Requirements

### Accuracy and calibration

- **NFR-001:** Every score-eligible calibrated heuristic shall maintain at most 2% false-positive rate and at least 80% recall on a reviewed dataset containing at least 20 positive opportunities and 20 negative contexts.
- **NFR-002:** Required-context coverage for every score-eligible heuristic shall be at least 90%.
- **NFR-003:** Zero uncalibrated heuristic or medium-confidence rule shall influence Score Core V2.
- **NFR-004:** Score invariants shall pass for 100% of supported fixture permutations.

### Determinism and compatibility

- **NFR-005:** Identical normalized inputs shall produce identical score, authority, grouping, and ordering in 100 consecutive runs across supported parallelism settings.
- **NFR-006:** 100% of pre-existing Report V1 required fields shall retain their name, type, and meaning.
- **NFR-007:** Zero stable rule identities or fingerprints shall change without an approved migration fixture.
- **NFR-008:** MSRV 1.97 shall compile and pass 100% of the applicable quality-gate suite.

### Performance and resource bounds

- **NFR-009:** On the approved 260-root corpus, median wall-clock regression from trust computation, normalization, and ordering shall remain at or below 10% against the pre-program baseline, excluding intentional additional analyzer profiles.
- **NFR-010:** Scoring, grouping, and ordering 100,000 normalized diagnostics shall complete in at most 100 ms and use at most 256 MiB additional resident memory on the reference CI runner.
- **NFR-011:** Cancellation or timeout propagation shall terminate child analyzer processes within 2 seconds after the configured deadline where the operating system permits.
- **NFR-012:** Captured non-JSON or error output shall be capped at 1 MiB per subprocess, with the truncated byte count reported.

### Privacy and security

- **NFR-013:** Default operation shall send zero source, path, diagnostic, repository, dependency, or environment data over the network on behalf of Rust Doctor telemetry.
- **NFR-014:** Telemetry shall attempt zero network requests unless explicitly enabled.
- **NFR-015:** One telemetry event shall not exceed 64 KiB.
- **NFR-016:** Clippy and compiler analysis shall create zero configuration files in the scanned repository.
- **NFR-017:** MCP shall expose zero mutating tools, accept zero project directories outside `$HOME`, retain a 300-second scan timeout, perform zero network access by default, and expose zero unsanitized project paths in errors.

### Reliability and scale

- **NFR-018:** A failure or panic in one custom rule or optional adapter shall terminate zero unrelated analysis passes.
- **NFR-019:** Workspaces containing 200 members and 100,000 diagnostics shall preserve deterministic package ownership, coverage, deduplication, and headline selection.
- **NFR-020:** Across 100% of unknown-variant conformance fixtures, the scanner shall produce zero panics and zero fabricated authoritative receipts.

## Edge Cases

| Scenario | Required behavior |
|---|---|
| A workspace member inherits `[workspace.lints]` while another overrides it | Resolve and report policy per member |
| Two Cargo features are mutually exclusive | Analyze the declared default profile; mark other profiles uncovered unless explicitly requested |
| A build script or proc macro interleaves text with Cargo JSON | Retain valid JSON messages and bound the classified text |
| A primary span points into macro expansion, generated code, or a dependency | Preserve provenance; abstain from unsafe source edits |
| One workspace package is healthy while another fails before analysis | Mark workspace authority false; do not hide the failed package |
| All optional external tools are absent | Keep Core Score stable; report skipped optional evidence |
| A required tool is absent, times out, exits non-zero, or changes format | Emit a failed receipt and non-authoritative dimension |
| cargo-audit and cargo-deny report the same advisory through different paths | Group by stable advisory root cause and retain paths |
| cargo-geiger reports transitive unsafe usage without a vulnerability | Classify as audit-only unsafe exposure |
| A rule has perfect precision but no labeled positive opportunities | Report recall unavailable and deny score eligibility |
| Required rule context is unavailable across most corpus roots | Report abstention and insufficient context coverage |
| One root cause produces thousands of occurrences | Apply its explicit bounded or root-cause aggregation policy |
| Two rules report one compiler-backed root cause with different severities | Preserve provenance and choose one canonical group priority |
| A suppressed P0 exists beside an unsuppressed lower-priority finding | Give the suppressed result zero score impact without hiding its state from permitted exports |
| A cache was generated by the legacy score | Invalidate or separate it by model version |
| An older Report V1 consumer ignores additive fields | Preserve every prior required field and its meaning |
| An unknown Clippy lint appears | Keep it observable, score-ineligible, and unranked |
| A fix is stale, overlapping, or macro-generated | Downgrade it to guidance-only |
| Telemetry sees an unknown rule or oversized event | Use the unknown bucket; compact or drop the bounded event |
| No scannable package exists | Emit no synthetic score and return a typed reason |

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---:|---:|---|
| Trust metadata becomes another unverified registry | Medium | High | Validate every score-eligible rule against machine-readable calibration or authority evidence |
| Score authority makes the product appear less decisive | Medium | Medium | Keep a provisional numeric score but make authority and missing evidence explicit |
| Requalification removes many current defaults | High | Medium | Prefer honest opt-in or advisory status; ship measured promotion paths rather than preserving noisy defaults |
| Recall labeling becomes too expensive | High | High | Start with the ten highest-volume or highest-impact rules and model reusable opportunities in deterministic fixtures |
| Public corpus labels drift with upstream repositories | Medium | High | Pin revisions, preserve fixture snapshots, hash corpus and toolchain, separate incomplete roots |
| Optional tools create machine-dependent results | High | High | Keep Core Score independent of optional availability; represent optional evidence through completeness and separate diagnostics |
| Worst-package headline penalizes large workspaces | Medium | Medium | Expose per-package distribution and aggregate health separately; use diff policy for CI regression workflows |
| External text parsers break on upgrades | High | Medium | Prefer structured output, version conformance fixtures, and degrade authority on unsupported versions |
| Adding context slows scans | Medium | Medium | Reuse discovery and metadata, compute context once per package or file, enforce the 10% median regression budget |
| Score V2 migration destroys historical comparability | High | Medium | Version every score, invalidate or separate caches, publish a one-time migration marker, never merge model series silently |
| Per-rule telemetry creates privacy risk | Low | High | Explicit opt-in, strict deny-list, bounded aggregate schema, unknown bucket, no code or identifiers |
| Scope expands into a new semantic compiler | Medium | High | Keep ADR 0002 boundary; no nightly, MIR, Dylint, unsafe, or rustc-private backend in this program |

## Non-Goals

1. Adding new rule families or maximizing the catalog count.
2. Reimplementing rustc, Clippy, borrow checking, type inference, MIR, or interprocedural analysis.
3. Shipping a nightly-only, `rustc_driver`, Dylint, or unsafe in-process backend.
4. Changing the five score dimensions or their weights.
5. Copying React Doctor's internal score formula.
6. Creating a remote or AI-generated score.
7. Automatically executing fixes through MCP.
8. Replacing cargo-audit, cargo-deny, cargo-geiger, cargo-shear, or cargo-semver-checks.
9. Treating unsafe exposure as a vulnerability.
10. Making all optional external tools mandatory.
11. Uploading source code, paths, diagnostic text, dependency identities, or repository metadata.
12. Redesigning the CLI, terminal theme, npm packaging, GitHub Action, or MCP security model.
13. Supporting arbitrary third-party dynamic scoring plugins.
14. Guaranteeing historical numeric comparability between the legacy score and Score Core V2.

## Files NOT to Modify

The following artifacts are protected during this program:

- `tasks/prd-react-doctor-parity.md`
- `tasks/prd-react-doctor-parity-status.json`
- Historical completed PRDs and status files under `tasks/`
- `docs/adr/0002-compiler-aware-rule-strategy.md` except through a separately approved superseding ADR
- Snapshot files under `tests/snapshots/` by direct editing
- `Cargo.lock` by direct editing
- Generated artifacts under `target/`
- App-managed databases, sessions, memories, plugin caches, browser state, and temporary agent folders

The score dimension weight constants must not change without explicit approval, even when their containing score module is modified for Score Core V2.

## Technical Considerations

These are implementation questions with a preferred direction, not authorization for speculative architecture.

1. **Where should trust metadata live?** Prefer extending the existing canonical catalog structures so CLI, MCP, evaluation, and Report V1 cannot drift into separate registries.
2. **How should score policy be represented?** Prefer a checked-in, versioned typed artifact compiled into the binary, with schema validation and an explicit model identifier.
3. **How should dimension coverage compose?** Prefer analyzer receipts normalized into package scope first, then dimension scope, then workspace authority. Avoid consumers recomputing coverage.
4. **How should workspace health be exposed?** Use worst authoritative package as the headline, plus minimum, median, distribution, and package list as descriptive portfolio data.
5. **How should Cargo JSON evolve safely?** Parse known structured fields line by line, request explicit metadata format versions, tolerate additive unknown data, and treat unknown semantic variants as non-authoritative until mapped.
6. **How should custom rules receive context?** Extend the existing rule context rather than adding a second rule trait unless current trait compatibility makes one bounded adapter necessary.
7. **How should abstention be represented?** Prefer analyzer receipts or evaluation events rather than user-facing diagnostics, while exposing aggregate abstention in completeness and rule metrics.
8. **How should optional tools affect results?** Preserve diagnostics and completeness, but keep Core Score deterministic across machines with different optional installations.
9. **How should score calibration choose exact penalties?** Generate candidate-model comparison from US-001, enforce US-004 invariants, store the chosen coefficients in a reviewable artifact, and keep dimension weights fixed.
10. **How should root-cause identity work?** Reuse stable advisory IDs, compiler codes, lint IDs, package IDs, and normalized rule-specific evidence. Avoid message-text hashing as the primary identity.
11. **How should evaluation avoid label leakage?** Separate fixture authoring from rule implementation review where practical, retain reviewer state, and exclude unknown labels from pass denominators.
12. **How should `cargo_metadata` forward compatibility be proven?** Confirm the exact crate-version API and unknown-variant behavior in US-011 before relying on it for authority.
13. **How should diff mode interact with score?** Keep full repository health and changed-code gate results distinct. Diff filtering must not silently redefine the canonical repository score.
14. **How should legacy score consumers migrate?** Add model version and authority additively, invalidate old cache keys, retain bare integer stdout, and publish explicit non-comparability.

## Success Metrics

| Metric | Baseline | Target | Timeframe |
|---|---:|---:|---|
| Default score-eligible heuristics with measured precision and recall | 0 uniformly qualified | 100% | Before Score Core V2 default |
| Default medium-confidence or heuristic rules affecting score without calibration | 18 identified medium-confidence defaults | 0 | EP-002 exit |
| Score-eligible heuristic false-positive rate | Not uniformly measured | At most 2% per rule | EP-003 exit and every release |
| Score-eligible heuristic recall | Not measured | At least 80% per rule | EP-003 exit and every release |
| Minimum reviewed evidence per score-eligible heuristic | Not enforced | At least 20 positive opportunities and 20 negative contexts | EP-003 exit |
| Dimensions shown healthy without authoritative analyzer evidence | Possible | 0 | EP-001 exit |
| Score models identifiable in reports and caches | Legacy implicit model | 100% carry model version | EP-001 exit |
| Default score variation caused only by optional tool installation | Possible | 0 points | EP-002 exit |
| Analyzer adapters covered by versioned conformance matrices | Compiler path partial; external adapters excluded from corpus | 100% of authority-capable adapters | EP-003 exit |
| Product surfaces using one canonical diagnostic comparator | Multiple consumer-specific paths | 100% of terminal, JSON, SARIF, MCP, CI, plan, and handoff | EP-004 exit |
| Public corpus complete Cargo roots | At least 260 pinned roots | At least 260 with incompleteness regression at most 0.2 percentage points | Final certification |
| Median scan overhead from trust, grouping, and score computation | Pre-program benchmark | At most 10% regression | Final certification |
| Repository files created by Clippy configuration | Temporary `clippy.toml` behavior exists | 0 | EP-002 exit |
| Telemetry network attempts while disabled | Aggregate telemetry exists | 0 | EP-003 exit |
| Self-scan score/report contradiction | 734 findings, 8 errors, score 98 under legacy model | Every score-affecting finding and missing evidence has explainable bounded impact and authority | Final certification |

## Open Questions

No product question blocks implementation. The following engineering confirmations are owned by their stories:

1. **Exact Score Core V2 penalties:** US-005 selects them from the versioned baseline and invariant suite. Default decision: keep current dimension weights, use explicit priority and bounded aggregation, reject any candidate that violates US-004.
2. **`cargo_metadata` forward-compatibility behavior:** US-011 verifies the exact 0.19 API and fixture behavior. Default decision: unknown semantic states degrade authority rather than fabricate package coverage.
3. **Telemetry delivery endpoint and retention:** US-013 may ship the local schema and validation without enabling delivery. Default decision: no endpoint and no network request until separately approved.
4. **Future compiler-deep backend:** explicitly deferred. Default decision: stable layers only; revisit through a new ADR after the stable system exposes a measured recall gap.

[/PRD]
