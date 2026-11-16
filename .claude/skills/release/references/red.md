# Release recovery

Read the failed job's log (`gh run view <id> --log-failed`) before choosing a move, and run the public-state probe from the skill, recording its output in `state.md`. Every retry targets this tag; confirm a run by its `headBranch` and `headSha`, never by "newest run".

## Burned or not decides the move

**Unburned.** The probe prints `absent` six times and crates.io answers `404`. Every build, launcher and verification job runs before either publish job, so a failure there always lands here. Nothing has reached a user: fix and re-tag the same version.

```bash
git tag -d "v$V"
git push origin ":refs/tags/v$V"
```

Commit the fix on `main`, follow `CI` and `Rust Doctor` to green on it, run the candidate again (skill step 4), then tag the fix commit (step 5). This is the only tag deletion the mandate covers.

**Burned.** Any of the six packages or the crate serves the version. Both publish jobs skip what their registry already serves, so a rerun after an infrastructure failure finishes the release instead of consuming a version. A failure with a source cause ships its fix under the next version: blocker 2 in the skill.

## Reproducing a failure

This machine runs Windows and WSL `Ubuntu-26.04`, which has cargo but no bun. The `win32-x64` leg reproduces natively. A Linux leg reproduces in an LF clone inside WSL: `git -c safe.directory='*' clone /mnt/c/dev/rust-doctor`, with the WSL script written to a file and run as `MSYS_NO_PATHCONV=1 wsl -d Ubuntu-26.04 -- bash /mnt/c/<script>`, under an explicit Linux-only `PATH`, since WSL appends the Windows one. A macOS-only failure reproduces on GitHub runners: a throwaway `debug/<topic>` branch carrying a loop workflow that reruns the failing test binary, deleted afterwards. Before building any loop, grep `tasks/e2e-catalog/` and `docs/e2e.md` for the symptom: the defect may already be a ledger row.

## The release run

| Evidence | Move |
|---|---|
| A `build` leg, `win32-x64`, `Packed launcher proof`, `Verify the crate package` or `Release archives` fails on a compile, test or packing error | Unburned by construction. Reproduce, fix the source, re-tag. |
| `tag vX.Y.Z does not name version ...` in a publish job | A bump surface was missed, which a green candidate rules out: unburned, so complete the bump and re-tag. |
| Failure with no source cause: a network fetch, a `rustup` download, a runner crash | `gh run rerun <id> --failed` once. A second identical failure needs diagnosis. Ignoring or deleting a test is never the fix. |
| `Publish to npm` refuses authentication (`ENEEDAUTH`, `E403`, `E404` on publish) | The package's trusted publisher on npmjs.com does not match `release.yml`, or a package is new and was never published by token. Blocker 1. |
| npm published some packages, then failed | Burned. An infrastructure cause: `gh run rerun <id> --failed`, the skip finishes the rest. A source cause: blocker 2. |
| `Publish to crates.io` fails in `crates-io-auth-action` | A transient token exchange: rerun once. A second failure is the trusted publisher on crates.io: blocker 1. |
| `crates.io answered <status> for version ...` | The pre-publish lookup met a registry error: rerun once. |
| `GitHub Release` fails after both registries published | `gh run rerun <id> --failed`: the job finishes a draft (uploads, then publishes) or uploads what a published release lacks. A missing note cannot pass a green candidate; if it happens anyway, stop: the note has to come from the tagged commit, which the registries already published from. |
| `cargo-binstall proof` red after its ten attempts | Index or CDN lag: rerun the job once. A second failure with the archives present is a packaging defect of a burned version: blocker 2. |
| Runner stays queued | Check githubstatus.com and runner availability. A queue alone is no reason to cancel. |
| A `::warning::` annotation | Read it. A rust-cache or Node deprecation notice is noise; a provenance, signature or archive warning is an open row. |

## After the run

| Evidence | Move |
|---|---|
| `releases/latest` is not `vX.Y.Z` | `gh release edit vX.Y.Z --latest`, then query again. |
| The release body differs from the note | `gh release edit vX.Y.Z --notes-file .github/releases/vX.Y.Z.md`: an immutable release locks its tag and archives, not its title and notes. A wording fix to the note itself is rewritten through `/write-release-notes`, then lands on `main` as its own `docs(release)` commit before the `gh release edit`. |
| `npm view rust-doctor dist-tags.latest` is older | Changing a dist-tag needs Arthur's npm session: blocker 1. |
| `npx --yes rust-doctor@X.Y.Z` fails on Windows only | The `win32-x64` package or the launcher's resolution is broken in a burned version: commit and gate the fix on `main`, then blocker 2. |
| `Install` red on the published release | An unlocked dependency broke against today's registry, as in #40. The `--locked` install the note advertises still works: record it, fix the pin on `main`, then blocker 2. |
| The floating-tag run published something | It cannot: its guard fails before either publish. If the probe says otherwise, stop and report. |
