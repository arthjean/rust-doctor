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
`git ls-files`). `validate_catalog` refuses any other prefix, and a pass that
fails degrades to a complete report carrying a `ReportError` at its stage with
the authoritative flag dropped.

**One home per fact.** This file is the map, not the record. Every rule below is
one to three lines and names the document that holds the reasoning behind it.
Read that document before editing the subsystem it covers, and write new
reasoning there rather than here. `docs/doc-budgets.json` caps this file and
each linked document; `bash scripts/verify-doc-budgets.sh` checks them and CI
replays it.

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
| Documentation budgets | `bash scripts/verify-doc-budgets.sh` |

Use `bun` under `npm/rust-doctor/`, never `npm` or `pnpm`. Run the lint and
test commands before calling a change complete: `.github/workflows/ci.yml`
replays them on every push and pull request, so a change that skips them fails
in the open instead of locally.

## Running the tool on this repository

The CLI opens the interactive report instead of the linear one when stdin and
stdout are both terminals. Nothing is asked before the scan: a run that names no
scope scans the whole workspace. Pass `--yes` for any scripted or agent-driven
run:

```bash
cargo run --release -- . --yes --verbose
```

## The subsystems

Each row is a module and the document holding what it is, the rules that hold it
together, and what each of those rules replaced. Read the document before
editing the module.

| Module | Home |
|---|---|
| `src/policy/`: the level algebra, the catalog, the noise rate, the candidate queue | [docs/subsystems/policy.md](docs/subsystems/policy.md) |
| `src/audit/`: the score, the category tallies, the core-v3 density penalty | [docs/subsystems/audit.md](docs/subsystems/audit.md) |
| `src/report/`, `src/render/`, `src/tui/`, `src/presentation/code_frame.rs` | [docs/subsystems/reporting.md](docs/subsystems/reporting.md) |
| `src/source_kernel/`, `src/source_text.rs`: the one workspace walk | [docs/subsystems/source-kernel.md](docs/subsystems/source-kernel.md) |
| `src/execution/`: the five producers, the toolchain probe, the Clippy stream | [docs/subsystems/execution.md](docs/subsystems/execution.md) |
| `src/git.rs`, `src/git_scope.rs`, `src/internal_error.rs` | [docs/subsystems/git-and-scope.md](docs/subsystems/git-and-scope.md) |
| `src/structure/`: the four detector families and their deadline | [docs/subsystems/structure.md](docs/subsystems/structure.md) |
| `src/delta/`: what a `--scope baseline` run calls new | [docs/subsystems/delta.md](docs/subsystems/delta.md) |

Every module holds its files under the 1000 lines `oversized_unit` reports, and
every `impl` block under the 500 it reports at, tests included: the code that
raises the rule has to pass it. Fourteen `the_X_holds_the_size_bound` tests are
what enforce that, one per module, each naming its own, and two of them cover
`src/cargo_health.rs` and `src/handoff.rs`, which have no section of their own.

## The catalog, the corpus and the report

- The 62 rules are declared once, in `src/policy/catalog.rs`, and published by
  `rust-doctor rules list --json`. Editing the catalog means regenerating
  `tests/corpus.json` with it and syncing the README's one rule count.
  [docs/catalog-and-corpus.md](docs/catalog-and-corpus.md)
- A new rule needs a trigger record in `tests/rule_evidence.json` before it
  ships, and a corpus measurement when the corpus can produce one. Only the
  first is unconditional. `.claude/skills/rule-candidate`, `rule-admit` and
  `corpus-adjudicate` carry the procedures.
  [docs/catalog-and-corpus.md](docs/catalog-and-corpus.md)
- `tests/corpus.json` is the measured record and the calibration of the score.
  It replays offline from a clone cache outside this repository, and
  `adjudication.position_proof` anchors every published site to a run that
  located it. [docs/catalog-and-corpus.md](docs/catalog-and-corpus.md)
- Any change to the report shape bumps `SCHEMA_VERSION` in `src/report.rs`,
  currently 16, and the frozen v7 archive keeps projecting.
  [docs/subsystems/reporting.md](docs/subsystems/reporting.md)

## Testing

- Production code carries no `unwrap`, `expect`, `panic!` or `dbg!`: use `?`,
  `ok_or(...)?`, `unwrap_or` or `match`. `tests/score_credibility_packs.rs`
  scans this repository and fails on any hit. [docs/testing.md](docs/testing.md)
- Every test that runs `cargo` or the built binary sets `CARGO_TARGET_DIR` to
  its own scratch directory through `support::scan_target(workspace)`, and no
  test touches the network. [docs/testing.md](docs/testing.md)
- Start every integration test crate with
  `#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]`, since
  `clippy.toml`'s `allow-*-in-tests` keys do not reach them (rust-clippy#13981).
  [docs/testing.md](docs/testing.md)
- The self-scan is a gate, never a frozen defect: evidence that a rule fires
  belongs on a fixture. [docs/testing.md](docs/testing.md)

## Delivery

- Dependencies are pinned exactly (`= 1.8.5`, not `^1.8`) and `Cargo.lock` is
  committed: `missing_lockfile` requires it for a binary crate.
- Releasing is one version bump in `Cargo.toml`, mirrored into
  `npm/rust-doctor/package.json` and `bun.lock`, then a `v<version>` tag whose
  note already exists at `.github/releases/v<version>.md`.
  [docs/publishing.md](docs/publishing.md) and `.claude/skills/release`
- Four workflows settle four different questions, and every one of them pins
  toolchain 1.97.1. [docs/ci-workflows.md](docs/ci-workflows.md)
- `skills/rust-doctor/` ships inside the crate and is checked against `--help`
  and `catalog()` by `tests/skill_contract.rs`.
  [docs/agent-skill.md](docs/agent-skill.md)

## Conventions

- English everywhere: comments, doc comments, assertion messages, CLI output,
  rule identifiers, commit messages. The only non-ASCII literals left are test
  data that deliberately exercise UTF-8 handling; leave them alone.
- Conventional Commits with a scope, lowercase summary:
  `feat(policy): grow the catalog to forty rules`. Use `!` for a breaking change
  to the report schema or the CLI surface.
- Keep the README's rule count in sync with `src/policy/catalog.rs`. The README
  carries that one number and nothing else about the catalog: the rules
  themselves are published by `rust-doctor rules list` and by
  rust-doctor.com/rules, which generates from that command.
