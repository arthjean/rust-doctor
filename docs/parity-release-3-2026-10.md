# Release 3 corpus replay: the rule families Clippy cannot see

Frozen record of EP-006 (`tasks/prd-react-doctor-parity.md`, US-028 to
US-033), replayed on 2026-09-26 with toolchain 1.97.1 from the clone cache,
both populations, under the catalog of 133 rules and the schema 20 report. The
seven rules this epic admitted are `clippy::undocumented_unsafe_blocks`,
`clippy::missing_transmute_annotations`, `clippy::crosspointer_transmute`,
`rust_doctor::source::blocking_call_in_async`,
`rust_doctor::source::sql_built_from_format`,
`rust_doctor::cargo::vulnerable_dependency` and
`rust_doctor::cargo::unmaintained_dependency`.

## Command

```bash
RUST_DOCTOR_CORPUS_DIR=~/.cache/rust-doctor/corpus \
RUST_DOCTOR_CORPUS_ARTIFACTS=~/.cache/rust-doctor/corpus-artifacts-ep006 \
cargo +1.97.1 test --test corpus_precision -- reproduce
```

Both reproduction tests passed, and the attestation relocated all 706
published sites. The harness now scans with `--advisory-db /dev/null`: the
RustSec database a machine holds is whatever `cargo audit` last fetched, so
reading it would make the record differ per machine and per day. Both advisory
rules are therefore `not_evaluated` on the corpus and measured on the fixture
database only.

## Per repository

"Before" is `tests/corpus.json` at commit `9b8c0ab`, "after" is this replay.
Findings count the production sites of the seven new rules. The agent
population is scanned with every Clippy rule off.

| Population | Repository | New-rule findings | Score before | Score after | Authoritative before | Authoritative after | Reasons |
|---|---|---|---|---|---|---|---|
| healthy | `anyhow` | 57 `undocumented_unsafe_blocks` | 88 | 73 | true | true | none |
| healthy | `async-channel` | 0 | 97 | 97 | true | true | none |
| healthy | `bytes` | 49 `undocumented_unsafe_blocks` | 89 | 82 | true | true | none |
| healthy | `fd` | 1 `undocumented_unsafe_blocks` | 87 | 87 | true | true | none |
| healthy | `hexyl` | 0 | 93 | 93 | true | true | none |
| healthy | `log` | 3 `undocumented_unsafe_blocks` | 90 | 90 | true | true | none |
| healthy | `ripgrep` | 0 | 90 | 90 | true | true | none |
| healthy | `serde_json` | 3 `undocumented_unsafe_blocks` | 88 | 88 | true | true | none |
| healthy | `smol` | 0 | 100 | 100 | true | true | none |
| healthy | `thiserror` | 0 | 85 | 85 | true | true | none |
| agent | `claudes-c-compiler` | 0 | 82 | 82 | true | true | none |
| agent | `ostendo` | 0 | 84 | 84 | false | false | `stage-failed` |
| agent | `artifexprocal` | 0 | 79 | 79 | true | true | none |
| agent | `mdr` | 0 | 88 | 88 | true | true | none |
| agent | `ralph-os` | 0 | 89 | 89 | true | true | none |
| agent | `vibesql` | 1 `sql_built_from_format` in production, 2 in tests; 3 `blocking_call_in_async` in tests | 73 | 58 | false | false | `stage-failed` |
| agent | `syncvibe` | 0 | 80 | 80 | true | true | none |
| agent | `coda` | 0 | 88 | 88 | true | true | none |

## What moved

`undocumented_unsafe_blocks` is the only new rule the healthy population
triggers, on 113 production sites. Its sample follows the protocol: a stride of
20, two passes blind to each other, `claude-opus-5-5` for both. Fifteen pairs
agree, twelve true positives and three false positives (an `unsafe fn` whose
stated contract the block is the direct use of, a one-line wrapper forwarding
`ptr::read`'s contract, and a transmute whose full soundness argument sits in a
comment not prefixed `SAFETY:`). Five pairs split and stay escalated for a human
verdict. The published rate is 20 %, interval 7 % to 45 %, above the 5 %
threshold, so the gate names the rule `noisy_on_healthy_code` and the score
discounts its sites by that rate. That discount is what holds `anyhow` at 73,
in `Needs work`, and `bytes` at 82.

On the agent side the SQL rule reaches one production site, in
`crates/vibesql-cli/src/sqlite_io.rs`. It interpolates a table name escaped by
quote doubling and binds every value through `?` placeholders, since an
identifier cannot be bound: it reads as a false positive of the detector, and
its `P1` ceiling alone moves `vibesql` from 73 to 58. The agent population is
not adjudicated for native rules, so the site carries no verdict here.

The healthy median falls from 89.5 to 89 and the agent median stays at 83, so
the separation the distribution gate requires stays positive. The lambda table
and the gate verdict are unchanged. Six of the seven rules ship `unproven` and
joined `ADMISSION_DEBT` in `tests/corpus_precision.rs`.
