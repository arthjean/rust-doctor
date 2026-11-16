---
name: release
description: Ship a rust-doctor release end to end, from version bump to green CI, both registries, the immutable GitHub Release, the floating Action tag, and rust-doctor.com.
disable-model-invocation: true
argument-hint: "[version, e.g. 0.10.0]"
---

# Release rust-doctor $ARGUMENTS

Take `$ARGUMENTS` from `main` to a version every installer serves (`npx rust-doctor`, `cargo install`, `cargo binstall`, `uses: arthjean/rust-doctor@v0`) and that rust-doctor.com describes, and run until the contract below is fully green. Arthur invokes this and walks away: the run is his hands on the keyboard for the whole release.

A version is **burned** the moment npm or crates.io accepts it: neither registry ever lets a `name@version` be reused, and an immutable GitHub Release locks its tag and archives. Every step before the tag exists to find defects while the version is still unburned.

## Mandate

Invoking `/release` authorizes, for this version only: the bump commit and its push to `main`; source fixes committed to `main` while the version is unburned; a `workflow_dispatch` candidate run of `release.yml`; the tag and its push; re-tagging the same version while it is unburned, per [recovery](references/red.md); `gh run rerun --failed` and `gh run cancel` on this version's runs; moving the floating major tag (`v0`) with the force push `docs/publishing.md` prescribes; `gh release edit` on this release's title, notes and latest flag; a temporary `release/vX.Y.Z` branch on `rust-doctor-web` and the release-sync commit pushed to its `main` (which deploys rust-doctor.com). Anything Arthur writes alongside the version overrides this skill.

Keep going between steps. Put status in one line beside the next tool call. A summary that names the next step, an offer to continue, and a menu of options are all still mid-run while any contract row is unmet. Stop only for one of the four blockers:

1. A credential or external service you cannot restore yourself (npm trusted publishing, the crates.io trusted publisher, GitHub or Vercel down past a retry).
2. A source fix needed once the version is burned: the next version number is Arthur's call. Commit and gate the fix on `main` first, so his answer is the only missing input.
3. A destructive action beyond the unburned re-tag and the floating-tag move: deleting a published release, force-pushing a branch, moving a tag a registry already published from.
4. The version is not plain `X.Y.Z`, collides with a tag or a registry version, does not advance the last release, or rides a breaking change on a patch bump.

If a line of this skill is what makes you pause, quote it in the report and say whether it is an explicit requirement or your reading of it.

## Done means

Every row holds, each with evidence:

- The tag-push `Release` run for `vX.Y.Z` succeeded with every job executed, `cargo-binstall proof` included.
- npm serves `X.Y.Z` for `rust-doctor` and the five `@rustdoctor/<platform>` packages, and `rust-doctor`'s `latest` dist-tag is `X.Y.Z`.
- crates.io serves `rust-doctor` `X.Y.Z` as its newest stable version, and the dispatched `Install` run (unlocked `cargo install` of the published release) succeeded.
- The GitHub Release `vX.Y.Z` is published, immutable, marked latest, carries the five `rust-doctor-<target>.tar.gz` archives, passes `gh release verify`, and its body equals `.github/releases/vX.Y.Z.md`.
- `npx --yes rust-doctor@X.Y.Z --version` from an empty scratch directory prints `X.Y.Z`.
- `v<major>` peels to the bump commit, and the `Release` run its push started is cancelled.
- rust-doctor.com serves `X.Y.Z` from a ready production deployment of the release-sync commit, per [site sync](references/site.md).

A red, skipped, or unverified row is an open row.

## Ledger

Keep the run's state in `runs/vX.Y.Z/state.md` beside this file. Its `runs/` folder is gitignored, which matters: the site's provenance refuses a rust-doctor checkout with any untracked file, so the ledger can live nowhere under a tracked path. `state.md` is a checklist of the steps below plus the facts as they land: previous tag, release base and bump SHAs, slice reports and their verdicts, run IDs and URLs, probe outputs, site commit and deployment URL. Tick each item as its criterion holds. On start, if `state.md` exists, resume from the first unticked item after re-verifying the last ticked one against live state.

`docs/publishing.md` and `docs/ci-workflows.md` own the pipeline's reasoning and `AGENTS.md` owns the gates; read all three before step 1. When they and `.github/workflows/release.yml` disagree, trust the workflow and name the discrepancy in the report.

**Public-state probe.** Preflight, step 7 and every recovery read the version's public state from this:

```bash
V=X.Y.Z
for p in @rustdoctor/linux-x64 @rustdoctor/linux-arm64 @rustdoctor/darwin-x64 @rustdoctor/darwin-arm64 @rustdoctor/win32-x64 rust-doctor; do
  printf '%s ' "$p"; npm view "$p@$V" version 2>/dev/null || echo absent
done
curl -s -o /dev/null -w 'crates.io %{http_code}\n' \
  -H 'user-agent: rust-doctor release check (https://github.com/arthjean/rust-doctor)' \
  "https://crates.io/api/v1/crates/rust-doctor/$V"
gh release view "v$V" --json isDraft,isImmutable --jq '"release draft=\(.isDraft) immutable=\(.isImmutable)"' 2>/dev/null || echo 'release absent'
```

The version is unburned while all six print `absent` and crates.io answers `404`.

## Steps

1. **Preflight.** Strip a leading `v` and require `^[0-9]+\.[0-9]+\.[0-9]+$`: the pipeline has no prerelease channel, since any `v*` tag publishes to npm `latest`, to crates.io and to the latest GitHub Release. `git fetch --tags origin`, confirm `main` is not behind `origin/main`, that `vX.Y.Z` exists neither locally nor in `git ls-remote --tags origin`, that it sorts above the previous tag, and that the probe shows it unburned (npm also burned 0.1.1 to 0.2.0 for good). The previous tag is `git describe --tags --abbrev=0 --match 'v[0-9]*.[0-9]*.[0-9]*' main`; the pattern skips `v0`. Local commits ahead of `origin/main` are Arthur's finished work and ship with this release; list them in `state.md`. Uncommitted changes are Arthur's work in progress: leave them untouched and release from `git worktree add --detach <scratch>/rust-doctor-release main` with `CARGO_TARGET_DIR=<scratch>/rust-doctor-release/target`, since the machine's global target directory lets a test binary keep a deleted worktree's manifest path. Never run `git config` inside the worktree: it writes the main checkout's shared config. Then read the latest `CI` and `Rust Doctor` runs on `origin/main`'s head: a red one is a red gate, and fixing it on `main` comes before the bump. *Criterion:* version, previous tag, base SHA, checkout path and both CI conclusions are in `state.md`, and both runs are green.

2. **Ledger and notes.** The ledger is an exact account of every change since the previous tag, built by subagents and reviewed by you.

   *Fan out.* List the range with `git rev-list --reverse --no-merges <previous>..HEAD` and cut it into contiguous slices of about ten commits, a commit with a very large diff taking a slice alone. Give each slice to its own `general-purpose` subagent, all launched in one message, with this brief: read the full diff of every commit in the slice (`git show --stat --patch <sha>`), not its message alone, and return one entry per SHA with: user-facing or internal; what changed for the user, down to exact values (rule ids with their level, tier, category and default; per-producer catalog counts; the score model and its constants; `SCHEMA_VERSION` and every report field added, renamed or removed; CLI subcommands, flags, defaults, exit codes and printed text; `rust-doctor.toml` keys; inputs and outputs of `action.yml` and `comment/action.yml`; launcher and installer behavior; platform-specific branches); anything breaking or needing migration; and `path:line` evidence for each claim. Tell it the report is read by a human and must list every SHA of its slice.

   *Review each report as it arrives, before accepting it.* Check that its SHA list equals its slice exactly. Open the cited hunk (`git show <sha> -- <path>`) for every user-facing claim and confirm the value and behavior match the diff; spot-check the commits it marked internal. A report with a missing SHA, a claim the diff does not support, or a vague entry ("improves the report") goes back to the same subagent through `SendMessage` with the precise gap, and is reviewed again on return. Mark each slice accepted in `state.md` only then. Once all slices are accepted, check that the union of their SHAs equals the range. When `tests/corpus.json` moved, tabulate each subject's score at both ends from `git show <rev>:tests/corpus.json`. An existing note in `.github/releases/` newer than the previous tag is an earlier draft: fold it in (0.8.0 renamed a `v0.11.0.md` draft) and reconcile every claim in it with the ledger.

   *Write the note through `/write-release-notes`, always.* Once every slice is accepted, invoke the `write-release-notes` skill with the Skill tool and `args: "v<previous>..HEAD"`, and write the note by following it: the accepted ledger is its evidence, `.github/releases/vX.Y.Z.md` its destination, and the conventions below are the project conventions it defers to. The note is never drafted by hand or before that invocation, and a later rewrite of it goes through the skill again. The workflow publishes this file verbatim as the release body and titles the release `vX.Y.Z`, so it is final before the tag:
   - No heading of its own. An opening paragraph, `X.Y.Z is ...`, naming the dominant change and, when anything breaks, pointing at `[Breaking changes](#breaking-changes)`.
   - `## Breaking changes`, one `###` per change: old behavior, new behavior, who is affected, then `**Upgrade path.**` with the exact command or YAML.
   - Theme sections in order of user impact. Every claim cites its commit as `([abc1234](https://github.com/arthjean/rust-doctor/commit/abc1234))`; a number that moved (catalog counts per producer, schema, score model, corpus scores) gets its old and new value, as a table when several move.
   - `## Internal`: one paragraph for CI, test and tracker commits.
   - `## Install`: `npx rust-doctor@latest` and `cargo install rust-doctor --locked` in `bash` fences, plus any installer the release adds. `every_advertised_cargo_install_is_locked` reads the newest note.
   - `**Full changelog:** https://github.com/arthjean/rust-doctor/compare/v<previous>...vX.Y.Z`
   - Last, and followed by nothing: `## Release metadata` holding a `yaml` fence with `version`, `date`, `previous_version`, `channel: stable`, `breaking`. The site strips that block and reads its badge from it only when nothing follows the fence.
   - US English, no em dash glyph, no AI attribution, issue references as `#N` and never a closing keyword.

   A `breaking: true` note on a patch bump is blocker 4. *Criterion:* every slice is accepted after your review, the union of their SHAs equals the range, `state.md` records the `write-release-notes` invocation that wrote the note, every user-facing entry maps to a line of the note, and the note holds no unknown the skill left unresolved.

3. **Bump.** Set `X.Y.Z` in `Cargo.toml`; run `cargo update --workspace` and confirm `git diff Cargo.lock` moves only the `rust-doctor` entry; set it in `npm/rust-doctor/package.json` (`version` and the five `optionalDependencies`), in the five entries of `npm/rust-doctor/bun.lock` (edit by hand: `bun install` would try to resolve packages npm does not serve yet), and in `action.yml`'s `version` default. `git grep -n <previous version>` over those five files returns nothing. Run `bash scripts/verify-doc-budgets.sh`, `bash scripts/verify-doc-refs.sh`, `cargo test --test docs_contract` and `cargo test --lib the_action_declares_its_inputs_and_defaults_to_this_release`. On this Windows machine the full `cargo clippy` and `cargo test` gates are red at HEAD for reasons unrelated to a release, so `ci.yml` on Linux and macOS is the gate: commit `chore(release): X.Y.Z` with a body saying why the minor or patch moves and what the note leads with (model: `git show bad7c10`), stage by explicit pathspec, push `main` (`git push origin HEAD:main` from a worktree), and follow the `CI` and `Rust Doctor` runs on the bump SHA with `gh run watch --exit-status <id>` in a background Bash call. *Criterion:* `origin/main` equals the bump commit and both runs on it are green.

4. **Candidate.** `gh workflow run release.yml --ref main`, then follow the `workflow_dispatch` run whose `headSha` is the bump commit. It builds the five binaries, proves cargo-binstall against the archives locally, and stops at `npm publish --dry-run`, `cargo publish --dry-run` and the release-note check, so it finds every packaging defect while the version is unburned. *Criterion:* the run is `success` and its summary is headed `Release candidate`.

5. **Tag.** `git tag vX.Y.Z <bump-sha>` (lightweight, like every tag since 0.5.0), then `git push origin vX.Y.Z`. *Criterion:* `git ls-remote --tags origin 'refs/tags/vX.Y.Z'` resolves to the bump commit.

6. **Follow the chain to green.** Find the `push` run of `release.yml` whose `headBranch` is `vX.Y.Z`, and follow it with `gh run watch --exit-status <id>` in a background Bash call; the harness wakes you when it exits. On each wake, read the real state with `gh run view <id> --json status,conclusion,jobs`; if the watcher ended without a conclusion, arm it again. Any red job goes through [recovery](references/red.md), which starts with the probe. While it runs, prepare the site change per [site sync](references/site.md), up to its green branch gate, without touching the site's `main`. *Criterion:* the run is `success`, no job is skipped, and its annotations hold no warning you have not read and justified.

7. **Verify what shipped.** Run the probe: six versions and crates.io `200`. Then: `npm view rust-doctor dist-tags.latest` and `curl` of `https://crates.io/api/v1/crates/rust-doctor` (`.crate.max_stable_version`) both print `X.Y.Z`; `gh release view vX.Y.Z --json isDraft,isPrerelease,isImmutable,assets` shows a published, immutable, non-prerelease release with exactly the names `bash scripts/pack-github-release.sh --targets` implies; `gh api repos/arthjean/rust-doctor/releases/latest --jq .tag_name` is `vX.Y.Z`; `gh release verify vX.Y.Z` passes, and so does `gh release verify-asset vX.Y.Z <archive>` on one downloaded archive; the body from `gh release view vX.Y.Z --json body --jq .body` diffs clean against the note, CRLF and trailing newline aside. From an empty scratch directory, `npx --yes rust-doctor@X.Y.Z --version`: this runs the `win32-x64` package, the one platform CI never runs end to end. Dispatch `gh workflow run install.yml` and follow it. *Criterion:* every check above holds, each recorded in `state.md`.

8. **Floating tag.** For `X` = 0 the tag is `v0`; a first `1.0.0` starts `v1` and leaves `v0` on the last 0.x. `git tag -f v<major> vX.Y.Z && git push -f origin v<major>`. That push matches `release.yml`'s `v*` filter and starts a run its version guard fails at the publish jobs, harmless but 25 runner-minutes of red: `gh run cancel` it at once. *Criterion:* `git ls-remote --tags origin 'refs/tags/v<major>'` names the bump commit and that run's conclusion is `cancelled`.

9. **Site.** Publish the change prepared in step 6, per [site sync](references/site.md). *Criterion:* the site sync criterion holds.

10. **Report.** Remove the release worktree if you made one (then `touch tests/skill_contract.rs` in the main checkout before trusting a `NotFound` there), and delete the site's `release/vX.Y.Z` branch. Report under three headings. **Blocked on me**: open decisions or `None`, plus a reminder to pull `main` when the release ran from a worktree. **Shipped**: version, tag and commit, run URLs with conclusions, npm, crates.io and release evidence, the `npx` output, site commit and deployment URL. **Found**: recoveries applied, discrepancies between docs and workflows, warnings accepted and why. Call the release done only when every contract row holds.
