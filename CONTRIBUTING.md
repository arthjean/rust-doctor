# Contributing to rust-doctor

Thank you for your interest in contributing to rust-doctor!

## Prerequisites

- Rust toolchain (MSRV: 1.85, edition 2024)
- `rustup component add clippy rustfmt`
- Optional: `cargo install cargo-audit cargo-deny` for supply-chain checks

## Setup

```bash
git clone https://github.com/arthjean/rust-doctor.git
cd rust-doctor
git config core.hooksPath .githooks
cargo build
cargo test
```

The `git config core.hooksPath .githooks` command enables pre-commit hooks that run `cargo fmt --check` and `cargo clippy` before each commit.

## Running Tests

```bash
cargo test                         # All tests (unit + integration + snapshots)
cargo test test_name               # Single test
cargo test --test integration      # Integration tests only
cargo test --test snapshots        # Snapshot tests only
cargo insta review                 # Review snapshot changes
```

## Code Style

All code must pass the CI lint check:

```bash
cargo clippy --all-targets -- -W clippy::all -W clippy::pedantic -W clippy::nursery -D warnings
cargo fmt --check
```

Key conventions:
- `#![forbid(unsafe_code)]` is non-negotiable
- `thiserror` for library errors, `anyhow` not used
- Parallelism: rayon for scan roots, `std::thread::scope` for passes

## How to Add a New Rule

1. **Create the rule** in the appropriate submodule under `src/passes/static_analysis/rules/`:
   - `error_handling.rs` for error-handling patterns
   - `performance.rs` for performance issues
   - `complexity.rs` for complexity checks
   - `security.rs` for security findings
   - `async_rules.rs` for async-specific issues
   - `framework.rs` for framework-specific rules

2. **Implement `CustomRule`** trait:
   ```rust
   impl CustomRule for YourRule {
       fn name(&self) -> &'static str { "your-rule-name" }
       fn category(&self) -> Category { Category::Performance }
       fn severity(&self) -> Severity { Severity::Warning }
       fn description(&self) -> &'static str { "What this rule detects" }
       fn fix_hint(&self) -> &'static str { "How to fix it" }
       fn check_file(&self, syntax: &syn::File, path: &Path) -> Vec<Diagnostic> {
           // Use syn::visit::Visit to walk the AST
           todo!()
       }
   }
   ```

3. **Register it** in the module's `all_rules()` function.

4. **Add an integration test** in `tests/integration.rs` with a `TempDir` containing a minimal Rust file that triggers the rule.

## PR Process

1. Fork and create a branch from `master`
2. Make your changes
3. Ensure all quality gates pass:
   ```bash
   cargo check
   cargo clippy --all-targets -- -W clippy::all -W clippy::pedantic -W clippy::nursery -D warnings
   cargo fmt --check
   cargo test
   ```
4. Open a PR with a clear title and description
5. Use imperative mood in commit messages ("Add feature" not "Added feature")

## License

By contributing, you agree that your contributions will be licensed under the same terms as the project (MIT OR Apache-2.0).
