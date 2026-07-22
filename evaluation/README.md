# Evaluation and regression gates

`rust-doctor-eval` separates network preparation from analysis. The checked
corpus manifest contains 100 GitHub repositories pinned to full commits and
declares a conservative minimum of 260 Cargo roots. Preparation verifies the
actual roots before it writes its own manifest.

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
checkout root. Dependency prefetching may populate that directory only during
the network phase; corpus execution never mounts the inherited host Cargo home
and rejects Cargo credential files. Rustup toolchains and the exact Cargo,
rustc, rustdoc and Clippy proxies are mounted read-only.

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
chain. Host paths are removed from failures.

## Diagnostic gate

```bash
cargo run --bin rust-doctor-eval -- delta \
  --baseline evaluation-results/approved.ndjson \
  --candidate /var/tmp/candidate.ndjson \
  --labels evaluation-results/labels.json \
  --promoted-rule hardcoded-secrets \
  --output /var/tmp/delta.json
```

The gate rejects missing, unpinned, or schema-incompatible baselines. It blocks
diagnostic increases above 0.5% of complete roots and incomplete-root increases
above 0.2 percentage points. A promotion sample contains every introduced
finding up to 100 deterministic findings per rule. More than 2% confirmed false
positives, missing labels, or uncertain labels blocks default activation.

A reviewed replacement baseline can acknowledge only the diagnostic-growth
threshold. The approval JSON must contain schema version `1.0`, the exact
candidate file SHA-256, a non-empty reviewer, and a review timestamp.

## Performance gate

```bash
cargo run --bin rust-doctor-eval -- benchmark \
  --manifest evaluation/benchmarks-v1.json \
  --binary target/release/rust-doctor \
  --baseline evaluation-results/benchmark-approved.json \
  --output /var/tmp/benchmark.json
```

The fixed matrix covers cold and warm full, files, lines, and baseline scans on
small, medium, large, and 20-member workspace fixtures. Records include wall
and CPU time, peak RSS, files per second, cache hit rate, and per-pass time.
Median or P95 wall-time growth above 10% blocks. The 100,000-line fixture also
blocks above 512 MiB peak RSS.

## Built artifacts

```bash
target/release/rust-doctor-eval smoke \
  --binary target/release/rust-doctor \
  --no-default-binary target/no-default/release/rust-doctor \
  --schema schemas/report-v1.schema.json \
  --npm-root npm
```

The smoke suite invokes terminal, score, JSON, SARIF, baseline, malformed and
failure paths from built binaries. It checks MCP initialize, tool discovery,
scan and cancellation, then packs and installs the current platform npm wrapper
with Bun and verifies that it launches the byte-identical embedded binary.
