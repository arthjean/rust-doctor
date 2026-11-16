# Release v0.10.0

## Facts
- Version: 0.10.0 (minor: score model core-v5 -> core-v6, schema 23 -> 24, two rules opt-in)
- Previous tag: v0.9.0 (bad7c10)
- Release base: 7991b7a (origin/main, clean checkout, no local commits ahead)
- Checkout: C:\dev\rust-doctor (clean, no worktree needed)
- Probe at preflight: six npm packages absent, crates.io 404, release absent

## Preflight CI on 7991b7a
- CI 42133373785: failure
  - Tests (macos-latest): product_proof::repository_json_has_exact_top_level_contract_without_absolute_private_paths, structure time-budget on a CLI self-scan that ad28912 did not cover. Fix: raise RUST_DOCTOR_STRUCTURE_TIME_BUDGET_SECS in that test.
  - Node launcher: launcher.test.mjs "forwards argv..." timed out at 5000 ms (passes in ~200 ms elsewhere; first failure of that job in the last 40 CI runs). No source change touched it in 7991b7a (comments only). Watching the fix push for recurrence.
- Rust Doctor 42133373801: success

## Steps
- [x] 1. Preflight: CI 42481589067 success + Rust Doctor 42481588874 success on 687b1b8 (fix for the macOS self-scan clock; Node launcher green on rerun, single flake). Probe re-run after restart: unburned.
- [x] 2. All slices accepted (union = range). Note .github/releases/v0.10.0.md written after invoking the write-release-notes skill (args v0.9.0..HEAD) in this session; every user-facing entry mapped; breaking: true on a minor bump (0.x).
- [x] 3. Bump ff91589 (on top of b6ed111) = origin/main; CI 42483471640 success, Rust Doctor 42483471754 success. Local gates: doc budgets, doc refs, docs_contract 16/16, the_action_declares... 1/1, skill_contract 8/8.
- [x] 4. Candidate 42484461103 (workflow_dispatch, headSha ff91589) success, summary "## Release candidate". binstall job skipped by design (tag only). Warnings: Node 20 deprecation, setup-bun cache save/restore failed (GitHub cache outage): noise.
- [x] 5. Tag v0.10.0 (lightweight) -> ff91589, confirmed by ls-remote.
- [x] 6. Tag run 42484823289 (push, headBranch v0.10.0, ff91589) success, 13/13 jobs executed incl. cargo-binstall proof. Warnings read: Node 20 deprecation, setup-bun cache (GitHub cache outage): noise.
- [x] 7. Six npm packages at 0.10.0 (darwin-x64/arm64 lagged a few minutes: "being processed"), crates.io 200, npm latest 0.10.0, crates max_stable 0.10.0, release published+immutable+not prerelease with the 5 archives pack --targets implies, releases/latest v0.10.0, gh release verify OK, verify-asset OK on the win32 archive, body == note (trailing newline aside), npx --yes rust-doctor@0.10.0 --version -> "rust-doctor 0.10.0" from an empty dir, Install 42485284254 success.
- [x] 8. v0 -> ff91589 (was bad7c10), ls-remote confirmed; its run 42485751754 cancelled.
- [x] 9. Site 5d2c17f on origin/main, Vercel success (https://vercel.com/strive-x/rust-doctor/6ndSfoKwp43J9NLgcxMqq1QMT5G5), Public contract push run 42486335800 success; prod softwareVersion 0.10.0, provenance commit ff91589, changelog count 8, core-v5 share shows retired.
- [x] 10. Worktrees removed, site branch deleted local+remote, report given.

## Ledger slices
- A (7fc9445 d6060ed 2ffda95 2a835ce 1815242 50975a4 7272520 8ddbf58): ACCEPTED. Checked action.yml/ci.rs 1.97.1->1.99.1, containment.rs pass-panicked + messages, clippy-thread-failed gone at HEAD, advisories.rs discover tie rules.
- B (b24e4b2 ff8d4ee 750fbed 329fbca): ACCEPTED. Checked scope.rs:157-172, score.rs:165, score_block.rs:94-96, screens.rs:172, markdown.rs:393-401 against the diffs.
- C (8774eb6 1f6799a 6b24c9e f8a0ab3 e1b0cfe): ACCEPTED. Checked resolution.rs:278,294,455, score.rs:19-66, share.rs:44, handoff.rs:232, suppression.rs:123,188, hotspots.rs:52,210. Note: 8774eb6 still labels core-v5; label moves in 5cd4592.
- D (2013077 1b7c220 aa65611 8ed9985): ACCEPTED. Checked why.rs:77,311, ceilings.rs P2=75/P3=None, normalize.rs:33 v2, duplication.rs:244,419,623, clippy.rs Off at :102,:318.
- E (5cd4592): ACCEPTED. Corpus scores recomputed with jq from v0.9.0 and HEAD tests/corpus.json, identical to the report (max move coda 88->83, -5). Found docs/delta-pairing-2026-10.md:123 "0.11.0 candidate", fixed to 0.10.0.
- F (8395e6e f2a3d45 3003096 1cbcd2d 98e608b ad28912 6bcdfc9 742a0b0 de6042a 5a20644): ACCEPTED. Checked lockfile.rs:26,47-60,93, source_text.rs as_compiled, README:29/35, pack --targets. Note: install.yml pins actions by tag, not commit (not a release.yml job, docs_contract only binds release.yml).
- G (3fc935f bcc23a3 ff0b929 4d1057a 687b1b8): ACCEPTED. 3fc935f verified (main.rs:497-522, 566-569, interrupt.rs:283-294); internal ones spot-checked by --stat (tests, CI, docs only).
- H (7991b7a): ACCEPTED. Internal. --help of every subcommand diffed v0.9.0 vs HEAD builds: identical except ci install --toolchain default 1.97.1->1.99.1 (7fc9445). Shebang re-added by stageWrapper (pack-local.mjs:102,109). Only user effect: library rustdoc gone from docs.rs (54 /// lines removed from src/lib.rs alone).
- I (b6ed111, mine): internal, docs/delta-pairing-2026-10.md:123 0.11.0 -> 0.10.0.
- Union check: 38 SHAs of v0.9.0..687b1b8 = A8+B4+C5+D4+E1+F10+G5+H1 = 38. OK.

## Site
- Worktree (LF) scratchpad/rust-doctor-web-release on release/v0.10.0. Local: lint, format:check, catalog:check-only, corpus:check OK; next build fails only on NTFS ':' in /rules/clippy::* (Windows-only). catalog:sync replay fails on Windows (baseline-materialization-failed, same with 0.9.0): rules.json written from `rules list --json` and checked.
- Gate 42485873154 failed: corpus sha256 hashed from a CRLF rust-doctor checkout. Resynced from LF worktree of v0.10.0, fixup pushed (ff), gate 42486013562 success. Linux provenance: requested_ref v0.10.0, commit ff91589, 142 descriptors. Squashed locally to 5d2c17f.
