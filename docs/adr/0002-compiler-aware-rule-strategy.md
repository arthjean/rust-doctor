# ADR 0002: Stable compiler-aware rule strategy

- Status: Accepted
- Date: 2026-07-22
- Owner: Rust Doctor maintainer
- Scope: EP-006 rule tranches

## Decision

Rust Doctor keeps stable Clippy, Cargo/rustc JSON, and `syn` as its only default analysis backends. No Dylint or rustc-driver dependency is introduced. Candidates that require resolved types, MIR, interprocedural ownership, or macro expansion remain experimental in `evaluation/rule-backlog-v1.json` until stable evidence exists.

This is a no-go decision for a deeper compiler backend in R3. It preserves Rust 1.97, `forbid(unsafe_code)`, the five release targets, the current single-package distribution, and Report V1 semantics. It also avoids pretending that syntax evidence can prove a type-level defect.

## Candidate model

The first five candidates under the deterministic tie-breaker were modeled against each backend. "Partial" means the backend can supply useful evidence but cannot prove the full rule without another adapter.

| Candidate | Stable Clippy + `syn` | Cargo/rustc JSON | Dylint | rustc-driver |
|---|---|---|---|---|
| `actix-web-data-lock` | Partial: AST call and capability gate | No relevant type fact in emitted diagnostics | Full type evidence | Full type evidence |
| `arc-cycle-risk` | Insufficient, remains experimental | Insufficient without a compiler diagnostic | Partial: aliases still obscure cycles | Partial: needs ownership graph beyond type identity |
| `atomic-relaxed-protocol` | Insufficient, remains experimental | Insufficient without an emitted lint | Partial: protocol intent remains unknown | Partial: MIR ordering visible, protocol intent absent |
| `await-holding-refcell-ref` | Partial: lexical guard/await evidence, opt-in | High only when rustc or Clippy emits an exact diagnostic | High | High |
| `axum-extension-request-state` | Partial: extractor syntax plus package capability | No relevant type fact in emitted diagnostics | High | High |

None of the five justifies a nightly backend. The two framework candidates and the RefCell candidate can ship as abstaining, opt-in syntax rules. The Arc and atomic candidates return to the experimental backlog.

## Constraint comparison

| Strategy | Rust support | Build coupling | Runtime cost relative to current scan | Binary/dependency delta | Distribution | Unsafe constraint | Precision ceiling |
|---|---|---|---|---|---|---|---|
| Stable Clippy + `syn` | Stable, MSRV 1.97 | Existing Cargo package build plus local AST | No new process; one existing Clippy run and the existing AST traversal | 0 new backend dependencies | Existing five targets | Preserved | High for emitted compiler facts, medium for AST heuristics |
| Cargo/rustc JSON | Stable, MSRV 1.97 with additive parsing | Existing Cargo invocation | 0 extra compiler invocations when reusing current message stream | 0 new dependencies | Existing five targets | Preserved | High only for facts the compiler emits |
| Dylint | Nightly toolchain coupled | Driver and lint library must match compiler internals | At least one additional compiler-driver execution | Not measured because MSRV and target gates fail before integration | Toolchain-specific dynamic artifacts | Cannot prove the project invariant across backend dependencies | High type precision, medium semantic intent |
| rustc-driver | Nightly commit coupled | Direct rustc-private API lockstep | Replacement or additional compiler-driver execution | Not measured because stable/MSRV gates fail before integration | Per-toolchain artifacts for all targets | Cannot prove the project invariant across backend dependencies | Highest compiler access, unchanged intent ambiguity |

The two rejected strategies were modeled rather than integrated. Numeric runtime and binary measurements would require adding a backend that already fails the stable toolchain and release-target gates, so recording fabricated deltas would be worse than an explicit no-go.

## Experimental backend contract

If a future backend satisfies every hard constraint, it must be behind a non-default Cargo feature, emit through the canonical diagnostic adapter, and be absent from default scores, completeness, and Report V1 check plans. Its owner is the Rust Doctor maintainer. Removal consists of deleting the feature, adapter, and optional dependency without a report migration because the backend may not alter V1 semantics.

Promotion requires measurements on all five release targets, the pinned corpus, and the fixed performance matrix. Failure of any constraint returns affected candidates to `experimental`; it never weakens MSRV, safety, or default distribution.
