# core-v7 calibration, 2026-11

This record measures the three changes of score model core-v7 on the corpus
before any of them enters the score, as assumption A3 of the field audit
remediation requires: a cause is charged `1 + ln(n)` sites, a workspace-scope
rule is charged `1 + ln(c)` over its `c` causes, and a rule the corpus never
measured is charged at the 5000 basis point prior the ranking already applies.
`src/audit/tests/calibration.rs` generates every figure below from
`tests/corpus.json` and from the audited workspace's counts. It is ignored by
default, since it measures the core-v6 record, and reruns with
`cargo test --lib calibration -- --ignored --nocapture`, which fails when the
figures below no longer match.

## Method

Six variants are computed: core-v6, each change alone, the three together,
and the three together with every withheld rule published at `Info`, the
demotion that ships beside them in the same release.

A corpus observation records each rule's distinct findings and each
dimension's density, not the site-level facts the score reads: which
findings are test code, which a member owns, which share a cause. So each
dimension's recorded density is split among its rules in proportion to their
core-v6 charge estimated from the counts, and a variant scales each rule's
share by what it changes in that rule's charge. The core-v6 variant therefore
reproduces every recorded score, which the harness asserts for all eighteen.
Tier ceilings are the recorded ones, since no variant moves a tier.

The corpus holds no `large_types_passed_by_value` finding and one
`secret_in_build_step` finding, so the cause variant equals core-v6 on it, and
the corpus measures the saturation and the prior.

The audited workspace was rescanned on Windows with the 0.10.0 binary at the
last revision committed before the audit was written. The audited revision was
never pinned, and the rescan differs from the audit by ten findings and 1133
lines while publishing the same score, the same 23 rules and the same five
dimensions. Its counts enter the harness without names: per rule, the
findings, the sites the score weighs, the sites a member owns, and the sizes of
the causes US-024 declares, read from the span and message of each finding.
Every weighed finding is a warning, so these variants are exact rather than
estimated.

## Verdict

The assumption holds. The three changes together keep a spread of 35 points,
keep the two populations across two bands, and keep the healthy median six
points above the agent one. Each change alone holds as well. US-025 and US-026
proceed.

Two effects are worth knowing before the replay of US-032 measures them
on the shipped binary. The prior lifts the one healthy subject under 75 into
Great, so the healthy population sits in one band, and the gate reads the bands
of both populations together, as the corpus suite does. The saturation moves
three agent subjects by three to seven points, through Dependencies, where
duplicate majors hold most of the findings, which is what US-028 changes again.

On the audited workspace the three changes take the score from 36 to 64: the
P1 secret weighs once, Security reaches its ceiling of 50, and the 315
by-value findings weigh as four causes.

## Figures

Corpus record generated 2026-10-29, model core-v6, 18 subjects.

### Per-subject scores

| Subject | Population | v6 | causes | saturation | prior | all three | + Info |
|---|---|---|---|---|---|---|---|
| anyhow | healthy | 73 | 73 | 73 | 76 | 76 | 76 |
| async-channel | healthy | 97 | 97 | 97 | 98 | 98 | 99 |
| bytes | healthy | 82 | 82 | 82 | 82 | 82 | 83 |
| fd | healthy | 86 | 86 | 87 | 88 | 88 | 88 |
| hexyl | healthy | 93 | 93 | 93 | 93 | 93 | 96 |
| log | healthy | 90 | 90 | 90 | 91 | 91 | 92 |
| ripgrep | healthy | 91 | 91 | 91 | 91 | 91 | 94 |
| serde_json | healthy | 88 | 88 | 88 | 89 | 89 | 90 |
| smol | healthy | 100 | 100 | 100 | 100 | 100 | 100 |
| thiserror | healthy | 88 | 88 | 88 | 91 | 91 | 94 |
| claudes-c-compiler | agent | 82 | 82 | 82 | 83 | 83 | 85 |
| ostendo | agent | 80 | 80 | 83 | 83 | 88 | 89 |
| artifexprocal | agent | 89 | 89 | 89 | 90 | 90 | 91 |
| mdr | agent | 65 | 65 | 65 | 65 | 65 | 65 |
| ralph-os | agent | 89 | 89 | 89 | 89 | 89 | 90 |
| vibesql | agent | 58 | 58 | 65 | 60 | 65 | 65 |
| syncvibe | agent | 80 | 80 | 86 | 84 | 86 | 87 |
| coda | agent | 83 | 83 | 83 | 84 | 84 | 85 |

### Distribution gate

| Variant | Spread | Healthy bands | Agent bands | Healthy median | Agent median | Gate |
|---|---|---|---|---|---|---|
| core-v6 | 42 | Great 9, Needs work 1 | Great 6, Needs work 2 | 89.00 | 81.00 | holds |
| cause weights alone | 42 | Great 9, Needs work 1 | Great 6, Needs work 2 | 89.00 | 81.00 | holds |
| workspace saturation alone | 35 | Great 9, Needs work 1 | Great 6, Needs work 2 | 89.00 | 83.00 | holds |
| unmeasured prior alone | 40 | Great 10 | Great 6, Needs work 2 | 91.00 | 83.50 | holds |
| all three | 35 | Great 10 | Great 6, Needs work 2 | 91.00 | 85.00 | holds |
| all three, withheld rules at Info | 35 | Great 10 | Great 6, Needs work 2 | 93.00 | 86.00 | holds |

### Audited workspace

1448 findings from 23 rules over 186759 production lines and 18 members.

| Variant | Score | Band | Security / Reliability / Maintainability / Performance / Dependencies |
|---|---|---|---|
| core-v6 | 36 | Critical | 5 / 60 / 70 / 63 / 1 |
| cause weights alone | 51 | Needs work | 37 / 60 / 70 / 95 / 1 |
| workspace saturation alone | 43 | Critical | 12 / 63 / 70 / 63 / 28 |
| unmeasured prior alone | 46 | Critical | 22 / 64 / 73 / 78 / 11 |
| all three | 64 | Needs work | 50 / 65 / 73 / 96 / 50 |
| all three, withheld rules at Info | 65 | Needs work | 50 / 66 / 76 / 96 / 50 |
