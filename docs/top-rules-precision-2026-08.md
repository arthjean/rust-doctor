# What the three rules the CLI recommends first are worth, August 2026

The scan ends on a line that tells the user what to fix: "Fix the top 3 rules to
reach a projected N/100". On this repository, and on most of the pinned corpus,
those three are `clippy::indexing_slicing`, `clippy::string_slice` and
`rust_doctor::structure::complex_function`. They were ranked by volume and
measured on five sites each, a sample `adjudication.sampling` already described
as unable to place a rule against the 5 % threshold.

This file publishes what they are worth on a sample that can. Every number is in
`tests/corpus.json` and re-derivable from it:

```bash
RUST_DOCTOR_CORPUS_DIR=<clone cache outside this repository> \
RUST_DOCTOR_CORPUS_ARTIFACTS=<scratch outside this repository> \
cargo test --test corpus_precision
```

Toolchain: cargo 1.97.1, rustc 1.97.1.

## The measurement

| Rule | Population | Reviewed before | Reviewed now | True positives | False positives | Rate |
|---|---|---|---|---|---|---|
| `clippy::indexing_slicing` | 241 | 5 | 40 | 0 | 40 | 100 % |
| `clippy::string_slice` | 40 | 5 | 40 | 0 | 40 | 100 % |
| `rust_doctor::structure::complex_function` | 34 | 5 | 31 | 4 | 27 | 87.09 % |

`clippy::string_slice` is the one that no longer needs an interval: 40 reviewed
of a population of 40 is the whole population, so 0 true positives is a count,
not an estimate. Every one of the forty slices takes its byte offsets from
something that already respects character boundaries: a `find` or `rfind`
result, an AhoCorasick match over the same string, an offset advanced by
`len_utf8`, the length of an ASCII literal a `starts_with` had just confirmed,
or a full-range `[..]`.

`clippy::indexing_slicing` is a stride of 40 over 241 sites, and it contains the
stride of 5 it replaces. With 0 true positives on 40 reviewed, the Wilson 95 %
interval puts the false-positive rate at or above 91 %. What establishes the
bound, in the sites read: a length check or a match arm fixing the length, a
fixed-size table indexed by a `u8`, an index produced by iterating the same
collection, a documented caller contract, or an invariant the module maintains.

`rust_doctor::structure::complex_function` is the only one of the three with
true positives: 4 of 31. They are functions stacking independent responsibilities
that a named helper could carry, `hexyl`'s `print_all` and `fd`'s `spawn_senders`
among them. The 27 false positives are the shape the criterion names: a match
over a closed enum, a state machine, a parser dispatch, a flat sequence of
independent cases. Splitting those scatters one decision across several places,
and a reviewer would reject it.

## How the verdicts were produced

Each added site was judged twice, by independent passes blind to each other,
each reading the site in the pinned source with its surrounding code rather than
the diagnostic alone. The protocol is `.claude/skills/corpus-adjudicate`.

| Rule | Judged twice | Agreement | Cohen's kappa | Escalated |
|---|---|---|---|---|
| `clippy::indexing_slicing` | 35 | 35 / 35 | undefined | 0 |
| `clippy::string_slice` | 35 | 35 / 35 | undefined | 0 |
| `rust_doctor::structure::complex_function` | 29 | 26 / 29 | 0.53 | 3 |

Two of those numbers say less than they look like they do. Kappa is undefined
for the Clippy rules because neither pass produced a single true positive:
with no variance there is nothing for chance agreement to be measured against,
so perfect agreement is what two passes reaching the same conclusion by the same
route would produce whether or not that conclusion is right. And 0.53 sits below
the 0.6 usually treated as the floor for acceptable agreement, which is an
honest description of the task: whether a function should be split is a
judgment, and two careful readers disagree about it roughly one time in ten.

The three sites where the passes disagreed are excluded from the sample rather
than settled by a tie-break, and wait for a human verdict:

| Site | Pass A | Pass B |
|---|---|---|
| `ripgrep` `crates/ignore/src/dir.rs:340` | true positive | false positive |
| `thiserror` `impl/src/expand.rs:31` | true positive | false positive |
| `thiserror` `impl/src/expand.rs:221` | true positive | false positive |

All three turn on the same question: whether four independent code generators
stacked in one function, or four near-identical matcher constructions differing
by a filename, are a decomposition a maintainer would accept. Excluding them
costs `complex_function` three sites of sample and leaves its rate resting on
verdicts both passes reached.

Provenance is recorded per site and published per rate. These three rules now
carry `["agent", "unrecorded"]`: the five original sites predate the field, the
rest were produced under the protocol above. A rate is worth its weakest
verdict, and none of these is certified beyond what two trained human reviewers
would agree on with each other.

## What this does not settle

The rules are not withdrawn, and measuring noise on ten repositories chosen for
their health says nothing about what a rule is worth on code that is not
healthy. `clippy::indexing_slicing` catching nothing in `ripgrep` is not
evidence that it catches nothing in a codebase written last week by an agent.

What it does settle is the ranking. The line that tells a user what to fix first
currently sorts by volume, and volume is exactly what these three rules have the
most of. Whether that line should weigh the measured rate is a product decision,
and this file exists so that it can be taken on numbers.
