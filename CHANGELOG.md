# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.1.20] - 2026-03-25

### Added
- `deny.toml` configuration for supply-chain checking with `cargo-deny`
- `CONTRIBUTING.md` with setup, code style, and "how to add a rule" guide
- `CHANGELOG.md` in Keep a Changelog format
- Pre-commit hooks (`.githooks/pre-commit`) for `cargo fmt` and `cargo clippy`
- `cargo deny check` step in CI workflow
- Tests for `output/terminal.rs`, `main.rs` gate functions, and 6 additional custom rules in integration tests
- Extracted MCP `ServerHandler` impl into dedicated `mcp/handler.rs` module

### Changed
- Deduplicated 6 `is_*_available()` functions into shared `process::is_cargo_subcommand_available()`
- Memoized file reads in clippy filter to avoid re-reading per diagnostic
- Eliminated double hashing in cache (`is_fresh_with_hash` computes hash once)
- Decomposed `print_score_box` into `render_header`, `render_dimension_bars`, `render_stats_footer`
- Added nursery lints to `Cargo.toml` lint configuration
- Hardened `.gitignore` with secrets/credential patterns
- Hardened `deny.toml`: `yanked = "deny"`, BSD-2/3-Clause in license allow-list

### Fixed
- Redundant file I/O in `filter_test_and_binary_lints` for restriction-group lints
- Pre-commit hook now shows error output instead of suppressing stderr

## [0.1.19] - 2026-03-24

### Fixed
- Resolved 3 warnings found by rust-doctor self-scan

## [0.1.18] - 2026-03-24

### Added
- Senior Rust reviewer skill with expert context in setup wizard

## [0.1.17] - 2026-03-24

### Changed
- Rewrote skill template for deep three-pass analysis

## [0.1.16] - 2026-03-23

### Fixed
- Prompt before overwriting existing skills or MCP config
- Default to CLI + Skills, improved agent selection UX

## [0.1.15] - 2026-03-23

### Added
- Interactive setup wizard for AI agent integration (`rust-doctor setup`)

### Fixed
- Score formula, lint count, and library example corrections in README and website

[unreleased]: https://github.com/arthjean/rust-doctor/compare/v0.1.20...HEAD
[0.1.20]: https://github.com/arthjean/rust-doctor/compare/v0.1.19...v0.1.20
[0.1.19]: https://github.com/arthjean/rust-doctor/compare/v0.1.18...v0.1.19
[0.1.18]: https://github.com/arthjean/rust-doctor/compare/v0.1.17...v0.1.18
[0.1.17]: https://github.com/arthjean/rust-doctor/compare/v0.1.16...v0.1.17
[0.1.16]: https://github.com/arthjean/rust-doctor/compare/v0.1.15...v0.1.16
[0.1.15]: https://github.com/arthjean/rust-doctor/releases/tag/v0.1.15
