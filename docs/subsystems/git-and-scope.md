# Git and the scan scope

`src/internal_error.rs` is the one error shape every stage reports through. It
sits on its own rather than inside `execution`, which made `scan_target`,
`configuration`, `repo_hygiene` and `baseline` import back the module that
imports them, four cycles a reader had to hold for nothing.

`src/git.rs` is the bounded process layer every producer that shells out to git
runs through, and `src/git_scope.rs` is one of its callers: the scope a scan
runs under, whole workspace, changed files, changed lines, or baseline
comparison. The others are `baseline.rs`, `repo_hygiene.rs` and `git_hook.rs`,
which is why the layer lives
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

One resolved shape, one constructor per case. `ResolvedScope` is the four
cases and `ScopeReport` the accessors over it; a second enum used to mirror it
variant for variant, so a fourth scope mode was an edit in five places.
`ScopeReport::files_scope` is the only way a file scope is built: it sorts,
deduplicates and bounds, and `includes` binary-searches that order. Production
and the tests used to establish that order separately, so a third construction
site would have broken the search in silence.

The repository answers for the base. Without `--base`, `git_scope/base.rs`
takes the first of `origin/HEAD`, `origin/main`, `origin/master`, `main` and
`master` that resolves, or `HEAD` on that very branch, and `scope.base_ref`
publishes it: a skill that hard-coded `--base main` failed on every `master`
repository. A merge base missing from a shallow clone reports `shallow-clone`
rather than a bare `merge-base-unavailable`. `--scope lines` intersects a
finding's span with the new-side ranges `git_scope/lines.rs` reads from
`git diff --unified=0`, so editing one line of a large file stops surfacing the
file's backlog without the second compilation baseline pays.

A lines scope keeps what the change caused even where it has no changed line.
A span-less finding, a lockfile entry or an inherited dependency, is about its
file and stays when that file changed; `lines.rs` records a pure deletion as a
point so a file only pruned still counts as changed. Dependency declarations
now carry their key's span (`cargo_health/keys.rs`), never their identity. A
clone family stays when any member meets the scope, and `project_diagnostics`
promotes that member to primary: filtering on the first member in sorted order
hid every new copy of older code.

`--scope contextual` is the lines scope plus each rule's `diff_scope`. An
`item` rule, whose cause can sit above its site (`await_holding_lock` lands on
the guard's `let`), is kept when the innermost `fn` around it holds a changed
line or a deletion point, nested `fn` and closure bodies excluded, which
`git_scope/items.rs` parses from the judged tree. A file that does not parse
falls back to the line test, and a `line` rule is never widened. The hooks stay
on `lines` until `docs/contextual-scope-2026-10.md` is replayed over a release.

Discovery stops its upward walk at the first directory holding `.git`, so a
manifest above an unrelated repository is never scanned. With none found, it
lists manifests below through `git ls-files` or a walk, four levels deep, past
hidden, `target` and `node_modules` trees: one is scanned and named on stderr,
several fail as `ambiguous-workspace`. A Tauri root used to be a dead end.

`--staged` judges the index. `baseline/staged.rs` reads it through the one
`GIT_INDEX_FILE` a hook hands over, refuses a conflicted or locked one, and
checks it out into a private tree the way a baseline snapshot is written; every
producer reads that tree instead of the working tree, building under
`rust-doctor/staged`.

The repository pass reads what git tracks and nothing else. Besides the
secret files, now including a `.cargo/credentials(.toml)` that holds a
registry token, it reports a binary's `Cargo.lock` that exists, is untracked
and is ignored, only when the workspace is the repository's top level: a
nested workspace's ignore file is the enclosing repository's choice, which is
what every fixture of this repository is. `repo_hygiene/workflows.rs` walks
each tracked `.github/workflows/*.yml` by indentation, no YAML parser, and
reports a step that compiles with a secret in its step, job or workflow env,
abstaining on anchors and on reusable-workflow jobs.

`the_git_layer_holds_the_size_bound_it_scans_for` and
`the_scope_holds_the_size_bound_it_reports_for` keep every file of both modules
under the 1000 lines `oversized_unit` reports, tests included. That is why each
carries its tests in a file of its own.

