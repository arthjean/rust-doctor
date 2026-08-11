# What the rules are worth on agent-authored Rust, August 2026

Every rate rust-doctor published until now was measured on ten public
repositories chosen for their health: crates from dtolnay, BurntSushi and
smol-rs. That is the wrong population to calibrate on. The tool exists for
people writing Rust with a coding agent, and nine of its rules had never fired
on the healthy corpus at all, which said nothing about whether they work.

This file publishes the first measurement on the second population: eight
repositories selected by commit-trailer evidence alone, at least half of all
commits carrying an agent attribution marker. Every number is in
`tests/corpus.json` under `agent_population.precision` and re-derivable from it.

```bash
RUST_DOCTOR_CORPUS_DIR=<clone cache outside this repository> \
RUST_DOCTOR_CORPUS_ARTIFACTS=<scratch outside this repository> \
cargo test --test corpus_precision
```

## What can be measured here, and what cannot

Clippy is switched off on this population. Its repositories are untrusted code,
and running Clippy means compiling them, which means executing their build
scripts and procedural macros. `the_agent_population_is_scanned_without_executing_untrusted_build_code`
holds that line, and a test now also refuses any Clippy rule carrying a rate on
this side.

So `clippy::indexing_slicing` and `clippy::string_slice`, the two rules the
ranking currently withholds, can never be measured here. The question of which
population should decide their rank has no answer and will not get one: the
healthy corpus is the only place they can be observed at all.

## The measurement

| Rule | Agent | Healthy |
|---|---|---|
| `rust_doctor::cargo::missing_lockfile` | **0 %** over 3 of 3 sites | never triggered |
| `rust_doctor::structure::orphan_module_file` | **6.25 %** over 16 of 16 sites | never triggered |
| `rust_doctor::structure::complex_function` | **70 %** over 40 of 1541 sites | 87.09 % over 31 of 34 |
| `rust_doctor::structure::stacked_allow_attribute` | 50 % over 2 of 2 sites | one site, in a test |

Two rules were adjudicated and still publish no rate, which is the floor doing
its job rather than a gap: `unused_dependency` (4 manifests reviewed, no false
positive among them) and `test_only_dependency` (2 reviewed, 1 false positive)
sit under the five-site minimum. Findings that share a manifest also share a
position, so they collapse into one reviewable site, and four manifests are not
a rate. They are reported here as an observation and withheld there as a
measurement.

`orphan_module_file` is the headline. Sixteen sites, fifteen of them files that
Cargo genuinely never compiles: modules shadowed by a same-named file one
directory up, a `mod tests` written as an inline block so the file beside it is
dead, a regression test the manifest forgot to declare. The single false
positive is a file reached by a `#[path]` attribute the detector cannot follow.
On healthy code this rule had never fired once.

`missing_lockfile` is three binaries shipped without a committed lockfile, all
three real.

`complex_function` is the only rule measurable on both sides, and it is the
answer to the question this measurement was run to settle: 70 % against 87 %.
The two rates differ, and in the direction one would expect, but they do not
invert. A rule that is mostly noise on curated code is mostly noise on agent
code too.

## How the verdicts were produced

Sixty-seven sites, each judged twice by independent passes blind to each other,
each reading the site in the pinned source with its surrounding code. The
protocol is `.claude/skills/corpus-adjudicate`.

| Sample | Judged twice | Agreement | Cohen's kappa | Escalated |
|---|---|---|---|---|
| The five native rules | 27 | 27 / 27 | 1.0 | 0 |
| `complex_function` | 40 | 40 / 40 | 1.0 | 0 |

Unlike the Clippy rules measured on healthy code, where perfect agreement said
nothing because neither pass produced a single true positive, these kappas rest
on real variance: 24 true positives among the native sites, 12 among the
complexity ones. Two independent readers agreeing on which of forty functions
deserve splitting is a stronger signal than two readers agreeing that nothing
does.

## What this does not change, yet

The ranking still discounts by the healthy rate. `CORPUS_NOISE` in
`src/policy/catalog.rs` mirrors `precision`, not `agent_population.precision`,
and this measurement does not move it. Two reasons. The agent samples are small,
two to forty sites against a population of up to 1541. And switching the
reference population is a product decision about what the tool claims, not a
consequence of a number: it would mean saying that what a rule costs a
maintainer of `ripgrep` is not what should decide what the tool recommends.

What the measurement does settle is that the decision can now be taken on
evidence. It also settles the smaller question underneath it: the five rules
that had never fired on healthy code are not broken. They were waiting for code
that had not been read by anyone.
