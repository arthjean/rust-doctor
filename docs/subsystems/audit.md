# The score block

`src/audit.rs` is the score and the category tallies: the two severity counts
the report publishes, the five dimensions and their weights, the tier ceilings
and the ranking of what to repair first. `src/audit/density.rs` is the core-v3
penalty itself, the severity weights, the per-producer denominators, the
kiloline floor and the exponential the density is read through.
`src/audit/source_inventory.rs` is the source-file count the score is computed
against, read from Cargo's dep-info rather than from a walk of its own, and
`src/audit/tests.rs` carries the tests, with `src/audit/tests/scale.rs` holding
what the score does when the workspace changes size.

Five rules hold it together, and each of them replaced something that had a
cost.

One aggregation, and the set-aside inside it. `aggregate_rules` reads every
diagnostic the report publishes and decides for itself which of them the score
charges for, so `occurrences` is what the reader is shown and `numerator` is
what the score bills: one distinct site per diagnostic, weighed by severity,
whatever a clone family names through `related`. The filter used to sit at one
of the two call sites instead: the report body ranked by a cost computed over a
population the score never charged, and a rule that only ever fired in a test
was ranked as though it had cost points.

One key for what to repair first. `expected_repair_value` is what repairing a
rule is expected to be worth, its cost discounted by the rate the corpus
adjudicated it wrong, and the score's projection and the order of the report
body both read it. It used to be private with the raw cost published beside it,
so the report named a rule as withheld for measured noise on one line and put it
at the top of what to work down on the next.
`a_rule_the_corpus_measured_wrong_is_not_ranked_first` is the input that puts
the two in competition.

One fact, stored once. The categories are published in their declaration order
and nothing restates that order: `Ord` derives from it and the tally map is
keyed by it, so the second list, the position table that mirrored it and the
pass that re-sorted an already sorted map are all gone. The four bare severity
members of a category are the schema-v7 spelling of `occurrences`, projected by
`Serialize` rather than stored beside it: held as fields they needed a recopy
pass to write them and four clauses of `is_valid` to check they still agreed,
and `share_url` summed the copy while `totals` summed the original. And a
rebuilt scope reads the inventory's completeness from the block that kept it,
rather than recovering it from `score.authoritative`, which also carries the
status and whether every diagnostic was catalogued: one uncatalogued rule
anywhere made every later scope non-authoritative for a reason that had nothing
to do with the inventory, and a flag that is its own input is a flag `is_valid`
can never catch a forgery of.

Every bound is a budget on work, never a filter on meaning.
`DEP_INFO_BYTES_LIMIT` bounds what one comparison may read out of Cargo's
dep-info, once for the scan rather than once per artifact, and a dep-info that
cannot be read falls back to the target root and says the count is a floor
rather than dropping the file. `Confined` names the three things a candidate
path can be, because a path that is not workspace source is a fact and a path
that did not resolve is the absence of one, and only the second costs the
inventory its completeness. The dep-info walk consumes its slice rather than
indexing it, so where the cursor may land is bounded by the slice and not by
arithmetic a reader has to replay.

`the_audit_holds_the_size_bound_it_scores_for` keeps every file of the module
under the 1000 lines `oversized_unit` reports, tests included. This was one
file of 1624 lines, one of the two that
`the_self_scan_names_this_repository_s_own_hotspots` froze as oversized, and the
only module of the crate near the bound with no such test: the block that
computes the score has to pass the rule it scores. `density.rs` and
`tests/scale.rs` came out of the same bound, since core-v3 carries a curve, a
lambda table and its own denominators, and the pair of them would have taken
`src/audit.rs` back over the thousand lines it reports at.

