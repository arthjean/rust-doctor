# Score Interpretation

## How the Score is Calculated

rust-doctor computes a 0-100 health score using **dimension-based weighted scoring** across 5 dimensions.

### Dimensions and Weights

| Dimension | Weight | What feeds into it |
|-----------|--------|--------------------|
| **Security** | 2.0x | `hardcoded-secrets`, `unsafe-block-audit`, `sql-injection-risk`, clippy security lints, cargo-audit CVEs, cargo-geiger unsafe counts |
| **Reliability** | 1.5x | `unwrap-in-production`, `panic-in-library`, `blocking-in-async`, `block-on-in-async`, `tokio-main-missing`, `tokio-spawn-without-move`, framework rules, clippy correctness lints |
| **Maintainability** | 1.0x | `high-cyclomatic-complexity`, `box-dyn-error-in-public-api`, `result-unit-error`, clippy architecture/style lints, cargo-shear unused deps |
| **Performance** | 1.0x | `excessive-clone`, `collect-then-iterate`, `large-enum-variant`, `unnecessary-allocation`, `string-from-literal`, clippy performance lints |
| **Dependencies** | 1.0x | cargo-audit advisories, cargo-deny findings, cargo-semver-checks violations, MSRV compliance |

### Scoring Algorithm

1. Per dimension, count **unique rule names** violated (not occurrence count):
   - Error severity: **-1.5 pts** per unique rule
   - Warning severity: **-0.75 pts** per unique rule
   - Info severity: **-0.25 pts** per unique rule

2. Each dimension starts at 100 and subtracts penalties: `dimension_score = clamp(100 - penalty, 0, 100)`

3. Final score is a weighted average:
```
score = round((Security×2.0 + Reliability×1.5 + Maintainability×1.0 + Performance×1.0 + Dependencies×1.0) / 6.5)
```

### Key Design Choice

Counts **unique rules violated**, NOT total occurrences. A file with 50 `.unwrap()` calls counts the same as one with 1 for scoring purposes. This rewards breadth of compliance over fixing a single rule repeatedly.

## Score Ranges

| Score | Label | Meaning |
|-------|-------|---------|
| 75-100 | **Great** | Project follows Rust best practices well. Minor issues only. |
| 50-74 | **Needs work** | Notable issues in one or more dimensions. Address HIGH/CRITICAL findings. |
| 0-49 | **Critical** | Significant health issues. Security or correctness problems likely present. Prioritize P0/P1 fixes. |

## Interpreting Dimension Scores

### Security Score
- **90-100:** No known vulnerabilities, minimal unsafe, no hardcoded secrets
- **70-89:** Minor unsafe usage or informational advisories
- **50-69:** Active CVEs in dependencies or unsafe blocks without safety comments
- **Below 50:** Hardcoded secrets, SQL injection risks, or critical dependency vulnerabilities

### Reliability Score
- **90-100:** Proper error handling, no panics in libraries, correct async patterns
- **70-89:** Some `.unwrap()` usage outside tests, minor correctness lints
- **50-69:** Blocking calls in async, missing runtime attributes, panic-prone library code
- **Below 50:** Multiple correctness errors — high crash risk in production

### Performance Score
- **90-100:** Efficient ownership patterns, no unnecessary allocations
- **70-89:** Some clone usage, minor allocation patterns
- **50-69:** Clone abuse, collect-then-iterate, large enum variants
- **Below 50:** Systematic performance anti-patterns

### Maintainability Score
- **90-100:** Clean API surfaces, low complexity, no unused dependencies
- **70-89:** Some style issues, minor complexity
- **50-69:** High complexity functions, poor error types in public API
- **Below 50:** Significant architectural issues

### Dependencies Score
- **90-100:** All deps up to date, no advisories, MSRV declared and correct
- **70-89:** Minor version gaps, informational advisories
- **50-69:** Outdated deps with known issues, missing MSRV
- **Below 50:** Active CVEs, license violations, semver breaks

## Skipped Passes

When external tools are not installed, their passes are skipped with an Info diagnostic. This means the corresponding dimension may appear healthier than reality. The scan result reports which passes were skipped.

To install all external tools:
```bash
rust-doctor --install-deps
```

## MCP Server Equivalence

The rust-doctor MCP server exposes the same analysis as the CLI. The `scan` tool accepts:
- `directory` (required), `diff` (optional base branch), `offline` (default: true), `ignore_project_config` (default: false)

The `score` tool accepts: `directory`, `offline`.

The MCP server enforces a **5-minute (300s) timeout** and restricts scans to directories under `$HOME`.
