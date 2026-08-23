# Git and the scan scope

`src/internal_error.rs` is the one error shape every stage reports through. It
sits on its own rather than inside `execution`, which made `scan_target`,
`configuration`, `repo_hygiene` and `baseline` import back the module that
imports them, four cycles a reader had to hold for nothing.

`src/git.rs` is the bounded process layer every producer that shells out to git
runs through, and `src/git_scope.rs` is one of its three callers: the scope a
scan runs under, whole workspace, changed files, or baseline comparison. The
other two are `baseline.rs` and `repo_hygiene.rs`, which is why the layer lives
beside them rather than inside the scope that used to own it: reading
`git_scope::run_git` to list a tree said scope where none was involved.

Four rules hold the two together, and each of them replaced something that had
a cost.

A call names the stage every one of its outcomes is reported at. `GitCall`
carries `stage`, and the four exits build their `InternalError` from it: git
could not start, stdout overflowed, stderr overflowed, the call failed. Two of
those are not named by the caller, and they used to be hard-coded to `scope`
whoever ran them, so a baseline snapshot whose git flooded stderr published
`stage: "scope"` in the JSON, at a bound `baseline.rs` lists in its own oracle.
One overflow failure now covers both streams, since a caller that cannot use
the answer does not care which stream made it unusable.

Every way out of the layer is bounded. `git_command` is private and
`run_git_status` is what a call answering with an exit code uses, both streams
closed at the pipe. The escape hatch it replaced handed a `Command` out and
`repo_hygiene` called `.output()` on it, which reads both streams with no bound
at all, in the one module built to make that impossible.

Validated once, resolved from that. `ScopeRequest::validate` returns a
`ValidatedScope` holding a `BaseSelector` that passed the closed grammar, and
`resolve` reads that and nothing else. Validation used to run twice over the
same request, once as the gate in `lib.rs` and once inside the resolution,
which left a failure branch in resolution that no input could reach: the same
shape the policy module removed, in the module next to it.

One resolved shape, one constructor per case. `ResolvedScope` is the three
cases and `ScopeReport` the accessors over it; a second enum used to mirror it
variant for variant, so a fourth scope mode was an edit in five places.
`ScopeReport::files_scope` is the only way a file scope is built: it sorts,
deduplicates and bounds, and `includes` binary-searches that order. Production
and the tests used to establish that order separately, so a third construction
site would have broken the search in silence.

`the_git_layer_holds_the_size_bound_it_scans_for` and
`the_scope_holds_the_size_bound_it_reports_for` keep every file of both modules
under the 1000 lines `oversized_unit` reports, tests included. That is why each
carries its tests in a file of its own.

