# Security Policy

## Supported versions

rust-doctor is distributed as a CLI binary, a library crate, an MCP server, an
npm package, and a GitHub Action. Security fixes are released against the latest
published version on [crates.io](https://crates.io/crates/rust-doctor) and
[npm](https://www.npmjs.com/package/rust-doctor). Please upgrade before
reporting an issue.

| Version | Supported |
| ------- | --------- |
| latest  | ✅        |
| older   | ❌        |

## Reporting a vulnerability

**Do not open a public issue for security problems.**

Report privately via one of:

- GitHub's [private vulnerability reporting](https://github.com/arthjean/rust-doctor/security/advisories/new)
- Email: **arthur.jean@strivex.fr**

Please include: affected version, reproduction steps, and the impact you
observed. You will get an acknowledgement within **72 hours** and a remediation
timeline after triage. Coordinated disclosure is appreciated — we will credit
you in the release notes unless you prefer otherwise.

## Threat model & scope

rust-doctor reads and analyzes **untrusted source code**, and the MCP server can
be pointed at **untrusted projects**. In-scope concerns:

- **Path traversal / arbitrary file writes** — the `--fix` writer and the MCP
  server must stay within the target project root.
- **Command/argument injection** — analysis passes spawn external `cargo`
  subcommands; crafted project metadata must not inject arguments.
- **MCP server hardening** — the scanned directory must resolve under `$HOME`,
  runs offline by default, enforces a 5-minute timeout, and sanitizes paths in
  error messages.
- **Resource exhaustion** — subprocess output is bounded and passes time out.

Out of scope: vulnerabilities in the external tools rust-doctor invokes
(`cargo-audit`, `cargo-deny`, `cargo-geiger`, etc.) — report those upstream.
