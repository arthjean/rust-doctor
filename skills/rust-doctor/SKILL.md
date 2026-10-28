---
name: rust-doctor
description: Use when Rust code just changed and must not regress, when the user asks to scan, audit or health-check a Cargo workspace, says "rust-doctor", or wants one of its findings explained, tuned or switched off.
---

# rust-doctor

`rust-doctor` scans a Cargo workspace with 142 curated rules and scores it out of
100. It runs locally, opens no network connection and uploads nothing; Cargo
downloads missing dependencies as `cargo build` does, and
`CARGO_NET_OFFLINE=true` forbids it. The scan runs `cargo clippy` inside the
workspace, which executes its build scripts and procedural macros, so scan
trusted local paths only.

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

Under the ceiling the score is a density, not a tally. The model is `core-v5`,
published as `audit.score.model`, and each dimension scores
`round(100 * exp(-D / lambda))`. D counts one distinct site per diagnostic, an
error weighing two, a warning one and an info nothing, each site weighed by the
tier of its rule, P3 one and every tier above doubling the one below, and each
rule discounted by the false-positive rate the pinned corpus measured for it,
over the size of what was scanned: Clippy, the source detectors and the
structural pass divide by the workspace's production kilolines, floored at
two, so the same finding costs a small crate more than a large one, while the
manifest and repository rules divide by nothing, since a workspace has one
`Cargo.toml` whatever its size, except that what a member's own manifest
holds is averaged over the members. Lambda is the density at which a dimension
falls to 37, tightest on security and most forgiving on reliability. The five
dimensions combine at weights 4 for security, 3 for reliability and 2 each for
maintainability, performance and dependencies, and the result lands in one of
three bands: 75 and above reads
`Great`, 50 to 74 `Needs work`, below 50 `Critical`. A clone family is one site
whatever its `related` array names, and findings outside production code stay
visible and cost nothing.

The report names the shortlist itself. `audit.score.projected_rule_ids` is the
three rules worth repairing first, in the order to repair them: each is the
rule that gives the most points back on top of the ones before it, through the
ceilings, discounted by its measured rate, so the rule holding a ceiling comes
first. `audit.score.projected_after_top_three` is the score they are worth.
`audit.score.withheld_rule_ids` is what the corpus found wrong more often than
right. Take that ranking as given rather than rebuilding one from the
categories.

## After changing Rust code

```bash
rust-doctor . --json --scope lines 2>/dev/null
```

Lines scope reports only the findings on the lines your change touched, and
baseline scope (`--scope baseline`) only what the change introduced, at the
cost of a second compilation of the base. Neither needs `--base`, since the
base is the first of `origin/HEAD`, `origin/main`, `origin/master`, `main` and
`master` that resolves, or `HEAD` when that branch is checked out, and
`scope.base_ref` names it. Repair every finding either returns before
committing, since each one is yours. Add `--include-untracked` to judge a file
you created and have not added yet; without it the terminal says how many
were left out. Add `--staged` to judge what `git commit` would record rather
than the working tree: it compares the index with `HEAD`.

## Auditing a workspace

```bash
rust-doctor . --json 2>/dev/null
```

Record `audit.score.value` as the baseline, then work the shortlist:

1. Read each diagnostic in `diagnostics`. It carries its `code`, `message`,
   `help`, `path`, `span` and `occurrences`, and, when Clippy wrote one, a
   `suggestion` with the `replacement` for the span and its `applicability`;
   only `machine-applicable` is safe to paste unread. Applying it is yours to
   do: the tool never edits a workspace it scans, so there is no fix
   subcommand and none is coming. It writes only build artifacts under Cargo's
   target directory, as `cargo clippy` itself does, including the
   `rust-doctor/baseline` and `rust-doctor/staged` directories a baseline or
   staged run keeps its dependencies in. `policy.rules` carries
   the `tier` and `category` of every rule the scan ran. A diagnostic at
   severity `info` is shown and costs nothing, such as a print in a binary
   target.
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

Every rule publishes, in `rules list --json` and in the handoff prompt, a
`not_a_defect_when` check: the situation in which a finding of it is not a
defect. Apply it to each site before editing; a site it describes is left
alone, and a rule that keeps hitting it is tuned below rather than fixed.

`rust-doctor rules list --json` prints every catalogued rule with its category,
producer, default level, tier and help, which is what a diagnostic's `code`
resolves against. `rust-doctor rules explain <id> . --json` adds the rule's
corpus noise and the level this workspace's `rust-doctor.toml` sets, with its
source and every `[[overrides]]` table touching it; `rust-doctor why
<path:line> --report report.json --json` names each finding at that line, the
directives near it and the one to write, without scanning again. Explain the
rule from its help before offering to switch it off: most findings people
dislike are real.

Prefer fixing the code, then the narrowest control:

- One native site: `// rust-doctor: allow(<rule-id>) -- <reason>` on the line
  above, or `# rust-doctor: allow(<rule-id>) -- <reason>` above a key in
  `Cargo.toml` or `.cargo/config.toml`. Without a reason it suppresses nothing.
  A Clippy lint takes `#[expect(<lint>, reason = "...")]` instead, and every
  directive is listed in `suppressions[]` with its status.
- One path: `[[overrides]]` in `rust-doctor.toml`, with `paths` globs and
  `rules` or `categories` levels. `[ignore] paths` drops a path from the report
  and the score, counted in `scan.ignored`; Clippy still compiles it.
- Files `.gitattributes` marks `linguist-generated` or `linguist-vendored`, and
  files opening on a generator header, are left out, counted in
  `scan.excluded_generated`.
- One run: `--rule <id>=off` or `--category <name>=<level>`, which win over the
  file. `#[allow]` attributes are themselves catalogued findings when they
  carry no reason.

## Commands

| Command | Purpose |
| --- | --- |
| `rust-doctor . --json` | Full structured report, the shape every step above reads |
| `rust-doctor . --json --scope lines` | Only the lines the branch touched |
| `rust-doctor . --json --scope contextual` | Those lines, plus the findings a rule reports above them in a function they changed, such as a lock held across an `.await` you added; for reviewing a change, not for the rescan loop |
| `rust-doctor . --json --scope baseline` | Only what the branch introduced |
| `rust-doctor . --json --scope files --base <REF>` | Only the files touched since a ref you name |
| `rust-doctor . --json --staged --scope lines` | Only what the next commit records |
| `rust-doctor . --json --package <NAME>` | One workspace member, scored alone; every scan publishes `audit.packages` |
| `rust-doctor rules list --json` | The catalog the binary shipped with |
| `rust-doctor rules explain todo . --json` | One rule, by id or unambiguous suffix, and how this workspace sets it |
| `rust-doctor why src/lib.rs:12 --json` | Why a line is reported, scanning only its member; `--report <FILE>` reads a saved report instead |
| `rust-doctor . --rule <id>=off` | Run without one rule |
| `rust-doctor . --category <name>=error` | Raise or lower a whole category |
| `rust-doctor . --blocking <none\|error\|warning>` | The level that makes the run exit non-zero |
| `rust-doctor . --json --advisory-db <PATH>` | Judge `Cargo.lock` against a RustSec database you name instead of the one `cargo audit` left |
| `rust-doctor . --json --max-duration 300` | Stop after 300 seconds, killing every process the scan started |
| `rust-doctor . --json --timings` | Publish how long each phase took, in milliseconds, under `timings` |
| `rust-doctor hook install git` | A pre-commit hook running `--staged --scope lines --blocking warning` |
| `rust-doctor hook install claude` | A Claude Code `Stop` hook that rescans each turn (`--shared` for the committed settings, `--no-clippy` for one that compiles nothing) |
| `rust-doctor hook install cursor` | A Cursor `stop` hook that asks for a follow-up on findings |
| `rust-doctor skill install --agent all --update` | Refresh this skill for Claude Code, Codex and Cursor after an upgrade |
| `rust-doctor setup --dry-run` | Plan the skill, the git hook and the end-of-turn hooks for the agents detected on `PATH` or in the workspace; without `--dry-run` it writes them, and never adds CI |
| `rust-doctor ci install --comment` | A GitHub Actions workflow judging each pull request in baseline scope, plus its sticky comment |
| `rust-doctor report markdown report.json --limit 20` | A saved `--json` report as Markdown, without scanning |

Every installer prints what it writes, takes `--dry-run`, and never writes
through a symlink. The skill, the git hook and the CI workflow never write over
a file rust-doctor did not create; the agent-hook installers merge their one
entry into an existing settings file and keep every other key. `hook install --update` rewrites
a hook rust-doctor wrote that an older release left behind. The pre-commit hook
refuses a commit that adds a warning, and lets one through, saying so, when
`rust-doctor` is not on `PATH` or the scan itself fails (exit 2), and git's
no-verify option skips it. The
end-of-turn hook runs `rust-doctor hook run claude`, which rescans with
`--scope lines --base HEAD --include-untracked --blocking warning`, exits 2
with up to ten findings when the turn introduced one, and never blocks when the
scan itself fails. That rescan runs `cargo clippy`, so the build scripts and
procedural macros of the tree the agent wrote execute at the end of every turn
with the user's authority, and the installer says so. `--no-clippy`, on
`hook install claude|cursor` or `hook run`, turns every Clippy rule off
whatever `rust-doctor.toml` says: that hook compiles nothing. The Cursor hook stays silent on a turn whose `status` is not
`completed` or whose `loop_count` is already 1.

| Exit status | Meaning |
| --- | --- |
| `0` | the scan completed and the gate passed |
| `1` | the gate failed, or the scan is incomplete |
| `2` | the scan failed, ran past --max-duration, or the arguments are invalid |
| `3-125` | --warning-exit-code: the gate passed with a finding --blocking warning would block |
| `130` | a SIGINT, SIGTERM or SIGHUP stopped the scan |

The precedence is 2, 1, the warning code, then 0.
A run cut by `--max-duration` exits 2 with `deadline-exceeded` in `errors` and
in `audit.score.reasons`, and keeps the findings collected before the limit. An interrupted run (Ctrl-C,
SIGTERM or SIGHUP) exits 130 with `interrupted` in `errors` and a null
`audit.score`, leaving no process behind.
When the score is partial, `audit.score.reasons` says why. Without a local
RustSec database the two advisory rules publish `not_evaluated` and the score
stays authoritative; the scan never downloads one.

Use `--json` for anything you parse. `--verbose` is for a human reading a
terminal, and a run with neither flag on a terminal opens an interactive report
that an agent cannot drive, unless one of the variables of
`NON_INTERACTIVE_MARKERS` is set to a non-empty value: the agent markers
`CLAUDECODE`, `CLAUDE_CODE`, `CODEX_SANDBOX`, `CODEX_CI`, `CURSOR_AGENT`,
`OPENCODE`, `GOOSE_TERMINAL`, `AMP_THREAD_ID`, `CLINE_ACTIVE`,
`AUGMENT_AGENT`, `TRAE_AI_SHELL_ID`, `AGENT_SESSION_ID` and
`AGENT_THREAD_ID`, the git hook markers `GIT_DIR` and `GIT_INDEX_FILE`, and
the CI markers `GITHUB_ACTIONS`, `GITLAB_CI`, `BUILDKITE`, `CIRCLECI`,
`TF_BUILD`, `JENKINS_URL`, `TEAMCITY_VERSION`, `CODEBUILD_BUILD_ID`,
`BITBUCKET_BUILD_NUMBER`, `TRAVIS` and `DRONE`. `CI` keeps it closed too,
unless it is `false`, `0` or empty. Any of them, or `--yes`, also stops the
score's count-up and the rewritten progress line, so a transcript holds the
final output only. Pass `--yes` to be sure. If the binary is not on `PATH`, prefix with
`npx rust-doctor@latest`.
