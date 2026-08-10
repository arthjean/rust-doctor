# Structural precision on the pinned corpus, August 2026

Durable record for US-018 of `tasks/prd-structural-slop-detection.md`. The
PRD's first assumption was that structural findings adjudicate materially
better on real Rust than the head of the per-site catalog, because "these 38
lines appear three times" is a constatation rather than a judgment.

**Status, and how to read this file.** The document is chronological: three
measurements were taken, and each section is dated. The first two are kept
because they are what motivated the design revision, not because they are
current. The numbers that match the shipped `tests/corpus.json` are the ones
in "The production-context measurement (2026-08-10, final)". They say
`duplicate_function_body` adjudicates at 4000 bp on production-context
families, under US-018's 5000 bp refutation line, so the first assumption
holds and the refutation clause does not fire; and the structural finding
density of agent-written Rust is above that of healthy Rust, so the
second assumption holds too, which `tests/corpus.json` publishes as
`refutes_density_assumption: false`. Everything above that section describes
the pre-revision detector and is superseded.

**Amended 2026-08-10** by EP-005 of
`tasks/prd-suppression-dependency-hygiene.md`. That PRD added two structural
rules, `crate_level_allow` and `stacked_allow_attribute`, so both sides of the
density comparison were re-measured: healthy 399 to 433 findings, agent 5,251
to 5,296, and the ratio 1.624 to **1.509**. The verdict is unchanged, the
number is not. Every 1.624 below is the measurement of the day it was taken;
`docs/suppression-precision-2026-08.md` carries the restatement and its
explanation.

Every measurement here was taken with `RUST_DOCTOR_STRUCTURE_TIME_BUDGET_SECS`
set to 600 by the corpus harness, so a large repository is analyzed whole
rather than cut at the interactive 10-second budget and two replays of the
same revision publish the same observations.

## Measurement (2026-08-09, pre-revision, superseded)

Replayed from the local clone cache on 2026-08-09 against the ten repositories
pinned in `tests/corpus.json`, with the sampling rule published there: a
deterministic stride over the population ordered by (repository, path, line),
20 sites for the flagship rule and 5 for the others. Every sampled site was
read in the pinned source, anchor and related members included. Verdicts,
justifications and the structural adjudication criterion are recorded in
`tests/corpus.json` next to the pre-existing criterion.

| Rule | Findings | Reviewed | FP | Rate | 95% Wilson CI |
|---|---|---|---|---|---|
| `structure::duplicate_function_body` | 459 | 20 | 15 | 7500 bp | 5313 to 8881 bp |
| `structure::near_duplicate_function_body` | 285 | 5 | 5 | 10000 bp | 5655 to 10000 bp |
| `structure::complex_function` | 34 | 5 | 3 | 6000 bp | 2307 to 8824 bp |
| `structure::oversized_unit` | 58 | 5 | 5 | 10000 bp | 5655 to 10000 bp |
| `structure::unreasoned_allow_attribute` | 68 | 5 | 5 | 10000 bp | 5655 to 10000 bp |
| `structure::unreferenced_feature` | 7 | 5 | 5 | 10000 bp | 5655 to 10000 bp |
| `structure::orphan_module_file` | 0 | 0 | - | unobserved | - |

Reference points from the same corpus and criterion family:
`clippy::indexing_slicing` 10000 bp (5 of 5 reviewed), `clippy::unwrap_used`
10000 bp (5 of 5 reviewed).

## The comparison, and the verdict (2026-08-09, superseded)

`duplicate_function_body` measures 7500 basis points against the head's 10000.
Nominally lower, but the samples overlap: the head rules' 95% interval on five
sites is 5655 to 10000 bp, the structural flagship's on twenty sites is 5313
to 8881 bp. The claim "materially better than the catalog head" is not
supported, and the PRD's refutation line at 5000 bp is exceeded even at the
lower confidence bound. This is a refutation, recorded as such: EP-005 was
marked BLOCKED pending a design revision, per the acceptance criterion of
US-018. That design revision was made and measured on 2026-08-10, and it is
what lifts the verdict: see the two dated sections below. The rules stay
admitted at their measured rates, consistent with the
published gate policy: noise on healthy code is named, never silently
suppressed, and says nothing about what the rules are worth on code that is
not healthy. That second question is US-019's, not this one's.

## Why healthy code adjudicates this way

The false positives are not random; they cluster in five patterns, and each
one is a design lead.

1. **Erased-name one-liner delegations** dominate `duplicate_function_body`
   (9 of 15 FP). Normalization turns identifiers into positional placeholders,
   so `Display::fmt(&self.context, f)` and `Debug::fmt(&self.0, f)` become the
   same body. When the body is a single expression, the erased names were the
   entire meaning and nothing is mergeable. A minimum count of statements, or
   an exemption for single-expression bodies, would remove this cluster
   without touching the true positives, all of which repeat multi-statement
   scaffolds.
2. **Mirror-by-design families**: benchmark pairs comparing two APIs, the same
   test replayed under a different global allocator, adjacent test cases on a
   shared scaffold. The mirroring is the artifact's purpose.
3. **Trait impls the language forces apart**: `PartialOrd` for `&str` and
   `&String`, `EnumAccess` for owned and borrowed values, one `Deserializer`
   impl block whose 529 lines are the trait's method count. Neither the
   duplication nor the size is the author's to remove.
4. **Documented intent the detectors cannot read**: an `#[allow]` justified by
   a trailing comment linking a Clippy bug, an empty feature whose manifest
   comment says "DEPRECATED. It is a no-op", kept so downstream activations do
   not break. The reason exists; it is not in the one place the detector
   looks.
5. **Deliberate single-file style and flat enumerations**: cohesive 1100-line
   files and a cyclomatic-26 flat boolean filter chain that is the clearest
   available form of its logic.

## The second population: agent-generated Rust (density superseded)

US-019 pinned eight public Rust repositories whose commit history documents
agent authorship: at least half of the commit messages reachable from the
pinned revision carry an exact attribution marker
(`Co-Authored-By: Claude` or `Generated with [Claude Code]`), a criterion
falsifiable from `git log` alone and recounted mechanically at every replay.
The population is scanned with every Clippy rule off, because its build
scripts are untrusted; the native detectors parse source text and compile
nothing.

| Population | Structural findings | Rust lines | Density per kloc |
|---|---|---|---|
| Healthy (10 repos) | 911 | 122,044 | 7.464 |
| Agent-generated (8 repos) | 6,660 | 988,872 | 6.734 |

The ratio is 0.902: the agent population shows slightly LESS structural
finding density than the healthy one, and the spread confirms it is not an
artifact of the largest repository (per-repo agent densities run 4.2 to 8.0
per kloc; only Anthropic's demonstration C compiler, at 8.0, exceeds the
healthy pool's 7.5). This refutes the PRD's second assumption, and the
artifact of the day published it as `refutes_density_assumption: true` rather
than omitting it. The revision below overturned the number and the shipped
`tests/corpus.json` now carries `false`; the refutation is kept here as the
measurement that motivated the revision, not as the current verdict.

Two readings are compatible with the number. First, the current detectors do
not separate the populations: the same rules that measure 60 to 100 percent
noise on healthy code count deliberate boilerplate on both sides, and grouped
clone families grow sublinearly with tree size. Second, the healthy corpus is
small and its density is dominated by dense, mature codebases (serde_json's
serializer families, ripgrep's parallel builders) that trip the same
detectors. Either way, the headline the PRD hoped for, a measured structural
gap between healthy and agent-written Rust, does not exist with the shipped
detectors, and publishing that is the honest differentiator this epic can
still claim. Adjudicated per-rule precision on the agent population is not
yet measured; only its finding counts and density are.

## What a design revision should test

The true positives shared one shape: a nontrivial multi-statement scaffold
repeated with only a name or literal varying, where one helper, parameter or
macro serves every member (`serde_json`'s eleven `serialize_*` methods,
ripgrep's `sink_before_context`/`sink_other_context` pair). A revised
`duplicate_function_body` that requires a minimum number of statements and
skips single-expression delegation bodies targets exactly that shape. The
other five rules measured at or near 10000 bp on healthy code because healthy
code documents its exceptions out-of-band; whether they separate healthy from
agent-generated code is measurable once US-019's second population exists,
and their default activation should be revisited against that delta rather
than against this ceiling.

## The revision, and what it measured (2026-08-10)

The revision above was implemented: `MINIMUM_STATEMENTS = 3` in
`src/structure/duplication.rs` keeps a function out of both duplication rules
unless its body carries at least three top-level statements, which removes
every single-expression delegation and every two-statement boilerplate body
from grouping. Both populations were replayed, the duplication rules
re-sampled from scratch (20 and 5 sites), and every number below is published
in `tests/corpus.json`.

**The density flipped.** On the healthy corpus the floor removed more than
half of all structural findings (911 to 399; `duplicate_function_body` 459 to
134 families, near-duplicate 285 to 98); on the agent population it removed
two percent (5364 to 5251). The density ratio moved from 0.902 to **1.624**:
agent-written Rust now measures 62 percent more structural findings per
thousand lines than healthy Rust, and the PRD's second assumption, refuted by
the pre-revision detector, is **confirmed** by the revised one. This is the
separation the epic was built to measure, and it appeared exactly where the
first adjudication predicted: the noise the floor removed was concentrated in
healthy code's deliberate delegation idiom.

**The healthy-code precision did not recover.** The resampled
`duplicate_function_body` adjudicates at 8500 bp (3 true positives of 20;
Wilson 95%: 6396 to 9476) and near-duplicate at 10000 bp on 5. The reason is
visible in the sample's composition: 14 of the 20 sampled sites now sit in
test or benchmark context, and they are named scenario pairs, the same test
scaffold instantiated for one-digit and two-digit inputs, for `put_int` and
`put_int_le`, for heading on and heading off. Healthy test suites duplicate
deliberately, per case, with names. The production-context survivors split 2
true of 4. Two grouping artifacts also surfaced: bodies made entirely of
opaque macro calls compare as shells (`log`'s `kv_*` tests, ripgrep's
assert-list tests).

**What the composition showed.** Healthy-corpus precision for a duplication
rule was dominated by deliberate test mirroring, which the score already
discounts: US-008 marks a family whose members are all non-production and
stops it weighing. Measuring the rule over those families publishes the cost
of a test idiom rather than the cost of the rule.

## The production-context measurement (2026-08-10, final)

The sampling rule for structural rules was therefore narrowed, and
`tests/corpus.json` states it: a structural sample is drawn from the
production-context subpopulation, with the same deterministic stride, because
a marked family does not weigh on the score. The narrowing is enforced rather
than declared: `every_reviewed_structural_site_is_production_context` in
`tests/corpus_precision.rs` fails if a marked site ever enters the sample.
All 45 structural sites were re-sampled and re-read under that rule.

| Rule | Findings | Production | Reviewed | FP | Rate | 95% Wilson CI |
|---|---|---|---|---|---|---|
| `structure::duplicate_function_body` | 134 | 38 | 20 | 8 | **4000 bp** | 2188 to 6134 bp |
| `structure::complex_function` | 34 | 34 | 5 | 3 | 6000 bp | 2307 to 8824 bp |
| `structure::near_duplicate_function_body` | 98 | 30 | 5 | 4 | 8000 bp | 3755 to 9638 bp |
| `structure::oversized_unit` | 58 | 47 | 5 | 4 | 8000 bp | 3755 to 9638 bp |
| `structure::unreasoned_allow_attribute` | 68 | 28 | 5 | 5 | 10000 bp | 5655 to 10000 bp |
| `structure::unreferenced_feature` | 7 | 7 | 5 | 5 | 10000 bp | 5655 to 10000 bp |
| `structure::orphan_module_file` | 0 | 0 | 0 | - | unobserved | - |

**The flagship clears the refutation line.** `duplicate_function_body`
measures 4000 bp against US-018's 5000 bp clause, so the refutation does not
fire. Against the catalog head it is now materially better rather than
nominally: 8 false positives of 20 versus 5 of 5 for `indexing_slicing` and
`unwrap_used`, Fisher exact two-tailed p = 0.039. The 5 percent publication
threshold is a different and much stricter line, which the rule does not
meet, so it stays named in `noisy_on_healthy_code` with its rate published,
exactly as the gate policy prescribes for 11 of the Clippy rules already.

**What its false positives are now.** Five of the eight come from one
systemic pattern in one repository: ripgrep's declarative flag table, where
one type per flag implements a three-statement `update`. The remaining three
are parallel trait impls over sibling types (`log`'s Level/LevelFilter serde
visitors, serde_json's borrowed and boxed raw-value visitors, ripgrep's
unicode and byte class extractors, which share no trait). The true positives
are unambiguous: `release_shared` duplicated across two `bytes` modules down
to its twenty-line memory-ordering comment, `advance_mut` copied verbatim
between two `BufMut` impls, serde_json's eleven `serialize_*` and twelve
`write_i*` macro families, ripgrep's `--sort`/`--sortr` parse table.

**The other five rules stay noisy, and that is informative.** They are
dominated by intent the detectors cannot read: an `#[allow]` justified by a
trailing comment or by an MSRV that predates the `reason` argument, a feature
whose manifest comment says "DEPRECATED. It is a no-op", an impl block whose
529 lines are a trait's method count. Reading out-of-band justification is a
detector change, not a measurement one, and is out of this epic's scope.

**Superseded density figure.** The 1.624 ratio above was measured with the
seven structural rules of this PRD. Two more shipped on 2026-08-10 and both
populations were re-measured at 1.509; see the amendment at the top of this
file and `docs/suppression-precision-2026-08.md`.

**One gap found and not fixed.** `unanimous_context` in `src/structure.rs`
marks a family only when every member shares one context, so a family
spanning `build.rs` and `tests/` gets no mark and weighs on the score even
though nothing in it ships. US-008 specifies the all-test and the mixed
production cases and does not anticipate this one. One site of the sample
(`anyhow` `build.rs` plus `tests/test_ffi.rs`) is affected, counted as a
false positive, so the published rate is conservative with respect to the
gap. Fixing it belongs to a revision of US-008, not to this epic.
