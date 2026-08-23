# The workflows

| Workflow | When | What it settles |
|---|---|---|
| `ci.yml` | push, pull request | Clippy clean, tests on Linux and macOS, the crate still compiles on Windows and on its declared MSRV 1.95, the Node launcher and its packed install |
| `dogfood.yml` | push, pull request | The repository scans itself with the binary built from the commit under review, in baseline scope on a pull request so only the findings the change introduces are judged |
| `release.yml` | tag `v*`, manual | The five platform binaries the launcher declares, then the six npm packages, the crate on crates.io and the GitHub Release. A tag publishes; a manual run stops at the two dry runs |
| `corpus.yml` | manual, pull request touching the record, its harness or a producer | Reproduces the pinned measurement of `tests/corpus.json` from the restored clone cache, under the toolchain the artifact names, and writes the position proof that anchors every published site |

Three deliberate gaps. There is no `cargo fmt --check` gate: the tree is not
rustfmt-clean, and reformatting it is a separate mechanical commit, not
something a CI file should decide. The Windows leg stops at `cargo check`
rather than `cargo test`: the launcher declares a `win32-x64` package, so the
build must keep working, but 14 of the 273 unit tests fail there as measured on
2026-08-12, on path separators, edition resolution and the hotspot self-scan.
That is a port to do, and a job that stays red forever teaches everyone to
ignore a red job. And `corpus.yml` is manual rather than nightly, since every
input it consumes is pinned, so a schedule would recompute the same answer at
the cost of compiling eighteen repositories. It also runs on a pull request
editing the record, the harness under `tests/support/corpus`, or one of the
five producers: the three things that move it, and where a site enters the
record with no run behind it. Every rule has an always-on test rescanning a
fixture, so a dead detector fails `cargo test`; what only the corpus catches is
one narrowed until it matches that fixture and no longer real code. A pull
request touching none of them never starts it, decided by the path filter
rather than a step that has already booted a runner, and a pull request that
matches no clone cache fails naming the key it missed rather than fetching
eighteen third-party repositories to answer a question about a diff.

The structural benchmark asserts a wall clock, and its bounds were measured on
a development machine. A slower machine declares itself through
`RUST_DOCTOR_BENCHMARK_ALLOWANCE`, a multiple the CI sets to 3, rather than
having the constants raised for everyone. It moves the two clocks only: the
counter assertions that prove the near-duplicate scoring stays nominated rather
than pairwise hold on any machine and are never relaxed.

The toolchain is pinned to 1.97.1 in every workflow rather than tracking
`stable`. Clippy's diagnostics are the product: 37 curated lints of the 62
catalogued rules are its own, and `tests/corpus.json` records the exact Clippy
version its measurement was taken under.

