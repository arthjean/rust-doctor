# Release 1 corpus replay: the correctness pack and the unscored population

Frozen record of US-006 (`tasks/prd-react-doctor-parity.md`), replayed on
2026-09-25 with toolchain 1.97.1 from the clone cache, both populations, under
the catalog that admits the 64 `std` members of Clippy's `correctness` group
(`docs/correctness-group-2026-09.md`) and the schema 18 report that publishes
uncatalogued warnings as unscored and names the reasons of a partial score.

## Command

```bash
RUST_DOCTOR_CORPUS_DIR=~/.cache/rust-doctor/corpus \
RUST_DOCTOR_CORPUS_ARTIFACTS=~/.cache/rust-doctor/corpus-artifacts-ep001 \
cargo +1.97.1 test --test corpus_precision -- reproduce
```

Both reproduction tests passed, and the attestation relocated all 671
published sites with the digest the record already carried.

## Per repository

"Before" is `tests/corpus.json` at tag v0.7.0, "after" is this replay.
Correctness findings count the diagnostics of the 64 admitted members. The
agent population is scanned with every Clippy rule switched off, which the
trust boundary requires, so its count is zero by construction and not by
measurement.

| Population | Repository | Correctness findings | Score before | Score after | Authoritative before | Authoritative after | Reasons | Stage errors |
|---|---|---|---|---|---|---|---|---|
| healthy | `anyhow` | 0 | 88 | 88 | true | true | none | none |
| healthy | `async-channel` | 0 | 97 | 97 | true | true | none | none |
| healthy | `bytes` | 0 | 89 | 89 | true | true | none | none |
| healthy | `fd` | 0 | 87 | 87 | true | true | none | none |
| healthy | `hexyl` | 0 | 93 | 93 | true | true | none | none |
| healthy | `log` | 0 | 90 | 90 | true | true | none | none |
| healthy | `ripgrep` | 0 | 90 | 90 | true | true | none | none |
| healthy | `serde_json` | 0 | 88 | 88 | true | true | none | none |
| healthy | `smol` | 0 | 100 | 100 | true | true | none | none |
| healthy | `thiserror` | 0 | 85 | 85 | true | true | none | none |
| agent | `claudes-c-compiler` | 0 | 82 | 82 | true | true | none | none |
| agent | `ostendo` | 0 | 84 | 84 | false | false | `stage-failed` | `source/module-not-found` |
| agent | `artifexprocal` | 0 | 79 | 79 | true | true | none | none |
| agent | `mdr` | 0 | 88 | 88 | true | true | none | none |
| agent | `ralph-os` | 0 | 89 | 89 | true | true | none | none |
| agent | `vibesql` | 0 | 73 | 73 | false | false | `stage-failed` | `source/module-not-found` |
| agent | `syncvibe` | 0 | 80 | 80 | true | true | none | none |
| agent | `coda` | 0 | 88 | 88 | true | true | none | none |

## What moved

No score moved. No admitted member fired on the ten healthy repositories,
which is what a group Clippy denies by default should find in code that passes
`cargo clippy`, so no member needed adjudication and none was demoted. The 64
members ship `unobserved` and joined `ADMISSION_DEBT` in
`tests/corpus_precision.rs`. None fires on the agent population, since Clippy
does not run there.

No healthy report carried an uncatalogued warning either, so the unscored
population changed no authoritative flag in the corpus.

The two non-authoritative agent reports now say why. Both carry
`stage-failed`, and in both the failed stage is `source`, with
`module-not-found`: a module declared under `tests/` that the source walk could
not resolve. The problem statement attributed these two partial scores to
uncatalogued warnings. The replay refutes that. They were already partial for
a stage failure that nothing published.

The score distribution, the lambda table and the gate verdict are unchanged,
so the `score_distribution` gate needed no decision.
