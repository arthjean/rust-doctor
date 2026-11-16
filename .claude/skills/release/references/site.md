# Sync rust-doctor.com

The site is `../rust-doctor-web` (remote `arthjean/rust-doctor-web`, deployed by Vercel from `main`). It has no `AGENTS.md`: its `README.md` holds the scripts and `PRODUCT.md` the voice any copy edit follows. The checkout may hold Arthur's work: stage by explicit pathspec, never `git add -A`, and when files the sync touches are dirty, work in `git -C ../rust-doctor-web worktree add --detach <scratch>/rust-doctor-web-release main` (run `bun install --frozen-lockfile` there). Below, `<release checkout>` is the rust-doctor checkout the release ran from, clean and at the bump commit.

## Prepare (during step 6)

Model the change on the previous release-sync commits:

```bash
git -C ../rust-doctor-web log --oneline -3 --grep='move the site onto'
git -C ../rust-doctor-web show --stat <previous-sync-commit>
```

**Always moves.** `FALLBACK_VERSION` in `src/lib/releases.ts`, and the three snapshots under `public/catalog/`, regenerated from the tagged commit:

```bash
cd <release checkout>
cargo build --locked --bin rust-doctor
bin="$(cargo metadata --format-version 1 --no-deps | jq -r .target_directory)/debug/rust-doctor.exe"
cd ../rust-doctor-web
export RUST_DOCTOR_BIN="$bin" RUST_DOCTOR_SOURCE_DIR="<release checkout>"
RUST_DOCTOR_SOURCE_REF=vX.Y.Z RUST_DOCTOR_SOURCE_SHA=<bump-sha> bun run catalog:sync
bun run corpus:sync
```

`catalog:sync` refuses a source checkout holding any untracked file, which is why the ledger lives under this skill's gitignored `runs/` folder. `rules.json` and `corpus.json` are platform-neutral; the provenance it writes here is a Windows replay, replaced below by the Linux one.

On this Windows machine (`core.autocrlf=true`, learned on 0.10.0) three things break that command block, so work from LF copies of both repositories: `git -c core.autocrlf=false worktree add ...` for the site and for the tag, with `RUST_DOCTOR_SOURCE_DIR` on the latter.
- The public-command loader matches its fences on `\n`, so a CRLF `cli-reference.mdx` yields "missing public command examples".
- `corpus:sync` hashes the source `tests/corpus.json`, so a CRLF checkout writes a `sha256` the Linux gate calls stale.
- The `baseline` public command fails on Windows with `baseline-materialization-failed`, as 0.9.0 already did, so `catalog:sync` cannot finish locally. Write `rules.json` the way the script does (`rules list --json`, sorted by `id` with `localeCompare`, `JSON.stringify(_, null, 2)` plus a newline) and check it with `catalog:check-only`; the branch gate replays the commands on Linux.
- `bun run build` fails locally on `/rules/clippy::*` because NTFS refuses `:` in a file name; the branch gate's `build:verified` is the build check.

**Moves when the release changed it.** For every value the ledger moved (the previous score model id, the previous `schema_version`, the previous catalog total and per-producer counts, a renamed flag, key or rule id), run `git -C ../rust-doctor-web grep -n -e <old value> -- src content scripts` and update every hit that states the current value. A hit naming it as history, like a retired model in a share-contract test, stays, and the newly retired value joins it. Known homes:

- Score model, `core-v7` since 0.12.0 retired `core-v6`: `CURRENT_MODEL` in `src/app/share/share-contract.ts`, `content/docs/scoring.mdx`, the share URLs in `src/components/terminal-data.ts`, the `sharePath` default in `scripts/audit-wcag.ts`, and the retired-model loop of `scripts/verify-public-contract.ts`. `corpus:check` fails while the scoring page or an observation names a model the contract does not. A model change also rewrites what `scoring.mdx` says the model charges; for `core-v7` that is the cause rule (`1 + ln(n)` sites per cause) and the 5000 basis point prior of an unmeasured rule.
- Catalog counts: the JSON-LD `featureList` in `src/app/layout.tsx`, `src/lib/data.ts`, `content/docs/index.mdx`, `content/docs/rules/index.mdx`, `content/docs/limitations.mdx`.
- Report schema: the `schema_version` example in `content/docs/report.mdx` and the fields it shows.
- CLI, configuration, Action and hooks: `content/docs/cli-reference.mdx`, `configuration.mdx`, `ci-cd.mdx`, `agents.mdx`, `suppressions.mdx`, `installation.mdx` (a new installer). Trace each claim to the ledger or the binary's `--help`.
- The network promise: when the README's wording of it changed, align `content/docs/privacy.mdx`, `content/docs/index.mdx` and the tagline in `src/app/layout.tsx` with it.

US English, no em dash glyph. Then the platform-neutral checks, locally:

```bash
bun run lint && bun run format:check && bun run catalog:check-only && bun run corpus:check && bun run build
```

**Branch gate.** `build:verified` and `audit:wcag` replay the binary and drive Chromium, so the site's own workflow runs them on Linux against the tag. Commit on a `release/vX.Y.Z` branch with the subject `feat(release): move the site onto rust-doctor X.Y.Z` and a body naming what moved, push the branch, then:

```bash
gh workflow run public-contract.yml -R arthjean/rust-doctor-web --ref release/vX.Y.Z -f rust_doctor_ref=vX.Y.Z
```

Follow that run to green, download its `rust-doctor-public-contract` artifact into the scratchpad (`gh run download <id> -R arthjean/rust-doctor-web -n rust-doctor-public-contract -D <dir>`), copy the `build-provenance.json` found inside over `public/catalog/build-provenance.json`, and amend the commit locally without pushing the branch again. That file is the Linux replay of the tag, and it names `requested_ref: vX.Y.Z` and the bump commit.

## Publish (step 9, after step 7 holds)

The site reads `releases/latest` at build time, so it publishes only once the release is public.

```bash
git -C ../rust-doctor-web push origin HEAD:main
git -C ../rust-doctor-web push origin --delete release/vX.Y.Z
```

Wait for the deployment of that exact commit: `gh api repos/arthjean/rust-doctor-web/commits/<sha>/status --jq '.statuses[] | "\(.context): \(.state)"'` reports `Vercel: success`, and the `Public contract` push run on that SHA goes green. Then check production:

```bash
curl -fsSL https://rust-doctor.com/ | grep -o '"softwareVersion":"[^"]*"'
curl -fsSL https://rust-doctor.com/catalog/build-provenance.json | jq -r .rust_doctor_source.commit_sha
curl -fsSL https://rust-doctor.com/docs/changelog | grep -c 'X\.Y\.Z'
```

When the score model moved, `curl -fsSL 'https://rust-doctor.com/share?s=80&m=<previous model>'` also contains `a model Rust Doctor has retired`. A `softwareVersion` still on the previous release is the hourly `revalidate` of `src/lib/releases.ts`: query again after it rather than redeploying. A production check that fails otherwise: promote the previous production deployment through the `vercel-cli` skill, confirm the site recovered, then fix forward on `main`.

**Criterion:** the local checks and the branch gate passed, the committed provenance names the bump commit and `vX.Y.Z`, the commit is on `origin/main` with `Vercel: success` and a green `Public contract` run, and the three production checks print `X.Y.Z`, the bump SHA and a non-zero count.
