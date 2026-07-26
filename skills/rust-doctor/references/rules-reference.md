# Rules Reference

Complete reference for all rust-doctor diagnostics with fix strategies.

## Custom AST Rules (19 rules)

### Error Handling

#### `unwrap-in-production` (Warning)
**Detects:** `.unwrap()` / `.expect()` outside test code (including `#[cfg(test)]` modules).
**Why:** Panics in production crash the process. In servers, this means dropped connections and potential data loss.
**Fix:**
```rust
// Before
let value = config.get("key").unwrap();

// After
let value = config.get("key")
    .context("missing 'key' in config")?;
```
Use `?` with `anyhow::Context` or pattern match. Reserve `unwrap()` for tests and proven-impossible `None`/`Err` cases with a comment.

---

#### `panic-in-library` (Error)
**Detects:** `panic!()`, `todo!()`, `unimplemented!()` in library files (not `main.rs`/`bin/`).
**Why:** Libraries should return errors, not crash the caller. `todo!()` left in published code is a time bomb.
**Fix:**
```rust
// Before
fn parse(input: &str) -> Config {
    todo!()
}

// After
fn parse(input: &str) -> Result<Config, ParseError> {
    Err(ParseError::NotImplemented)
}
```

---

#### `box-dyn-error-in-public-api` (Warning)
**Detects:** `pub fn` returning `Result<_, Box<dyn Error>>`.
**Why:** Callers can't match on specific error variants. Makes error handling fragile.
**Fix:** Define a custom error enum with `thiserror`, or use `anyhow::Error` for application code.

---

#### `result-unit-error` (Warning)
**Detects:** `pub fn` returning `Result<_, ()>`.
**Why:** The unit error `()` carries no information about what went wrong.
**Fix:** Replace with a meaningful error type: `Result<_, MyError>` or at minimum `Result<_, String>`.

---

### Performance

#### `excessive-clone` (Warning / Info)
**Detects:** 3+ `.clone()` calls in a single file (threshold configurable via `rules_config`). Per-diagnostic severity: Warning when clone is inside a loop, Info otherwise. The rule's metadata severity is Warning.
**Why:** Excessive cloning often masks ownership issues and creates unnecessary heap allocations.
**Fix:** Restructure ownership — use references, `Cow<'_, T>`, or `Arc<T>` for shared data. In loops, clone outside the loop if the value doesn't change per iteration.

---

#### `string-from-literal` (Info, opt-in)
**Detects:** `String::from("literal")` / `"literal".to_string()` where `&str` would suffice.
**Why:** Unnecessary heap allocation for string literals that are already `&'static str`.
**Fix:**
```rust
// Before
let name = String::from("default");

// After
let name = "default"; // if &str is acceptable
// or
let name: Cow<'_, str> = "default".into(); // if sometimes owned
```
**Note:** Disabled by default. Enable with `enable = ["string-from-literal"]` in config.

---

#### `collect-then-iterate` (Warning)
**Detects:** `.collect::<Vec<_>>().iter()` — collecting into a Vec only to iterate over it.
**Why:** Creates a transient heap allocation with zero benefit. The iterator chain can continue directly.
**Fix:**
```rust
// Before
let results: Vec<_> = items.iter().map(process).collect();
for r in results.iter() { ... }

// After
for r in items.iter().map(process) { ... }
```

---

#### `large-enum-variant` (Warning)
**Detects:** Enum variants with >3x field count disparity.
**Why:** The enum's size equals its largest variant. One bloated variant wastes memory for every instance of the enum.
**Fix:** Box the large variant's payload:
```rust
// Before
enum Message {
    Ping,
    Data(Vec<u8>, HashMap<String, String>, Metadata),
}

// After
enum Message {
    Ping,
    Data(Box<DataPayload>),
}
```

---

#### `unnecessary-allocation` (Warning)
**Detects:** `Vec::new()` / `String::new()` inside loops.
**Why:** Allocates on every iteration. Move the allocation outside the loop and clear/reuse it.
**Fix:**
```rust
// Before
for item in items {
    let mut buf = Vec::new();
    process(item, &mut buf);
}

// After
let mut buf = Vec::new();
for item in items {
    buf.clear();
    process(item, &mut buf);
}
```

---

### Architecture / Maintainability

#### `high-cyclomatic-complexity` (Warning)
**Category:** Architecture (maps to Maintainability dimension in score calculation).
**Detects:** Functions with cyclomatic complexity > 15.
**Why:** Complex functions are hard to test, understand, and maintain. High complexity correlates with bug density.
**Fix:** Extract sub-functions, use early returns, replace nested `if/else` chains with `match` or guard clauses.

---

### Security

#### `hardcoded-secrets` (Error)
**Detects:** String literals (len > 8) assigned to variables matching `api_key`, `password`, `token`, `secret`, `credential`, etc. Checks `let`, `const`, `static`, field assignment.
**Why:** Secrets in source code end up in version control, logs, and compiled binaries.
**Fix:** Use environment variables or a secrets manager:
```rust
// Before
const API_KEY: &str = "sk-live-abc123def456";

// After
let api_key = std::env::var("API_KEY")
    .context("API_KEY env var not set")?;
```

---

#### `unsafe-block-audit` (Warning)
**Detects:** `unsafe {}` blocks and `unsafe fn`. Respects `#![forbid(unsafe_code)]` crate-level attribute.
**Why:** Unsafe code bypasses Rust's safety guarantees. Each block needs a `// SAFETY:` comment documenting invariants.
**Fix:** Add a safety comment, or eliminate the unsafe if a safe alternative exists:
```rust
// SAFETY: pointer is valid and aligned because we allocated it
// with Vec::as_mut_ptr() and the Vec is alive for this scope.
unsafe { *ptr = value; }
```

---

#### `sql-injection-risk` (Error)
**Detects:** `format!()` passed to `.query()`, `.execute()`, `.raw()`, `.query_as()`, `.execute_raw()`.
**Why:** String interpolation in SQL queries enables SQL injection attacks.
**Fix:** Use parameterized queries:
```rust
// Before
sqlx::query(&format!("SELECT * FROM users WHERE id = {}", user_id))

// After
sqlx::query("SELECT * FROM users WHERE id = $1")
    .bind(user_id)
```

---

### Async

#### `blocking-in-async` (Error)
**Detects:** `std::thread::sleep`, `std::fs::*`, `std::net::*` inside `async fn` (skips `spawn_blocking` context).
**Why:** Blocking calls starve the async runtime, degrading throughput for all tasks on that thread.
**Fix:** Use async equivalents:
```rust
// Before
async fn read_config() -> String {
    std::fs::read_to_string("config.toml").unwrap()
}

// After
async fn read_config() -> String {
    tokio::fs::read_to_string("config.toml").await.unwrap()
}
```

---

#### `block-on-in-async` (Error)
**Detects:** `.block_on()` / `block_on()` called inside `async fn`.
**Why:** Calling `block_on` from within an async context deadlocks the executor.
**Fix:** Use `.await` instead of `block_on()`. If calling sync code, use `spawn_blocking`.

---

### Framework (conditionally active)

#### `tokio-main-missing` (Error)
**Active when:** tokio/async-std/smol detected in dependencies.
**Detects:** `async fn main()` without `#[tokio::main]` (or equivalent runtime attribute) in `main.rs`.
**Fix:** Add `#[tokio::main]` above the main function.

---

#### `tokio-spawn-without-move` (Error)
**Active when:** tokio detected in dependencies.
**Detects:** `tokio::spawn(async { ... })` without `move` keyword.
**Why:** Without `move`, the spawned task borrows from the parent scope — but `tokio::spawn` requires `'static`.
**Fix:** Add `move`: `tokio::spawn(async move { ... })`.

---

#### `axum-handler-not-async` (Warning)
**Active when:** axum detected in dependencies.
**Detects:** Non-async functions using axum extractor types (Json, Path, Query, State, Extension, Form, Header).
**Fix:** Make the handler `async fn`.

---

#### `actix-blocking-handler` (Warning)
**Active when:** actix-web detected in dependencies.
**Detects:** Blocking std calls inside actix-web handler functions.
**Fix:** Use `actix_web::web::block()` for CPU-bound work, or async alternatives for I/O.

---

## Clippy Integration (55+ lints)

rust-doctor runs `cargo clippy --message-format=json` with custom severity/category mappings for 55+ lints. Key lint groups:

**Error Handling:** `unwrap_used`, `expect_used`, `panic`, `indexing_slicing`, `unwrap_in_result`, `panic_in_result_fn`, `exit`

**Performance:** `box_collection`, `clone_on_copy`, `redundant_clone`, `needless_collect`, `large_enum_variant`, `inefficient_to_string`, `unnecessary_to_owned`, `large_stack_arrays`, `large_futures`, `useless_vec`

**Security:** `undocumented_unsafe_blocks`, `multiple_unsafe_ops_per_block`, `transmute_ptr_to_ref`, `cast_ptr_alignment`, `fn_to_numeric_cast`, `mem_forget`

Unlisted lints inherit clippy's default severity and map to the Style category.

## External Tool Passes

| Pass | What it checks | Install |
|------|---------------|---------|
| cargo-audit | CVEs in dependencies (RustSec Advisory DB) | `cargo install cargo-audit` |
| cargo-deny | License compliance, duplicate crates, advisory DB, banned crates | `cargo install cargo-deny` |
| cargo-shear | Unused dependencies in Cargo.toml | `cargo install cargo-shear` |
| cargo-geiger | Unsafe code usage across dependency graph | `cargo install cargo-geiger` |
| cargo-semver-checks | Semver violations between releases | `cargo install cargo-semver-checks` |
| coverage | Test coverage analysis | (built-in) |
| MSRV check | Validates declared `rust-version` in Cargo.toml | (built-in) |

Missing tools are skipped with an Info diagnostic — they don't fail the scan.
