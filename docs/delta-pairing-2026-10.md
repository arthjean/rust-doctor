# Delta pairing on real history

Frozen record of US-028 (`tasks/prd-react-doctor-gap-closure.md`), measured on
2026-10-14. It answers one question before US-029 changes what a
`--scope baseline` run calls introduced: which pairing removes the refactor
noise of today's delta without hiding a finding the commit wrote.

## Method

`tests/delta_pairing.rs` replays the pairs. Each side was scanned once by the
release binary of `1f5d913` (rust-doctor 0.7.0, schema 21, toolchain 1.98.0)
with `--json --yes --advisory-db /dev/null`, and the pairing ran on the two
reports and the two trees extracted with `git archive`:

- (a) today's evidence pairing, called through the library
  (`rust_doctor::delta_harness::pair`);
- (b) (a), then each leftover current finding paired with a leftover base
  finding of the same rule on the Git-renamed path whose line corresponds
  through `git diff --unified=0 --find-renames`: the same line shifted by the
  hunks above it, or both lines inside one modified hunk;
- (c) react-doctor's count per (renamed path, rule) over every finding;
- (d), added to inform the choice: (a), then (c)'s count applied to the
  leftovers alone.

Pairs: the 50 last commits of `1f5d913` touching `src/`, from `1f6f2cbd3`
back to `1ea323516`, and the 10 last commits touching a `.rs` file of each of
the 8 agent-population repositories, from the commit `tests/corpus.json` pins.
A pair whose two sides carry different errors is excluded, since it measures a
pass that ran on one side only: two syncvibe pairs (a build failure) and three
vibesql pairs (the structure time budget). 125 pairs were measured. ralph-os
needs a nightly target, so its Clippy pass failed on both sides of every pair
and its 10 pairs measure the native findings alone.

Every finding (a) introduces and (b), (c) or (d) does not is a candidate. Each
of the 36 was classified by reading the diff: false when the base already held
it and the commit moved or edited it, true when the commit wrote it. The
verdicts are `tests/fixtures/delta-pairing/verdicts.json`. A count policy
cannot say which member of a group it pairs, so in a group of `f` false and `t`
true candidates where it removes `k`, it is credited `min(k, f)` false removed
and charged `k - f` true hidden.

## Result

| Repository | Pairs | (a) | (b) | (c) | (d) | (a) fixed | (a) cross-file |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| rust-doctor | 50 | 63 | 62 | 70 | 60 | 40 | 10 |
| artifexprocal | 10 | 64 | 57 | 57 | 57 | 17 | 0 |
| claudes-c-compiler | 10 | 114 | 114 | 114 | 114 | 8 | 0 |
| coda | 10 | 3 | 2 | 2 | 2 | 1 | 0 |
| mdr | 10 | 22 | 20 | 19 | 19 | 4 | 0 |
| ostendo | 10 | 46 | 39 | 49 | 38 | 20 | 11 |
| ralph-os | 10 | 8 | 7 | 7 | 7 | 1 | 0 |
| syncvibe | 8 | 16 | 15 | 14 | 13 | 32 | 1 |
| vibesql | 7 | 23 | 21 | 21 | 20 | 4 | 1 |
| **Total** | **125** | **359** | **337** | **353** | **330** | 127 | 23 |

Of the 36 candidates, 29 are false introductions and 7 true.

| | (b) | (c) | (d) |
| --- | ---: | ---: | ---: |
| False introductions removed | 22 | 29 | 29 |
| True introductions hidden | 0 | 0 | 0 |
| False introductions added over (a) | 0 | 23 | 0 |
| Same-file swaps hidden | 0 | 0 | 0 |

(c) adds 23 introductions (a) does not make: every one is a finding (a) paired
across files by its stable fingerprint, in a commit that extracted a module
without a Git rename (`1f6f2cbd3` moves the credential scan out of
`src/repo_hygiene.rs`, ostendo `2cca181f0` splits `content.rs` in three). A
count per file reads each as a fix in the old file and an introduction in the
new one. No pair shows a hidden swap, the case where (c) reports nothing
although a fix and a new finding of one rule meet in one file.

The 7 false introductions (b) leaves are edits its line map cannot see:

- a statement moved into another function and reformatted (rust-doctor
  `2d677d12c`, `src/hook/agent.rs`, two `print_stdout`);
- a rewrite whose new lines land in a pure insertion hunk (syncvibe
  `6e69b5ac0`, `chat.rs`, two `string_slice`);
- an anchor that moves one line when a constant is inserted between a doc
  comment and its function (mdr `7c06f15dc`);
- a function split into a wrapper and an `_impl` holding the old body (vibesql
  `b4cd87ab3`, `complex_function`);
- one test of a duplicate family shifted by inserted lines (ostendo
  `ffbb93b4b`).

The 7 true candidates are three new `println!` and one new `string_slice` in
syncvibe `6e69b5ac0`, two new `println!` in rust-doctor `2d677d12c`, and a
`println!()` that (a)'s first-in, first-out pairing reports at the old line
(203) while the commit inserted it at line 28: a true introduction, misplaced,
which is the defect US-029's nearest-line pairing removes.

## Decision rule

The PRD adopts (b) if it removes at least 80 percent of the false
introductions (c) removes. It removes 22 of 29, 75.9 percent, so the rule does
not adopt it and the choice goes to Arthur in the PRD's Open Questions. US-029
waits on it.

What the record puts in front of that choice: (c) as specified removes 29 and
adds 23, a net 6, because it gives up the evidence pairing that follows code
across files. (d) keeps that pairing and removes the 29 without adding any. (b)
and (d) hid nothing on these 125 pairs; their blind spots differ in reach. (b)
can hide a swap inside one hunk, (d) a swap anywhere in one file, and (d)
cannot say which member of a group it paired, which US-029's nearest-line
ranking would have to decide.

## Reproducing it

```bash
cargo test --release --test delta_pairing -- --ignored --nocapture
```

It reads the clone cache at `RUST_DOCTOR_CORPUS_DIR`, else
`~/.cache/rust-doctor/corpus`, and skips with a message when it is absent. It
scans with `RUST_DOCTOR_PAIRING_BINARY`, else the binary the build made, only
the commits whose report is not already under `target/delta-pairing/reports`,
and writes `target/delta-pairing/results.json`.
