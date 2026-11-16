# When the tag run goes red

The tag run publishes to four places in order, and each one answers
differently to a second attempt. Find the failing job first
(`gh run view <id> --log-failed`), then read the matching case.

## Nothing was published yet

The `build`, `build-windows` and `launcher` jobs run before either registry, so
a failure in any of them burns nothing. Re-run the failed jobs
(`gh run rerun <id> --failed`) for an infrastructure flake, or fix on `main`,
delete the tag and cut it again on the new commit:

```bash
git push origin :refs/tags/v<version>
git tag -d v<version>
```

The number is only burned once a registry has accepted it.

## The npm job failed partway through six packages

Safe to re-run. `publish()` asks `npm view` before every package and skips what
the registry already serves, so a re-run publishes the remainder rather than
failing on what landed. The order is the contract: the five native packages,
then the wrapper that names them as exact optional dependencies.

What a re-run cannot do is replace a bad tarball. A published `name@version` is
burned on npm even after `npm unpublish`, so a package that shipped wrong is
superseded by the next number, never replaced.

## The crates.io job failed

Safe to re-run. The job queries `https://crates.io/api/v1/crates/rust-doctor/<version>`
and skips the upload on a 200. Anything other than 200 or 404 stops the job on
purpose: a transport failure read as "not published" is how a release talks
itself into an upload the registry refuses.

`cargo publish` failing on the verification build means the crate does not
compile from its own tarball. Check `exclude` in `Cargo.toml`: `skills/` has to
stay in, since `src/skill.rs` embeds it with `include_str!`.

## The GitHub Release job failed with a missing note

Both registries have already published at this point, so the version is burned
and correct. The release page is the only thing missing. Write the note, commit
it on `main`, and publish the body by hand rather than moving the tag:

```bash
gh release create "v<version>" --title "v<version>" \
  --notes-file .github/releases/v<version>.md --latest --verify-tag
```

A re-run of the job edits an existing release rather than failing, so it is
also safe once the note is on the tagged commit.

## The version is burned and the build was wrong

Ship the next number. Do not delete a tag whose version a registry accepted:
the tag is the only record of what that published version was built from.
Bump, write a new note stating what the superseded version did, and run the
whole skill again.
