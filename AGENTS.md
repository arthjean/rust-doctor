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
| `release.yml` | tag `v*`, manual | The five platform binaries the launcher declares, then the six npm packages. A tag publishes; a manual run stops at `npm publish --dry-run`. crates.io stays unwired, marked `PUBLICATION HOOK` |
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

## The policy module

`src/policy.rs` is the level algebra: the two override kinds, the precedence
between a request, a configuration file and the shipped default, and the
`PolicyPlan` every producer reads. `catalog.rs` is the 62 declarations and the
lookup over them, `catalog/validate.rs` their admissibility, `catalog/tests.rs`
the tests, `noise.rs` the adjudicated rate the score ranks by, and
`coverage.rs` the candidate queue.

Four rules hold it together, and each of them replaced something that had a
cost.

Validated once, compiled from that. `PolicyInput::validate` returns a
`ValidatedPolicy`, and a plan compiles from that and from nothing else, so
compilation is infallible. Validation used to run twice over the same catalog,
once as the gate in `lib.rs` and once inside the compilation, which left a
complete failure branch in `prepare_with` that no input could ever reach.

One lookup, total. Four tables of the module are sorted by identifier and
binary-searched, and `by_id` is the one way through them. The four hand-written
copies indexed the slice they had just searched, which is five of the
`indexing_slicing` findings the tool reports on its own source, and one of them
indexed twice on the same line.

One fact, stored once. A `PlannedRule` carries the source of its level and
answers `restamped()` from it. The boolean that used to sit beside the source
was a second place to keep the same fact true.

The catalog is not copied to be tested. `synthetic_catalog()` builds the
shipped list plus one rule rather than restating it, the frozen oracle compares
against `catalog()` rather than against the private `RuleDefinition` it
projects from, and the Clippy command is asserted as a fixed head followed by
one `-W` per active rule in catalog order. The three hand-written copies these
replaced all drifted silently: a rule admitted to `CATALOG` and forgotten in
the second list left every assertion passing over a catalog that no longer
shipped, and a field added to the published shape changed
`rules list --json` without moving the record that was supposed to freeze it.

`the_policy_holds_the_size_bound_the_catalog_publishes_for` keeps every file of
the module under the 1000 lines `oversized_unit` reports, tests included. That
is why the tests and the validation have files of their own: the module that
declares the rule has to pass it.

## The agent skill

`skills/rust-doctor/` is the skill an agent loads to drive the tool: `SKILL.md`
is the three branches it serves, a regression check on the branch, a full audit,
and explaining or switching off a rule, and `references/expert-review.md` is the
review it applies to a file the catalog already flagged.

`rust-doctor skill install` writes it into a workspace, and `src/skill.rs`
embeds both documents with `include_str!`: an install reaches no network, and a
binary carries the skill of its own version rather than whatever the latest
branch holds. The refusal to overwrite is the creation of the skill directory
itself, so either the whole skill lands or nothing does and no half-installed
skill points at a reference that was never written. `src/tui/workflow.rs` makes
the same guarantee one file at a time, with `create_new`.

It lives in this repository rather than beside the agent that installs it
because a skill naming a flag is a second copy of the CLI surface, and the copy
drifts. The one it replaced documented `--plan`, `--score`, `--fix`,
`--install-deps` and `--diff <ref>`, none of which this binary has ever
accepted, so the agent died on its first command, and its references described
19 kebab-case rules and a scoring model with no tier ceiling in it.
`tests/skill_contract.rs` checks every long flag the skill documents against
`--help`, every rule id it names against `catalog()`, and the rule count it
states against the same list, so the drift is a red test rather than a support
thread.

## Publishing

`npx rust-doctor@latest` resolves the unscoped `rust-doctor` wrapper, which
carries the five `@rustdoctor/<platform>` binaries as exact-version optional
dependencies. Releasing is one version bump in `Cargo.toml`, mirrored into
`npm/rust-doctor/package.json` and `bun.lock`, then a `v<version>` tag:
`release.yml` refuses a tag that disagrees with the manifest.

Two scars from the retired account constrain the names. `@rust-doctor/linux-x64`
still resolves, owned by `npm-support`, so that scope cannot be reclaimed and
the binaries live under `@rustdoctor/*`. Versions 0.1.1 through 0.2.0 of
`rust-doctor` were unpublished and are burned for good, since npm never lets a
`name@version` be reused: the line restarts at 0.3.0 and can never go back.
That is also why the publish job skips what the registry already serves instead
of failing, so a re-run after a partial failure finishes the release rather than
consuming a version.

The job stores no secret. Each of the six packages names this repository's
`release.yml` as its trusted publisher on npmjs.com, and the registry
authenticates the run by its OIDC identity, which is why the job upgrades npm
past 11.5.1 and asks for `id-token: write`. A new package has to be published
once by token before it can be given a trusted publisher, since the setting
lives in a package's settings: 0.3.0 went out that way, and the token was
revoked afterwards. Every package is published with `--provenance`, which ties
a tarball to the commit and the workflow that built it.

## Running the tool on this repository

The CLI opens the interactive report instead of the linear one when stdin and
stdout are both terminals. Nothing is asked before the scan: a run that names no
scope scans the whole workspace. Pass `--yes` for any scripted or agent-driven
run:

```bash
cargo run --release -- . --yes --verbose
```

## The linear report

`src/render.rs` is the report every non-interactive run prints: the error type,
the terminal options, the three entry points, the eight section renderers and
the one styled-line primitive they all write through. `src/render/score_header.rs`
is the score block, and each of the two carries its tests in a file of its own.

Three rules hold it together, and each of them replaced something that had a
cost.

One width, guaranteed rather than branched on. `MIN_WIDTH` is at least
`score_block::MIN_BLOCK_COLUMNS`, asserted at compile time, and every entry
point normalizes to it, so the score block always fits. That deleted a whole
dimension the module used to pay for and never reach: an optional constructor,
a `drawn` flag threaded back through two modules, and a second single-line
renderer whose bar truncated where the shared `bar_fill` rounds up. The two
questions the geometry answers are now separate: `right_column_width` is total,
`bar_width` is the one that says whether the block fits, and only the
interactive report, which really does draw at forty columns, reads the second.

One frame builder. A frame is the four rows composed under one `Palette`, and
the palette is the only thing separating the counting frames from the scrolling
ones and from the frame the block freezes on. Three builders used to carry the
same loop, two of them identical but for a bolted-in `index == 1`, and the
frozen frame was computed once in each of the two paths that show it.

One row, built from its pieces. A `Row` carries the segments it paints
differently, so the score line is never formatted and split back apart on
`/ 100` to find its denominator, nor the branding on `" ("` to find its URL. A
cut now lands inside one piece and leaves the pieces before it whole, which is
what the comment on that code already claimed and what the string round-trip
could not do.

`the_report_holds_the_size_bound_it_reports_for` keeps every file of the module
under the 1000 lines `oversized_unit` reports, tests included. That is why the
two test modules have files of their own: the report has to pass the rule it
prints.

## The interactive report

`src/tui/` is a transposition of React Doctor's Ink application, screen for
screen: the landing score block and its action menu, the split review with the
rule list on the left and the detail on the right, the agent handoff, and the
two GitHub Actions screens. `model.rs` carries the geometry of
`resolve-report-layout.ts` unchanged, so the same terminal size picks the same
split, stacked or compact arrangement the reference picks. `screens/menu.rs` is
the shape the four menu screens share, drawing and reading keys once each;
`screens/viewer.rs` is the split review; `screens.rs` keeps the score block.
`text.rs` composes styled spans and measures them in display columns,
`canvas.rs` owns the terminal, and `tui.rs` is the state machine and the frame
loop.

Two things the two reports share rather than each transposing. `score_block` is
the score block's model: the faces, the label a score carries, how a value
fills the bar, how much room the block needs and the cadence of its count-up.
`terminal_text` is the sanitizer and the ruler. Both are public on the library
for the binary to reach, and both are public for that reason alone. A second
copy of either is what let the two reports disagree on the bar rounding, on the
guard column, and on which escape sequences a diagnostic could smuggle into a
frame.

Three structural rules hold it together. Each screen carries its own cursor
inside the `View` enum, never a flat set of cursors beside a view tag: a cursor
that outlives the list it indexes is how a menu ends up pointing past its own
actions. Every menu answers to one `screens::input`, so no screen carries its
own hard-coded upper bound. And `canvas.rs` truncates every frame to the
terminal minus its last row and last column, because a row that wraps moves the
cursor away from where the next rewind expects it and corrupts every frame
after; screens still size themselves, but none of them can break the loop.

Three things it deliberately does not do. It asks nothing before the scan, the
way React Doctor's TUI path hard-codes `skipPrompts: true` in
`resolve-tui-scan-scope.ts`: the reader answers questions after seeing findings,
not before. It never takes the alternate screen:
frames are rewritten in place the way Ink does, so the last one survives in the
scrollback. And it never renders on a run that asked for `--json`, `--yes` or
`--verbose`, or on a scan that failed, because those readers are pipes, CI,
agents, and someone who needs an error message the report has no room for. Every
existing test asserts against the linear renderer, which is unchanged.

The only file the tool writes into a scanned workspace is
`.github/workflows/rust-doctor.yml`, only from the CI menu entry, and never over
an existing file (`src/tui/workflow.rs`): the refusal is the creation itself,
`create_new`, rather than a check followed by a write. That workflow installs
the published
launcher, `npm install -g rust-doctor@<version>`, pinned to the version of the
binary that wrote it: the pin comes from `CARGO_PKG_VERSION` rather than a
string in the template, so a release cannot forget to move it, and a generated
gate keeps scanning with the rule set its author saw.

## The code frame

`src/presentation/code_frame.rs` is the only place the tool reads a scanned
file's contents back out to a terminal, so it is where the trust boundary is
enforced one file at a time: the path is decoded and canonicalized, the handle
is opened and then revalidated against the live path by device and inode, the
bytes are refused if they carry a NUL or do not decode, and every escape
sequence is neutralized before a byte reaches a frame. Both reports call
`code_frame`, and `src/presentation/code_frame/tests.rs` carries the tests.

Three rules hold it together, and each of them replaced something that had a
cost.

One reader, walking lines. `read_window` reads line by line to the window it
needs and decodes only the lines it keeps. The byte prefix it replaced capped
the read at eight kilobytes and then asked whether the reported line was in it,
so every finding past roughly the first two hundred lines of a file printed
`Code frame unavailable`, which is most of a file `oversized_unit` reports on at
a thousand. It also cut a character in half whenever a multi-byte one straddled
the cap, and a file that valid reported `InvalidUtf8` for every frame it had,
including the ones on its first line.

Every bound is a budget on work, never on reach. `SCAN_MAX_BYTES` bounds what
one frame may scan, `LINE_MAX_BYTES` what one line may contribute, and
`FRAME_MAX_COLUMNS` what one line may render, and none of the three decides
which line is reachable. That is the distinction the byte prefix collapsed:
a window disguised as a budget answers `Unavailable` for a line it simply never
looked at.

One gutter, published by the frame. `CodeFrame::gutter_width` is the width both
reports lay their rows out from. The linear report used to hard-code four
columns while the interactive one computed its own, so a frame reaching line ten
thousand slid the source row one column right and left the caret row where it
was, in one report and not the other. Four columns are now its floor, not its
ceiling.

`the_frame_holds_the_size_bound_the_report_reports_for` keeps both files of the
module under the 1000 lines `oversized_unit` reports, tests included, for the
reason the rest of the crate holds it: the code that renders the finding has to
pass the rule that raised it.

## The source kernel

`src/source_kernel.rs` is the types and the detector registry. `walk.rs` is the
workspace walk that fills them, `aliases.rs` the per-unit import map,
`detectors.rs` the two native detectors and `references.rs` the crate names the
dependency rules judge a manifest against. `src/source_text.rs` sits outside the
module on purpose: span arithmetic and the two syntax-text helpers are read by
`repo_hygiene` and `cargo_health`, which parse no Rust at all and have no
business depending on the walk for a span type.

Four rules hold it together, and each of them replaced something that had a
cost.

One walk, one enumeration. `enumerate` loads and parses each reachable file
once, and every source-reading producer works off that single `Enumeration`.
That is why `references::collect` and `references::mentioned` take the
enumeration rather than a path: nothing here reads the disk twice.

One answer to unanimity. A file several targets reach publishes only what all of
them agree on: its package, its target, its non-production context, the crate
aliases in scope, the identity a candidate two units emit is merged under. Six
places used to answer that question with six different pieces of code.
`unanimous` is the one answer, and disagreement abstains rather than
arbitrating, because a finding naming the wrong package is worse than a finding
naming none.

One typed outcome per load. `Loaded` says whether a file was skipped or whether
the global byte budget is gone, since only the second ends the walk. That
difference used to be recovered by searching the rendered error message for the
substring `total-bytes`, so renaming a display string moved the control flow.
`Limit` now names each bound once, and the budget charges what left the disk
rather than what survived decoding: a file that is not UTF-8 was still read, and
a walk that did not charge it could read the per-file limit repeatedly with the
total never moving.

One test-code policy, named. `SourceUnit::is_test_code` asks Cargo's target kind
first, which covers `tests/`, `benches/` and `examples/` wherever a manifest
configures them to sit, and falls back to the path convention only for a module
file no target names on its own. A detector that stays quiet in test code calls
`in_test_code`; `disabled_tls` does, because verification is routinely disabled
against a self-signed test server, and `dynamic_shell` deliberately does not,
because an interpolated shell command is a finding wherever it runs. The reason
is written above each rather than left to be inferred from an asymmetry.

`the_kernel_holds_the_size_bound_it_enumerates_for` keeps every file of the
module, and `source_text.rs` with it, under the 1000 lines `oversized_unit`
reports, tests included. That is why the walk and the tests have files of their
own: the kernel that feeds the rule has to pass it.

## Running the producers

`src/execution.rs` is the orchestration and nothing else: what a run holds
constant, the toolchain it resolves, the order the five producers run in, and
the shape of what came back. `execution/clippy.rs` is the pass that compiles,
`execution/messages.rs` the `--message-format=json` stream it answers on,
`execution/baseline.rs` the dual run a `--scope baseline` comparison needs, and
each carries its tests in a file of its own.

Four rules hold it together, and each of them replaced something that had a
cost.

One list of the producers that degrade rather than abort.
`ExecutionResult::producer_errors` is that list, and `is_complete` and
`report::errors` both read it. It used to be written twice, as a four-clause
conjunction in `execution.rs` and four `if let` blocks with the stage spelled
again in `report.rs`, and `cargo_health` was in the second list only: a
workspace whose `.cargo/config.toml` could not be read published a
`dependencies` error under `"status": "complete"`, with the score still calling
itself authoritative.
`every_producer_error_drops_the_authoritative_flag_at_its_own_stage` asserts the
four together, because the defect was never the missing clause.

The result is assembled from what came back, never asked back for what it was
given. `ExecutionContext::run` owns the metadata to the end, so the three
`Option` dances that no input could make `None`, and the workspace-root clone
that only existed to survive an early move, are gone with them. The same run
carries `ExecutionContext` rather than seven positional parameters, four of
which were identical on both sides of a baseline comparison.

Every bound is a budget on work, never a filter on meaning. `LINE_MAX_BYTES`
bounds what one line of Cargo's stream may contribute, `MESSAGE_MAX_COUNT` how
many messages one scan may keep, `RUSTUP_OUTPUT_LIMIT` what a toolchain probe
may print, and none of them decides which diagnostic the report may publish: an
oversized line is counted malformed and the stream keeps going, a flood stops
with an error at the `parsing` stage. The reader had no bound at all in a crate
whose git layer documents that every way out of it is bounded, and the scanned
workspace's procedural macros are what decide how many diagnostics Cargo emits.
`src/bounded_read.rs` is the one primitive both layers read streams through.

Three versions or none. `Toolchain` is the three the report attributes its
findings to, held by one `Option`: `resolve_toolchain` returns all three or
fails before any producer starts, so a report naming a cargo and no rustc was a
shape the type allowed and no code path could produce.

`the_execution_holds_the_size_bound_it_scans_for` keeps every file of the module
under the 1000 lines `oversized_unit` reports, tests included. The module was
one file of 977 lines with no such test, the only one of the crate both near the
bound and unguarded, and it carried the whole Clippy story that `mod clippy`
was supposed to name.

## Git and the scan scope

`src/internal_error.rs` is the one error shape every stage reports through. It
sits on its own rather than inside `execution`, which made `scan_target`,
`configuration`, `repo_hygiene` and `baseline` import back the module that
imports them, four cycles a reader had to hold for nothing.

`src/git.rs` is the bounded process layer every producer that shells out to git
runs through, and `src/git_scope.rs` is one of its three callers: the scope a
scan runs under, whole workspace, changed files, or baseline comparison. The
other two are `baseline.rs` and `repo_hygiene.rs`, which is why the layer lives
beside them rather than inside the scope that used to own it: reading
`git_scope::run_git` to list a tree said scope where none was involved.

Four rules hold the two together, and each of them replaced something that had
a cost.

A call names the stage every one of its outcomes is reported at. `GitCall`
carries `stage`, and the four exits build their `InternalError` from it: git
could not start, stdout overflowed, stderr overflowed, the call failed. Two of
those are not named by the caller, and they used to be hard-coded to `scope`
whoever ran them, so a baseline snapshot whose git flooded stderr published
`stage: "scope"` in the JSON, at a bound `baseline.rs` lists in its own oracle.
One overflow failure now covers both streams, since a caller that cannot use
the answer does not care which stream made it unusable.

Every way out of the layer is bounded. `git_command` is private and
`run_git_status` is what a call answering with an exit code uses, both streams
closed at the pipe. The escape hatch it replaced handed a `Command` out and
`repo_hygiene` called `.output()` on it, which reads both streams with no bound
at all, in the one module built to make that impossible.

Validated once, resolved from that. `ScopeRequest::validate` returns a
`ValidatedScope` holding a `BaseSelector` that passed the closed grammar, and
`resolve` reads that and nothing else. Validation used to run twice over the
same request, once as the gate in `lib.rs` and once inside the resolution,
which left a failure branch in resolution that no input could reach: the same
shape the policy module removed, in the module next to it.

One resolved shape, one constructor per case. `ResolvedScope` is the three
cases and `ScopeReport` the accessors over it; a second enum used to mirror it
variant for variant, so a fourth scope mode was an edit in five places.
`ScopeReport::files_scope` is the only way a file scope is built: it sorts,
deduplicates and bounds, and `includes` binary-searches that order. Production
and the tests used to establish that order separately, so a third construction
site would have broken the search in silence.

`the_git_layer_holds_the_size_bound_it_scans_for` and
`the_scope_holds_the_size_bound_it_reports_for` keep every file of both modules
under the 1000 lines `oversized_unit` reports, tests included. That is why each
carries its tests in a file of its own.

## The structural pass

`src/structure.rs` is the pass and nothing else: the family map, the deadline,
the identity of a finding. The four detector families sit beside it,
`suppression.rs` (3 rules), `duplication.rs` with `normalize.rs` (2),
`hotspots.rs` (2) and `manifest.rs` (2), and each publishes a `RULES` table.
`rules()` is their union, and
`the_pass_produces_every_catalogued_structural_rule` compares it against the
catalog, so a rule cannot be published by `rules list` and left out of the scan.

Three rules hold it together, and each of them replaced something that had a
cost.

One traversal per unit. `Inventory::of` walks the tree once and collects the five
node kinds the families read between them. Four walks used to spend the wall
clock four times over the same nodes, which is what forced the substring
pre-filter in `manifest.rs`: a heuristic answering a question the walk answers
exactly.

One `Active`, the set of rule ids the policy left on, rather than a boolean pair
per family. A pair per family is a place the next rule has to be declared twice,
once to be read and once to be counted, and the counting is what a four-clause
condition used to decide, silently, for the whole pass.

One writer into the family map, and one phase reporting its own partiality.
`record_family` is the only way in, so no producer inserts over a family another
one is still merging into. And a phase that stops at the deadline returns that
fact: reading the clock afterwards cannot tell a pass that stopped from a pass
that merely finished late, and calling a complete report partial drops the
score's authoritative flag for nothing.

`the_pass_holds_its_own_size_bound` keeps every file of the module under the 1000
lines `oversized_unit` reports, tests included. That is why the suppression
rules, the benchmark and the two largest test modules have files of their own:
the pass has to pass its own rule.

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

## Conventions

- English everywhere: comments, doc comments, assertion messages, CLI output,
  rule identifiers, commit messages. The only non-ASCII literals left are test
  data that deliberately exercise UTF-8 handling; leave them alone.
- Conventional Commits with a scope, lowercase summary:
  `feat(policy): grow the catalog to forty rules`. Use `!` for a breaking change
  to the report schema or the CLI surface.
- Keep the README's rule count in sync with `src/policy/catalog.rs`, which is
  the single list every producer's rules are declared in. The README carries
  that one number and nothing else about the catalog: the rules themselves are
  published by `rust-doctor rules list` and by rust-doctor.com/rules, which
  generates from that command.
