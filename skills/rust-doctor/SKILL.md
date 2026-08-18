---
name: rust-doctor
description: Use when Rust code just changed and must not regress, when the user asks to scan, audit or health-check a Cargo workspace, says "rust-doctor", or wants one of its findings explained, tuned or switched off.
---

# rust-doctor

`rust-doctor` scans a Cargo workspace with 62 curated rules and scores it out of
100. It runs locally, reaches no network and uploads nothing. The scan runs
`cargo clippy` inside the workspace, which executes its build scripts and
procedural macros, so scan trusted local paths only.

## The ceiling decides what to fix first

Every rule carries a tier, and the worst tier present caps the score however
clean the rest is:

| Worst tier present | Its dimension caps at | The whole score caps at |
| --- | --- | --- |
| P0 | 20 | 40 |
| P1 | 50 | 65 |
| P2 | 75 | uncapped |
| P3 | uncapped | uncapped |

One P0 finding makes a hundred P3 repairs worth nothing. Read `audit.score.worst_tier`
and `audit.score.applied_ceiling` first, and repair what sets the ceiling.

Under the ceiling a rule costs points by severity times an occurrence step (one
site, then two to five, then six to twenty, then more), summed over five
weighted dimensions: security counts double, reliability one and a half, and
maintainability, performance and dependencies once each. Findings in test code
stay visible and cost nothing.

The report names the shortlist itself. `audit.score.projected_rule_ids` is the
three rules worth repairing first, each discounted by the false-positive rate
the pinned corpus measured for it, and `audit.score.projected_after_top_three`
is the score they are worth. `audit.score.withheld_rule_ids` is what was too
noisy to rank. Take that ranking as given rather than rebuilding one from the
categories.

## After changing Rust code

```bash
rust-doctor . --json --scope baseline --base main 2>/dev/null
```

Baseline scope reports only what the change introduced. Repair every finding it
returns before committing, since each one is yours.

## Auditing a workspace

```bash
rust-doctor . --json 2>/dev/null
```

Record `audit.score.value` as the baseline, then work the shortlist:

1. Read each diagnostic in `diagnostics`. It carries its `code`, `message`,
   `help`, `path`, `span` and `occurrences`, and `policy.rules` carries the
   `tier` and `category` of every rule the scan ran.
2. Open the flagged file and read around the span. Report no finding you have
   not read the source of.
3. Read [references/expert-review.md](references/expert-review.md) and apply it
   to the file you just opened. A flagged line usually sits next to unflagged
   problems the catalog has no rule for, and finding those is the reason an
   agent runs this rather than reading the score alone.
4. Fix the root cause rather than the flagged symptom, and show the before and
   after:

```
#### rust_doctor::source::disabled_tls_verification (security, P0)
src/client.rs:42, inside `fn connect()`, public API
- before: `.danger_accept_invalid_certs(true)`
- after:  `.danger_accept_invalid_certs(cfg!(test))`
- why:    every caller of this client accepted forged certificates in production.
```

5. Rescan. The pass is done when the new value reaches
   `projected_after_top_three`, or when you name which projected rule fell short
   and why. A value that did not move means the ceiling did not move: check
   `worst_tier` again.

Close on the score delta, the dimensions that moved, the fixes applied with
their `file:line`, what expert review found beyond the catalog, and what you
left.

## Explaining or tuning a rule

`rust-doctor rules list --json` prints every catalogued rule with its category,
producer, default level, tier and help, which is what a diagnostic's `code`
resolves against. Explain the rule from its help before offering to switch it
off: most findings people dislike are real.

There is no suppression comment. A rule is turned off for one run with
`--rule <id>=off` or `--category <name>=<level>`, and durably in
`rust-doctor.toml` at the workspace root. Prefer the narrowest control, and
prefer fixing the code: `#[allow]` attributes are themselves catalogued findings
when they carry no reason.

## Commands

| Command | Purpose |
| --- | --- |
| `rust-doctor . --json` | Full structured report, the shape every step above reads |
| `rust-doctor . --json --scope baseline --base main` | Only what the branch introduced |
| `rust-doctor . --json --scope files --base main` | Only the files the branch touched |
| `rust-doctor rules list --json` | The catalog the binary shipped with |
| `rust-doctor . --rule <id>=off` | Run without one rule |
| `rust-doctor . --category <name>=error` | Raise or lower a whole category |
| `rust-doctor . --blocking <none\|error\|warning>` | The level that makes the run exit non-zero |

Use `--json` for anything you parse. `--verbose` is for a human reading a
terminal, and a run with neither flag on a terminal opens an interactive report
that an agent cannot drive. If the binary is not on `PATH`, prefix with
`npx rust-doctor@latest`.
