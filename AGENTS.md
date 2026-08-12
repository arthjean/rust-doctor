# AGENTS.md

rust-doctor is one Rust crate (edition 2024, rustc 1.95 or later): a local-first
CLI that inspects a trusted Cargo workspace with curated Clippy lints and native
detectors, then scores it out of 100. `src/lib.rs` exposes
`inspect(InspectRequest) -> Report`, `src/main.rs` is the CLI on top of it,
`src/tui/` is the interactive report it opens on a terminal, and
`npm/rust-doctor/` is a Node launcher for the released binary.

The catalog holds 62 rules across five producers, and a rule's id prefix names
its producer: `clippy::*` (37 curated lints, `Producer::Clippy`),
`rust_doctor::source::*` (2, `SourceKernel`, error stage `source`),
`rust_doctor::cargo::*` (11, `CargoHealth`, stage `dependencies`, which judges
the manifests and `.cargo/config.toml`), `rust_doctor::structure::*` (9,
`Structure`, stage `structure`) and `rust_doctor::repo::*` (3, `Repo`, stage
`repo`, the only pass that reads outside the Cargo model, enumerating through
`git ls-files`). `validate_catalog`
refuses any other prefix, and a pass that fails degrades to a complete report
carrying a `ReportError` at its stage with the authoritative flag dropped.

## Trust boundary

Inspecting a workspace runs `cargo clippy` inside it, and Cargo executes that
workspace's `build.rs` files and procedural macros. Inspect trusted local paths
only. Never scan a path taken from an issue, a bug report, or any source outside
this repository. Clippy is the only pass that compiles anything: the four
native producers parse source text, read manifests or ask git what it tracks,
and build nothing.

The tool never reaches the network, never uploads, never emits telemetry. Keep
it that way: no HTTP client, no analytics dependency, no phone-home. `--json`
reports stay workspace-relative, with no absolute path, no environment variable,
and no user data.

## Commands

| Goal | Command |
|---|---|
| Build | `cargo build --release` |
| Test | `cargo test` |
| Lint, must be clean | `cargo clippy --all-targets --no-deps -- -D warnings` |
| Node launcher tests | `cd npm/rust-doctor && bun test tests` |
| Packed launcher smoke | `cd npm/rust-doctor && bun run smoke:packed` |

Use `bun` under `npm/rust-doctor/`, never `npm` or `pnpm`. Run the lint and
test commands before calling a change complete: `.github/workflows/ci.yml`
replays them on every push and pull request, so a change that skips them fails
in the open instead of locally.

## Workflows

| Workflow | When | What it settles |
|---|---|---|
| `ci.yml` | push, pull request | Clippy clean, tests on Linux and macOS, the crate still compiles on Windows and on its declared MSRV 1.95, the Node launcher and its packed install |
| `dogfood.yml` | push, pull request | The repository scans itself with the binary built from the commit under review, in baseline scope on a pull request so only the findings the change introduces are judged |
| `release.yml` | tag `v*`, manual | The five platform binaries the launcher declares, as artifacts. It publishes nothing: both publication points are marked `PUBLICATION HOOK` and left unwired |
| `corpus.yml` | manual | Reproduces the pinned measurement of `tests/corpus.json` from a fresh clone cache, under the toolchain the artifact names |

Three deliberate gaps. There is no `cargo fmt --check` gate: the tree is not
rustfmt-clean, and reformatting it is a separate mechanical commit, not
something a CI file should decide. The Windows leg stops at `cargo check`
rather than `cargo test`: the launcher declares a `win32-x64` package, so the
build must keep working, but 14 of the 273 unit tests fail there as measured on
2026-08-12, on path separators, edition resolution and the hotspot self-scan.
That is a port to do, and a job that stays red forever teaches everyone to
ignore a red job. And `corpus.yml` is manual rather than nightly, since every
input it consumes is pinned, so a schedule would recompute the same answer at
the cost of compiling eighteen repositories.

The structural benchmark asserts a wall clock, and its bounds were measured on
a development machine. A slower machine declares itself through
`RUST_DOCTOR_BENCHMARK_ALLOWANCE`, a multiple the CI sets to 3, rather than
having the constants raised for everyone. It moves the two clocks only: the
counter assertions that prove the near-duplicate scoring stays nominated rather
than pairwise hold on any machine and are never relaxed.

The toolchain is pinned to 1.97.1 in every workflow rather than tracking
`stable`. Clippy's diagnostics are the product: 37 of the 62 catalogued rules
are Clippy lints, and `tests/corpus.json` records the exact Clippy version its
measurement was taken under.

## Running the tool on this repository

The CLI opens a scope menu when stdin and stdout are both terminals, and then
the interactive report instead of the linear one. Pass `--yes` for any scripted
or agent-driven run:

```bash
cargo run --release -- . --yes --verbose
```

## The interactive report

`src/tui/` is a transposition of React Doctor's Ink application, screen for
screen: the landing score block and its action menu, the split review with the
rule list on the left and the detail on the right, the agent handoff, and the
two GitHub Actions screens. `model.rs` carries the geometry of
`resolve-report-layout.ts` unchanged, so the same terminal size picks the same
split, stacked or compact arrangement the reference picks. `screens.rs` draws,
`text.rs` measures in display columns and sanitizes every span, and `tui.rs` is
the state machine and the frame loop.

Two things it deliberately does not do. It never takes the alternate screen:
frames are rewritten in place the way Ink does, so the last one survives in the
scrollback. And it never renders on a run that asked for `--json`, `--yes` or
`--verbose`, or on a scan that failed, because those readers are pipes, CI,
agents, and someone who needs an error message the report has no room for. Every
existing test asserts against the linear renderer, which is unchanged.

The only file the tool writes into a scanned workspace is
`.github/workflows/rust-doctor.yml`, only from the CI menu entry, and never over
an existing file (`src/tui/workflow.rs`).

## Invariants the tests enforce

- **The crate passes its own rules.** Production code carries no `unwrap`,
  `expect`, `panic!`, or `dbg!`: use `?`, `ok_or(...)?`, `unwrap_or`, or
  `match`. `tests/score_credibility_packs.rs` scans this repository with the
  concurrency pack and fails on any hit.
- **No catalogued Clippy rule is `deny` by default**
  (`no_catalogued_clippy_rule_is_denied_by_default`). A `deny` rule cannot be
  switched off: dropping its `-W` restores Clippy's refusal and turns a scan
  into a compilation failure. `clippy::async_yields_async` and
  `clippy::unused_io_amount` were rejected for that reason.
- **The published catalog matches the shipped policy**
  (`the_published_catalog_matches_the_shipped_policy`). Editing
  `src/policy/catalog.rs` means `tests/corpus.json` has to be regenerated with
  it.
- **The score ranks by the rate the corpus adjudicated**
  (`the_noise_the_score_ranks_by_matches_the_adjudicated_rate`). `CORPUS_NOISE`
  in `src/policy/catalog.rs` mirrors the measured rates of `tests/corpus.json`,
  because the report ranks what to fix first by what repairing each rule is
  expected to be worth: its cost to the score discounted by its measured noise.
  Re-adjudicating a rule means moving both. The rate ranks, it never penalizes:
  what a rule costs the score is what it reported.
- **Two populations, two rates, no verdict crossing between them**
  (`each_population_publishes_its_own_rate_from_its_own_sites`). Every reviewed
  site carries a `population`: `healthy` says what a rule costs on code nobody
  wants disturbed, `agent` what it is worth on the code this tool exists for.
  Each rate is derived from its own sites against its own observations, and a
  Clippy rule can never carry an `agent` rate, since Clippy is switched off on
  untrusted code. `CORPUS_NOISE` mirrors the healthy rates today; switching that
  reference is a product decision, not a consequence of a number.
- **The JSON report is versioned.** Any change to the report shape bumps
  `SCHEMA_VERSION` in `src/report.rs`, currently 14, and the frozen v7 archive
  keeps projecting: `project_v11_wire_to_v7` in `tests/support/mod.rs` strips
  the members added since, which is what proves no historical field ever
  disappeared or changed type.
- **Dependencies are pinned exactly** (`= 1.8.5`, not `^1.8`) in `Cargo.toml`,
  and `Cargo.lock` is committed. The `missing_lockfile` detector requires it for
  a binary crate.
- **Structural rules default to warning, never error.** The
  `rust_doctor::structure::*` rules live in `src/structure/`, run on the same
  file set the source kernel enumerates, and report a clone family as one
  diagnostic whose `related` array names every member beyond the first. Their
  fingerprint is computed from the normalized content hash, never from source
  positions, so inserting lines above a finding leaves `--scope baseline`
  unmoved. A structural pass failure degrades to a complete non-structural
  report with a `ReportError` at stage `structure`. The pass stops at a
  wall-clock budget of 10 seconds and says so;
  `RUST_DOCTOR_STRUCTURE_TIME_BUDGET_SECS` overrides it, which is how the
  corpus harness makes an observation independent of machine load, and why the
  published structural measurement was taken at 600 seconds.

## Working in `tests/`

- Start every integration test crate with
  `#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]`. The
  `allow-*-in-tests` keys in `clippy.toml` do not cover integration test crates
  (rust-clippy#13981).
- Every test that runs `cargo` or the built binary must set `CARGO_TARGET_DIR`
  to its own scratch directory. Without it, Cargo's artifact GC deletes rlibs
  the running test binaries still reference and `cargo test` fails
  nondeterministically with "extern location does not exist".
- Shared helpers live in `tests/support/` and are pulled in with `mod support;`.
  Fixtures live under `tests/fixtures/<domain>/`, where frozen JSON oracles are
  compared field by field.
- No test touches the network.

## The pinned corpus

`tests/corpus.json` pins ten public repositories by commit and records the
adjudicated precision of every rule. The measurement replays from a local clone
cache, never from the network:

```bash
RUST_DOCTOR_CORPUS_DIR=<clone cache outside this repository> \
RUST_DOCTOR_CORPUS_ARTIFACTS=<scratch outside this repository> \
cargo test --test corpus_precision
```

Both paths must sit outside this repository. The reproduction tests return
silently when the variables are unset, and
`no_corpus_repository_is_committed_in_this_repository` fails if corpus code is
ever committed here.

A new rule is admitted on measured precision, not on intuition. The gate refuses
default activation only for a zero-tolerance tier rule with a confirmed false
positive; every other rule is published with its measured noise rate.

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

The category bounds the tier through `TIER_WINDOWS` in `src/policy/catalog.rs`:
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

## Conventions

- English everywhere: comments, doc comments, assertion messages, CLI output,
  rule identifiers, commit messages. The only non-ASCII literals left are test
  data that deliberately exercise UTF-8 handling; leave them alone.
- Conventional Commits with a scope, lowercase summary:
  `feat(policy): grow the catalog to forty rules`. Use `!` for a breaking change
  to the report schema or the CLI surface.
- Keep the README's rule count and native detector table in sync with
  `src/policy/catalog.rs`, which is the single list every producer's rules are
  declared in.
