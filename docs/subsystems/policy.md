# The policy module

`src/policy.rs` is the level algebra: the two override kinds, the precedence
between a request, a configuration file and the shipped default, and the
`PolicyPlan` every producer reads. `catalog.rs` is the 62 declarations and the
lookup over them, `catalog/validate.rs` their admissibility, `catalog/tests.rs`
the tests, `noise.rs` the adjudicated rate the score ranks by, and
`coverage.rs` the candidate queue.

Five rules hold it together, and each of them replaced something that had a
cost.

One answer to "which of my rules are on". `ActiveRules::of(plan, producer)` is
the set a producer asks for once and reads per finding, derived from the
catalog's own `producer` field so no producer keeps a second list. The
structural pass had a private version of it; `cargo_health` hoisted eleven
booleans at the top of `inspect` and negated eight of them in one conjunction,
which made that function the crate's own worst complexity hotspot at cyclomatic
32, and `inspect_release_profile` then asked the plan again for two of the
rules the conjunction had already answered for. `repo_hygiene` hoisted three
more.

Validated once, compiled from that. `PolicyInput::validate` returns a
`ValidatedPolicy`, and a plan compiles from that and from nothing else, so
compilation is infallible. Validation used to run twice over the same catalog,
once as the gate in `lib.rs` and once inside the compilation, which left a
complete failure branch in `prepare_with` that no input could ever reach.

One lookup, total. Four tables of the module are sorted by identifier and
binary-searched, and `by_id` is the one way through them. The four hand-written
copies indexed the slice they had just searched, which is five of the
`indexing_slicing` findings the tool reports on its own source, and one of them
indexed twice on the same line.

One fact, stored once. A `PlannedRule` carries the source of its level and
answers `restamped()` from it. The boolean that used to sit beside the source
was a second place to keep the same fact true.

The catalog is not copied to be tested. `synthetic_catalog()` builds the
shipped list plus one rule rather than restating it, the frozen oracle compares
against `catalog()` rather than against the private `RuleDefinition` it
projects from, and the Clippy command is asserted as a fixed head followed by
one `-W` per active rule in catalog order. The three hand-written copies these
replaced all drifted silently: a rule admitted to `CATALOG` and forgotten in
the second list left every assertion passing over a catalog that no longer
shipped, and a field added to the published shape changed
`rules list --json` without moving the record that was supposed to freeze it.

`the_policy_holds_the_size_bound_the_catalog_publishes_for` keeps every file of
the module under the 1000 lines `oversized_unit` reports, tests included. That
is why the tests and the validation have files of their own: the module that
declares the rule has to pass it.

