# The documentation gates

`AGENTS.md` is read into every agent session before the first tool call, so its
size is a per-session cost rather than a matter of taste, and every number it
states is a claim an agent acts on without checking. Three gates hold the
documentation to that, and each of them replaced a failure the prose had already
produced.

## The word budgets

`docs/doc-budgets.json` carries a ceiling for the map and for every standing
document it links, and `scripts/verify-doc-budgets.sh` checks them.

A ceiling ratchets both ways. A file over its ceiling fails, and a file under 85
percent of it fails too, so the words a document no longer needs lower its
ceiling in the commit that removes them instead of accumulating as slack for the
next writer. A new ceiling is the word count plus 5 percent, which is the band a
document may grow in before it has to earn more. Raising one is a deliberate
edit, justified in the pull request.

What this replaced: `AGENTS.md` had reached 9755 words, and it reached them by
being the only place a fact could go. Splitting it into this document set took
the map back to about 1100 and moved the reasoning onto the subsystem pages, but
nothing about a split prevents the same growth from starting over, which is what
the ceilings are for.

Every markdown file under `docs/` is listed, under `budgets` or under `frozen`,
and the script fails on one that is listed nowhere. Without that check the way
out of a ceiling is a new unlisted file, which is the cheapest move available to
anyone writing under a budget. `frozen` holds the measurement records: they
carry no ceiling because they are not standing guidance but what a run measured
on a date, and they are not edited afterwards.

## The references

`scripts/verify-doc-refs.sh` fails on any path under `docs/` named anywhere in the
repository that does not resolve, and on a relative markdown link whose target
is not on disk. Source comments, skills, workflows and the documents themselves
all cite paths; renaming a document breaks every one of those citations in
silence, and the reader who follows one opens nothing and falls back to the map.

## The contract

`tests/docs_contract.rs` recomputes the numbers the standing documents state
from the binary that produces them, the way `tests/skill_contract.rs` does for
the shipped skill.

What this replaced: the map stated the report schema as 15 while
`src/report.rs` declared 16, and that statement survived a rewrite that touched
every line of the file it sat in. The same rewrite left the map saying three of
the four workflows pin toolchain 1.97.1 while its own home document said all
four do, and left `docs/testing.md` counting eleven size-bound tests where the
map counted fourteen. Reorganizing prose does not catch a stale number, however
carefully it is done. Recomputing the number does.

Under contract: the size of the catalog and its five per-producer counts, the
schema version and the ledger of what each version added, the two bounds
`oversized_unit` reports at, the count and the naming of the size-bound tests,
and the number of workflows together with the toolchain every one of them pins.
A number a document states that no test can recompute is a number to delete
rather than one to gate.
