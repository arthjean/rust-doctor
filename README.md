# rust-doctor

[![Crates.io](https://img.shields.io/crates/v/rust-doctor)](https://crates.io/crates/rust-doctor)
[![npm](https://img.shields.io/npm/v/rust-doctor)](https://www.npmjs.com/package/rust-doctor)
[![CI](https://github.com/ArthurDEV44/rust-doctor/actions/workflows/ci.yml/badge.svg)](https://github.com/ArthurDEV44/rust-doctor/actions/workflows/ci.yml)
[![Crates.io Downloads](https://img.shields.io/crates/d/rust-doctor)](https://crates.io/crates/rust-doctor)
[![npm Downloads](https://img.shields.io/npm/dm/rust-doctor)](https://www.npmjs.com/package/rust-doctor)

A unified code health tool for Rust — scan, score, and fix your codebase.

rust-doctor scans Rust projects for security, performance, correctness, architecture, and dependency issues, producing a 0–100 health score with actionable diagnostics.

### See it in action →

https://github.com/user-attachments/assets/6766a5d8-9a47-4eb8-892e-76c1a23eb122

## Features

- **700+ clippy lints** with severity overrides and category mapping
- **19 custom AST rules** via syn: error handling, performance, security, async, architecture, framework anti-patterns
- **Async anti-pattern detection**: blocking calls in async, block_on in async context
- **Framework-specific rules**: tokio, axum, actix-web
- **Dependency auditing**: CVE detection via cargo-audit, unused deps via cargo-machete
- **Health score**: 0–100 with ASCII doctor face output
- **MCP server**: integrate with Claude Code, Cursor, or any MCP-compatible AI tool
- **Diff mode**: scan only changed files for fast CI feedback
- **Category-specific scanning**: scan only specific diagnostic categories (e.g. `--category performance`) and automatically bypass unrelated slow passes (like cargo-audit, cargo-deny, cargo-geiger, etc.) for up to 35% faster scans!
- **Workspace support**: scan all crates or select specific members
- **Inline suppression**: `// rust-doctor-disable-next-line <rule>`
- **Multiple output modes**: terminal, `--json`, `--score`, `--sarif`
- **Setup wizard**: `rust-doctor setup` — auto-detects Claude Code, Cursor, Windsurf and configures MCP or installs skills in one command
- **Claude Code skill**: `/rust-doctor` slash command — no MCP setup needed
- **Library crate**: use rust-doctor programmatically via `lib.rs`
- **NO_COLOR support**: respects the NO_COLOR environment variable

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
curl -fsSL https://github.com/ArthurDEV44/rust-doctor/releases/latest/download/install.sh | bash
```

### PowerShell installer (Windows)

```powershell
irm https://github.com/ArthurDEV44/rust-doctor/releases/latest/download/install.ps1 | iex
```

### GitHub Releases

Download pre-built binaries from [GitHub Releases](https://github.com/ArthurDEV44/rust-doctor/releases).

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

# JSON output
rust-doctor --json

# Scan only changed files
rust-doctor --diff

# Scan against a specific branch
rust-doctor --diff main

# Fail CI on errors
rust-doctor --fail-on error

# Scan specific workspace members
rust-doctor --project core,api

# Verbose output with file:line details
rust-doctor --verbose

# Scan only a specific diagnostic category (bypasses unrelated slow checks for speed)
rust-doctor --category performance

# Install missing external tools (cargo-deny, cargo-audit, etc.)
rust-doctor --install-deps

# Run as MCP server
rust-doctor --mcp

# Setup wizard — configure AI agents automatically
rust-doctor setup
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
/plugin install rust-doctor@ArthurDEV44/rust-doctor
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
npx skills add https://github.com/ArthurDEV44/rust-doctor --skill rust-doctor
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

## GitHub Actions

```yaml
- uses: ArthurDEV44/rust-doctor@v1
  with:
    token: ${{ secrets.GITHUB_TOKEN }}
    fail-on: warning
```

The action posts a PR comment with the health score, error/warning counts, and top diagnostics.

## Configuration

Create a `rust-doctor.toml` in your project root, or add `[package.metadata.rust-doctor]` to your `Cargo.toml`:

```toml
# rust-doctor.toml
verbose = false
fail_on = "none"

[ignore]
rules = ["excessive-clone", "string-from-literal"]
files = ["**/generated/**", "tests/**"]
```

CLI flags override config file values.

## Inline Suppression

```rust
// rust-doctor-disable-next-line unwrap-in-production
let value = some_option.unwrap();

let x = risky_call(); // rust-doctor-disable-line
```

## Rules

### Custom AST Rules (19 rules)

| Category | Rule | Severity |
|----------|------|----------|
| Error Handling | `unwrap-in-production` | Warning |
| Error Handling | `panic-in-library` | Error |
| Error Handling | `box-dyn-error-in-public-api` | Warning |
| Error Handling | `result-unit-error` | Warning |
| Performance | `excessive-clone` | Warning |
| Performance | `string-from-literal` | Info |
| Performance | `collect-then-iterate` | Warning |
| Performance | `large-enum-variant` | Warning |
| Performance | `unnecessary-allocation` | Warning |
| Architecture | `high-cyclomatic-complexity` | Warning |
| Security | `hardcoded-secrets` | Error |
| Security | `unsafe-block-audit` | Warning |
| Security | `sql-injection-risk` | Error |
| Async | `blocking-in-async` | Error |
| Async | `block-on-in-async` | Error |
| Framework | `tokio-main-missing` | Error |
| Framework | `tokio-spawn-without-move` | Error |
| Framework | `axum-handler-not-async` | Warning |
| Framework | `actix-blocking-handler` | Warning |

### Clippy Lints (75+ with overrides)

rust-doctor runs `cargo clippy` with pedantic, nursery, and cargo lint groups. 75+ lints have explicit category and severity overrides across: Error Handling, Performance, Security, Correctness, Architecture, Cargo, Async, Style.

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

## Score Calculation

The score uses weighted dimension scoring across 5 dimensions (Security ×2.0, Reliability ×1.5, Maintainability ×1.0, Performance ×1.0, Dependencies ×1.0). Each dimension is scored as `100 - (unique_error_rules × 1.5) - (unique_warning_rules × 0.75) - (unique_info_rules × 0.25)`, and the overall score is the weighted average, clamped to 0–100.

The score counts unique rules violated, not occurrences — fixing one instance of `.unwrap()` won't change the score, but eliminating all `.unwrap()` calls removes the penalty entirely.

| Score | Label | Doctor |
|-------|-------|--------|
| 75–100 | Great | ◠ ◠ |
| 50–74 | Needs work | • • |
| 0–49 | Critical | x x |

## License

MIT OR Apache-2.0
