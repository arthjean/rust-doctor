# Testing

## Invariants the tests enforce

- **The crate is not a hotspot of its own scan**
  (`no_unit_of_this_crate_s_own_source_is_a_hotspot`). A structural pass over
  this repository names no `oversized_unit` and no `complex_function` anywhere
  under `src/`. It replaced a test that asserted the opposite, that
  `src/report.rs` was oversized, which froze the crate's largest self-violation
  in place: repairing the file failed the suite. Evidence that a rule fires
  belongs on a fixture, and `tests/rule_evidence.json` names the tests that
  carry it; what belongs in a self-scan is the gate. The fourteen
  `the_X_holds_the_size_bound` tests stay beside it, because each fails on its
  own module and says which one. Four of them are new: the report, the
  dependency pack, the handoff and the interactive report all carried files over
  the bound with no test naming them, and `src/cargo_health.rs` and
  `src/handoff.rs` were over it only because their tests were still inline.
- **The documents state what the binary computes**
  (`tests/docs_contract.rs`). The catalog size, the schema version, the bounds
  and the counts are recomputed rather than reread.
  [doc-gates.md](doc-gates.md)
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
  in `src/policy/noise.rs` mirrors the measured rates of `tests/corpus.json`,
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
  `SCHEMA_VERSION` in `src/report.rs`, and the frozen v7 archive keeps
  projecting: `project_current_wire_to_v7` in `tests/support/mod.rs` strips the
  members added since, which is what proves no historical field ever disappeared
  or changed type. The version itself is stated once, in
  [subsystems/reporting.md](subsystems/reporting.md).
- **Dependencies are pinned exactly** (`= 1.8.5`, not `^1.8`) in `Cargo.toml`,
  and `Cargo.lock` is committed. The `missing_lockfile` detector requires it for
  a binary crate.
- **Structural rules default to warning, never error.** The
  `rust_doctor::structure::*` rules live in `src/structure/`, run on the same
  file set the source kernel enumerates, and report a clone family as one
  diagnostic whose `related` array names every member beyond the first. A
  structural pass failure degrades to a complete non-structural report with a
  `ReportError` at stage `structure`. The pass stops at a wall-clock budget of 10
  seconds and says so; `RUST_DOCTOR_STRUCTURE_TIME_BUDGET_SECS` overrides it,
  which is how the corpus harness makes an observation independent of machine
  load, and why the published structural measurement was taken at 600 seconds.
- **A structural family is matched on its content, never on its message.** The
  identity of the family is `blake3(domain, rule, normalized key)`, it carries no
  span, no path and no count, and `delta.rs` matches a structural diagnostic on
  it through `structural_identity` rather than on the message and source excerpt
  every other diagnostic is matched on. That is not an optimization: every
  structural message states a number the next edit moves, so a message-keyed
  match reports a finding older than the branch as introduced by it, under the
  `--scope baseline` that `dogfood.yml` runs on every pull request.
  `a_structural_finding_survives_the_count_its_message_states` is the test, and
  the near-duplicate key is the smallest digest of the family rather than the
  list of all of them for the same reason.

## Working in `tests/`

- Start every integration test crate with
  `#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]`. The
  `allow-*-in-tests` keys in `clippy.toml` do not cover integration test crates
  (rust-clippy#13981).
- Every test that runs `cargo` or the built binary must set `CARGO_TARGET_DIR`
  to its own scratch directory. Without it, Cargo's artifact GC deletes rlibs
  the running test binaries still reference and `cargo test` fails
  nondeterministically with "extern location does not exist". The second failure
  mode is quieter and was live in four places: a fixture already compiled under
  an inherited target directory replays with no warning at all, so an oracle
  reads as a scan that found nothing rather than as a cache hit.
  `support::scan_target(workspace)` is the shared answer, keyed on the scanned
  path so two fixtures never share a cache and one workspace scanned twice
  always does. `execution::execute_into` is the same seam inside the crate, for
  the unit tests that scan in process.
- Unit tests that need a scratch directory call
  `test_scratch::scratch(area, name)`. The file is reached from both crate roots
  with `#[path]` rather than copied into each, because it had been written six
  times and the copies had already drifted on whether a failed `create_dir_all`
  refuses.
- Shared helpers live in `tests/support/` and are pulled in with `mod support;`.
  Fixtures live under `tests/fixtures/<domain>/`, where frozen JSON oracles are
  compared field by field.
- No test touches the network.

