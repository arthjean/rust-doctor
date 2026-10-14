# The baseline delta

`src/delta.rs` is the comparison a `--scope baseline` run publishes: which
findings the branch introduced, which it inherited, and which it fixed. It is a
multiset pairing between two independent scans, and the identity it pairs on is
evidence first, the normalized source excerpt the span covers hashed with the
rule and the message, because a line number moves on every edit above it and a
message states counts the next commit changes. `src/delta/tests.rs` carries the
tests and `src/delta/tests/oracle.rs` the 32-case adversarial oracle frozen in
`tests/fixtures/baseline/delta-oracle.json`, replayed twenty times per run.

The base side builds under `<target directory>/rust-doctor/baseline`, beside
what `cargo clippy` itself writes, so a scan writes only build artifacts under
Cargo's target directory and `cargo clean` removes them. It used to build under
the snapshot's temporary root, removed after every run, and each baseline paid a
cold build of the whole dependency graph. A directory that cannot be created or
written falls back to that temporary target, and Cargo's build-directory lock
serializes two runs on one workspace.

Five rules hold it together, and each of them replaced something that had a
cost.

A pairing is decided by what the diff did, never by arrival order.
`delta/diff_map.rs` reads two bounded `git diff-index` calls before any pass,
the plumbing because `git diff` writes the index back: `--name-status -z` maps
every base path through Git's renames, and a `--unified=0` patch under one
mebibyte maps every base line through the hunks above it. Within one key, each
base finding takes the current finding nearest its mapped line, so a duplicate
inserted above an old one is the one reported introduced; first-in, first-out
reported the old one. Once evidence has paired what it can, the findings left
in one (renamed path, rule) pair by count, `rule_count_matches` in the summary:
a variable renamed inside `cfg.get("k").unwrap()` used to read as a fix and an
introduction. US-028 measured three policies on 125 real commit pairs
(`docs/delta-pairing-2026-10.md`), and this one, (d), removed all 29 false
introductions without adding any. The accepted blind spots are a fix and an
addition of one rule in one file, which offset each other, and an identical
excerpt fixed in one file and added in another, which evidence follows across
files as it follows a module extraction without a rename. A map git could not
read, too large or past the deadline, publishes `line-map-unavailable` as a
notice and pairs on raw line distance: it decides which finding is reported,
never how many.

A candidate is a diagnostic, not a row beside one. `Candidate` borrows the
`Diagnostic` it speaks for. The two index-parallel slices it replaced reached
the matcher as four separate arguments kept aligned by nothing but the reader's
attention, and the ten places that walked them all indexed a slice that a length
mismatch would have panicked on, in a crate whose manifest denies `panic`. The
same change dropped fourteen of the nineteen `clone` calls in the file, since a
key lives for one pass and the diagnostics outlive every pass.

Every pass is named, and the pass says what a match on it means. The four passes
used to be four calls carrying six anonymous closures, two of them identical
word for word, and a trailing positional `bool` announcing that the match was a
move. Nothing tied that flag to the key: the key of the last pass omits the
path, and the flag said so a second time, so the count the report publishes
could disagree with the pairing that produced it. `cross_file_matches` now
counts the pairs whose two published paths differ, along a rename or on moved
evidence. The moved pass runs after the message passes, so a message
match on the original file wins over a proof match elsewhere, which is a product
decision rather than a consequence:
`a_message_match_on_the_original_file_wins_over_a_moved_proof` is the one input
that puts the two in competition, and it is asserted so changing the order is a
moved assertion rather than a silent shift in what the gate calls new.

A published path is decoded before anything is opened, and the handle is
revalidated. `workspace_path` percent-encodes `%` and every control character of
a path the report publishes, so opening the published spelling literally found
no file and every finding in a file whose name carried one silently lost its
proof. `workspace_path::same_file` is the identity check both this module and
the code frame close the canonicalize-then-open race with, hosted once rather
than written twice.

A bound is a budget on work, never a filter on meaning. `SOURCE_BYTES_BUDGET`
bounds what one comparison may read and `PROOF_BYTES_BUDGET` what it may
normalize; the two used to share one constant named for neither, so the ceiling
read as half of what it was. A diagnostic whose evidence is out of budget falls
back to its message rather than disappearing, and `LineIndex` is the one
arithmetic that turns a reported line and character column into a byte offset,
replacing a sorted position set, a peekable state machine and a helper mutating
two collections at once.

The stage a failure names is this one. Refusing a comparison over
`DIAGNOSTIC_LIMIT` diagnostics used to be reported with the git baseline's own
failure, so a run that hit the diagnostic ceiling published `stage: "baseline"`
with `baseline-limit-exceeded` and told the reader their git snapshot exceeded a
limit, which was true of nothing.

`the_delta_holds_the_size_bound_it_matches_for` keeps every file of the module
under the 1000 lines `oversized_unit` reports, tests included: the pass that
decides what a branch introduced has to pass the rule it publishes. That is why
the oracle has a file of its own, why the thirty-two cases sit in six families
rather than in one 181-line `match`, and why a comparison and the verdict it has
to produce are data in `identity_cases` and `pairing_cases` rather than six test
bodies of the same shape.

