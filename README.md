# rust-doctor

<p align="center">
  <a href="https://crates.io/crates/rust-doctor"><img alt="Crates.io" src="https://img.shields.io/crates/v/rust-doctor?logo=rust"></a>
  <a href="https://www.npmjs.com/package/rust-doctor"><img alt="npm" src="https://img.shields.io/npm/v/rust-doctor?logo=npm"></a>
  <a href="https://docs.rs/rust-doctor"><img alt="docs.rs" src="https://img.shields.io/docsrs/rust-doctor?logo=docsdotrs&label=docs.rs"></a>
  <a href="https://github.com/arthjean/rust-doctor/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/arthjean/rust-doctor/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://crates.io/crates/rust-doctor"><img alt="Downloads" src="https://img.shields.io/crates/d/rust-doctor?label=downloads"></a>
  <a href="#license"><img alt="License" src="https://img.shields.io/crates/l/rust-doctor"></a>
  <img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.97-blue?logo=rust">
</p>

**The one-command health check for your Rust project.** rust-doctor scans for security, performance, correctness, architecture, and dependency issues, then folds everything into a single 0–100 score with diagnostics you can act on.

It combines 34 custom AST rules with Clippy and optional Cargo analyzers behind one canonical diagnostic contract. CLI, API, MCP, and Action scans emit versioned Report V1; SARIF and editor integrations project the same rule and source identities. Supply-chain analysis prefers `cargo-deny` and falls back to `cargo-audit` when `cargo-deny` is unavailable.

## Quickstart

`npx` downloads a pre-built native binary, so installing rust-doctor does not require compiling it:

```bash
npx rust-doctor            # scan the current directory and print the score
```

Scanning requires Cargo because project discovery runs `cargo metadata`. Full compiler-aware analysis also requires rustc and Clippy; other Cargo analyzers are optional and degrade explicitly when unavailable.

Prefer cargo? `cargo install rust-doctor`. Want it in your AI agent? `npx rust-doctor install`. Other formats are in [Installation](#installation). Browse the generated rule catalog and product documentation at [rust-doctor.vercel.app](https://rust-doctor.vercel.app).

### See it in action →

https://github.com/user-attachments/assets/6766a5d8-9a47-4eb8-892e-76c1a23eb122

## Where it fits

Rust already has excellent point tools. rust-doctor runs them together, adds rules they don't cover, and turns the result into one number you can track over time.

| You're using | It gives you | rust-doctor adds |
|---|---|---|
| `cargo clippy` | 700+ built-in lints | Category + severity mapping, 34 custom AST rules (security, async, framework, architecture), and a single 0–100 score |
| `cargo audit` / `cargo deny` | CVE and supply-chain checks | `cargo-deny` as the primary adapter, `cargo-audit` as its fallback, plus geiger and cargo-shear |
| Separate CI steps | Each tool's own output | Report V1, `--sarif`, scoped and baseline scans, completeness gates, and stable PR reporting |
| Coding agents and editors | Code generation and inline diagnostics | MCP, skills, and handoffs for agents; one LSP contract for VS Code, Cursor, and Zed |

## Features

- **700+ Clippy lints**, including 74 with explicit severity overrides and category mapping
- **34 custom AST rules** via [syn](https://crates.io/crates/syn): error handling, performance, security, async, architecture, and framework anti-patterns
- **Async anti-pattern detection**: blocking calls and `block_on` inside an async context
- **Framework-aware rules**: dependency-gated tokio, axum, and actix-web checks plus capability packs gated by Cargo version, features, and target
- **Supply-chain auditing**: advisories, licenses, bans, unsafe usage, and unused dependencies through optional Cargo adapters
- **Canonical rule policy**: typed rule, category, tag, path, threshold, and output-surface controls
- **A 0–100 health score** across five weighted dimensions, with an ASCII doctor that reacts to the result
- **Report V1**: stable diagnostic identities, explicit outcomes, completeness, score authority, package ownership, and structured errors
- **Six scan scopes**: full, files, changed, lines, staged index, and baseline comparison
- **Honest partial analysis**: check states, one wall-clock budget, cancellation, and `--require-complete`
- **MCP server**: 4 read-only tools + 2 expert audit prompts for any MCP client
- **Category scans**: select one or more categories, skip irrelevant passes, and get a score scoped to that selection
- **Workspace support**: scan every crate or pick specific members
- **Inline suppression**: `// rust-doctor-disable-next-line <rule>`
- **Output and handoff modes**: terminal, Report V1 JSON, score, SARIF, bounded diagnostic dumps, agent handoff, and stateless sharing
- **Agent installer**: reversible skill, MCP, and staged-hook setup for Claude Code, Cursor, Codex, OpenCode, and Windsurf
- **Editor diagnostics**: a feature-gated LSP plus VS Code-compatible and Zed adapters
- **Managed CI**: GitHub baseline, review, status, and SARIF channels plus a GitLab gate-only scaffold
- **Privacy by default**: zero telemetry network requests without explicit consent and no report upload for `--share`
- **Distributed everywhere**: CLI binary, library crate, MCP server, npm package, and GitHub Action

## Installation

### npm / npx (recommended for MCP users)

```bash
npx rust-doctor --mcp
```

Or install globally:

```bash
npm install -g rust-doctor
```

This downloads a pre-built native binary for your platform. Installation needs no Rust compiler; scanning still requires Cargo.

### cargo install (from source)

```bash
cargo install rust-doctor
```

### cargo binstall (pre-built binary)

```bash
cargo binstall rust-doctor
```

### Shell installer (Linux/macOS)

```bash
curl -fsSL https://github.com/arthjean/rust-doctor/releases/latest/download/install.sh | bash
```

### PowerShell installer (Windows)

```powershell
irm https://github.com/arthjean/rust-doctor/releases/latest/download/install.ps1 | iex
```

### GitHub Releases

Download pre-built binaries from [GitHub Releases](https://github.com/arthjean/rust-doctor/releases).

Available platforms:
- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`
- `x86_64-apple-darwin`
- `aarch64-apple-darwin`
- `x86_64-pc-windows-msvc`

## Usage

```bash
# Scan current directory
rust-doctor

# Scan a specific directory
rust-doctor /path/to/project

# Get bare score for CI
rust-doctor --score

# Report V1 JSON (pretty, compact, or atomic file)
rust-doctor --json
rust-doctor --json --json-compact
rust-doctor --json --json-out report.json

# SARIF for code-scanning consumers
rust-doctor --sarif

# Report findings in changed files, including untracked files
rust-doctor --scope changed --include-untracked

# Report findings intersecting changed lines
rust-doctor --scope lines --base main

# Analyze the exact Git index snapshot
rust-doctor --staged

# Compare head with a merge-base and report introduced/fixed findings
rust-doctor --baseline --base main

# Report only explicit paths
rust-doctor --scope files --files src/lib.rs,src/api.rs

# Compare changed files against a specific branch
rust-doctor --scope changed --base main

# Fail CI on errors
rust-doctor --blocking error

# Bound the whole scan and require every mandatory analyzer to complete
rust-doctor --max-duration 120 --require-complete

# Scan specific workspace members
rust-doctor --project core,api

# Verbose output with file:line details
rust-doctor --verbose

# Scan only security and performance findings
rust-doctor --category security,performance

# Disable one analyzer family explicitly
rust-doctor --disable-adapter network-dependent

# Hide warning details in terminal output and bound workspace concurrency
rust-doctor --warnings hide --jobs 4

# Audit findings hidden by inline rust-doctor directives
rust-doctor --no-respect-inline-disables

# Install missing external tools (cargo-deny, cargo-audit, etc.)
rust-doctor --install-deps

# Run as MCP server
rust-doctor --mcp

# Write bounded diagnostic groups and hand them to Codex
rust-doctor --output-dir rust-doctor-report --handoff codex

# Print a stateless public summary URL without uploading the report
rust-doctor --share

# Install agent integration; `setup` remains an alias
rust-doctor install --agent codex --yes --hook pre-commit

# Inspect effective rules, explain one rule, or explain one source location
rust-doctor rules list --category security
rust-doctor rules explain unwrap-in-production
rust-doctor why src/lib.rs:42

# Report binary, toolchain, target, and OS versions without building the project
rust-doctor version
```

### Output contract

`--json` emits Report V1 data. `--json-compact` selects compact serialization and `--json-out` selects an atomic file destination when combined with `--json`. Report construction, scan outcome, completeness, score authority, and quality-gate result are independent fields: an empty diagnostic list is not evidence of a complete or authoritative scan. Expected discovery, configuration, and scan failures remain schema-valid when JSON output is available.

The checked [Draft 2020-12 schema](schemas/report-v1.schema.json) is the machine contract. [Report V1 migration rules](docs/report-v1-migration.md) define additive compatibility and when a new schema version is required.

Terminal diagnostics are written to stderr and the score box is written to stdout. `--score` writes one bare integer when an authoritative score is available; otherwise stdout stays empty and stderr explains why. `--json` and `--sarif` write machine output to stdout; `--json --json-compact` removes indentation, and `--json --json-out <path>` atomically writes JSON to the selected file instead.

`--score`, `--sarif`, and `--json` are mutually exclusive output modes. `--json-compact` and `--json-out` only take effect with `--json`; otherwise they are accepted and ignored, matching React Doctor. `--color` and `--no-color` affect terminal rendering only and conflict when both are explicit.

`--output-dir` and `--handoff` do not alter the computed report, stdout/stderr routing, or gate result. Online terminal runs include a sanitized stateless sharing URL by default. `--share` requests the same local URL explicitly when the default footer is suppressed.

### Scan scopes and completeness

| Scope | Invocation | Contract |
|---|---|---|
| Full | `rust-doctor` | Analyze the discovered Cargo project or selected workspace members |
| Files | `--scope files --files src/lib.rs` | Report explicit project-relative paths |
| Changed | `--scope changed [--base main]` | Report affected files; uncommitted work has no historical snapshot |
| Lines | `--scope lines [--base main]` | Report findings intersecting changed lines; degrade visibly to files when ranges are unavailable |
| Staged | `--staged [--scope files\|lines]` | Analyze the exact Git index snapshot; `lines` reports only diagnostics intersecting indexed hunks |
| Baseline | `--baseline [--base main]` | Compare head and merge-base findings, reporting introduced, fixed, and degraded states |

Scope is both an execution input and a reporting contract. File-local AST rules read only selected files. Clippy and package/workspace analyzers may still execute at package scope, then report only diagnostics allowed by the requested scope.

Every Report V1 instance accounts for planned and analyzed files plus completed, skipped, failed, timed-out, and cancelled checks. `--max-duration` applies one wall-clock budget to the complete scan. `--require-complete` returns exit code `1` when required work is incomplete.

### Category scans

`--category` accepts a comma-separated selection from `error-handling`,
`performance`, `security`, `correctness`, `architecture`, `dependencies`,
`async`, `framework`, `cargo`, and `style`.

```bash
rust-doctor --category security
rust-doctor --category security,dependencies --score
```

Category scans filter custom rules before analysis and avoid external passes
that cannot produce a selected category. Clippy still runs because its lint
registry spans several categories. Diagnostics, package scores, the overall
score, and the terminal score card are restricted to the selected categories.
When several scoring dimensions are selected, their standard weights are
preserved and unselected dimensions are excluded from the average.

## Exit Codes

rust-doctor follows React Doctor's compact CLI contract:

| Code | Meaning |
|------|---------|
| `0` | The command succeeded and no effective gate blocked |
| `1` | Invalid input, setup/scan/output failure, incomplete required analysis, or a blocking finding |
| `130` | The process received `SIGINT` or `SIGTERM` |

Errors block by default. `--blocking none` makes findings advisory. When a
reliable score exists, `--score` prints it bare and does not fail only because
findings exist.

| Report | Completeness policy | Finding/score gate | Exit |
|---|---|---|---|
| Unavailable | Any | Any | `1` |
| Available but incomplete | Required | Any | `1` |
| Available | Complete or incompleteness allowed | Blocked | `1` |
| Available | Complete or incompleteness allowed | Passed | `0` |

Gate the build on errors:

```bash
rust-doctor --blocking error
```

## AI Agent Setup (recommended)

The fastest way to integrate rust-doctor with your AI coding agent:

```bash
npx rust-doctor@latest install
```

`setup` remains an alias for `install`. The installer auto-detects Claude Code, Cursor, Codex, OpenCode, and Windsurf, then previews a reversible plan:

- **CLI + Skills** (default): installs a `SKILL.md` that teaches the agent the rust-doctor workflow
- **MCP Server**: configures the existing `rust-doctor --mcp` stdio entry
- **Staged hook**: adds a namespaced pre-commit block without replacing unrelated hook content

Use `--dry-run` for an exact preview, `--yes` for non-interactive installation, and `uninstall` to remove only Rust Doctor-managed files or marked blocks:

```bash
rust-doctor install --agent codex --mcp --hook pre-commit --dry-run
rust-doctor uninstall --agent codex --dry-run
```

For manual setup, see the sections below.

## MCP Server

rust-doctor includes a built-in [Model Context Protocol](https://modelcontextprotocol.io/) server, allowing AI coding assistants to scan and analyze Rust projects directly.

### Tools

| Tool | Description |
|------|-------------|
| `scan` | Scan a Rust project for code health issues. Returns diagnostics with a 0–100 health score. |
| `score` | Get the health score (0–100) of a Rust project as a single integer. |
| `explain_rule` | Get a detailed explanation of a rule: what it checks, why it matters, and how to fix violations. |
| `list_rules` | List all available rules with their categories and severities. |

All tools are read-only (`readOnlyHint: true`).

The `scan` tool accepts full, files, changed, lines, staged, and baseline scopes. MCP scans default to offline mode, require an absolute project path under the user's home directory, and expose canonical rule documentation through `rule://<rule-id>` resources.

### Prompts

| Prompt | Description |
|--------|-------------|
| `deep-audit` | Comprehensive 6-phase expert audit: codebase exploration, static analysis, deep code review (51-item checklist), best practices research, synthesis report, and remediation choices (implement all / generate PRD / manual). |
| `health-check` | Quick scan + prioritized remediation plan (P0–P3) + fix workflow. |

### Claude Code

**Automatic setup (recommended):**

```bash
rust-doctor install  # detects Claude Code and configures MCP or installs skill
```

**Or one-command MCP install:**

```bash
claude mcp add --transport stdio rust-doctor -- npx -y rust-doctor --mcp
```

**Or via Claude Code plugin:**

```
/plugin install rust-doctor@arthjean/rust-doctor
```

**Or add manually** to your `~/.claude/settings.json`:

```json
{
  "mcpServers": {
    "rust-doctor": {
      "command": "rust-doctor",
      "args": ["--mcp"]
    }
  }
}
```

**Or share with your team** via `.mcp.json` in your project root (committed to git):

```json
{
  "mcpServers": {
    "rust-doctor": {
      "command": "npx",
      "args": ["-y", "rust-doctor", "--mcp"]
    }
  }
}
```

### Cursor

Add to your `.cursor/mcp.json`:

```json
{
  "mcpServers": {
    "rust-doctor": {
      "command": "npx",
      "args": ["-y", "rust-doctor", "--mcp"]
    }
  }
}
```

### VS Code

Add to your `.vscode/settings.json`:

```json
{
  "mcp": {
    "servers": {
      "rust-doctor": {
        "type": "stdio",
        "command": "npx",
        "args": ["-y", "rust-doctor", "--mcp"]
      }
    }
  }
}
```

### Windsurf

Add to your `~/.codeium/windsurf/mcp_config.json`:

```json
{
  "mcpServers": {
    "rust-doctor": {
      "command": "npx",
      "args": ["-y", "rust-doctor", "--mcp"]
    }
  }
}
```

### Other MCP clients

rust-doctor uses stdio transport. Any MCP client that supports stdio can connect by running `rust-doctor --mcp`.

Built with [rmcp](https://crates.io/crates/rmcp) v1.x (official Rust MCP SDK).

## Claude Code Skill (no MCP required)

If you prefer slash commands over MCP servers, rust-doctor ships a Claude Code skill.

**Automatic install (recommended):**

```bash
rust-doctor install  # choose "CLI + Skills", select Claude Code
```

**Or via npx:**

```bash
npx skills add https://github.com/arthjean/rust-doctor --skill rust-doctor
```

**Or copy manually:**

```bash
cp -r skills/rust-doctor/ ~/.claude/skills/rust-doctor/
```

**Usage:**

```
/rust-doctor                    # scan current project
/rust-doctor --scope changed    # report changed-file findings
/rust-doctor --staged           # scan the exact index snapshot
/rust-doctor --baseline         # report introduced and fixed findings
/rust-doctor --plan             # scan + remediation plan
/rust-doctor src/               # scan a specific directory
```

The skill runs the `rust-doctor` CLI under the hood, parses the output, categorizes findings by priority, and provides actionable fix guidance with before/after code.

## Editor diagnostics

Build the binary with the editor server enabled:

```bash
cargo install rust-doctor --features lsp
```

The [VS Code and Cursor extension](editors/vscode) and [Zed extension](editors/zed) both launch `rust-doctor --lsp`, negotiate Rust Doctor editor protocol major 1 independently from the binary version, use 300 ms file-local analysis by default, expose hover metadata and safe suppression actions, and keep project-wide on-save checks opt-in. An empty configuration path uses normal project discovery and defaults; set a path only to require that specific file. See each editor directory for binary-path and packaging instructions.

## Managed CI

Install or preview the least-privilege GitHub workflow:

```bash
rust-doctor ci install --scope baseline --blocking warning
rust-doctor ci install --dry-run
rust-doctor ci config --review-comments=true --commit-status=true
rust-doctor ci upgrade --version v1
```

`ci config` and `ci upgrade` mutate only the marker-owned workflow block. `ci install --pr` creates a branch and pull request only after local Git and provider validation succeeds; a failed push or PR creation restores the original checkout and removes the temporary remote branch when one was created. GitLab is supported as a gate-only scaffold with `rust-doctor ci install --provider gitlab`; it uses baseline scope only for merge requests with a non-empty base SHA and otherwise runs full scope. Comments, statuses and SARIF remain GitHub-only channels.

The Action can also be configured directly:

```yaml
- uses: arthjean/rust-doctor@v1
  with:
    scope: baseline
    blocking: warning
    require-complete: true
    comment: true
    review-comments: false
    commit-status: true
    sarif: true
    token: ${{ secrets.GITHUB_TOKEN }}
```

Pull requests resolve their base locally, then use the paginated GitHub API only when history is unavailable. API fallback is reported as a degraded files scope and is never labeled introduced-only. Reporting channels degrade independently: a denied comment, status or SARIF permission does not replace the configured scan gate.

## Configuration

Create a `rust-doctor.toml` in your project root, or add `[package.metadata.rust-doctor]` to your `Cargo.toml`:

```toml
# rust-doctor.toml
verbose = false
fail_on = "none"

[rules.unwrap-in-production]
severity = "error"
surfaces = ["terminal", "score", "ci-failure", "pr-comment", "sarif", "mcp"]

[categories.performance]
severity = "info"

[[path_overrides]]
pattern = "tests/**"
severity = "off"

[ignore]
files = ["**/generated/**"]
```

CLI flags override config file values.

Policy precedence is deterministic: catalog default, tag, category, exact rule, then the last matching path override. A `surfaces` list controls where an active rule is visible; it does not activate the rule. Test, benchmark, example, and generated-source findings are excluded from score and CI-failure surfaces by default unless an explicit policy includes them.

Use the transactional rule commands instead of editing policy by hand. Every mutation validates the catalog and TOML first, preserves unrelated formatting, writes atomically, and supports `--dry-run`:

```bash
rust-doctor rules explain unwrap-in-production
rust-doctor rules set unwrap-in-production error --dry-run
rust-doctor rules enable string-from-literal --dry-run
rust-doctor rules disable excessive-clone --dry-run
rust-doctor rules category performance info --dry-run
rust-doctor rules ignore-tag style --dry-run
```

## Privacy, telemetry, and sharing

Local CLI, MCP, LSP, and Action runs send no telemetry by default. Enabling telemetry requires explicit consent to one HTTPS endpoint; loopback HTTP is accepted only for local development. Events contain aggregate product and completeness data, never source, paths, repository identity, diagnostic messages, Git remotes, environment values, or command arguments. Delivery is attempted once with no durable event queue or cross-project identifier.

```bash
rust-doctor telemetry status
rust-doctor telemetry enable --endpoint https://telemetry.example.com --yes
rust-doctor telemetry disable
```

`--no-telemetry`, `RUST_DOCTOR_TELEMETRY=0`, and `--offline` override stored consent. `--share` constructs a versioned URL locally from a bounded aggregate summary. It uploads no report or diagnostic data and includes no repository identity.

## Inline Suppression

```rust
// rust-doctor-disable-next-line unwrap-in-production
let value = some_option.unwrap();

let x = risky_call(); // rust-doctor-disable-line
```

## Rules

### Custom AST Rules (34 rules) - heuristic

These rules analyze the syntax tree only (via `syn`): no type resolution, no
macro expansion. They run fast and offline, but emit a **heuristic** signal, not
a type-checked verdict. The canonical catalog is built directly from the 34 rule
implementations and the Clippy registry. MCP `list_rules` and `explain_rule`
render that catalog, and tests assert these counts, so adding a rule does not
require maintaining a second documentation table.

#### Known heuristic limitations

Without type information these rules have documented blind spots. They're still
worth surfacing, but a finding is a prompt to look, not a confirmed defect:

- `unwrap-in-production`: matches `.unwrap()`/`.expect()` syntactically; cannot tell a provably-infallible unwrap from a risky one.
- `large-enum-variant`: counts a variant's fields, not its byte size; a few wide-type fields can outweigh many small ones.
- `blocking-in-async`: flags known blocking calls by name inside async fns; cannot follow calls into other functions or resolve aliased imports.
- `sql-injection-risk`: flags string-built queries heuristically; cannot confirm the interpolated value is actually untrusted input.

### Clippy Lints (74 with overrides) - type-aware

rust-doctor runs `cargo clippy` with pedantic, nursery, and cargo lint groups. Exactly 74 lints have explicit category and severity overrides across: Error Handling, Performance, Security, Correctness, Architecture, Cargo, Async, Style. Unlike the custom rules above, Clippy resolves types against the compiler, so its findings are more authoritative.

### External Tools (optional, auto-detected)

These tools are optional: rust-doctor records unavailable adapters as explicit skipped checks. Run `rust-doctor --install-deps` to install them all at once.

| Tool | Install | What it does |
|------|---------|-------------|
| clippy | `rustup component add clippy` | 700+ lint checks |
| cargo-deny | `cargo install cargo-deny` | Primary supply-chain adapter (advisories, licenses, bans) |
| cargo-audit | `cargo install cargo-audit` | Advisory fallback when cargo-deny is unavailable |
| cargo-geiger | `cargo install cargo-geiger` | Unsafe code auditing across dependency tree |
| cargo-shear | `cargo install cargo-shear` | Unused dependency detection |
| cargo-semver-checks | `cargo install cargo-semver-checks` | Semver violation detection |

## Library Usage

The versioned public API returns Report V1 without terminal rendering, process exit, or implicit network access:

```rust
use std::path::Path;
use rust_doctor::api::{ScanRequest, ScanScope};

let mut request = ScanRequest::new(Path::new("/path/to/project"));
request.options.scope = ScanScope::Changed {
    base: Some("main".to_string()),
    include_untracked: true,
};

let report = rust_doctor::api::scan(request)?;
println!("Diagnostics: {}", report.summary.diagnostic_count);
if let Some(score) = report
    .summary
    .score
    .filter(|_| report.summary.score_authoritative)
{
    println!("Score: {score}");
}
```

`ScanOptions` exposes typed configuration overrides, adapter policy, workspace parallelism, deadline, cancellation, and every CLI scope. `scan_batch` preserves request order and successful sibling reports when another project fails; `invalidate_cache` removes only Rust Doctor's project cache. Full API docs are on [docs.rs/rust-doctor](https://docs.rs/rust-doctor).

## Score Calculation

**Read the 0–100 score as a compass, not a thermometer.** It points you toward
the weakest dimension; it isn't a precision measurement. The per-dimension
scores (shown in the terminal box and in `--json`) carry the real signal:
they tell you *where* to act.

### How it's computed

The score is a weighted average across 5 dimensions:

| Dimension | Weight | Covers |
|-----------|--------|--------|
| Security | ×2.0 | Security rules (hardcoded secrets, unsafe, SQL injection) |
| Reliability | ×1.5 | Correctness, error handling, async, framework |
| Maintainability | ×1.0 | Architecture, style |
| Performance | ×1.0 | Performance |
| Dependencies | ×1.0 | Cargo, dependencies, advisory findings (RUSTSEC / cargo-deny) |

Each dimension starts at 100 and loses points per **unique rule** violated,
weighted by severity:

`dimension = 100 − (unique_error_rules × 1.5) − (unique_warning_rules × 0.75) − (unique_info_rules × 0.25)`

The dimension is clamped to `[0, 100]`, and the overall score is the weighted
average of the five, also clamped to `[0, 100]`.

With `--category`, only dimensions represented by the selected categories enter
the weighted average. Selecting multiple categories from the same dimension,
such as `correctness` and `error-handling`, combines their unique violated rules
inside that dimension.

The score counts unique rules, not occurrences: fixing one `.unwrap()` won't
move it, but removing the last `.unwrap()` drops the penalty entirely.

| Score | Label | Doctor |
|-------|-------|--------|
| 75–100 | Great | ◠ ◠ |
| 50–74 | Needs work | • • |
| 0–49 | Critical | x x |

### Known limits

- **Dimension saturation.** Penalties are linear and the floor is 0, so once a
  dimension accumulates ~67 distinct Error-severity rules (`100 ÷ 1.5`), it sits
  at 0 and further distinct rules in that dimension stop moving the number. It is
  directional past that point, not proportional.
- **Heuristic inputs.** The custom AST rules are `syn`-only (no types, no macro
  expansion), so part of what feeds the score is a heuristic signal; see
  [Rules](#rules). Clippy is type-aware. External adapters instead inspect
  manifests, lockfiles, dependency graphs, or tool-specific compiler metadata.
  The score does not currently weight these evidence models differently.
- **Surface policy.** Only diagnostics visible on the canonical `score` surface
  enter the calculation. Test, benchmark, example, and generated-source findings
  are excluded from score and CI failure by default unless policy includes them.
- **Partial analysis.** A report may retain a score after a timeout, cancellation,
  or required-check failure, but `summary.score_authoritative` is false. Never gate
  on the integer without checking completeness or using `--require-complete`.
- **Hand-tuned weights.** The dimension weights and severity penalties are
  deliberate but not empirically calibrated; treat cross-project score
  comparisons with caution.
- **Nothing to scan.** When discovery and scope planning find no applicable
  source, manifest, lockfile, package, or workspace work, Report V1 returns
  `nothing_to_scan`, a null score, and a non-authoritative summary. A manifest-only
  scope may still schedule dependency or package checks.

## Contributing

Contributions are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) for the dev
setup, the CI gates to run before opening a PR (`cargo fmt`, `cargo clippy`,
`cargo test`), and the guide to authoring a new rule. By participating you agree
to the [Code of Conduct](CODE_OF_CONDUCT.md). For security issues, follow the
[Security Policy](SECURITY.md): please don't open a public issue.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
