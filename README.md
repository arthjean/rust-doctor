<img alt="Rust Doctor" src="./assets/rust-doctor-mark.png" width="52" height="36">

[![version](https://img.shields.io/npm/v/rust-doctor?style=flat&colorA=000000&colorB=000000)](https://npmjs.com/package/rust-doctor)
[![downloads](https://img.shields.io/npm/dt/rust-doctor.svg?style=flat&colorA=000000&colorB=000000)](https://npmjs.com/package/rust-doctor)

Your agent writes bad Rust, this catches it.

Rust Doctor scans your Cargo workspace with 142 curated rules and finds issues across security, correctness, reliability, performance, maintainability, and dependencies. It ends on a score out of 100 and the three rules worth fixing first.

Works on any Cargo workspace - a single binary crate, a library, a virtual workspace with dozens of members, you name it.

Everything runs locally: rust-doctor opens no network connection, uploads nothing and sends no telemetry. Cargo downloads missing dependencies as `cargo build` does, and `CARGO_NET_OFFLINE=true` forbids it. Inspect trusted local workspaces only, since Cargo runs the build scripts and procedural macros of whatever it compiles. [Limits →](https://rust-doctor.com/docs/limitations)

[Website →](https://rust-doctor.com/docs)

## Install

### 1. Quick start

Run this at your workspace root to get an audit.

```bash
npx rust-doctor@latest
```

### 2. Install for agents

Once you have an audit, set up the loop so your coding agent fixes what it found. The skill teaches it which finding to repair first, how the score is capped, and what to look for in a file the scan already flagged; the hooks rescan each commit and each agent turn.

```bash
npx rust-doctor@latest setup
```

This detects Claude Code, Codex and Cursor from their commands on `PATH` and their project directories, then writes the skill for each, the git pre-commit hook, and the end-of-turn hook of Claude Code or Cursor, from the binary rather than the network. It prints the plan and checks every file before writing any (`--dry-run` stops there), never writes over a file it did not create, and merges the end-of-turn hook into an existing agent settings file, keeping every other key. `skill install` installs the skill alone.

The pre-commit hook scans what the commit records, in `--scope lines`, and refuses a commit that adds a warning, the level the end-of-turn hook blocks on too. `git commit --no-verify` skips it, and a scan that cannot run lets the commit through, saying it was not judged.

For a one-off handoff instead, the report sends the findings straight to an agent, with the rules, their spans, and the scan scope already written into the prompt.

```bash
rust-doctor          # then pick "Hand off to an agent"
```

Works with Claude Code, Codex, and Cursor, and copies the same context to your clipboard for any other agent.

### 3. Run in CI

Rust Doctor reviews every pull request and reports only the issues your change introduced, not your existing backlog. Set it up with one command, or from the report menu:

```bash
rust-doctor ci install --comment
```

This writes `.github/workflows/rust-doctor.yml` at the top of your git repository, scanning the Cargo workspace below it when there is one (`src-tauri/`, `backend/`), pinned to the version that wrote it and to the toolchain it was validated on, and never overwrites a file it did not write (`--update` rewrites its own). Pull requests fail only on what they introduce, at the level `--blocking` sets; pushes to your default branch report without failing. `--comment` also posts the result as one sticky comment per pull request, edited on each push, from a second job that holds `pull-requests: write` and runs none of the pull request's code; a pull request from a fork gets a read-only token, and a warning instead.

To compose your own workflow, use the Action, `uses: arthjean/rust-doctor@v0`, with the outputs `score`, `authoritative`, `introduced`, `fixed` and `exit-code`. Grant its job `contents: read` only: it builds the pull request's code, build scripts included. The sticky comment comes from `arthjean/rust-doctor/comment@v0` in a second job, the only one holding `pull-requests: write`, which checks nothing out and reads the report the scan uploaded (`comment-on-clean: false` skips it on a clean pull request):

```yaml
permissions:
  contents: read

jobs:
  scan:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v5
        with:
          fetch-depth: 0
          persist-credentials: false
      - uses: arthjean/rust-doctor@v0

  comment:
    needs: scan
    if: always() && github.event_name == 'pull_request'
    runs-on: ubuntu-latest
    permissions:
      pull-requests: write
    steps:
      - uses: arthjean/rust-doctor/comment@v0
```

The Action fails only a pull request on its findings: a push reports without failing. Check out with `fetch-depth: 0`, since a pull request scan needs the base's history; from a shallow checkout the Action fetches the base itself when the token allows it.

The binary still opens no network connection: the comment is posted by the workflow's own GitHub token.

[CI docs →](https://rust-doctor.com/docs/ci-cd)

### 4. Configure rules

You can configure which rules to run and how to run them in `rust-doctor.toml`, or from the command line.

```bash
rust-doctor --rule clippy::unwrap_used=off
rust-doctor --category performance=error
```

[Learn more →](https://rust-doctor.com/docs/configuration)

## Privacy

The CLI reports nothing to anyone. No network connection of its own, no upload, no telemetry, no crash reporting, and nothing to opt out of. The binary carries no HTTP client and no analytics dependency.

A `--json` report stays inside your workspace and carries only:

- Diagnostics: rule id, category, severity before and after policy, and a workspace-relative path with its span
- Score: the number, its dimensions, the gate verdict, and the rules withheld from the ranking
- Errors: which pass failed and why, when one did

No absolute path, no environment variable, no user data.

[Privacy docs →](https://rust-doctor.com/docs/privacy)

## Contributing

[Issues welcome!](https://github.com/arthjean/rust-doctor/issues)

## License

MIT OR Apache-2.0, at your option. See [LICENSE-MIT](./LICENSE-MIT) and
[LICENSE-APACHE](./LICENSE-APACHE).
