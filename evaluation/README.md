# Evaluation and regression gates

`rust-doctor-eval` separates network preparation from analysis. The checked
corpus manifest contains 100 GitHub repositories pinned to full commits and
declares a conservative minimum of 260 Cargo roots. Preparation verifies the
actual roots before it writes its own manifest.

The manifest pins evaluation profile `1.2`: every Rust Doctor custom rule is
forced on at warning severity, repository config and inline suppressions are
ignored, and execution is offline. Workspace metadata uses Cargo's `--no-deps`
policy, which retains workspace packages and declared dependencies without
fetching external crates. Compiler and dependency adapters are excluded because
their output depends on each repository's build environment; their conformance
is gated separately. Every record carries the profile and catalog SHA-256 plus
exact expected, attempted, and reported roots.

```bash
cargo run --bin rust-doctor-eval -- prepare \
  --manifest evaluation/corpus-v1.json \
  --checkout-root /var/tmp/rust-doctor-corpus \
  --prepared-out /var/tmp/rust-doctor-corpus/prepared.json

cargo run --bin rust-doctor-eval -- corpus \
  --manifest evaluation/corpus-v1.json \
  --prepared /var/tmp/rust-doctor-corpus/prepared.json \
  --checkout-root /var/tmp/rust-doctor-corpus \
  --binary target/release/rust-doctor \
  --tool-revision "$(git rev-parse HEAD)" \
  --output /var/tmp/candidate.ndjson
```

Preparation is the only network phase. Linux scans fail closed unless
`bubblewrap` is available. Each checkout is read-only, network and inherited
environment are removed, and build output is confined to a disposable scratch
directory. `prepare` creates a marked `.rust-doctor-cargo-home` below the
checkout root. Corpus execution never mounts the inherited host Cargo home and
rejects Cargo credential files. Rustup toolchains and the exact Cargo, rustc,
rustdoc and Clippy proxies are mounted read-only.

Each sandbox defaults to 2 GiB aggregate resident memory, 64 processes and a
2 GiB scratch tmpfs. The limits are configurable with
`--sandbox-memory-mib`, `--sandbox-process-limit` and
`--sandbox-scratch-mib`; zero and overflowing limits fail before launch.
Symlink escapes, resource exhaustion and oversized output are recorded as
structured failures before a favorable record can be produced. A failed or
incomplete repository gets at most two sequential retries.

Corpus lines conform to
[`schemas/corpus-record-v1.schema.json`](schemas/corpus-record-v1.schema.json).
They contain commit and package roots, tool revision, completeness, diagnostic
and per-rule counts, duration, stable evidence, attempts, and the full failure
chain. Before every scan, the runner rechecks the clean index and worktree,
submodule state, and Git tree digest recorded during preparation. Host paths are
removed from failures.

## Diagnostic gate

```bash
cargo run --bin rust-doctor-eval -- delta \
  --baseline /var/tmp/approved/corpus-baseline.ndjson \
  --baseline-approval evaluation/approvals/corpus-baseline.json \
  --candidate /var/tmp/candidate.ndjson \
  --labels evaluation-results/labels.json \
  --output /var/tmp/delta.json
```

The gate rejects missing, unpinned, or schema-incompatible baselines. It blocks
diagnostic increases above 0.5% of complete roots and incomplete-root increases
above 0.2 percentage points. A promotion sample contains every introduced
finding up to 100 deterministic findings per rule. More than 2% confirmed false
positives, missing labels, or uncertain labels blocks default activation.
Promotions are derived from the two catalogs, so callers cannot omit a promoted
rule. Labels bind repository, Cargo root, rule, site, and evidence fingerprint.

The base-controlled EP-006 protected workflow must be dispatched from
`master` with an open same-repository pull request and its exact full head SHA.
It verifies the candidate ancestry, builds untrusted binaries in a disposable
namespace with read-only sources and oracle inputs, and transfers only
hash-bound outputs to fresh trusted jobs. Those jobs run the production-rule
mutation oracle and pinned corpus, verify every artifact identity and evidence
member, and compare against the approved baseline.

The trusted promotion policy keeps all EP-006 candidates opt-in unless a later
base-controlled policy explicitly qualifies them. The final job proves that
thresholds are unchanged, no rule became default-enabled, no promotion review
was requested, and every delta reason is confined to an expected EP-006 rule.
It then emits immutable mutation, corpus, delta, and empty-promotion evidence.
Candidate code receives no secrets and cannot modify the workflow, policy,
evaluator, or final evidence used by the dispatched run.

A reviewed replacement baseline can acknowledge only the diagnostic-growth
threshold. Approval JSON binds the exact subject SHA-256, repository commit,
successful protected workflow run, artifact ID, artifact digest, reviewer and
review timestamp. The gate verifies that immutable artifact through the GitHub
API before comparing it.

## Performance gate

```bash
cargo run --bin rust-doctor-eval -- benchmark \
  --manifest evaluation/benchmarks-v1.json \
  --binary target/release/rust-doctor \
  --baseline /var/tmp/approved/benchmark-baseline.json \
  --baseline-approval evaluation/approvals/performance-baseline.json \
  --output /var/tmp/benchmark.json
```

The fixed matrix covers cold and warm full, files, lines, and baseline scans on
small, medium, large, and 20-member workspace fixtures. Records include wall
and CPU time, peak RSS, files per second, cache hit rate, and per-pass time.
Median or P95 wall-time growth above 10% blocks. The 100,000-line fixture also
blocks above 512 MiB peak RSS. Percentage regressions below an absolute 50 ms
increase are ignored. Gate mode requires at least three repetitions, an
approved baseline, and matching fixture, diagnostic, host-class, toolchain and
repetition fingerprints. Baseline and candidate binary SHA-256 values are
recorded separately and each is bound to its artifact; requiring those two
different builds to have the same hash would make a regression comparison
impossible. Set `RUST_DOCTOR_BENCHMARK_HOST_CLASS` to the protected runner
class. Use `--record` only to generate an unapproved review candidate.

## Built artifacts

```bash
target/release/rust-doctor-eval smoke \
  --binary target/release/rust-doctor \
  --no-default-binary target/no-default/release/rust-doctor \
  --schema schemas/report-v1.schema.json \
  --npm-platform-package artifacts/rust-doctor-npm-linux-x64-0.2.0.tgz \
  --npm-wrapper-package artifacts/rust-doctor-npm-0.2.0.tgz \
  --bun "$(command -v bun)" \
  --archive artifacts/rust-doctor-x86_64-unknown-linux-gnu.tar.gz \
  --crate-package target/package/rust-doctor-0.2.0.crate
```

The smoke suite invokes terminal, score, JSON, SARIF, baseline, malformed and
failure paths from built binaries. It checks MCP initialize, tool discovery,
every reporting scope, and deadline-observed cancellation. It installs the
final npm tarballs, executes each extracted native archive, and builds the
exact verified `.crate` contents with locked dependencies before running its tests.

## Truth dataset and calibration baseline

```bash
cargo run --bin rust-doctor-eval -- truth \
  --binary target/debug/rust-doctor \
  --output evaluation/truth-baseline-v1.json \
  --tool-revision "$(git rev-parse HEAD)"
```

`evaluation/truth-dataset-v1.json` is the versioned labeled dataset behind Score Core V2. It models **positive opportunities** and **negative contexts**, not just emitted findings, so recall is measurable and a quiet rule cannot look accurate by never firing. Records conform to [`schemas/truth-case-v1.schema.json`](schemas/truth-case-v1.schema.json) and carry rule ID, fixture, opportunity location, expected applicability, expected emission, expected priority, context dimensions, label provenance, and reviewer state.

Labels live as `//~ pos` and `//~ neg` markers inside `evaluation/truth/fixtures/<rule>/*.rs.fixture`, and the dataset is re-derived from them on every run: an edited fixture whose digest or label lines no longer match the dataset fails the job instead of silently changing the measured population. The same pass is the MSRV gate — a fixture that no longer parses fails with its path and the exact parser reason. Fixtures use the `.rs.fixture` extension so intentionally defective code never enters Rust Doctor's own scan; the job materializes each one into a throwaway Cargo crate at its declared target path.

A case whose `reviewer_state` is `unreviewed`, `disputed`, `stale`, or `unknown` is excluded from every pass statistic, and the affected rule is reported `evidence-incomplete` rather than implicitly passing.

The job records per rule: true positives, false positives, false negatives, true negatives, precision, recall, false-positive rate, required-context coverage, emitted count, exact score contribution, and scan completeness. The baseline pins toolchain, dataset digest, configuration fingerprint, rule-catalog fingerprint, and score-model version so the measurement is reproducible.

Adding `--calibration-out` regenerates `evaluation/calibration-v1.json`, the reviewed artifact the catalog consults before granting score eligibility to a calibrated heuristic. The gate requires at least 80% recall over 50 independent positive opportunities, at least 90% required-context coverage, and a one-sided exact 95% Clopper-Pearson upper false-positive bound at or below 2%. With zero false positives, that bound requires 149 independent negative contexts. A rule that misses any condition keeps its diagnostics but contributes exactly zero to the Core Score. Regenerating a selected truth slice replaces its measured records while preserving reviewed activation decisions for catalog rules outside that slice.

## Decision-quality benchmark and Score Core model

The evaluator derives a review draft from the hash-bound protected corpus
without repository names, source paths, messages, package identities, or source
fragments:

```bash
target/debug/rust-doctor-eval decision-quality \
  --corpus target/ep003-corpus-baseline/corpus-baseline.ndjson \
  --corpus-approval evaluation/approvals/corpus-baseline.json \
  --binary target/debug/rust-doctor \
  --reviews target/decision-quality-review-draft.json \
  --generate-reviews \
  --reviewed-at 2026-07-27
```

Draft records are deliberately `unreviewed`: generation cannot certify its own
labels. An independent reviewer assigns health bands and ordered remediation
sets before the accepted artifact is checked in as
`evaluation/decision-quality-v1.json`.

The checked artifact contains 66 reviews: 60 pseudonymized complete public
projects plus six controlled contract anchors covering clean, two independent
Needs Work profiles, and two independent Critical profiles. Fifteen
repository-root holdouts cover all three health bands. The independent review
does not consult numeric score penalties. Records conform to
[`schemas/decision-quality-v1.schema.json`](schemas/decision-quality-v1.schema.json).

The release gate compares the previous and selected score artifacts on band agreement, top-three remediation overlap, monotonicity, optional-tool invariance, bounded duplicate stability, and reviewer-label safety:

```bash
target/debug/rust-doctor-eval decision-quality \
  --corpus target/ep003-corpus-baseline/corpus-baseline.ndjson \
  --corpus-approval evaluation/approvals/corpus-baseline.json \
  --binary target/debug/rust-doctor \
  --reviews evaluation/decision-quality-v1.json \
  --previous-model evaluation/score-model-v2.0.json \
  --model evaluation/score-model-v2.json \
  --output evaluation/score-model-migration-v2.1.json
```

`evaluation/score-model-v2.json` is the single source for dimension weights,
priority penalties, occurrence cap, P0 ceiling, and label thresholds. Model 2.1
keeps the approved weights and penalties, defines Great at 95 or above, Needs
Work at 50 or above, and caps a confirmed P0 at 49. It binds the exact
decision-quality dataset digest; a missing, invalid, stale, weight-changing, or
benchmark-regressing model fails the gate.
The model also binds the migration report digest. Score construction validates
that the report targets the selected version, matches the reviewed dataset,
passes every candidate invariant, and contains no gate reason.

## Release certification

Release certification uses the release binary and pinned local inputs. Run the
truth and adapter contracts first:

```bash
cargo build --release --bin rust-doctor --bin rust-doctor-eval

target/release/rust-doctor-eval truth \
  --binary target/release/rust-doctor \
  --dataset evaluation/truth-dataset-v1.json \
  --output evaluation/truth-baseline-v1.json \
  --tool-revision "$(git rev-parse HEAD)"

cargo test --lib --all-features conformance::tests
cargo test --lib --all-features real_binary_smoke_uses_qualified_contract -- --ignored
```

Prepare and execute the pinned corpus with the commands at the start of this
document. The candidate must contain at least 260 complete roots and every
record must carry the exact release source revision. The approved baseline is
the hash-bound artifact named by
`evaluation/approvals/corpus-baseline.json`.

Artifact, cross-surface, scale, performance, and interruption evidence is
release-only:

```bash
cargo test --test cross_surface_ordering
cargo test --release --lib \
  ordering::tests::canonical_decision_scale_gate -- --ignored --exact
cargo test --release --test deadlock_regression \
  release_scale_workspace_is_deterministic_without_deadlock -- --ignored --exact
cargo test --release --test release_interruption \
  unix::signals_terminate_analyzer_groups_within_two_seconds -- --ignored --exact

target/release/rust-doctor-eval benchmark \
  --binary target/release/rust-doctor \
  --baseline /var/tmp/approved/benchmark-baseline.json \
  --baseline-approval evaluation/approvals/performance-baseline.json \
  --output /var/tmp/candidate-benchmark.json

target/release/rust-doctor-eval smoke \
  --binary target/release/rust-doctor \
  --no-default-binary target/no-default/release/rust-doctor \
  --lsp-binary target/lsp/release/rust-doctor \
  --schema schemas/report-v1.schema.json \
  --npm-platform-package artifacts/rust-doctor-npm-linux-x64-0.2.0.tgz \
  --npm-wrapper-package artifacts/rust-doctor-npm-0.2.0.tgz \
  --bun "$(command -v bun)" \
  --archive artifacts/rust-doctor-x86_64-unknown-linux-gnu.tar.gz \
  --crate-package target/package/rust-doctor-0.2.0.crate \
  --action action.yml
```

The gate runner executes the fixed commands above and writes one result artifact
per gate. Set the packaged-surface and approved-baseline paths, then capture and
assemble the evidence. The timestamp is the source commit time so a checked
manifest is reproducible:

```bash
export CERT_NO_DEFAULT_BINARY=target/no-default/release/rust-doctor
export CERT_LSP_BINARY=target/lsp/release/rust-doctor
export CERT_NPM_PLATFORM_PACKAGE=artifacts/rust-doctor-npm-linux-x64-0.2.0.tgz
export CERT_NPM_WRAPPER_PACKAGE=artifacts/rust-doctor-npm-0.2.0.tgz
export CERT_NATIVE_ARCHIVE=artifacts/rust-doctor-x86_64-unknown-linux-gnu.tar.gz
export CERT_CRATE_PACKAGE=target/package/rust-doctor-0.2.0.crate
export CERT_CORPUS_CANDIDATE=/var/tmp/candidate.ndjson
export CERT_CORPUS_BASELINE=/var/tmp/approved/corpus-baseline.ndjson
export CERT_PERFORMANCE_BASELINE=/var/tmp/approved/benchmark-baseline.json

revision="$(git rev-parse HEAD)"
generated_at="$(git show -s --format=%ct HEAD)"
for gate in quality-gates cross-surface artifact-smoke workspace-scale \
  decision-overhead corpus-runtime interruption
do
  bash scripts/release/certification-gate.sh \
    "$gate" target/release/rust-doctor "$revision" "$generated_at"
done
bash scripts/release/assemble-certification-evidence.sh \
  target/release/rust-doctor \
  "$revision" \
  "$generated_at" \
  evaluation/release-evidence-v1.json
```

Certification rejects a missing gate artifact, a failed verdict, an unexpected
gate command, a stale artifact hash, or a binary/revision/timestamp mismatch:

```bash
target/release/rust-doctor-eval certify \
  --binary target/release/rust-doctor \
  --corpus /var/tmp/candidate.ndjson \
  --corpus-baseline /var/tmp/approved/corpus-baseline.ndjson \
  --tool-revision "$revision" \
  --generated-at-utc "$generated_at" \
  --output evaluation/certifications/decision-quality-v1.json
```

Certification closes in two commits because a Git commit cannot contain its
own hash. The first workflow run certifies source revision A and uploads the
candidate evidence. Record its protected run, artifact identity, digest, and
corpus SHA-256 in `evaluation/approvals/ep004-certification.json`, using the
same approval shape as the corpus baseline. Commit only that approval, the
generated evidence, the EP-004 status closure, and the migration-link closure
in revision B. On B, the workflow reads A from the checked manifest, rejects
any other A-to-B drift, downloads and verifies A's exact corpus evidence,
rebuilds A, and byte-compares the regenerated manifests with B. Any later code
or gate change makes the checked certification stale and fails verification.

The checked manifest conforms to
`evaluation/schemas/certification-v2.schema.json`. It contains no repository
names, absolute paths, source, diagnostic messages, or secrets. Missing corpus
arguments, fewer than 260 complete roots, stale artifact hashes, unsupported
adapter evidence, failed held-out metrics, cross-surface mismatches, performance
regressions, or interruption failures produce `certified: false` and exit `1`.
