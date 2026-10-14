# Native rules re-measured after EP-003 (2026-10-14)

The pinned corpus replayed from the local clone cache with the fixes of EP-003
(US-014 to US-021) in the binary, toolchain 1.97.1, against the record of
commit `a0470c2`. The healthy population did not move: its ten observations,
findings digests included, are identical. Every delta below is on the agent
population.

## What moved

| Rule | Agent findings | Cause |
|---|---|---|
| `rust_doctor::source::blocking_call_in_async` | 3 to 8 | syncvibe, five calls in `pub async fn run()` of `crates/syncvibe-cli/src/app.rs` that `use syncvibe_core::models::*;` used to hide (US-015): one `std::fs::write`, four `std::process::Command` waits |
| `rust_doctor::structure::orphan_module_file` | 16 to 15 | ostendo `src/markdown/parser_tests.rs`, the adjudicated false positive, now reached |
| `rust_doctor::cargo::unused_dependency` | 5 to 17 | ostendo and vibesql no longer fail the source stage |
| `rust_doctor::cargo::test_only_dependency` | 3 to 4 | same |
| `rust_doctor::structure::duplicate_function_body` | 1476 to 1477 | ostendo `parser_tests.rs`, now walked, as test code |
| `rust_doctor::structure::near_duplicate_function_body` | 1120 to 1122 | same |

`sql_built_from_format`, `dynamic_shell_command`, `disabled_tls_verification`
and `hardcoded_credential` did not move on either population. The review's
fix, which reads the type written on a `let` whose value a format string
captures (`let limit: i64 = page.limit;`), replayed on both populations at
14:17 the same day and moved nothing either.

## The walk fix behind the orphan delta

A `#[path]` on a `mod` declared outside an inline block resolves against the
directory of the file holding it, even when that file is `src/a/b.rs` (the
Reference, "The path attribute"). The walk resolved it under `src/a/b/`, so
`#[cfg(test)] #[path = "parser_tests.rs"] mod tests;` in ostendo's
`src/markdown/parser.rs`, and `#[path = "../common/mod.rs"] mod common;` in
vibesql's `tests/update_constraint_tests/not_null_constraints.rs`, were lost.
The first is the orphan false positive the census adjudicated. Under US-020
each loss also withheld its whole package, which dropped two adjudicated true
positives, ostendo `src/code/pty.rs` and vibesql's
`select/executor/index_optimization/order_by.rs`, until the walk was fixed.
Both are reported again, and the seed
`orphan_module_file--path-in-file-module` pins the shape.

The agent orphan scope was a census of twelve. Its false positive left the
population, so its reviewed site and its pair left the record, and the census is
eleven: zero false positives, interval 0 to 2588 basis points, still
indecisive, kappa undefined for want of variance. `UNDATED_PAIRS` is 238.

## Side effects

With the source stage complete, ostendo and vibesql publish authoritative
scores, and the dependency rules reach them. Four of vibesql's twelve new
`unused_dependency` sites are false positives: `md-5` and `prettytable-rs` are
imported under their library names, `md5` and `prettytable`, and the rule only
looks for the package name. The seed `unused_dependency--library-renamed`
declares it fires with that reason; the fix belongs to the cargo rules, outside
EP-003. ostendo moves from 84 to 79, and the agent median from 83 to 81.

Six reviewed manifest sites of artifexprocal were published at line 0 while
the binary of `ac221f2` reports manifest findings at their entry line, which is
why the agent replay of that commit already failed. They are re-anchored to the
line the report locates for the same dependency, verdicts unchanged.
