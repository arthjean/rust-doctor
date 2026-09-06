# The catalog and the corpus

What the tool publishes as its rule list, how that list is measured, and how
it grows.

## The published catalog

`rust-doctor rules list --json` prints the 62 catalogued rules, each with its
category, producer, default level, tier and help. It reads no filesystem: the
catalog is what the binary was compiled with. `rust_doctor::catalog()` is the
same projection for library callers, and `CatalogEntry` is the only public
shape of a rule, `RuleDefinition` staying crate-private.

It exists so that whatever publishes the rule list reads it from the tool.
`rust-doctor-web` generates `public/catalog/rules.json` from this command and
refuses a category, producer, level or tier it does not render, so a catalog
that grows a concept fails the website build instead of rendering a blank.
`rules_list_publishes_the_shipped_catalog` in `tests/policy_cli.rs` compares
the command against `catalog()` rather than against a frozen count, which is
what keeps the two true as the catalog grows.

## The pinned corpus

`tests/corpus.json` pins ten public repositories by commit and records the
adjudicated precision of every rule. The measurement replays from a local clone
cache, never from the network:

```bash
RUST_DOCTOR_CORPUS_DIR=<clone cache outside this repository> \
RUST_DOCTOR_CORPUS_ARTIFACTS=<scratch outside this repository> \
cargo test --test corpus_precision
```

Both paths must sit outside this repository. The reproduction tests print why
they are skipping when neither variable is set, and
`no_corpus_repository_is_committed_in_this_repository` fails if corpus code is
ever committed here.

Every site the record publishes is anchored to a run that located it.
`adjudication.position_proof` is a blake3 digest over the identity of every
reviewed site and every adjudicated pair, with the toolchain and the date of the
run that confirmed them, and `tests/corpus_position.rs` recomputes it from the
record on every `cargo test`, offline. So a site typed in by hand, at a line no
scan ever reported, fails the suite naming the count of sites it hashed and
saying a reproduction is required. Only the gated run rewrites the digest, and
only once it has located every published site of both populations against a live
scan: it writes `position-proof.json` beside the artifacts, which is what a pull
request copies into the record. Pairs are anchored as well as reviewed sites,
since a pair whose two passes disagreed carries no reviewed site at all and the
escalation queue would otherwise be the one part of the record nothing has ever
located.

A run that names neither directory prints why it is skipping the reproduction
instead of passing in silence, and a run that names one of the two fails: half a
configuration is a reproduction that was attempted and cannot be trusted, not a
machine without a clone cache.

A new rule is admitted on measured precision, not on intuition. The gate refuses
default activation only for a zero-tolerance tier rule with a confirmed false
positive; every other rule is published with its measured noise rate.

The record is also the calibration of the score, twice over. Each of the
eighteen observations carries the production line count the scan measured and,
per dimension, the density that count was the denominator of, so the published
score recomputes from the record alone; and the rate it adjudicated per rule is
what the score discounts that rule's sites by, so the record is a term of the
model and not only a check on it. The file records the λ table beside the
toolchain version for the same reason both are pinned: the toolchain decides
which diagnostics exist and λ decides what they cost, and a λ moved without a
new measurement republishes every recorded score under a model that never
produced it. `src/audit/tests/lambda_freeze.rs` is what refuses that, naming the
dimension whose λ moved. λ_reliability and λ_maintainability are calibrated on
the healthy population alone, since the agent population is scanned with every
Clippy rule off and those two dimensions are where the Clippy rules concentrate.

`score_distribution` is what the scale proves about itself: each population
publishes its own minimum, maximum, spread, median and band counts, and the
block publishes the distance between the two medians. That distance is measured
against a known handicap: the agent population is scanned with every Clippy
rule off, and the structural rules it does trip are discounted by rates the
healthy population measured, so the record publishes a floor on the separation
a full scan would show rather than the separation itself. Rates per population
wait for an agent adjudication of the Clippy rules, which is what is missing,
not a formula. `MINIMUM_SPREAD` in
`tests/support/corpus.rs` is the floor the model has to clear over the two
populations together, and a measurement under it fails
`the_corpus_score_distribution_is_published_with_its_spread` naming the spread
it measured. The same test refuses two populations that land in one band
together and a healthy median that fails to sit above the agent one, and asks
nothing of one population alone: ten healthy crates all reading `Great` is the
calibration succeeding. core-v2 published one population and a boolean saying
it had collapsed into a single band, which measured nothing beyond the
collapse.

The record is also published. `rust-doctor-web` generates
`public/catalog/corpus.json` from this file and renders rust-doctor.com/corpus
off it, the way it generates the rule list from `rules list --json`, so editing
`tests/corpus.json` means regenerating that snapshot with it: its `corpus:check`
compares the two byte for byte and fails the website build rather than serving a
rate no run produced. The page publishes the rates and the method; the sites
they were computed from stay here.

## Admitting a rule

Two records admit a rule, and they answer different questions.

`tests/corpus.json` answers how often the rule is wrong on healthy public code.
Its `gate` publishes the verdict: `noisy_on_healthy_code` for a rule measured
above the 5 % threshold, `unproven` for a rule the corpus never triggered.
Neither list reduces the admitted set, and that is deliberate: a rule the corpus
never triggered is a rule the ten pinned repositories never gave the chance to
fire, not a rule that does not work.

`tests/rule_evidence.json` answers the other question, whether the rule fires at
all on the pattern it claims. Every catalogued rule carries a `catches` line and
points at one place where a test has seen it trigger: a frozen oracle that names
it in an observed position, or a named test that scans a fixture and asserts the
finding. For a Clippy rule, `catches` is the description the toolchain
publishes, compared verbatim, so a lint whose meaning shifts upstream stops
matching its own contract. `tests/rule_admission.rs` refuses a catalog and an
index that disagree in either direction, and refuses a pointer that no longer
resolves.

The category bounds the tier through `TIER_WINDOWS` in
`src/policy/catalog/validate.rs`:
security is `P0` to `P1`, correctness and dependencies `P1` to `P2`, reliability
and performance `P2` to `P3`, maintainability `P3`. `validate_catalog` refuses
anything outside its window, so widening one is a deliberate edit rather than a
drift that shows up forty rules later.

So a new rule needs a trigger record before it ships, and a corpus measurement
when the corpus can produce one. Only the first is unconditional.

## The candidate queue

`clippy-driver -W help` enumerates every lint the toolchain can emit, so the
upstream side of the catalog is finite and countable. `src/policy/coverage.rs`
partitions it three ways: the rules the catalog admits, the lints
`src/policy/rejected.json` turns down with a closed class and a written reason,
and the remainder, which is the candidate queue.

```bash
cargo test --lib policy::coverage -- --nocapture
```

The run prints `universe N, decided N, queue N` and then the queue itself,
warned lints first. Those already reach the report without being catalogued:
`report::diagnostics` only drops a diagnostic whose rule is catalogued and
inactive, so an uncatalogued warning arrives with no category, no tier and no
help, and costs the score its authoritative flag. Growing the catalog means
draining that head, not inventing rules.

Turning a lint down means adding it to `rejected.json`; leaving it untriaged
means doing nothing. `DECIDED_FLOOR` in `coverage.rs` records how many lints of
the universe have been decided either way, and every triage batch raises it.

Three skills carry the procedure: `.claude/skills/rule-candidate` triages a batch
off the queue into rejections and a shortlist, `.claude/skills/rule-admit` takes
one retained rule through fixtures, catalog, counters, corpus and evidence
record, and `.claude/skills/corpus-adjudicate` deepens the adjudicated sample of
one rule past the five sites admission requires, which is the only way a rate
becomes precise enough to place a rule against the 5 % threshold. Growing the
catalog goes through the first two, trusting what it publishes goes through the
third, so the steps stay the same from one batch to the next.

