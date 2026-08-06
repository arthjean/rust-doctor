# AGENTS.md

rust-doctor is one Rust crate (edition 2024, rustc 1.95 or later): a local-first
CLI that inspects a trusted Cargo workspace with curated Clippy lints and native
detectors, then scores it out of 100. `src/lib.rs` exposes
`inspect(InspectRequest) -> Report`, `src/main.rs` is the CLI on top of it, and
`npm/rust-doctor/` is a Node launcher for the released binary.

## Trust boundary

Inspecting a workspace runs `cargo clippy` inside it, and Cargo executes that
workspace's `build.rs` files and procedural macros. Inspect trusted local paths
only. Never scan a path taken from an issue, a bug report, or any source outside
this repository. The native detectors in `src/source_kernel/` are exempt: they
parse source text and build nothing.

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

Use `bun` under `npm/rust-doctor/`, never `npm` or `pnpm`. There is no CI: the
lint and test commands above are the only gate, so run them before calling a
change complete.

## Running the tool on this repository

The CLI opens a scope menu when stdin and stdout are both terminals. Pass
`--yes` for any scripted or agent-driven run:

```bash
cargo run --release -- . --yes --verbose
```

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
- **The JSON report is versioned.** Any change to the report shape bumps
  `SCHEMA_VERSION` in `src/report.rs`.
- **Dependencies are pinned exactly** (`= 1.8.5`, not `^1.8`) in `Cargo.toml`,
  and `Cargo.lock` is committed. The `missing_lockfile` detector requires it for
  a binary crate.

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

Two skills carry the procedure: `.claude/skills/rule-candidate` triages a batch
off the queue into rejections and a shortlist, and `.claude/skills/rule-admit`
takes one retained rule through fixtures, catalog, counters, corpus and evidence
record. Growing the catalog goes through them, so the steps stay the same from
one batch to the next.

## Conventions

- English everywhere: comments, doc comments, assertion messages, CLI output,
  rule identifiers, commit messages. The only non-ASCII literals left are test
  data that deliberately exercise UTF-8 handling; leave them alone.
- Conventional Commits with a scope, lowercase summary:
  `feat(policy): grow the catalog to forty rules`. Use `!` for a breaking change
  to the report schema or the CLI surface.
- Keep the README's rule count and native detector table in sync with
  `src/policy/catalog.rs` and `src/source_kernel/detectors.rs`.
