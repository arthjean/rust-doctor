---
name: rule-admit
description: Add one retained Clippy lint or native detector to rust-doctor's catalog, end to end, with its fixtures, evidence record, corpus entry and every hard-coded counter kept in sync. Use when the user asks to admit a rule, add a lint to the catalog, ship a rule that triage retained, or says "admit clippy::x". Handles one rule per run. Not for deciding which lints to consider, which is rule-candidate.
---

# Admit one rule

Admission is one rule at a time. The catalog is small enough that every counter
around it is written out by hand, so the work is less about the rule than about
leaving nothing out of sync. Everything below is enforced by a test: skipping a
step fails the suite, it does not slip through.

Read the "Admitting a rule" and "Invariants the tests enforce" sections of
`AGENTS.md` first. Verify the suite is green before starting, so that a failure
during the run belongs to the rule.

## Step 1: Record what the rule catches

Take the toolchain's own description:

```bash
clippy-driver -W help | grep <lint-name>
```

That line is the `catches` field, verbatim. It has to match, a test compares
them. A native detector has no upstream line, so write one sentence naming the
pattern, not the fix.

Pick the category and the tier, and check the pair against `TIER_WINDOWS` in
`src/policy/catalog.rs`. Write `help` as an instruction to the user: what to do
instead, in one sentence, without restating the problem.

## Step 2: Build the trigger fixture

The rule needs a place where a test sees it fire. Follow the pattern of the
rules admitted before it rather than inventing one.

For a lint whose dimension matches an existing pack, add the triggering form to
`tests/fixtures/score-credibility/packs/<pack>/src/lib.rs`, the quiet
counterpart to `src/negatives.rs` of the same pack, and the expected diagnostic
to the pack's `oracle.json` under `positive`, with `code`, `category`, `tier`,
`path`, `line` and `occurrences`. The packs are `panic`, `performance` and
`concurrency`.

Otherwise use the isolated form: a crate under
`tests/fixtures/rule-scaling-kernel/positive/<kebab-name>/` and an entry in
`tests/fixtures/rule-scaling-kernel/oracle.json` under `rules`, carrying `id`,
`category`, `help`, `clippy_default`, `message`, `positive_fixture` and its
spans.

The negative form matters as much as the positive one. A rule with no recorded
silence is a rule nobody has checked for over-reach.

## Step 3: Enter the catalog

Add the `RuleDefinition` static in `src/policy/catalog.rs`, in alphabetical
position, then add its reference to `CATALOG`. `validate_catalog` refuses an
unsorted catalog, a duplicate id, an unknown category, a producer whose prefix
disagrees with the id, and a tier outside its category window.

## Step 4: Move every hard-coded counter

These are written out by hand and each one has its own test. Search rather than
trust this list, which drifts, but expect to touch all of it. Measured on the
admission of three rules on 2026-08-06: skipping any one of them costs a full
suite run to rediscover.

Counts, which move by one:

- `src/policy/catalog.rs`: the length of `CATALOG`, the length of
  `SYNTHETIC_CATALOG`, which is one more, the `CATALOG.len()` assertion, the
  `clippy_ids.len()` assertion, and `active_rules(Producer::Clippy).count()`.
- `src/execution.rs`: the two command-length assertions, shaped `7 + 2 * N`,
  where `N` counts the active Clippy rules carried under `-W`.
- `src/policy/coverage.rs`: `DECIDED_FLOOR`, since the rule leaves the queue.
- `tests/score_credibility_packs.rs`: the catalogued Clippy count.
- `tests/score_credibility_kernel.rs` and `tests/configuration_kernel.rs`: the
  published `policy.rules` length, plus the index of the last rule.
- `tests/persistent_configuration_product_proof.rs`: the `RULES` array length
  and the published length.
- `README.md`: the rule count line and the native detector table.

Lists, which gain the identifier in alphabetical position:

- `src/policy/catalog.rs`: `CATALOG`, `SYNTHETIC_CATALOG`, and the expected
  argument list of the synthetic proof, which omits the categories it turns off.
- `tests/persistent_configuration_product_proof.rs`: `RULES`.
- `tests/policy_gate_product_proof.rs`: the `--rule <id>=off` list that disables
  the whole Clippy producer, which only prunes the scan when it names every one.
- `tests/fixtures/rule-scaling-kernel/oracle.json`:
  `compatibility.policy_clippy_pruning.inactive_rules`.
- `tests/fixtures/local-cli-experience/audit-core-v2.json`: `rule_tiers`.

Frozen bytes and hashes, which are regenerated from the observed run:

- `tests/fixtures/rule-scaling-kernel/v7-full-report.json` and
  `v7-baseline-report.json`: one rule object inside `policy.rules`, shaped
  `{id, category, level, source}`. The v7 archive proves no historical field
  disappeared or changed type, and an entry added to an array is neither.
- `tests/fixtures/rule-scaling-kernel/oracle.json`:
  `compatibility.git_change_scope_output_hashes` and
  `persistent_configuration_output_hashes`. Both hash a whole report, which
  carries the policy, so both move with every widening. The failing test prints
  the observed evaluation; take the hashes from it.
- `src/audit.rs`: the dimension the new rule saturates in
  `the_catalog_drives_the_score_out_of_its_top_label`, which builds one
  diagnostic per catalogued rule.

```bash
rg -n "; 4[0-9]\]|len\(\), 4[0-9]|2 \* 3[0-9]|rules\[4[0-9]\]" src tests
```

## Step 5: Publish the rule

Add the entry to the `catalog` array of `tests/corpus.json`, with `id`,
`default_level` and `tier`. The published catalog has to match the shipped
policy, a test compares them field by field.

Then the precision. If a corpus clone cache is available, replay the measurement
and update `precision` and `gate` from the artifacts:

```bash
RUST_DOCTOR_CORPUS_DIR=<clone cache outside this repository> \
RUST_DOCTOR_CORPUS_ARTIFACTS=<scratch outside this repository> \
cargo test --test corpus_precision
```

If the corpus never triggers the rule, it ships `unobserved` and its name joins
`ADMISSION_DEBT` in `tests/corpus_precision.rs`, whose length moves with it.
That list is one-way by design: state in the report why the rule enters it, and
never drop another name to make room.

## Step 6: Record the evidence

Add the entry to `tests/rule_evidence.json`, sorted by id, with `catches` from
step 1 and exactly one pointer: `oracle` for a path under `tests/fixtures/`, or
`test` as `<file>::<function>` for a rule proven by a named test.

## Step 7: Verify

```bash
cargo test --test rule_admission
cargo test --lib policy::coverage
cargo test
cargo clippy --all-targets --no-deps -- -D warnings
```

Run the first two while iterating, the last two before calling it done.

## Step 8: Report

State the rule, its category, tier and help, where its trigger is recorded,
whether the corpus measured it or it entered the admission debt, every counter
moved, and the commands that ran with their result. Use a Conventional Commit
with a scope, for instance `feat(policy): admit clippy::<name>`.

## Examples

### Example 1: a performance lint the corpus measures

User says: "admit clippy::large_stack_arrays"

1. `catches` from the toolchain, category `performance`, tier `P3`.
2. Triggering form and quiet counterpart in the `performance` pack, expected
   diagnostic in its oracle.
3. Static and `CATALOG` entry, counters moved from 40 to 41 and 33 to 34.
4. Corpus replayed, precision published.
5. Evidence entry pointing at the performance pack oracle.

### Example 2: a rule the corpus never triggers

The same run, except step 5 ends with the rule `unobserved`, its name added to
`ADMISSION_DEBT` and the array length raised, with the reason stated in the
report.

## Common issues

### The published catalog no longer matches the shipped policy

Cause: `src/policy/catalog.rs` changed and `tests/corpus.json` did not.
Solution: mirror the entry in the `catalog` array, keeping the same order.

### The `catches` field does not match the toolchain

Cause: it was paraphrased instead of copied.
Solution: copy the `-W help` line verbatim. When the upstream wording is poor,
`help` is where the useful sentence goes.

### The command-length assertions fail

Cause: a Clippy rule was added but `5 + 2 * N` still counts the old total.
Solution: raise `N` in both assertions of `src/execution.rs`. Only rules whose
producer is Clippy and whose default level is active are carried under `-W`.

### A test that runs cargo fails with "extern location does not exist"

Cause: the test does not set its own `CARGO_TARGET_DIR`, so Cargo's artifact GC
deleted rlibs a running test binary still references.
Solution: give the new test its own scratch target directory, as its neighbours
do.
