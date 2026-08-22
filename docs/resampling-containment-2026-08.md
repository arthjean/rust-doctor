# Re-sampling containment after the context correction, August 2026

Durable record for US-005 of `tasks/prd-measurement-integrity.md`. The spike
produces no code. It answers one question, asked before US-007 re-adjudicates
anything: once the out-of-line `#[cfg(test)]` correction shrinks the four agent
structural populations, does the deterministic stride still land on the sites
already on record, or does re-sampling orphan verdicts that were paid for?

Measured on 2026-08-22, from a gated reproduction of the pinned corpus under
the corrected classification, with `RUST_DOCTOR_STRUCTURE_TIME_BUDGET_SECS` at
600 as every published structural measurement is. The population of a scope is
its production-context subpopulation, one entry per `(repository, path, line)`,
ordered by that same triple, which is the order `adjudication.sampling_plan`
draws over.

## What the correction removes

| Scope, agent population | Population before | After | Removed | Reviewed | Surviving |
|---|---|---|---|---|---|
| `oversized_unit` | 828 | 811 | 17 | 20 | 18 |
| `near_duplicate_function_body` | 204 | 186 | 18 | 20 | 17 |
| `duplicate_function_body` | 226 | 202 | 24 | 19 | 17 |
| `orphan_module_file` | 16 | 12 | 4 | 16 | 12 |

"Surviving" is the reviewed sites the corrected population still holds. The
eleven that do not survive are the eleven
`every_reviewed_structural_site_is_production_context` fails on before the
correction: nine were reclassified by the out-of-line `#[cfg(test)]` fix, and
the remaining two sit on
`crates/vibesql-storage/benches/hnsw_recall_benchmark.rs`, families straddling
the bench and `crates/vibesql-storage/src/database/indexes/hnsw.rs`. Those two
abstain and are still charged, correctly, because their production member is
real duplication; they leave the sample rather than the report.

The "After" column is the population under the predicate the record now
publishes, which is a second correction on top of the classification fix and
the reason two of these figures moved after this spike was first written. The
absence of a `context` field is not sufficient on its own: a finding whose
anchor sits under a `tests`, `benches` or `examples` directory, or in a file
named `tests.rs`, is excluded from the population even when the scan attributes
no context to it, because that is exactly the predicate
`every_reviewed_structural_site_is_production_context` applies to what the
record may publish. Measuring over a population wider than the one the record
may draw from is how a stride lands on a position no reviewed site can occupy,
which is what the two `hnsw_recall_benchmark.rs` families did: one stride
position and one carried position, both unjudgeable. The two duplication scopes
lose six and three further sites to it, `near_duplicate_function_body` reading
186 rather than 188 and `duplicate_function_body` 202 rather than 205, and both
lose one survivor.

The ordering was confirmed rather than assumed: the twenty reviewed
`oversized_unit` sites land on the recorded stride positions `[0, 41, 82, ...,
786]` of the population before the correction, and their positions after it
drift downward monotonically by the number of sites removed above them, 0 at the
head and 13 at the tail. So the population this spike enumerates is the
population the published plan was drawn over.

## Containment

The stride recomputes every position from `n`, so a population that loses 2
percent of its entries does not keep its sample. `stride(811, 20)` is
`[0, 40, 81, ...]` against the recorded `[0, 41, 82, ...]`: the two lists share
their first element and nothing else.

| Scope | Retained by the new stride | Dropped | Smallest target that contains every survivor |
|---|---|---|---|
| `oversized_unit` | 1 of 18 | 17 | 712 |
| `near_duplicate_function_body` | 4 of 17 | 13 | 164 |
| `duplicate_function_body` | 4 of 17 | 13 | 176 |
| `orphan_module_file` | 12 of 12 | 0 | 12 |

`orphan_module_file` contains totally, and for the reason it always did: its
corrected population of 12 is below `PROTOCOL_TARGET`, so
`k = min(20, 12) = 12` and the stride is the whole population. Its draw needs no
new site. What it does need is pairs: its sixteen verdicts predate the protocol
cutoff, so bringing it under the protocol means judging its twelve surviving
sites twice, not finding new ones.

## The mechanism, per US-005's third acceptance criterion

Raising the target to a containing multiple is refused for the three other
scopes. Containment costs 712, 164 and 176 doubly judged sites against a
protocol target of 20; the first alone is 35 times the target and is a census of
the scope, not a sample of it.

The mechanism is the second one US-005 names: **an explicit carry-over**. The
plan of each scope is redrawn at `PROTOCOL_TARGET` over the corrected
population, and the surviving sites the new stride does not select are kept in
`adjudication.reviewed` and named as carried over, at their positions in the
corrected canonical order.

It is chosen over a fresh draw that discards them for a reason that is not
sentiment about wasted work. Both cost the same adjudication: the sites the new
stride adds have to be judged either way, 19, 16 and 16 of them. What separates
the two is the sample the rate is then computed over, 37, 33 and 33 sites
against 20, drawn by two strides over orderings that differ by a few percent of
their entries. The carried sites are not a convenience subpopulation: they are a
uniform draw over almost the same list, so the union is a fair superset rather
than a sample with a shape of its own.

What US-007 has to build for it, since `sampling_plan` cannot express it today:

- `SamplingPlan` carries `carried_over`, the positions into the corrected
  canonical order of sites a previous plan of the same scope drew, still in the
  population, whose verdicts the record keeps. Sorted, unique, disjoint from
  `indices`, every position below `observed`.
- `sampling_defects` counts `indices.len() + carried_over.len()` where it counts
  `indices.len()` today, so a carried site is a site the plan accounts for
  rather than one it silently drops. `target` stays `PROTOCOL_TARGET` and
  `indices` stays `stride(observed, target)`: the carry-over extends the sample,
  it never edits the draw.
- `adjudication.sampling` states the carry-over in prose beside the stride, and
  a test refuses a carried position that is also drawn.

## Cost of US-007, from these numbers

63 sites need a pair they do not have: 19, 16 and 16 newly drawn, plus the 12
surviving `orphan_module_file` sites that were judged once before the cutoff.
Each is two passes blind to each other, so 126 verdicts, none of which may be
reached without opening the code at the pinned revision. The figure US-007
actually paid is higher, 46 more blind judgments, because its first draw ran
over the population before the path predicate above narrowed it: those sites
were judged, they are in the corrected population, and they are carried rather
than discarded, which is why the three samples ship at 37, 43 and 48 sites
rather than at the 37, 33 and 33 predicted here.

The rates the re-adjudication moves, stated here so US-007 states the same
previous values: `oversized_unit` 500 bp, `near_duplicate_function_body`
5000 bp, `duplicate_function_body` 3157 bp, `orphan_module_file` 625 bp. On the
surviving verdicts alone, before a single new site is judged, they read 555,
4705, 2352 and 833 bp. Those four numbers are not a result: they are what the
shortened samples say, and they are published here only to show the direction
the correction pushes each scope, which is down for the two duplication rules
and up for the two others.

## Verdict

Containment is achievable for every one of the four scopes. US-007 is not
blocked.
