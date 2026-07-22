# rust-doctor

<p align="center">
  <a href="https://crates.io/crates/rust-doctor"><img alt="Crates.io" src="https://img.shields.io/crates/v/rust-doctor?logo=rust"></a>
  <a href="https://www.npmjs.com/package/rust-doctor"><img alt="npm" src="https://img.shields.io/npm/v/rust-doctor?logo=npm"></a>
  <a href="https://docs.rs/rust-doctor"><img alt="docs.rs" src="https://img.shields.io/docsrs/rust-doctor?logo=docsdotrs&label=docs.rs"></a>
  <a href="https://github.com/arthjean/rust-doctor/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/arthjean/rust-doctor/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://crates.io/crates/rust-doctor"><img alt="Downloads" src="https://img.shields.io/crates/d/rust-doctor?label=downloads"></a>
  <a href="#license"><img alt="License" src="https://img.shields.io/crates/l/rust-doctor"></a>
  <img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.85-blue?logo=rust">
</p>

**The one-command health check for your Rust project.** rust-doctor scans for security, performance, correctness, architecture, and dependency issues, then folds everything into a single 0–100 score with diagnostics you can act on.

It runs `cargo clippy`, `cargo-audit`, `cargo-deny`, `cargo-geiger`, and 34 custom AST rules in one pass, and ships as a CLI, a library crate, an [MCP](https://modelcontextprotocol.io/) server, an npm package, and a GitHub Action, so it works in your terminal, your CI, and inside Claude Code, Cursor, or any MCP agent.

```console
$ rust-doctor                          # rust-doctor scanning its own codebase

   ◠ ◠    rust-doctor
    ▽     99 / 100   Great

   ████████████████████████████████████████

   Security 99 · Reliability 99 · Maintainability 100 · Performance 99 · Dependencies 99

   ✓ 0 errors   ⚠ 44 warnings   ℹ 42 infos   ·   60 files scanned in 32.9s
```

## Quickstart

No Rust toolchain required — `npx` downloads a pre-built native binary for your platform:

```bash
npx rust-doctor            # scan the current directory and print the score
```

Prefer cargo? `cargo install rust-doctor`. Want it in your AI agent? `npx rust-doctor setup`. Other formats are in [Installation](#installation).

### See it in action →

https://github.com/user-attachments/assets/6766a5d8-9a47-4eb8-892e-76c1a23eb122

## Where it fits

Rust already has excellent point tools. rust-doctor runs them together, adds rules they don't cover, and turns the result into one number you can track over time.

| You're using | It gives you | rust-doctor adds |
|---|---|---|
| `cargo clippy` | 700+ built-in lints | Category + severity mapping, 34 custom AST rules (security, async, framework, architecture), and a single 0–100 score |
| `cargo audit` / `cargo deny` | CVE and supply-chain checks | One pass that also runs clippy, geiger, and machete — skipping gracefully when a tool isn't installed |
| Separate CI steps | Each tool's own output | One command with `--json`, `--sarif`, `--diff`, `--score`, and PR comments |
| Claude Code / Cursor | Code generation | An MCP server and a slash-command skill, so the agent scans, scores, and fixes as it writes |

## Features

- **700+ clippy lints** with explicit severity overrides and category mapping
- **34 custom AST rules** via [syn](https://crates.io/crates/syn): error handling, performance, security, async, architecture, and framework anti-patterns
- **Async anti-pattern detection**: blocking calls and `block_on` inside an async context
- **Framework-aware rules**: tokio, axum, actix-web — only run when the dependency is present
- **Supply-chain auditing**: CVEs via `cargo-audit`, bans/licenses via `cargo-deny`, unsafe via `cargo-geiger`, unused deps via `cargo-machete`
- **A 0–100 health score** across five weighted dimensions, with an ASCII doctor that reacts to the result
- **MCP server**: 4 read-only tools + 2 expert audit prompts for Claude Code, Cursor, Windsurf, or any MCP client
- **Diff mode**: `--diff` scans only changed files for fast CI feedback
- **Workspace support**: scan every crate or pick specific members
- **Inline suppression**: `// rust-doctor-disable-next-line <rule>`
- **Output modes**: terminal, `--json`, `--score` (bare integer for CI), `--sarif` (GitHub code scanning)
- **`--fix`**: apply machine-applicable fixes to source files
- **Setup wizard**: `rust-doctor setup` auto-detects Claude Code, Cursor, and Windsurf and wires up MCP or installs the skill in one command
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

This downloads a pre-built native binary for your platform — no Rust toolchain required.

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

# JSON output (pretty, compact, or atomic file)
rust-doctor --json
rust-doctor --json-compact
rust-doctor --json-out report.json

# SARIF for code-scanning consumers
rust-doctor --sarif

# Scan only changed files, including untracked files
rust-doctor --scope changed --include-untracked

# Scan against a specific branch
rust-doctor --scope changed --base main

# Fail CI on errors
rust-doctor --blocking error

# Require every mandatory analyzer to complete
rust-doctor --require-complete

# Scan specific workspace members
rust-doctor --project core,api

# Verbose output with file:line details
rust-doctor --verbose

# Hide warning details in terminal output and bound workspace concurrency
rust-doctor --warnings hide --jobs 4

# Audit findings hidden by inline rust-doctor directives
rust-doctor --no-respect-inline-disables

# Install missing external tools (cargo-deny, cargo-audit, etc.)
rust-doctor --install-deps

# Run as MCP server
rust-doctor --mcp

# Setup wizard — configure AI agents automatically
rust-doctor setup

# Inspect effective rules or explain one source location
rust-doctor rules list --category security
rust-doctor why src/lib.rs:42

# Report binary, toolchain, target, and OS versions without building the project
rust-doctor version
```

### Output contract

Terminal diagnostics are written to stderr and the score box is written to stdout. `--score` writes one bare integer to stdout. `--json`, `--json-compact`, and `--sarif` write machine output to stdout; `--json-out` atomically writes JSON to the selected file instead.

`--score`, `--sarif`, `--json`, and `--json-compact` are mutually exclusive. `--json-out` may be combined with `--json` or `--json-compact`, but conflicts with `--score` and `--sarif`. `--color` and `--no-color` affect terminal rendering only and conflict when both are explicit.

## Exit Codes

rust-doctor returns distinct exit codes so CI pipelines can tell a quality-gate
failure apart from a crash:

| Code | Meaning |
|------|---------|
| `0` | Success: scan completed and all quality gates passed |
| `1` | Setup error: MCP server, installer, or `--install-deps` failed |
| `2` | Scan error: project discovery, analysis, or output rendering failed |
| `3` | Quality gate failed: score below `[score] fail_below` or `--blocking` threshold reached |
| `4` | Required analysis incomplete while `--require-complete` is active |

Gate the build on a quality failure without masking a crash:

```bash
rust-doctor --blocking error
if [ $? -eq 3 ]; then
  echo "Quality gate failed"
  exit 1
fi
```

## AI Agent Setup (recommended)

The fastest way to integrate rust-doctor with your AI coding agent:

```bash
npx rust-doctor@latest setup
```

The wizard auto-detects installed agents (Claude Code, Cursor, Windsurf) and lets you choose:

- **CLI + Skills** (default) — installs a `SKILL.md` that teaches your agent to use the rust-doctor CLI with deep analysis capabilities
- **MCP Server** — configures the `rust-doctor --mcp` stdio server in your agent's config file

The wizard handles detection, configuration, and verification in one command. For manual setup, see the sections below.

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

### Prompts

| Prompt | Description |
|--------|-------------|
| `deep-audit` | Comprehensive 6-phase expert audit: codebase exploration, static analysis, deep code review (51-item checklist), best practices research, synthesis report, and remediation choices (implement all / generate PRD / manual). |
| `health-check` | Quick scan + prioritized remediation plan (P0–P3) + fix workflow. |

### Claude Code

**Automatic setup (recommended):**

```bash
rust-doctor setup  # detects Claude Code and configures MCP or installs skill
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
rust-doctor setup  # choose "CLI + Skills", select Claude Code
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
/rust-doctor --diff             # scan changed files only
/rust-doctor --fix              # scan + apply fixes
/rust-doctor --plan             # scan + remediation plan
/rust-doctor src/               # scan a specific directory
```

The skill runs the `rust-doctor` CLI under the hood, parses the output, categorizes findings by priority, and provides actionable fix guidance with before/after code.

## Editor diagnostics

Build the binary with the editor server enabled:

```bash
cargo install rust-doctor --features lsp
```

The VS Code and Cursor extension lives in `editors/vscode`; the Zed extension lives in `editors/zed`. Both launch `rust-doctor --lsp`, use 300 ms file-local analysis by default, expose hover metadata and safe suppression actions, and keep project-wide on-save checks opt-in. See each editor directory for binary-path and packaging instructions.

## Managed CI

Install or preview the least-privilege GitHub workflow:

```bash
rust-doctor ci install --scope baseline --blocking warning
rust-doctor ci install --dry-run
rust-doctor ci config --review-comments=true --commit-status=true
rust-doctor ci upgrade --version v1
```

`ci config` and `ci upgrade` mutate only the marker-owned workflow block. `ci install --pr` creates a branch and pull request only after local Git and provider validation succeeds. GitLab is supported as a gate-only scaffold with `rust-doctor ci install --provider gitlab`; comments, statuses and SARIF remain GitHub-only channels.

The Action can also be configured directly:

```yaml
- uses: arthjean/rust-doctor@v1
  with:
    scope: baseline
    blocking: warning
    require-complete: true
    comment: true
    commit-status: true
    sarif: true
    token: ${{ secrets.GITHUB_TOKEN }}
```

Pull requests resolve their base locally, then use the paginated GitHub API only when history is unavailable. Reporting channels degrade independently: a denied comment, status or SARIF permission does not replace the configured scan gate.

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

#### Known heuristic limitations (⚠)

Without type information these rules have documented blind spots. They're still
worth surfacing, but a finding is a prompt to look, not a confirmed defect:

- `unwrap-in-production` — matches `.unwrap()`/`.expect()` syntactically; cannot tell a provably-infallible unwrap from a risky one.
- `large-enum-variant` — counts a variant's fields, not its byte size; a few wide-type fields can outweigh many small ones.
- `blocking-in-async` — flags known blocking calls by name inside async fns; cannot follow calls into other functions or resolve aliased imports.
- `sql-injection-risk` — flags string-built queries heuristically; cannot confirm the interpolated value is actually untrusted input.

### Clippy Lints (74 with overrides) - type-aware

rust-doctor runs `cargo clippy` with pedantic, nursery, and cargo lint groups. Exactly 74 lints have explicit category and severity overrides across: Error Handling, Performance, Security, Correctness, Architecture, Cargo, Async, Style. Unlike the custom rules above, Clippy resolves types against the compiler, so its findings are more authoritative.

### External Tools (optional, auto-detected)

These tools are optional — rust-doctor gracefully skips any that are missing and shows which passes were skipped. Run `rust-doctor --install-deps` to install them all at once.

| Tool | Install | What it does |
|------|---------|-------------|
| clippy | `rustup component add clippy` | 700+ lint checks |
| cargo-deny | `cargo install cargo-deny` | Supply-chain checking (advisories, licenses, bans) |
| cargo-audit | `cargo install cargo-audit` | CVE vulnerability scanning |
| cargo-geiger | `cargo install cargo-geiger` | Unsafe code auditing across dependency tree |
| cargo-machete | `cargo install cargo-machete` | Unused dependency detection |
| cargo-semver-checks | `cargo install cargo-semver-checks` | Semver violation detection |

## Library Usage

rust-doctor is available as a library crate:

```rust
use std::path::Path;

// Discover the project (finds Cargo.toml, loads config)
let (dir, info, config) = rust_doctor::discovery::bootstrap_project(
    Path::new("/path/to/project"), false,
)?;

// Resolve config with defaults
let resolved = rust_doctor::config::resolve_config_defaults(config.as_ref());

// Run the scan
let result = rust_doctor::scan::scan_project(&info, &resolved, false, &[], true)?;
println!("Score: {}/100 ({})", result.score, result.score_label);
```

Full API docs are on [docs.rs/rust-doctor](https://docs.rs/rust-doctor).

## Score Calculation

**Read the 0–100 score as a compass, not a thermometer.** It points you toward
the weakest dimension; it isn't a precision measurement. The per-dimension
scores (shown in the terminal box and in `--json`) carry the real signal — they
tell you *where* to act.

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

The score counts unique rules, not occurrences — fixing one `.unwrap()` won't
move it, but removing the last `.unwrap()` drops the penalty entirely.

| Score | Label | Doctor |
|-------|-------|--------|
| 75–100 | Great | ◠ ◠ |
| 50–74 | Needs work | • • |
| 0–49 | Critical | x x |

### Known limits

- **Dimension saturation.** Penalties are linear and the floor is 0, so once a
  dimension accumulates ~67 distinct Error-severity rules (`100 ÷ 1.5`), it sits
  at 0 and further distinct rules in that dimension stop moving the number — it's
  directional past that point, not proportional.
- **Heuristic inputs.** The custom AST rules are `syn`-only (no types, no macro
  expansion), so part of what feeds the score is a heuristic signal — see
  [Rules](#rules). Clippy and external-tool findings are type-aware. The score
  does not currently weight heuristic vs type-aware findings differently.
- **Hand-tuned weights.** The dimension weights and severity penalties are
  deliberate but not empirically calibrated; treat cross-project score
  comparisons with caution.
- **Empty projects.** A directory with no Rust source files scores 100 and emits
  `No Rust source files found` — expected, not a clean bill of health.

## Contributing

Contributions are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) for the dev
setup, the CI gates to run before opening a PR (`cargo fmt`, `cargo clippy`,
`cargo test`), and the guide to authoring a new rule. By participating you agree
to the [Code of Conduct](CODE_OF_CONDUCT.md). For security issues, follow the
[Security Policy](SECURITY.md) — please don't open a public issue.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
