---
name: corpus-adjudicate
description: Adjudicate one catalogued rule on the pinned corpus, deepening its reviewed sample past the five sites admission requires. Use when the user asks to adjudicate a rule, or when another skill needs a rate precise enough to place a rule against the threshold. Handles one rule per run. Not for adding a rule to the catalog, which is rule-admit.
---

# Adjudicate one rule

Admission asks whether a rule fires on the pattern it claims. This asks the
other question, how often it is wrong on healthy code, and it is the question
the published rate answers. `adjudication.sampling` states plainly that five
sites resolve steps of twenty points and cannot place a rule against the five
percent threshold, so deepening a sample is what turns a published number into
a defensible one.

One word governs the whole run. A verdict reached without opening the code is
**hearsay**: inferred from a neighbouring site, from the rule's reputation, or
from the tool's own claim. Hearsay is not a weaker verdict, it is not a verdict,
and it never enters the measurement. Everything below exists to keep it out.

Read the "The pinned corpus" and "Admitting a rule" sections of `AGENTS.md`
first, and the `adjudication.criterion` field of `tests/corpus.json`, which is
the definition every verdict is measured against. Verify the suite is green
before starting, so a failure during the run belongs to this work.

## What the harness already enforces

Four invariants hold whatever you do. They catch a corrupted sample, never
hearsay, which reads exactly like a real verdict and is yours to keep out:

- The rate is derived, never written. `precision()` in `tests/support/corpus.rs`
  computes every published rate from `adjudication.reviewed` alone, and a test
  asserts the published field equals the recomputation.
- A reviewed site must name a finding the corpus actually produced, at its
  published `path` and `line`, and in its published `context`. Both are checked
  in `the_published_observations_reproduce_the_pinned_corpus_run`.
- `(rule, repository, path, line)` is unique. A site reviewed twice makes the
  rule incomplete and withholds its rate.
- `MINIMUM_REVIEWED_SITES` is a floor of five, not a ceiling. The only upper
  bound is `reviewed <= findings`.

## Step 1: Replay the corpus and build the population

The measurement replays from the local clone cache, never from the network.
Both paths sit outside this repository:

```bash
RUST_DOCTOR_CORPUS_DIR=<clone cache outside this repository> \
RUST_DOCTOR_CORPUS_ARTIFACTS=<scratch outside this repository> \
cargo test --test corpus_precision
```

The run writes one report per repository under `<artifacts>/reports/<name>.json`
and materializes each pinned tree under `<artifacts>/work/<name>`. Those reports
are the population: every diagnostic carrying a category is a finding of its
rule.

Collect the population of the target rule across all ten reports, keep one entry
per `(repository, path, line)`, and order it canonically by that same triple.
For a `rust_doctor::structure::*` rule, restrict the population to findings with
no `context` field, which is what production means: a family marked as test,
bench, example or build-script material is published but weighs nothing, and
measuring the rule over those families publishes the cost of an idiom rather
than the cost of the rule. A test enforces this restriction.

**Done when** the population count is stated and every entry carries a
repository, a path and a line drawn from a report, not from memory.

## Step 2: Sample by stride

From a population of `n` sites, take `k = min(target, n)` at indices
`floor(i * n / k)` for `i` in `[0, k)`. A regular stride from the first site, so
the sample follows the same distribution as the findings a user receives. This
is the method `adjudication.sampling` publishes, and the reason it is a stride
rather than a random draw is reproducibility: anyone can recompute which sites
were reviewed.

Sites already present in `adjudication.reviewed` stay. Deepening a sample means
recomputing the stride for the new `k` and adjudicating the sites it adds, not
re-judging the ones already recorded, whose verdicts remain valid observations
of the same population.

**Done when** the stride is listed, and every already-recorded site is either
inside it or named as sitting outside it. A stride that silently drops a
recorded verdict changes the published rate without saying so.

## Step 3: Adjudicate, with the code in front of you

Read each site in the materialized tree at `<artifacts>/work/<repository>`, with
the surrounding code, never the diagnostic alone. This is the single largest
factor in whether the verdict is any good: an agent with repository access
identifies false positives at roughly the rate a reviewer does, while the same
model judging a finding in isolation degrades toward guessing.

Judging a site by analogy with its neighbour in the same file is the most
comfortable form of hearsay, and the one that shows up in practice: a run that
reports thirty-five verdicts of which twelve say "same pattern as the site
above" has thirty-five entries and twenty-three verdicts. Prefer a short honest
count to a full list: state how many sites you actually opened, and emit only
those.

Apply `adjudication.criterion` verbatim. In short, a finding is a true positive
when the flagged site should actually be changed, and a false positive when the
construct is correct as written, when the surrounding code establishes its
safety, or when the context makes the flagged behavior the intended one.
Confirming the pattern is present is a different, mechanical check that proves
the span is not corrupted and says nothing about value.

Each site produces a verdict, a one-sentence justification naming what in the
code decided it, and the context read from the report rather than guessed.

**Two independent passes.** Judge the sample twice, each pass blind to the
other's verdicts. Where the two agree, the verdict stands. Where they disagree,
the site goes to Arthur: a disagreement is the signal that the site is hard, and
an agent settling its own tie removes exactly the information the second pass
was there to produce. Report the disagreement count, it is the honest measure of
how much the sample can be trusted.

Delegating each pass to a separate subagent, with no shared context and no
mention of any prior verdict, is what makes the second pass independent rather
than a rereading.

**Done when** every site in the stride carries two verdicts from sites that were
opened, the agreement count and Cohen's kappa are computed, and the disagreeing
sites are listed with both justifications. Kappa is undefined when neither pass
produced a single verdict of one kind: report that rather than a perfect
agreement figure, which in that case measures nothing.

## Step 4: Record the sites

Add each site to `adjudication.reviewed` in `tests/corpus.json`:

```json
{
  "context": "production",
  "justification": "One sentence naming what in the code decided the verdict.",
  "line": 67,
  "path": "src/buf/buf_impl.rs",
  "provenance": "agent",
  "repository": "bytes",
  "rule": "clippy::indexing_slicing",
  "verdict": "false_positive"
}
```

`provenance` is `agent` for a verdict produced under this protocol and `human`
for one a person read and judged. `unrecorded` belongs to the sites judged
before the field existed and stays theirs.

The file is canonical JSON. Write it back exactly as it is read, or the diff
fills with reformatting:

```python
json.dumps(document, indent=2, sort_keys=True, ensure_ascii=False) + "\n"
```

## Step 5: Regenerate what derives from the sites

- `precision`: for the target rule, `reviewed` is the new count,
  `false_positives` and `true_positives` the tallies, the rate is
  `false_positives * 10000 / reviewed` in integer arithmetic, and `provenance`
  the sorted distinct provenances of its sites.
- `gate`: a rule above `threshold_basis_points` joins `noisy_on_healthy_code`,
  one below leaves it. A `P0` rule with a confirmed false positive is refused
  and loses default activation; nothing else is refused.
- `adjudication.sampling`: state the new `k` for this rule and why, as the entry
  for `duplicate_function_body` already does. The published method has to
  describe what was actually done, including any site excluded for disagreement
  and the resulting count.

**Done when** `cargo test --test corpus_precision` passes without the cache,
which is the harness recomputing every rate from the sites and comparing it to
what you wrote.

## Step 6: Verify

```bash
cargo test --test corpus_precision
RUST_DOCTOR_CORPUS_DIR=<cache> RUST_DOCTOR_CORPUS_ARTIFACTS=<scratch> \
  cargo test --test corpus_precision -- the_published_observations_reproduce_the_pinned_corpus_run
cargo test
cargo clippy --all-targets --no-deps -- -D warnings
```

The second command is the one that matters and the one it is tempting to skip:
without the cache the position and context checks return silently, and a site
with a wrong line passes every other test in the suite.

## Step 7: Report

State the rule, the population size, the sample before and after, the rate
before and after, the disagreement count between the two passes, and any site
escalated to Arthur. When the rate moves materially, say which direction and
what the added sites had in common: a rate that collapses when the sample grows
means the first five sites were not representative, and that is a finding about
the method, not only about the rule.

Commit with a Conventional Commit, for instance
`test(corpus): adjudicate clippy::indexing_slicing on forty sites`.

## Common issues

### The replay says a reviewed site is absent from the corpus

Cause: the position was taken from a stale report, or the line was transcribed
by hand. Solution: recompute the population from the current artifacts. Never
adjust a line to make the check pass; the check exists because that adjustment
is exactly what invalidates a rate.

### A structural rule fails the production-context test

Cause: the sample was drawn from the whole population instead of the production
subpopulation. Solution: filter findings with no `context` field before the
stride, and recompute.

### The rate is unchanged but the gate moved

Cause: expected. The gate is recomputed from every rule at once, and a rule
leaving `noisy_on_healthy_code` changes a published list even when its own
number did not move much. Check the list against the threshold rather than
against memory.
