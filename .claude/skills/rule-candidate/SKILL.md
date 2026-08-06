---
name: rule-candidate
description: Triage a batch of Clippy lints from rust-doctor's computed candidate queue into rejections or retained rules. Use when the user asks to triage candidates, grow the catalog, decide which lints to add, work through the candidate queue, or says "next batch of lints". Produces motivated rejections in src/policy/rejected.json and a shortlist for rule-admit. Not for adding a rule to the catalog, which is rule-admit.
---

# Triage a batch of candidate lints

The upstream side of this catalog is finite: `clippy-driver -W help` enumerates
every lint of the toolchain. Triage walks that list in batches and turns each
lint into one of three states. Nothing here edits the catalog; retaining a lint
only hands it to `rule-admit`.

Read the "Admitting a rule" and "The candidate queue" sections of `AGENTS.md`
before starting.

## Step 1: Read the queue

```bash
cargo test --lib policy::coverage -- --nocapture
```

The run prints `universe N, decided N, queue N` and then the queue itself, one
lint per line as `level`, `id`, `groups`. Take the batch off the top unless the
user named a theme. The head is the warned lints, and that order is deliberate:
they already reach the user's report uncatalogued, with no category, no tier and
no help, and they cost the score its authoritative flag.

Default batch size is 20. Announce the batch before working it.

## Step 2: Decide each lint

Fetch what the lint actually does. The toolchain carries the one-line
description, and the rationale lives in the lint declaration itself, never in
memory. The published `lints.json` no longer resolves, so read the source: pull
the Clippy tree once per session into a scratch directory and extract every
`declare_clippy_lint!` doc comment, which gives "What it does", "Why is this
bad?" and "Known problems" for all lints at once.

```bash
curl -sSL -o clippy.tar.gz \
  https://github.com/rust-lang/rust-clippy/archive/refs/heads/master.tar.gz
tar xzf clippy.tar.gz --strip-components=1 --wildcards "*/clippy_lints/src/*"
```

That tree is `master`, not the pinned toolchain. The wording of a stable lint
rarely moves, but check the toolchain's own one-line description when the two
seem to disagree.

Three outcomes, and only two of them are edits.

**Reject** when the lint should never ship, for a reason that holds whatever
workspace is scanned. Append to `src/policy/rejected.json`, sorted by id, with a
closed class and one written sentence ending in a period:

- `deny-by-default`: the toolchain denies it, so a scan cannot carry it. Valid
  only when the toolchain confirms it, and a test enforces that.
- `covered`: an admitted rule already reports the same defect.
- `style-only`: a matter of taste, not a defect the score should move for.
- `out-of-scope`: outside what a workspace scan claims to inspect.
- `noisy`: measured noise leaves it unusable at any default level. Needs a
  measurement, not an impression.

**Retain** when the lint names a real defect a user should fix. Record the
proposed category and tier, and check the pair against `TIER_WINDOWS` in
`src/policy/catalog.rs`: security is `P0` to `P1`, correctness and dependencies
`P1` to `P2`, reliability and performance `P2` to `P3`, maintainability `P3`. A
pair outside its window means the category or the tier is wrong, not that the
window should grow.

**Leave untriaged** when the answer needs evidence this batch cannot produce.
That is not an edit: the lint stays in the queue. Say so rather than guess.

Keep the bar high. A lint that fires on correct code more often than on defects
costs more than it returns, and the user's report is the budget being spent.

## Step 3: Raise the floor and verify

Set `DECIDED_FLOOR` in `src/policy/coverage.rs` to the new decided count, then:

```bash
cargo test --lib policy::coverage
cargo clippy --all-targets --no-deps -- -D warnings
```

## Step 4: Report

State the batch size, the count per outcome, and the new coverage line. List the
retained lints with their proposed category and tier, in the order they should
be admitted. End by naming `rule-admit` as the next step, one rule at a time.

## Examples

### Example 1: routine batch

User says: "triage the next batch of candidates"

1. Run the coverage test, read `universe 815, decided 35, queue 715`.
2. Take the 20 warned lints at the head.
3. Reject 14 with a class and a reason, retain 3, leave 3 untriaged.
4. Raise `DECIDED_FLOOR` from 35 to 49.
5. Report the three retained lints with their category and tier.

### Example 2: themed batch

User says: "look at the async lints we could add"

1. Run the coverage test.
2. Filter the queue on the `clippy::suspicious` and `clippy::correctness` groups
   and on async-related names instead of taking the head.
3. Same three outcomes, same verification.

## Common issues

### The queue is empty or absurdly small

Cause: the `-W help` table changed shape and the parser silently matched
nothing.
Solution: `the_toolchain_publishes_a_finite_lint_universe_with_its_groups` fails
first in that case. Fix the parser in `src/policy/coverage.rs`, never the floor.

### A rejection fails the deny-by-default test

Cause: the lint was classified `deny-by-default` while the toolchain warns or
allows it, or the reverse after a toolchain upgrade.
Solution: reclassify with the class that actually applies. A lint that stopped
being denied upstream is a candidate again.

### Two rules would report the same defect

Cause: the lint overlaps an admitted rule.
Solution: reject as `covered` and name the admitted rule in the reason. Two
diagnostics on one site read as two defects to the user.
