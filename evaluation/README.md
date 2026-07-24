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

The pull-request certification writes
`promotion-review-template.json` beside the initial delta report. It contains
the exact hash-ordered, deduplicated sample used by the gate, with no source
text or host path. After that immutable candidate artifact is reviewed,
`evaluation/approvals/ep006-candidate.json` binds its GitHub run, artifact
digest and NDJSON SHA-256, while `evaluation/approvals/ep006-labels.json` binds
the reviewed labels to the same SHA-256. The EP-006 promotion workflow downloads
that earlier candidate instead of rescanning the corpus, verifies both artifact
identities through the GitHub API, and emits the final delta and promotion
sample even when a threshold blocks activation.

Both candidate collection and final promotion use the
`ep006-protected-evidence` environment. It forbids self-review and admin bypass,
requires an independent reviewer, and permits only the exact evidence branch.
Candidate verification checks that live policy and the successful GitHub
Actions deployment created by it.

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
