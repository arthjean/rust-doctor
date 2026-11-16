# The score block

`src/audit.rs` is the score and the category tallies: the two severity counts
the report publishes, the five dimensions and their weights, the tier ceilings
and what a rule is charged, under report schema 25 and score model core-v7.
`src/audit/density.rs` is the penalty itself, the severity and tier weights,
the per-producer denominators, the kiloline floor and the exponential the
density is read through.
`src/audit/ranking.rs` is what to repair first: the projection, the rules it
withholds and the discount both read.
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

What a site is charged. A rule's sites are weighed by severity, by the tier of
the rule, each tier doubling the one below, and discounted by the smoothed rate
the corpus adjudicated the rule wrong, an unmeasured rule at the 5000 basis
point prior that rate gives a rule nobody reviewed, so the ranking applies no
second factor. Findings sharing a declared `cause` weigh `1 + ln(n)` sites,
and a workspace-scope rule counts each cause once, scaled by
`(1 + ln(c)) / c`: 303 copies of one type, or one variable in three workflows,
is one decision. docs/core-v7-calibration-2026-11.md measured
the curve before it shipped. Under
core-v3 the tier was only a ceiling and the rate only ranked, so a site of
`await_holding_lock` cost what a site of `useless_vec` cost. An `Info` site
weighs zero and stays authoritative: it is the level a producer publishes a
fact the workspace cannot act on at, a `println!` in a binary target. An
uncatalogued note weighs nothing and voids nothing. What still drops
`authoritative` is published as `score.reasons`, a closed `ScoreReason` set,
and the flag is true exactly when it is empty. A reason is a stage that ran
and failed, so the score is partial; `score.unverified` names a check that
never ran, such as `advisories` without a database, and leaves the flag alone.

What core-v6 changed. A withheld rule caps nothing: `scoring_tier` in
`src/audit/ceilings.rs` skips a rule smoothed above 5000 basis points, since
one correct `process::exit` on such a rule held reliability at 75. Since
core-v7 such a rule also publishes at `Info` unless the request sets its
level, and a duplicate major warns only when members alone require one of its
versions, so a member manifest can remove it. Versions compare by Cargo
compatibility class, inline `#[cfg(test)]` items leave the denominator, and
schema 24 publishes `audit.score.ceilings[]` and
`audit.categories[].production`.

One climb for what to repair first. `RuleAggregation::projection` fills its
three places one at a time with the rule whose repair, on top of the ones
already named, gives the most points back through the ceilings the published
value takes, discounted by the corpus rate; when nothing moves, the rules
holding the worst tier come first. A static key ordered by density relief
named three `P3` rules on a workspace one `P1` finding held at 65 and promised
the 65 it already had, which `the_rule_holding_the_ceiling_is_named_first`
replays. A rule the corpus found wrong more often than right is withheld from
the projection and published as withheld: the old threshold was an expected
value rounding to zero millionths of a point, which no rule ever reached, so
the report recommended `print_stderr` in the entry point of a binary. The
report body lists the projected rules first, in projection order, then the rest
by expected repair value, so the two never disagree.

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

One model, per member too. `packages.rs` runs the same build over each
scanned member's production lines and the findings attributed to it, with the
workspace's reasons, and a narrower scope rebuilds them the way it rebuilds the
workspace. A finding no member claims, a repository finding or one in a file two
members reach, weighs on the workspace score alone. Core-v5 averages what
members claim on a rule counted against the workspace instead of adding it up,
so the workspace lands between its worst and best member: added up, paneflow's
dependencies sat at 37 under members at 51 and above. The block is absent rather
than empty on a report that failed before the workspace answered. A member
Clippy never linted has no score, and a workspace none of whose members was
linted has none either: a hundred over code nothing linted read as clean. A run
a signal stopped publishes no score at all.

`the_audit_holds_the_size_bound_it_scores_for` keeps every file of the module
under the 1000 lines `oversized_unit` reports, tests included: the block that
computes the score has to pass the rule it scores. `density.rs` and
`tests/scale.rs` came out of the same bound, since the model carries a curve, a
lambda table and its own denominators, and the pair of them would have taken
`src/audit.rs` back over the thousand lines it reports at; `ranking.rs` and
`tests/ranking.rs` followed for the same reason.

