# AGENTS.md

> Source unique des conventions de ce dépôt pour tous les agents de code (Claude Code, Codex, Copilot, Cursor).
> `CLAUDE.md`, `.github/copilot-instructions.md` et `.cursor/rules/rust-doctor.mdc` pointent ici : modifier ce fichier, pas eux.

rust-doctor — Rust code health scanner. CLI binary, library crate, MCP server, npm package, GitHub Action.
Il scanne un projet pour les problèmes de sécurité, performance, correctness, architecture et dépendances, et produit un score de santé 0–100 avec des diagnostics actionnables.

**Edition:** Rust 2024, MSRV 1.85, single crate (not a workspace).

## Commands

```bash
cargo build                        # Debug build (with MCP server)
cargo build --no-default-features  # Build without MCP server
cargo test                         # All tests (unit + integration + snapshots)
cargo test test_name               # Single test
cargo test --test integration      # Integration tests only
cargo test --test snapshots        # Snapshot tests only
cargo insta review                 # Review snapshot changes (after test failures)
cargo clippy --all-targets -- -W clippy::all -W clippy::pedantic -W clippy::nursery -D warnings  # CI lint check
cargo fmt --check                  # Format check
```

`RUSTFLAGS="-Zproc-macro-backtrace-on-nightly-only"` n'est PAS nécessaire — la feature `span-locations` de `proc-macro2` gère le mapping source.

## Execution Pipeline

```
main.rs → --mcp flag? → mcp::run_mcp_server() (stdio transport, rmcp SDK)
        → otherwise  → discovery::bootstrap_project()  → config resolution
                      → scan::scan_project()            → orchestrator
                      → output::render_*()              → terminal/json/score/sarif
```

Scan pipeline (`scan.rs`) :

1. `resolve_scan_roots()` — workspace members or single project
2. `run_passes()` — parallel over scan roots (rayon), then parallel passes per root (`std::thread::scope`)
3. `dedup_diagnostics()` → `filter_to_changed_files()` (si `--diff`) → `apply_inline_suppressions()`
4. Score calculation → output

## Project Structure

```
src/
├── main.rs            # CLI entry point, --mcp flag dispatches to mcp::run_mcp_server()
├── lib.rs             # Module declarations + crate-root re-exports for passes
├── scan.rs            # Scan orchestrator: resolve roots → run passes → dedup → score
├── scanner.rs         # AnalysisPass trait + ScanOrchestrator
├── diagnostics.rs     # Diagnostic, ScanResult, Severity, Category — central types
├── config.rs          # Config resolution (CLI > TOML > Cargo.toml metadata > defaults)
├── output/            # Score calculation + terminal/JSON rendering
│   ├── mod.rs         # render_score(), render_json(), re-exports, tests
│   ├── score.rs       # calculate_score(), dimension weights, score_label()
│   └── terminal.rs    # render_terminal(), print_score_box(), print_diagnostics()
├── mcp/               # MCP server (rmcp v1.2.0, stdio, feature-gated)
│   ├── mod.rs         # Server struct, entry point, ServerHandler impl
│   ├── types.rs       # Input/output schemas (ScanInput, ScoreInput, etc.)
│   ├── tools.rs       # Tool + prompt handler implementations
│   ├── helpers.rs     # discover_and_resolve(), format_scan_report(), group_diagnostics()
│   └── rules.rs       # Rule documentation (explain_rule, list_rules)
├── passes/            # Analysis passes grouped by domain
│   ├── security/      # Security-focused passes
│   │   ├── audit.rs   # cargo-audit pass
│   │   ├── deny.rs    # cargo-deny pass
│   │   └── geiger.rs  # cargo-geiger pass
│   ├── static_analysis/  # Code analysis passes
│   │   ├── clippy/    # Clippy integration (55+ lint registry)
│   │   │   ├── mod.rs
│   │   │   └── lint_registry.rs
│   │   └── rules/     # Custom AST rules (syn::visit::Visit)
│   │       ├── mod.rs         # RulesPass + CustomRule trait + all_custom_rules()
│   │       ├── error_handling.rs  # unwrap, panic, box-dyn-error, result-unit-error
│   │       ├── performance.rs     # clone, string-from-literal, collect-iterate, large-enum, allocation
│   │       ├── complexity.rs      # high-cyclomatic-complexity
│   │       ├── security.rs        # hardcoded-secrets, unsafe-block-audit, sql-injection
│   │       ├── async_rules.rs     # blocking-in-async, block-on-in-async
│   │       └── framework.rs       # tokio-main, axum-handler, actix-blocking, tokio-spawn
│   └── quality/       # Quality & dependency passes
│       ├── coverage.rs
│       ├── msrv.rs
│       ├── machete.rs
│       └── semver_checks.rs
├── setup/             # `rust-doctor setup` — agent integration scaffolding
│   ├── mod.rs
│   ├── detect.rs      # Detect installed agents/editors
│   ├── mcp_config.rs  # MCP client config generation
│   ├── skill.rs       # Skill file generation
│   └── templates/
├── run.rs             # Helpers extracted from main.rs: MCP dispatch, bootstrap, render, quality gate
├── discovery.rs       # Project detection (frameworks, dependencies, workspace)
├── diff.rs            # Git diff filtering
├── cache.rs           # Incremental cache (.rust-doctor-cache.json)
├── suppression.rs     # Inline suppression (// rust-doctor-disable-next-line)
├── process.rs         # Subprocess runner with timeout
├── fixer.rs           # Auto-fix suggestions
├── plan.rs            # Remediation plan generation
├── sarif.rs           # SARIF output format
├── deps.rs            # Dependency analysis
├── workspace.rs       # Cargo workspace resolution
├── error.rs           # 7 thiserror error types
└── cli.rs             # clap CLI definition
tests/
├── integration.rs     # Temp Rust projects with known violations
└── snapshots.rs       # insta JSON snapshot tests
```

Note: `lib.rs` re-exports pass modules at the crate root (`pub(crate) use passes::security::audit`, etc.) so that `use crate::audit` paths work throughout the codebase.

**Module visibility.** Public API (`pub mod`) : `cli`, `config`, `deps`, `diagnostics`, `discovery`, `error`, `fixer`, `mcp`, `output`, `plan`, `run`, `sarif`, `scan`, `setup`. Interne (`pub(crate) mod`) : `passes` (re-exporté en `audit`, `clippy`, `rules`, etc.), `cache`, `diff`, `process`, `scanner`, `suppression`, `workspace`.

## Subsystem Notes

**Analysis passes** (`scanner.rs`) — toutes implémentent le trait `AnalysisPass` (`name()` + `run()` → `Vec<Diagnostic>`), exécutées en parallèle via `std::thread::scope`. `PassError::Skipped` couvre les outils externes non installés : émet un diagnostic Info au lieu d'échouer.

**Custom rules** (`src/passes/static_analysis/rules/`) — trait `CustomRule` dans `rules/mod.rs`, chaque règle parcourt l'AST via `syn::visit::Visit`. Collectées par `all_custom_rules()`. Chaque règle tourne dans un `catch_unwind` : une règle qui panique émet un warning sans crasher le scan. Les règles framework et async sont incluses conditionnellement selon `ProjectInfo.frameworks` (détecté depuis les dépendances au moment de la discovery).

**Score** (`output/score.rs`) — pondération sur 5 dimensions (Security ×2.0, Reliability ×1.5, Maintainability ×1.0, Performance ×1.0, Dependencies ×1.0). Compte les **règles uniques** violées, pas les occurrences. Clampé à [0, 100].

**Clippy** (`passes/static_analysis/clippy/`) — spawn `cargo clippy --message-format=json`, timeout 120s. 55+ lints dans le `LINT_REGISTRY` statique avec overrides de catégorie/sévérité. Les lints non listés héritent des défauts clippy et mappent sur `Category::Style`.

**MCP server** (`src/mcp/`, feature-gated) — 4 tools (scan, score, explain_rule, list_rules), tous read-only. Hardening : répertoire obligatoirement sous `$HOME`, timeout 5 minutes, mode offline par défaut, sanitisation des chemins dans les erreurs. rmcp v1.2.0 sur transport stdio.

**Configuration priority** — CLI flags > `rust-doctor.toml` > `[package.metadata.rust-doctor]` dans Cargo.toml > défauts. `--no-project-config` court-circuite la config fichier (utilisé par MCP sur projets non fiables).

**Output routing** — diagnostics vers **stderr**, boîte de score vers **stdout**. Intentionnel pour le piping (`--score` écrit un entier nu sur stdout).

## Code Style

- Clippy pedantic enabled (`must_use_candidate`, `module_name_repetitions`, `missing_errors_doc`, `missing_panics_doc` allowed)
- Custom errors with `thiserror::Error` — no `anyhow`, no `Box<dyn Error>` in library code
- `Result<T, E>` + `?` operator everywhere — `unwrap()` only in tests
- Two parallelism levels: rayon for scan roots, `std::thread::scope` for passes within a root
- `PassError::Skipped` for missing external tools — graceful degradation, not failure
- `proc-macro2` avec la feature `span-locations` fournit ligne/colonne depuis les nœuds AST `syn`
- Cache incrémental (`.rust-doctor-cache.json`) clé par hash de config + hash de contenu des fichiers
- Profil release en `opt-level = "z"` (optimisé taille) pour la distribution du binaire npm

## Testing

- Unit tests: inline `#[cfg(test)]` modules in each source file
- Integration tests: temp Rust projects via `tempfile::TempDir`, `fast_config()` skips external passes
- Snapshot tests: `insta` JSON snapshots for serialization stability
- After changing snapshots: `cargo insta review`
- Self-scan tests: several modules scan rust-doctor itself as sanity check

## Architecture Rules

### Always
- New analysis passes implement `AnalysisPass` trait (`name()` + `run()` → `Vec<Diagnostic>`)
- New custom rules implement `CustomRule` trait in the appropriate `passes/static_analysis/rules/` submodule
- Diagnostics go to stderr, score to stdout (intentional for piping)
- MCP tools are read-only — no filesystem writes, directory under `$HOME` only
- Run `catch_unwind` around custom rules — a panicking rule must not crash the scan

### Ask First
- Changing score weights (Security ×2.0, Reliability ×1.5, Maintainability ×1.0, Performance ×1.0, Dependencies ×1.0)
- Adding new MCP tools or modifying security hardening
- Changing module visibility (pub vs pub(crate))

### Never
- Use `anyhow` — this project uses typed errors with `thiserror`
- Add `unsafe` blocks in production code
- Skip `catch_unwind` on custom rules
- Break the stderr/stdout output routing convention

## Anti-Friction Rules (claude-doctor)

Règles pour éviter les patterns de friction détectés par `claude-doctor` sur ce projet : edit-thrashing, restart-cluster, repeated-instructions, negative-drift, error-loop, excessive-exploration.

### Editing discipline (anti edit-thrashing)

- Read the full file before editing. Plan all changes, then make ONE complete edit.
- If you've edited the same file 3+ times, STOP. Re-read the user's original requirements and re-plan from scratch.
- Prefer one large coherent edit over multiple small incremental ones.

### Stay aligned with the user (anti repeated-instructions, rapid-corrections)

- Re-read the user's last message before responding. Follow through on every instruction completely — don't partially address requests.
- Every few turns on a long task, re-read the original request to verify you haven't drifted from the goal.
- When the user corrects you: stop, re-read their message, quote back what they actually asked for, and confirm understanding before proceeding.

### Act, don't explore (anti excessive-exploration)

- Don't read more than 3-5 files before making a change. Get a basic understanding, make the change, then iterate.
- Prefer acting early and correcting via feedback over prolonged reading and planning.

### Break loops (anti error-loop, restart-cluster)

- After 2 consecutive tool failures or the same error twice, STOP. Change your approach entirely — don't retry the same strategy. Explain what failed and try something genuinely different.
- When truly stuck, summarize what you've tried and ask the user for guidance rather than retrying.

### Verify output (anti negative-drift)

- Before presenting your result, double-check it actually addresses what the user asked for.
- If the diff doesn't map cleanly to the user's request, don't ship it — re-plan.

## Compaction Guidance

Lors d'une compaction de contexte, toujours préserver : la liste des fichiers modifiés, les commandes de test en cours, les types d'erreur ou impls de trait en cours de modification, et l'étape du scan pipeline sur laquelle porte le travail.
