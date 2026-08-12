<img alt="Rust Doctor" src="./assets/rust-doctor-mark.png" width="52" height="36">

[![version](https://img.shields.io/npm/v/rust-doctor?style=flat&colorA=000000&colorB=000000)](https://npmjs.com/package/rust-doctor)
[![downloads](https://img.shields.io/npm/dt/rust-doctor.svg?style=flat&colorA=000000&colorB=000000)](https://npmjs.com/package/rust-doctor)

Your agent writes bad Rust, this catches it.

Rust Doctor scans your Cargo workspace with 62 curated rules and finds issues across security, correctness, reliability, performance, maintainability, and dependencies. It ends on a score out of 100 and the three rules worth fixing first.

Works on any Cargo workspace - a single binary crate, a library, a virtual workspace with dozens of members, you name it.

Everything runs locally: no network, no upload, no telemetry. Inspect trusted local workspaces only, since Cargo runs the build scripts and procedural macros of whatever it compiles. [Limits →](https://rust-doctor.com/docs/limitations)

[Website →](https://rust-doctor.com/docs)

## Install

### 1. Quick start

Run this at your workspace root to get an audit.

```bash
npx rust-doctor@latest
```

### 2. Hand off to agents

Once you have an audit, the report hands the findings to your coding agent, with the rules, their spans, and the scan scope already written into the prompt.

```bash
rust-doctor          # then pick "Hand off to an agent"
```

Works with Claude Code, Codex, and Cursor, and copies the same context to your clipboard for any other agent.

### 3. Run in CI

Rust Doctor reviews every pull request and reports only the issues your change introduced, not your existing backlog. Set it up from the report menu:

```bash
rust-doctor          # then pick "Add to GitHub Actions"
```

This writes `.github/workflows/rust-doctor.yml`, pinned to the version that wrote it, and never overwrites an existing file. The gate exits non-zero only when a diagnostic reaches the blocking level, which you change anytime with `--blocking`.

[CI docs →](https://rust-doctor.com/docs/ci-cd)

### 4. Configure rules

You can configure which rules to run and how to run them in `rust-doctor.toml`, or from the command line.

```bash
rust-doctor --rule clippy::unwrap_used=off
rust-doctor --category performance=error
```

[Learn more →](https://rust-doctor.com/docs/configuration)

## Privacy

The CLI reports nothing to anyone. No network call, no upload, no telemetry, no crash reporting, and nothing to opt out of. The binary carries no HTTP client and no analytics dependency.

A `--json` report stays inside your workspace and carries only:

- Diagnostics: rule id, category, severity before and after policy, and a workspace-relative path with its span
- Score: the number, its dimensions, the gate verdict, and the rules withheld from the ranking
- Errors: which pass failed and why, when one did

No absolute path, no environment variable, no user data.

[Privacy docs →](https://rust-doctor.com/docs/privacy)

## Contributing

[Issues welcome!](https://github.com/arthjean/rust-doctor/issues)

MIT OR Apache-2.0
