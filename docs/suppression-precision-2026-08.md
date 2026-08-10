# Suppression, dependency and hygiene precision on the pinned corpus, August 2026

Durable record for US-017 and US-018 of
`tasks/prd-suppression-dependency-hygiene.md`. Eleven rules took the catalog
from 51 to 62 across four families: suppression audit, dependency truth,
release profile hardening and repository hygiene. This file publishes what each
of them costs on healthy public Rust, including the nine that cost nothing
because they never fired.

Every number here is published in `tests/corpus.json` and re-derivable from it.
The measurement replays the ten pinned repositories from a local clone cache,
never from the network:

```bash
RUST_DOCTOR_CORPUS_DIR=<clone cache outside this repository> \
RUST_DOCTOR_CORPUS_ARTIFACTS=<scratch outside this repository> \
cargo test --test corpus_precision
```

Toolchain: cargo 1.97.1, rustc 1.97.1, clippy 0.1.97. The structural time budget
is set to 600 seconds by the harness, so a large repository is analyzed whole
rather than cut at the interactive 10-second budget.

## The sample, and what it can resolve

The adjudication criterion and the sampling rule are published verbatim in
`tests/corpus.json`. A finding is a true positive when the flagged site should
actually be changed at the pinned revision under the published scope of the
rule; it is a false positive when the construct is correct as written or when
the context makes the flagged behavior the intended one. The sample is a
deterministic stride over the population ordered by (repository, path, line),
`k = min(5, n)` sites, and a structural rule draws from its production-context
subpopulation because a family marked test, bench, example or build script does
not weigh on the score.

Two consequences worth stating before the table. A five-site sample resolves
steps of 20 points, so it separates a rule with no observed false positive from
one that has some and cannot place a rule precisely against the 5 percent
publication threshold. A three-site sample resolves steps of 33 points, which is
weaker still: `unchecked_release_overflow` is measured on its whole population
of three, so the rate is exact for the corpus and says very little about the
next repository.

## Measured on healthy code (2026-08-10)

| Rule | Tier | Findings | Production | Reviewed | FP | Rate | 95% Wilson CI |
|---|---|---|---|---|---|---|---|
| `structure::crate_level_allow` | P2 | 33 | 10 | 5 | 5 | **10000 bp** | 5655 to 10000 bp |
| `cargo::unchecked_release_overflow` | P3 | 3 | 3 | 3 | 1 | **3333 bp** | 614 to 7924 bp |
| `structure::stacked_allow_attribute` | P3 | 1 | 0 | 0 | - | withheld | - |
| `cargo::permissive_lint_table` | P2 | 0 | 0 | 0 | - | unobserved | - |
| `cargo::unused_dependency` | P2 | 0 | 0 | 0 | - | unobserved | - |
| `cargo::test_only_dependency` | P2 | 0 | 0 | 0 | - | unobserved | - |
| `cargo::release_debug_symbols` | P3 | 0 | 0 | 0 | - | unobserved | - |
| `cargo::permissive_rustflags` | P2 | 0 | 0 | 0 | - | unobserved | - |
| `repo::tracked_secret_file` | P1 | 0 | 0 | 0 | - | unobserved | - |
| `repo::hardcoded_credential` | P1 | 0 | 0 | 0 | - | unobserved | - |
| `repo::unignored_build_output` | P3 | 0 | 0 | 0 | - | unobserved | - |

Both measured rules are named in `noisy_on_healthy_code` with their rates
published, and both keep their default activation: the gate refuses only a
zero-tolerance `P0` carrying a false positive, and neither rule is `P0`. Nine
rules read `unproven` and one reads `incomplete`, which is the honest shape of
a corpus that does not commit the defects they aim at.

## `crate_level_allow`: 5 of 5, and the reason is structural

The PRD's first HIGH assumption was that the suppression family would adjudicate
below 20 percent false positives on healthy public Rust, reasoning that a
crate-level `allow` is a declaration about a file rather than a judgment about a
call site. **The assumption is refuted, at the ceiling.** Every one of the five
sampled production sites is a false positive, and the sample is representative
rather than unlucky: the ten production sites split into three recurring shapes
and all three are correct as written.

1. **The toolchain-compatibility pair.** Three of the five sampled sites are
   literally the same line, `#![allow(unknown_lints, mismatched_lifetime_syntaxes)]`,
   at the root of `anyhow`, `serde_json` and `thiserror`'s proc-macro crate. It
   silences a rustc lint that does not exist on the older compilers those crates
   support, and `unknown_lints` is what keeps those compilers from rejecting the
   name. There is no item to scope it to: the subject is the toolchain.
2. **The documented pedantic policy.** `serde_json`'s two crate-level blocks
   carry their reason inline, per group: "integer and float ser/de requires
   these sorts of casts", "correctly used", "things are often more readable this
   way". The intent exists, in a comment, in the one place the detector does not
   read. Scoping the cast exemptions to items would repeat them across the whole
   serializer.
3. **The mixed-context family.** The `anyhow` site is a file-wide allow of one
   pedantic formatting lint in `build.rs`, grouped with `tests/test_ffi.rs`
   because the two files carry byte-identical attributes. Neither file ships,
   but `unanimous_context` in `src/structure.rs` marks a family only when every
   member shares one context, so this one is unmarked and weighs on the score.
   The gap was found and recorded during the previous epic
   (`docs/structural-precision-2026-08.md`, last section) and is still open; the
   site is counted as a false positive, so the published rate is conservative
   with respect to it.

This puts `crate_level_allow` where the five other attribute-reading and
manifest-reading structural rules already sit, between 60 and 100 percent on
healthy code, and for the same reason: healthy code documents its exemptions
out of band. The rule stays admitted at 10000 bp, per the published policy that
noise on healthy code is named rather than silently suppressed, and says nothing
about what the rule is worth on code that is not healthy. The next section is
the beginning of that second question.

`stacked_allow_attribute` produced exactly one finding on the ten repositories,
`serde_json`'s `tests/test.rs:2373`, and it sits in an integration test,
outside the production subpopulation a structural rate is drawn from. There is
nothing to adjudicate, so the rate is withheld and the rule reads `incomplete`
rather than being credited with a measured zero. It is the one `incomplete`
entry in the frozen admission debt, and the debt comment says so.

## `unchecked_release_overflow`: 1 of 3, and the false positive is instructive

Three of the ten repositories produce a binary and none of them sets
`overflow-checks = true` under `[profile.release]`.

- `fd` (true positive): the profile tunes `lto`, `strip` and `codegen-units` and
  leaves overflow checks at Cargo's default of `false`. Nothing states the
  omission as a decision, the tool spends its time in directory traversal
  rather than in integer arithmetic, and a wrap in a size or depth computation
  would be silent.
- `hexyl` (true positive): offsets, column widths and group boundaries are all
  computed from the size of the input; the profile sets only `lto` and
  `codegen-units`. A wrap corrupts the rendering rather than failing.
- `ripgrep` (false positive): the manifest itself sets
  `overflow-checks = false` in `[profile.release-lto]`, the profile its
  distributed binaries are built with. The absence under `[profile.release]` is
  a decision the project already took knowingly, for a search tool whose hot
  loops are index arithmetic. The rule restates a tradeoff its reader has
  settled, which is exactly the case its help text asks the reader to decline.

That is the shape the PRD's Risk 3 predicted: the rule reads as an opinion
where the project has already formed one. It sits at `P3` in `reliability`,
caps nothing, and names the measured cost of enabling the check so the reader
can decline it. The published 3333 bp is the rate of a three-site sample and its
confidence interval, 614 to 7924 bp, spans the threshold; it is a corpus fact,
not a forecast.

## The nine silent rules, and what their silence means

Zero findings on ten curated repositories is a measurement of the corpus, not of
the rule. Read positively, it says these ten repositories declare no permissive
`[lints]` table, carry no neutralizing rustflag in `.cargo/config.toml`, ship no
full debug symbols unstripped, track no secret-bearing file, commit no
credential-shaped literal, keep their target directory ignored, and declare no
dependency their own sources never reference. That is what a healthy Rust
repository looks like, and it is the baseline the rules were written against.

The two `security`-tier rules, `tracked_secret_file` and `hardcoded_credential`,
therefore ship with **zero adjudicated sites**, not with a measured zero. The
PRD's Month-1 target of 0 confirmed false positives over 20 adjudicated sites is
not reached, because the corpus supplied no site to adjudicate: the MEDIUM
assumption under "Assumptions" anticipated exactly this outcome and asked which
of the two it would be. It is `unproven`. Both rules sit at `P1`, so a hit caps
the security dimension at 50 and the overall score at 65 rather than collapsing
it, and the zero-tolerance refusal, which applies to `P0` only, had nothing to
refuse. Their trigger evidence is a fixture test, not a corpus site
(`tests/repo_hygiene.rs`), and `tests/rule_evidence.json` names it.

`permissive_lint_table` deserves one note of its own: it is the rule that makes
the other 61 harder to switch off, and the ten repositories give it nothing to
say because none of them silences a catalogued rule from its manifest. A corpus
of agent-authored repositories does not either, yet, which is the next
paragraph.

## The agent-authored population, unadjudicated

The eight repositories of the second population, selected by commit-trailer
evidence alone, are scanned with every Clippy rule off because their build
scripts and proc-macros are untrusted. Their per-rule counts are published in
`tests/corpus.json` and no site of theirs is adjudicated, so these are finding
counts and nothing more.

| Rule | Healthy (10 repos) | Agent (8 repos) |
|---|---|---|
| `structure::crate_level_allow` | 33 | 43 |
| `structure::stacked_allow_attribute` | 1 | 2 |
| `cargo::unchecked_release_overflow` | 3 | 8 |
| `cargo::unused_dependency` | 0 | 5 |
| `cargo::test_only_dependency` | 0 | 3 |
| the other six | 0 | 0 |

Two entries are worth naming. `unchecked_release_overflow` fires on 8 of 8
agent repositories against 3 of 10 healthy ones, which is a statement about how
often each population ships a binary with a hand-tuned profile rather than about
overflow. And `unused_dependency` plus `test_only_dependency` fire only on the
agent side, both in one repository (`artifexprocal`, 5 and 3), which is the first
mechanical evidence for the PRD's premise that a manifest and its code drift
apart faster when an agent writes both. Eight findings in one repository of eight
is far too little to publish as a rate, and it is not published as one.

## The structural density, restated

`crate_level_allow` and `stacked_allow_attribute` are structural rules, so they
enter the density comparison the previous epic published. The comparison moves,
and both populations move the same way:

| Population | Structural findings | Rust lines | Density per kloc |
|---|---|---|---|
| Healthy (10 repos) | 399 to **433** | 122,044 | 3.269 to **3.547** |
| Agent-generated (8 repos) | 5,251 to **5,296** | 988,872 | 5.310 to **5.355** |

The ratio moves from **1.624 to 1.509**: agent-written Rust still measures about
half again as many structural findings per thousand lines as healthy Rust, so the
second assumption of `tasks/prd-structural-slop-detection.md` still holds and
`tests/corpus.json` still publishes `refutes_density_assumption: false`. The
ratio fell because the two new rules add proportionally more to the smaller
healthy population (34 findings on 399) than to the agent one (45 on 5,251):
crate-level suppression is an idiom of mature libraries supporting old
compilers, not of agent output. The final table of
`docs/structural-precision-2026-08.md` carries a dated pointer to this
restatement.

## What this measurement did not do

No report field changed, so `SCHEMA_VERSION` stays at 13 rather than being
bumped: the eleven rules reach `--json` through the existing `Diagnostic` shape,
and the only new value anywhere is the `repo` error stage, a string in a field
that already existed. A consumer reading schema 13 keeps parsing.

No rule was withdrawn, and no rate removed a rule: that is the published policy,
not an omission. No agent-population site was adjudicated, for any rule, new or
old. Recall is not measured anywhere: the corpus says how much noise a rule
produces on healthy code, never how much of the defect it catches, which needs
adversarial fixtures and is a separate question. And the nine unobserved rules
stay unobserved until a corpus that commits their defects exists; a fixture
proves a rule fires, which is a different and weaker claim, published in
`tests/rule_evidence.json`.
