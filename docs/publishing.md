# Publishing

`npx rust-doctor@latest` resolves the unscoped `rust-doctor` wrapper, which
carries the five `@rustdoctor/<platform>` binaries as exact-version optional
dependencies. Releasing is one version bump in `Cargo.toml`, mirrored into
`npm/rust-doctor/package.json` and `bun.lock`, then a `v<version>` tag:
`release.yml` refuses a tag that disagrees with the manifest.

Two scars from the retired account constrain the names. `@rust-doctor/linux-x64`
still resolves, owned by `npm-support`, so that scope cannot be reclaimed and
the binaries live under `@rustdoctor/*`. Versions 0.1.1 through 0.2.0 of
`rust-doctor` were unpublished and are burned for good, since npm never lets a
`name@version` be reused: the line restarts at 0.3.0 and can never go back.
That is also why the publish job skips what the registry already serves instead
of failing, so a re-run after a partial failure finishes the release rather than
consuming a version.

All six packages are staged before they are published, the wrapper included.
`pack-release.mjs` lays out the five native ones from the build matrix's
artifacts and stages the wrapper through the same `stageWrapper` the local proof
uses, so what a release uploads is what `bun run smoke:packed` installs. That is
also where the license text reaches the tarballs: `LICENSE-MIT` and
`LICENSE-APACHE` sit at the repository root, and npm auto-includes a license
only from a package root and only under a name it recognizes, which
`LICENSE-MIT` is not. So each of the six declares the pair in `files` and is
handed the two files at staging. Before that, six tarballs each declared
`MIT OR Apache-2.0` and carried neither text, and the wrapper was the one
package published from the checkout rather than from a staged directory, which
is what left it out of reach.

The GitHub Release is the third thing a tag publishes, after the two
registries, because a release page announcing a version npm or crates.io refused
is a claim with nothing behind it. Its body is `.github/releases/v<version>.md`,
written before the tag rather than assembled from the commit subjects, and a tag
whose note is missing fails the job instead of shipping a `git log`. Every run
checks the note exists, tag or not, so the candidate build is what catches a
version bumped without one; only a tag creates the release, and a re-run edits
the existing one so a partial release finishes rather than needing a new tag.

The job stores no secret. Each of the six packages names this repository's
`release.yml` as its trusted publisher on npmjs.com, and the registry
authenticates the run by its OIDC identity, which is why the job upgrades npm
past 11.5.1 and asks for `id-token: write`. A new package has to be published
once by token before it can be given a trusted publisher, since the setting
lives in a package's settings: 0.3.0 went out that way, and the token was
revoked afterwards. Every package is published with `--provenance`, which ties
a tarball to the commit and the workflow that built it.

crates.io publishes from the same tag, in the `crates` job, on the same
arrangement: `rust-lang/crates-io-auth-action` exchanges the run's OIDC identity
for a token that lives for one run, and the crate names this repository and this
workflow as its trusted publisher. Nothing else is shared between the two jobs,
because the two registries answer differently. `npm publish --dry-run` asks the
registry and refuses a version already published, so the npm skip has to guard
its dry run; `cargo publish --dry-run` never asks, so the crates.io skip guards
the upload alone and the validation keeps running between releases.

The two lines have to be told apart. npm restarted at 0.3.0 because 0.1.1
through 0.2.0 were unpublished there; crates.io still serves its own 0.2.0,
published 2026-06-15 from this same account, because crates.io has no
unpublish at all. So the numbers below 0.3.0 mean different things on the two
registries, and the first version this workflow ships to both is the first one
that means the same code on each. `exclude` in `Cargo.toml` is what separates
the crate from the repository around it: the Node launcher, the PRDs, the agent
tooling and `.github/` stay out, `skills/` stays in because `src/skill.rs`
embeds it with `include_str!` and a tarball without it does not compile.

