# Spike US-019: is verbosity a profitable strategy under core-v3?

Measured 2026-08-20 against `hexyl` at the commit `tests/corpus.json` pins
(`abc20a380c8c2d9d76c1976222725d3211cef809`), scanned with the release binary
built from this working tree, toolchain 1.97.1. The clone came from the local
corpus cache, the padded copies were built outside this repository, and nothing
reached the network. No corpus source is committed here: this file is the whole
artifact.

`hexyl` was picked because it is small enough to scan seven times (2,010
production lines), its worst tier is P3 so no ceiling hides the density, and it
publishes 74, the closest of the ten healthy repositories to a band edge.

## What was padded

Both fillers are new modules declared from `src/lib.rs`, so the walk counts them
as production source and Clippy compiles them.

Rule-free filler is documentation and one public constant per module: a doc
banner, `pub const PADDING_MARK_i`, then comment and blank lines. It is what a
line counter defined as `str::lines` cannot tell from code, and no catalogued
rule fires on it.

Duplicated filler is the same nine-line function body repeated under different
names, which is what a verbose generator actually emits.

Both were written in files of 700 lines so that `oversized_unit`, which reports
at 1,000, never fires on the padding itself. That bound is the only constraint
the strategy has to respect, and respecting it is free.

## The two curves

| Padding | Production lines | Rule-free score | Duplicated score |
| --- | --- | --- | --- |
| none | 2,010 | 74 (Needs work) | 74 (Needs work) |
| +10 % | 2,212 | 76 | 75 |
| +50 % | 3,017 | 80 | 79 |
| +100 % | 4,023 | **84 (Great)** | **83 (Great)** |

Per dimension at +100 %, from 37 / 39 / 88 at the baseline: maintainability
reaches 61 with rule-free filler and 56 with duplicated functions, reliability
59 with either, performance 94 with either. Security and dependencies stay at
100, since neither producer behind them divides by kilolines.

The structural rules the duplicated filler fired: exactly one additional
`rust_doctor::structure::duplicate_function_body`, at every padding size.
`near_duplicate_function_body` (1), `complex_function` (3) and `oversized_unit`
(1) are the baseline's own and did not move. One family is one diagnostic
whatever its `related` array names, so 20 copies and 200 copies of the same
function cost the same single site.

## Verdict

Padding is profitable at every measured ratio, and the two curves are one point
apart. Doubling the workspace with text that carries no code at all buys ten
points and moves `hexyl` across the band edge into `Great`. Duplicating existing
functions instead of writing comments costs one point of that gain.

The break-even is exact rather than empirical. A per-kiloline dimension holding
`N` weighted sites over `K` kilolines is unchanged by `ΔK` kilolines of padding
carrying `F` new weighted sites when `F / ΔK = N / K`, its current density. For
`hexyl` that is 2.99 sites per kiloline on maintainability (6 scored sites over
2.010 kilolines) and 8.46 on reliability (17 of its 18 sites: the eighteenth is
`rust_doctor::cargo::unchecked_release_overflow`, which cargo-health divides by
one, so the 9.46 the corpus record publishes for that dimension is this density
plus that constant, and padding dilutes only the first term).

The duplicated-function generator produces exactly one new site whatever the
padding, so its detection rate is 4.95 sites per padded kiloline at +10 %, 0.99
at +50 % and 0.50 at +100 %: it opens above the maintainability break-even and
falls through it at about 335 padded lines, then keeps falling, because the
family stays one site however many copies it holds. That crossing is visible in
the dimension the family lands in: duplicated padding takes maintainability from
37 to 35 at +10 %, the one measured point where detection outruns the
break-even, then to 46 at +50 % and 56 at +100 % as the same single site
dilutes. The total rises anyway, 74 to 75 to 79 to 83, because no other
per-kiloline dimension gains a single site and reliability and performance
decay against the larger denominator. Padding stops being profitable at roughly
one detected maintainability site per 335 lines written, or one reliability site
per 118, and no filler a generator would plausibly emit sustains that past the
first few hundred lines.

So A1 is a number: **the catalog charges nothing at all for comment padding and
one site for a hundred duplicated functions, which by +100 % is a sixth of the
density that dimension would have to be charged to hold its score, and the
charge lands in the one dimension weighted 2 of 13 while every other dimension
is credited with the padding.**

## Follow-up, and what is not adopted now

Filed here rather than as a change, because every mitigation below is either a
Non-Goal of this PRD or a rule change the admission procedure owns.

**The mitigation considered:** charge a clone family per member rather than per
family, so padding by duplication scales its own cost. Not adopted. It
contradicts FR-4 and the published invariant that a structural family is one
site, it would charge legitimate repetition the same way, and it is a
re-adjudication of three catalogued rules, which the Non-Goals of this PRD
exclude. It also only touches the duplicated curve: the rule-free curve, which
is the more profitable of the two, has nothing to charge.

**Counting non-blank non-comment lines** would answer the rule-free curve
directly and is the explicit Non-Goal "Physical lines, one definition, frozen".
Not adopted here; it is the first thing to reconsider if the band a padded
workspace publishes ever matters commercially.

**A padding detector**, a rule reporting a unit that is mostly comment or
generated table, is the shape that fits the catalog rather than the score. It
needs a fixture, a corpus adjudication and an evidence record, which is
`rule-candidate` and `rule-admit`, not this epic.

**What already mitigates it:** the gate that runs in CI is `--scope baseline`,
which reports only what a change introduced. Padding a branch to raise its
absolute score does not remove a single finding the baseline comparison reports,
so the profitable move exists only against the absolute number a reader looks at
once.

## Replaying it

Clone the pinned commit into a cache outside this repository, copy it per
variant, write `padding_<i>.rs` files of at most 700 lines into `src/` and
declare them from `src/lib.rs`, then scan each copy with its own
`CARGO_TARGET_DIR`:

```bash
RUST_DOCTOR_STRUCTURE_TIME_BUDGET_SECS=600 \
CARGO_TARGET_DIR=<scratch outside this repository>/<variant> \
  rust-doctor <padded copy> --json --yes
```

Read `audit.production_lines` and `audit.score` from each report. The padding
sizes are 10 %, 50 % and 100 % of the 2,010 baseline lines.
