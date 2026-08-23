# The structural pass

`src/structure.rs` is the pass and nothing else: the family map, the deadline,
the identity of a finding. The four detector families sit beside it,
`suppression.rs` (3 rules), `duplication.rs` with `normalize.rs` (2),
`hotspots.rs` (2) and `manifest.rs` (2), and each publishes a `RULES` table.
`rules()` is their union, and
`the_pass_produces_every_catalogued_structural_rule` compares it against the
catalog, so a rule cannot be published by `rules list` and left out of the scan.

Three rules hold it together, and each of them replaced something that had a
cost.

One traversal per unit. `Inventory::of` walks the tree once and collects the five
node kinds the families read between them. Four walks used to spend the wall
clock four times over the same nodes, which is what forced the substring
pre-filter in `manifest.rs`: a heuristic answering a question the walk answers
exactly.

One `Active`, the set of rule ids the policy left on, rather than a boolean pair
per family. A pair per family is a place the next rule has to be declared twice,
once to be read and once to be counted, and the counting is what a four-clause
condition used to decide, silently, for the whole pass.

One writer into the family map, and one phase reporting its own partiality.
`record_family` is the only way in, so no producer inserts over a family another
one is still merging into. And a phase that stops at the deadline returns that
fact: reading the clock afterwards cannot tell a pass that stopped from a pass
that merely finished late, and calling a complete report partial drops the
score's authoritative flag for nothing.

`the_pass_holds_its_own_size_bound` keeps every file of the module under the 1000
lines `oversized_unit` reports, tests included. That is why the suppression
rules, the benchmark and the two largest test modules have files of their own:
the pass has to pass its own rule.

