# The baseline delta

`src/delta.rs` is the comparison a `--scope baseline` run publishes: which
findings the branch introduced, which it inherited, and which it fixed. It is a
multiset pairing between two independent scans, and the identity it pairs on is
evidence first, the normalized source excerpt the span covers hashed with the
rule and the message, because a line number moves on every edit above it and a
message states counts the next commit changes. `src/delta/tests.rs` carries the
tests and `src/delta/tests/oracle.rs` the 32-case adversarial oracle frozen in
`tests/fixtures/baseline/delta-oracle.json`, replayed twenty times per run.

Four rules hold it together, and each of them replaced something that had a
cost.

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
could disagree with the pairing that produced it. `cross_file_matches` is now
the length of what the moved pass returned. That pass runs last, so a message
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

