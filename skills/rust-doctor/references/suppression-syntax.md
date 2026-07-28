# Suppression Syntax

## Inline Suppressions

rust-doctor supports two comment forms to suppress specific diagnostics:

### Suppress a specific rule on the next line

```rust
// rust-doctor-disable-next-line unwrap-in-production
let value = config.get("key").unwrap(); // this unwrap is intentional — key always exists after validation
```

### Suppress all rules on the current line

Works both as an inline end-of-line comment and as a standalone full-line comment:

```rust
// Inline form (end of line):
let value = config.get("key").unwrap(); // rust-doctor-disable-line

// Standalone form (full line comment, suppresses the same line):
// rust-doctor-disable-line
let value = config.get("key").unwrap();
```

## Pass Control

Enable or disable entire analysis categories:

```toml
lint = true            # enable/disable clippy + custom rules pass (default: true)
dependencies = true    # enable/disable audit/deny/shear/geiger passes (default: true)
```

## Configuration-Level Suppression

### Ignore specific rules globally

In `rust-doctor.toml`:
```toml
[ignore]
rules = ["unwrap-in-production", "clippy::too_many_lines"]
```

Or in `Cargo.toml`:
```toml
[package.metadata.rust-doctor.ignore]
rules = ["unwrap-in-production"]
```

### Ignore specific files/directories

```toml
[ignore]
files = ["**/generated/**", "tests/**", "benches/**"]
```

Glob patterns are supported.

### Enable opt-in rules

Some rules are disabled by default (e.g., `string-from-literal`):
```toml
[ignore]
enable = ["string-from-literal"]
```

### Per-rule configuration

```toml
[rules_config.excessive-clone]
threshold = 5        # custom threshold (default: 3)
severity = "error"   # override severity
enabled = false      # disable this rule
```

## Score Threshold

Set a minimum passing score:
```toml
[score]
fail_below = 80
```

The CLI exits with code 1 if the score falls below this threshold.

## Fail-On Severity

Exit with code 1 when any diagnostic at or above a severity threshold is found:

```bash
rust-doctor --fail-on warning    # fail on warnings and errors
rust-doctor --fail-on error      # fail on errors only
rust-doctor --fail-on none       # never fail on diagnostics (default)
```

In config:
```toml
fail_on = "warning"
```

## When to Suppress

**Good reasons to suppress:**
- False positive (the rule doesn't apply to this specific case)
- Intentional pattern (e.g., `unwrap()` after a guard that guarantees `Some`)
- Generated code you don't control
- Test code where panics are acceptable

**Bad reasons to suppress:**
- "I'll fix it later" — use the `--plan` flag instead for tracking
- "The score is good enough" — suppressed issues are still issues
- Blanket suppression of entire categories

Always add a comment explaining WHY the suppression exists.
