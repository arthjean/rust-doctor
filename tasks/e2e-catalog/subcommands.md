# rust-doctor e2e catalog: non-scan subcommands and `rust-doctor.toml`

Binary under test: `target/release/rust-doctor` 0.7.0, built 2026-10-20 08:36 from HEAD `b5389f7`.
Every "verified: yes" row was run on 2026-10-20 against that binary, in copies under
a scratch work directory that was not kept,
with the isolated environment described in section 0. Raw logs: `runs/s01.log` ... `runs/s20.log`,
probe scripts `s01-*.sh` ... `s25.sh` in the same work directory. The live repository was never written.

Coverage abbreviations (existing Rust tests): AH `tests/agent_hooks.rs`, AL `tests/agent_loop.rs`,
ASN `tests/agent_skill_notice.rs`, EX `tests/explain_cli.rs`, PC `tests/policy_cli.rs`,
CRM `tests/ci_and_report_markdown.rs`, SC `tests/setup_cli.rs`, CK `tests/configuration_kernel.rs`,
PRC `tests/precision_controls.rs`, STK `tests/structure_kernel.rs`, SKC `tests/skill_contract.rs`,
`unit:` an in-crate `#[cfg(test)]` module. "none" means no existing test drives that path through the binary.

Scenario count: 337 (CL 13, RL 33, WY 35, RM 22, HG 30, HA 32, HR 27, SK 28, SU 19, CI 34, CF 44, WF 20). Verified empirically: 321 yes, 2 partial, 14 not run (7 of those already covered by Rust tests).

---

## 0. Harness contract (shared by every file, `e2e-helpers.ts`)

- `env -i` style spawn: `HOME=<tmp>/home` (asserted still empty at suite end), `PATH=<tmp>/bin:<cargo bin>:/usr/bin:/bin`
  where `<tmp>/bin/rust-doctor` is a symlink to the binary (the git hook and agent hooks run it by name),
  `RUSTUP_HOME`/`CARGO_HOME` passed through read-only (a fake HOME otherwise hides the toolchains),
  `CARGO_NET_OFFLINE=true`, `GIT_CONFIG_GLOBAL=<tmp>/gitconfig` (user, `init.defaultBranch=main`,
  `commit.gpgsign=false`), `GIT_CONFIG_NOSYSTEM=1`, `TERM=dumb`, one `CARGO_TARGET_DIR` per test.
  No `CLAUDECODE`, `CODEX_SANDBOX`, `CURSOR_AGENT`, `CI`, `GIT_DIR`, `GIT_INDEX_FILE` unless the test sets it.
- Agent detection is controlled by `PATH`: a `fake-agents/` dir with executable `claude`, `codex`,
  `cursor-agent` stubs (`#!/bin/sh\nexit 0`), added per test. The developer's real `~/.local/bin/claude`
  and `~/.bun/bin/codex` must never be on the test `PATH`.
- Scans pass `--advisory-db /dev/null` for determinism (the real `~/.cargo/advisory-db` is 80 days old and
  prints "The advisory database is 80 days old." otherwise).
- `treeHash(dir)`: sha1 over sorted relative paths + file contents, excluding `.git/objects` and `target/`.
  Every `--dry-run` and every refusal asserts `treeHash` unchanged.
- Fixtures built per test (never shared mutable): `clean` (one crate `e2e-clean`, edition 2024, `[workspace]`,
  `Cargo.lock`, `.gitignore` `/target`, `src/lib.rs` = `pub fn value() -> u8 {\n    1\n}\n`, initial commit on `main`);
  `twoMember` (virtual workspace `a`, `b`, `a/src/lib.rs` with `todo!()` on line 2, root `rust-doctor.toml`
  setting `clippy::todo = "error"`, `correctness = "off"`, override `b/**` todo off, override `a/src/*.rs`
  correctness error); `nested` (git root with `crates/x` workspace); `tauri` (git root, workspace in `src-tauri/`).
  Note: a module named `gen` does not compile in edition 2024 (reserved keyword); use `generated`.
- YAML: `Bun.YAML.parse` (present in bun 1.4.2) for workflow assertions. PyYAML would read the `on:` key as
  boolean `True`; Bun's YAML 1.2 keeps it a string.
- Timing reference: a scan of `clean` takes about 0.15 s warm, so the whole suite stays fast.

---

## 1. CLI surface (`cli-surface.test.ts`)

| ID | Setup | Command | Expected observable | Coverage | Verified |
|---|---|---|---|---|---|
| CL-01 | none | `--version` | stdout `rust-doctor 0.7.0\n`, exit 0; equals `Cargo.toml` `version` | none | yes |
| CL-02 | none | `<each subcommand> --help` (rules, rules list, rules explain, why, report, report markdown, setup, hook, hook install, hook install git/claude/cursor, hook run, skill, skill install, ci, ci install) | exit 0; each documented flag present (`--dry-run`, `--update`, `--shared`, `--agent` values `claude, codex, cursor, all, detected`, `--blocking` values `none, error, warning`, `--toolchain` default `1.97.1`, `--limit` default 20) | SKC (flags the skill names only) | yes |
| CL-03 | none | bare `rules`, `hook`, `hook install`, `skill`, `ci`, `report` | usage on stderr, exit 2 | none | yes |
| CL-04 | none | `skill -h` vs `skill --help` | `-h`: "Install the agent skill"; `--help`: "The skill an agent reads to drive the tool, written into the workspace." (two different abouts) | none | yes |
| CL-05 | none | `rust-doctor 'rules --help'` (one argv) | treated as PATH: "Scan failed: path is not a directory (discovery/no-manifest)", exit 2 | none | yes |
| CL-06 | dir `./setup` holding a crate | `setup --dry-run`, then `./setup --yes --json` | first runs the setup subcommand (exit 2, no agent); second scans the directory (schema 23 JSON). Help documents only `./inspect` | none | yes |
| CL-07 | none | `hook install codex` | clap "unrecognized subcommand 'codex'", tip `claude`, exit 2 | none | yes |
| CL-08 | none | `hook run codex` | "invalid value 'codex' for '<AGENT>'", values `claude, cursor`, exit 2 | none | yes |
| CL-09 | none | `skill install --agent bogus` | "invalid value 'bogus'", lists 5 values, exit 2 | none | yes |
| CL-10 | none | `rules explain $'\x1b[31mtodo'`, `why $'src/\x1b[2Jlib.rs'` | stderr carries the raw ESC byte (0x1b) of argv, exit 2. Contrast: `--rule` values are redacted (PC) | none | yes (finding F9) |
| CL-11 | none | `rules list \| head -1`, `rules list --json \| head -1`, `rules explain todo \| head -1`, `why ... --report f \| head -1`, `report markdown f \| head -1` | producer exit 0 (`PIPESTATUS[0]`), no stderr | none | yes |
| CL-12 | full suite | after every test | fake `HOME` still empty: no subcommand writes `~/.claude`, `~/.cursor`, `~/.agents` | none | yes |
| CL-13 | none | `rules list`, `rules explain todo`, `report markdown f`, `why x:1 --report f` with `PATH=""` | all succeed: none spawns a process | EX, CRM (partial) | yes (explain, why, markdown) |

## 2. `rules list` / `rules explain` (`rules.test.ts`)

| ID | Setup | Command | Expected observable | Coverage | Verified |
|---|---|---|---|---|---|
| RL-01 | empty dir | `rules list` | 142 lines, each 4 TAB-separated fields `id, category, tier, help`; first line `clippy::absurd_extreme_comparisons\tcorrectness\tP1\t...`; exit 0 | PC `rules_list_without_json_stays_line_oriented` | yes |
| RL-02 | none | `rules list` prefixes | 104 `clippy`, 13 `rust_doctor::cargo`, 5 `rust_doctor::repo`, 11 `rust_doctor::source`, 9 `rust_doctor::structure` | PC (catalog) | yes |
| RL-03 | none | `rules list --json` | array of 142; keys exactly `category, default_level, diff_scope, help, id, not_a_defect_when, producer, requires, tier, title`; tiers P0 3 / P1 81 / P2 25 / P3 33; every `default_level` `warn`; `diff_scope` line 126 / item 14 / file 2; producers `clippy` 104, `cargo-health` 13, `source-kernel` 11, `structure` 9, `repo` 5 | PC `rules_list_publishes_the_shipped_catalog` | yes |
| RL-04 | none | `rules list --json` vs README count and `skills/rust-doctor/SKILL.md` "142 curated rules" | the three numbers agree | SKC (skill) | yes (142) |
| RL-05 | none | `rules list extra` | "unexpected argument 'extra' found", exit 2 | none | yes |
| RL-06 | none | `rules list --json --json` | "cannot be used multiple times", exit 2 | none | yes |
| RL-07 | none | last `::` segment of every id | all unique, so the `Ambiguous` error branch of `rules explain` is unreachable from the CLI today (assert uniqueness so a future collision is noticed) | unit `src/explain/tests.rs` | yes |
| RL-08 | none | `rules explain todo` | stdout lines: `clippy::todo`, `Producer: clippy`, `Category: correctness`, `Tier: P2`, `Default level: warn`, `Help: Replace todo! ...`, `Rule: https://rust-doctor.com/rules/clippy%3A%3Atodo`, `Corpus noise: unmeasured`; no `Effective level` line; exit 0 | EX | yes |
| RL-09 | none | `rules explain clippy::todo --json` | catalog entry flattened + `url`, `corpus_noise_basis_points: null`, `corpus_reviewed_sites: null`; no `effective` key | none | yes |
| RL-10 | none | `rules explain clippy::exit [--json]` | `Corpus noise: 86% on 5 adjudicated sites`; JSON 8571 / 5 | none | yes |
| RL-11 | none | `rules explain rust_doctor::source::dynamic_shell_command` | `Producer: source-kernel`, `Tier: P0`, URL percent-encodes every `::` | none | yes |
| RL-12 | none | `rules explain tood` | stderr `rust-doctor: no catalogued rule is named `tood`; did you mean `clippy::todo`?`, stdout empty, exit 2 | EX | yes |
| RL-13 | none | `rules explain zzzzzzzzzzzz` | stderr `...; `rust-doctor rules list` prints them all.`, exit 2 | none | yes |
| RL-14 | none | `rules explain ""` | same "prints them all" message with empty backticks, exit 2 | none | yes |
| RL-15 | none | `rules explain` (no RULE) | clap "required arguments were not provided: <RULE>", exit 2 | none | yes |
| RL-16 | twoMember | `rules explain todo .` | adds `Effective level: error (config-rule)`, `Override: off under b/**`, `Override: error under a/src/*.rs` (table order); exit 0 | EX (first two lines) | yes |
| RL-17 | twoMember | `rules explain todo . --json` | `effective = {level: error, source: config-rule, overrides: [{paths:[b/**], rules:{clippy::todo: off}, categories:{}}, {paths:[a/src/*.rs], rules:{}, categories:{correctness: error}}]}` | none | yes |
| RL-18 | twoMember | `rules explain absurd_extreme_comparisons .` | `Effective level: off (config-category)` and `Override: error under a/src/*.rs`, although the override cannot switch the rule on (see CF-11): the text does not say so | none | yes (finding F12) |
| RL-19 | twoMember | `rules explain clippy::exit .` | `Effective level: warn (default)`, no Override line | none | yes |
| RL-20 | twoMember | `rules explain todo a` (member dir) | `Effective level: warn (default)`; the scan of `a` reports `clippy::todo` at `error` / `config-rule` (root config). Mismatch | none | yes (finding F4) |
| RL-21 | twoMember + `b/rust-doctor.toml` setting todo off | `rules explain todo b` | `Effective level: off (config-rule)`, a file the scan never reads (CK `only_the_workspace_root_configuration_is_consulted`) | none | yes (finding F4) |
| RL-22 | none | `rules explain todo /nonexistent/path` | exit 0, `Effective level: warn (default)`: a mistyped PATH gives a confident wrong answer | none | yes (finding F5) |
| RL-23 | twoMember | `rules explain todo Cargo.toml` (PATH is a file) | stderr `rust-doctor: Could not read rust-doctor.toml.`, exit 2 | none | yes |
| RL-24 | dir with `[rules]\n"clippy::todo" = "warning"` | `rules explain todo DIR` | stderr `rust-doctor: Invalid rust-doctor.toml at line 3, column 18.` (3-line file), exit 2, stdout empty | none | yes |
| RL-25 | `nope = 1` | same | `Invalid rust-doctor.toml at line 1, column 1.`, exit 2 | none | yes |
| RL-26 | `[rules]\n"clippy::nope" = "off"` | same | `Unknown rule selector in rust-doctor.toml.`, exit 2 | none | yes |
| RL-27 | `[ignore]\npaths = ["/abs"]` | same | `Invalid glob in rust-doctor.toml at ignore.paths[0].`, exit 2 | none | yes |
| RL-28 | `[structure]\ncyclomatic-threshold = 0` | same | `Structure thresholds in rust-doctor.toml must be between 1 and 10000.`, exit 2 | none | yes |
| RL-29 | 65537-byte config | same | `rust-doctor.toml exceeds 65536 bytes.`, exit 2 | none | yes |
| RL-30 | config bytes `\xff\xfe` | same | `rust-doctor.toml must be UTF-8.`, exit 2 | none | yes |
| RL-31 | `rust-doctor.toml` is a directory | same | `rust-doctor.toml is not a regular file.`, exit 2 | none | yes |
| RL-32 | empty dir | `rules explain todo DIR` | `Effective level: warn (default)`, exit 0 | none | yes |
| RL-33 | twoMember | `rules explain todo .` then `treeHash`, config mtime | no write, config mtime unchanged | CK (scan side only) | no |

## 3. `why` (`why.test.ts`)

| ID | Setup | Command | Expected observable | Coverage | Verified |
|---|---|---|---|---|---|
| WY-01 | twoMember | `why a/src/lib.rs:2` | `clippy::todo (error)`, `Level source: config-rule`, message, `Help:`, `Rule:`, frame header `a/src/lib.rs:2:5`, `> 2 \|     todo!()`, caret line `^^^^^^^`, `To suppress this site: #[expect(clippy::todo, reason = "<why this site is not a defect>")]`; stderr empty; exit 0 | EX | yes |
| WY-02 | twoMember | `why a/src/lib.rs:2:5 --json` | `location {path, line 2, column 5}`, `hits[0]` keys `rule_id, severity, level_source, message, help, url, path, span{line_start,column_start,line_end,column_end}, suppressions[], directive`; no `absence` key | EX | yes |
| WY-03 | twoMember | `why ./a/src/lib.rs:2` | `./` stripped, same hit | none | yes |
| WY-04 | twoMember, cwd `a/src` | `why lib.rs:2` | resolves to `a/src/lib.rs` relative to the workspace root, same hit | none | yes |
| WY-05 | twoMember | `why <absolute>/a/src/lib.rs:2` | same hit | none | yes |
| WY-06 | twoMember | `why a/src/lib.rs:6` | stdout exactly `Nothing is reported at a/src/lib.rs:6.\n` | EX | yes |
| WY-07 | twoMember | `why a/src/lib.rs:999` | `Nothing is reported at a/src/lib.rs:999.`, exit 0 (no bounds check) | none | yes |
| WY-08 | twoMember | `why b/src/lib.rs:2` | `Nothing is reported` (override `b/**` todo off) | none | yes |
| WY-09 | twoMember, saved `--json` report | `why a/src/lib.rs:2 --report R` with `PATH=""` | byte-identical to WY-01 text (452 bytes) | EX (JSON only) | yes |
| WY-10 | same | `why a/src/lib.rs:2 --report R --json` | same hit, `location.column: null` | EX | yes |
| WY-11 | same, cwd `/` | `why a/src/lib.rs:2 --report R` | hit still matched by the path as written; root falls back to `.` | none | yes |
| WY-12 | `{"a":1}` | `why a/src/lib.rs:2 --report notareport.json` | exit 0, `Nothing is reported at a/src/lib.rs:2.`: no schema check, unlike `report markdown` (RM-09) | none | yes (finding F6) |
| WY-13 | report with `schema_version: 24` | `why ... --report future.json` | accepted, hit printed (RM-11 refuses the same file) | none | yes (finding F6) |
| WY-14 | none | `why x:2 --report /nonexistent.json` | stderr `rust-doctor: /nonexistent.json: the report could not be read (entity not found).`, exit 2 | none | yes |
| WY-15 | empty file | `why x:2 --report empty.json` | `...: the report is not valid JSON (line 1, column 0).`, exit 2 | none | yes |
| WY-16 | none | `why` with `a/src/lib.rs:0`, `:-1`, no `:LINE`, `:2:0`, `:x`, `:2:x`, `:2` (empty path) | stderr `rust-doctor: why <arg>: expected PATH:LINE[:COLUMN] with a positive LINE.`, stdout empty, exit 2 (7 cases) | EX (3 cases) | yes |
| WY-17 | report | `why a/src/lib.rs:2:3:4 --report R` | parsed as path `a/src/lib.rs:2`, line 3, column 4: `Nothing is reported at a/src/lib.rs:2:3.`, exit 0 | none | yes |
| WY-18 | report | `why a/src/we:ird.rs:3 --report R` | colon kept in path, `Nothing is reported at a/src/we:ird.rs:3.` | none | yes |
| WY-19 | twoMember | `why a/src/nope.rs:1` | `rust-doctor: why a/src/nope.rs: the file cannot be read.`, exit 2 | none | yes |
| WY-20 | file outside any package | `why notareport.json:1` | `rust-doctor: why notareport.json: no Cargo package holds this file.`, exit 2 | none | yes |
| WY-21 | twoMember | `why Cargo.toml:1` (virtual root manifest) | `no Cargo package holds this file.`, exit 2: workspace-level manifest findings cannot be explained live, only with `--report` | none | yes |
| WY-22 | twoMember | `why a/Cargo.toml:1` | scans member `a`, `Nothing is reported at a/Cargo.toml:1.` | none | yes |
| WY-23 | workspace with `nope = 1` config | `why src/lib.rs:4` | `rust-doctor: why src/lib.rs: the scan failed: Invalid rust-doctor.toml at line 1, column 1.`, exit 2 | none | yes |
| WY-24 | `[ignore] paths = ["src/vendored"]` | `why src/vendored/mod.rs:2` live and `--report` | `src/vendored/mod.rs is ignored by rust-doctor.toml: nothing there is reported.` | none | yes |
| WY-25 | file opening `// @generated by build tooling. DO NOT EDIT.` | `why src/out/mod.rs:3` live and `--report` | `src/out/mod.rs is declared generated or vendored: nothing there is graded.` | none | yes |
| WY-26 | committed `.gitattributes` `src/out/** linguist-generated`, no header | live `why src/out/mod.rs:2` | `declared generated or vendored` | none | yes |
| WY-27 | same, `--report` of a scan whose `scan.excluded_generated` is 1 | `why src/out/mod.rs:2 --report R` | `Nothing is reported at src/out/mod.rs:2.`: live and saved answers differ although the module doc says both answer from the same document | none | yes (finding F13) |
| WY-28 | `.gitattributes` marks an untracked file generated | live `why` | the file is graded and its finding shown (attributes apply to `git ls-files` only) | none | yes |
| WY-29 | `#[expect(clippy::todo, reason = "...")]` over a `todo!()` | `why src/lib.rs:9` | `Nothing is reported at src/lib.rs:9.`; the `#[expect]` is not named, and `suppressions[]` does not list it | none | yes |
| WY-30 | `// rust-doctor: allow(rust_doctor::source::dynamic_shell_command) -- reason` above a site | `why src/shell.rs:4 --report R` | `Nothing is reported at src/shell.rs:4.`; the applied directive in `suppressions[]` (status `applied`) is not mentioned | none | yes (finding F14) |
| WY-31 | directive without `-- reason` | `why src/shell.rs:8 --report R` | hit + `Suppression at src/shell.rs:7: rust_doctor::source::dynamic_shell_command (missing-reason)`, no `To suppress` line | unit | yes |
| WY-32 | report whose diagnostic message holds ESC / newline | `why ... --report R` | ESC stripped (sanitize), message multiline-safe | unit | no |
| WY-33 | native finding in `Cargo.toml` | `why Cargo.toml:N --report R` | directive suggestion uses `# rust-doctor: allow(<id>) -- <why ...>` | unit | no |
| WY-34 | twoMember, `CLAUDECODE=1` | `why ... --report R` | stderr empty (no skill notice) | none | yes |
| WY-35 | twoMember | live `why` | stderr empty: no progress lines, no interactive report | none | yes |

## 4. `report markdown` (`report-markdown.test.ts`)

| ID | Setup | Command | Expected observable | Coverage | Verified |
|---|---|---|---|---|---|
| RM-01 | full `--json` report of twoMember | `report markdown R` | `## rust-doctor`, `**Score: 94/100 (Great)**, authoritative.`, table header `\| Rule \| Location \| Severity \| Message \|`, linked rule, `a/src/lib.rs:2`, `error`, backticks escaped `\``, footer `1 finding.`; exit 0 | CRM golden | yes |
| RM-02 | same | `--limit 0` | no table, `Showing 0 of 1 findings.` | none | yes |
| RM-03 | same | `--limit 1000` | accepted | none | yes |
| RM-04 | same | `--limit 1001` | `invalid value '1001' for '--limit <N>': 1001 is not in 0..=1000`, exit 2 | none | yes |
| RM-05 | same | `--limit -1` | `unexpected argument '-1' found`, tip `-- -1`, exit 2 | none | yes |
| RM-06 | same | `--limit abc` | `invalid digit found in string`, exit 2 | none | yes |
| RM-07 | none | `report markdown /nonexistent.json` | `rust-doctor: /nonexistent.json: the report could not be read (entity not found).`, exit 2 | none | yes |
| RM-08 | none | `report markdown <dir>` | `...: the report could not be read (is a directory).`, exit 2 | none | yes |
| RM-09 | `{"a":1}` | render | `the report carries no schema_version, and this binary reads 23.`, exit 2 | none | yes |
| RM-10 | `{"schema_version":23}` | render | `the file is not a rust-doctor report.`, exit 2 | none | yes |
| RM-11 | report with schema 24 | render | `the report has schema_version 24, and this binary reads 23: render it with the rust-doctor that wrote it.`, exit 2 | CRM (17) | yes |
| RM-12 | schema 19 / 20 | render | 19 refused (exit 2), 20 accepted (oldest readable) | none | yes |
| RM-13 | empty file | render | `not valid JSON (line 1, column 0)`, exit 2 | CRM (truncated JSON) | yes |
| RM-14 | report piped | `report markdown -` | `-: the report could not be read (entity not found).`: `-` is not stdin | none | yes |
| RM-15 | report piped | `report markdown /dev/stdin` | renders | none | yes |
| RM-16 | 64 MiB + 1 sparse file | render | `larger than 64 MiB`, stdout empty, exit 2 | CRM | no (covered) |
| RM-17 | report whose message is `<img src=x onerror=alert(1)> \| pipe \| \`code\` [link](http://evil) \x1b[31mred\n@everyone` and path `a/src/<b>x</b>\|.rs` | render | `&lt;img ...&gt;`, `\|` escaped, `\``, `\[link\]` with `&#8203;` breaking its `http:` URL, ESC removed, newline flattened, `@&#8203;everyone`; path HTML-escaped and pipe-escaped | unit | yes |
| RM-18 | report of a failed scan (invalid config) | render | `**Score: none**, not authoritative.`, `**The scan reported 1 error:**`, `- configuration config-invalid: Invalid rust-doctor.toml at line 1, column 1.`, `No finding was collected.`, exit 0 | CRM (scope-failure golden) | yes |
| RM-19 | baseline report from WF-13 | render | `**Against the base:** 1 introduced, 0 fixed.`, one row, `1 introduced finding.` | CRM golden | yes |
| RM-20 | none | render `\| head -1` | exit 0 | none | yes |
| RM-21 | fake `cargo git rustc clippy-driver rustup npm gh` on PATH that leave marks | render | nothing marked | CRM | no (covered) |
| RM-22 | every refusal above | render | stdout empty | CRM | yes |

## 5. `hook install git` (`hook-git.test.ts`)

| ID | Setup | Command | Expected observable | Coverage | Verified |
|---|---|---|---|---|---|
| HG-01 | clean | `hook install git --dry-run` | stdout `Would write .git/hooks/pre-commit:\n\n` + script; exit 0; `treeHash` unchanged; no file | AH | yes |
| HG-02 | clean | `hook install git` | `Wrote .git/hooks/pre-commit:` + script; file mode `0755`; content exactly the 8-line script with `exec rust-doctor . --yes --staged --scope lines`; `sh -n` passes | AH | yes |
| HG-03 | after HG-02 | rerun | `The rust-doctor pre-commit hook is already installed in .git/hooks/pre-commit.`, exit 0, tree unchanged | AH | yes |
| HG-04 | after HG-02 | rerun with `--update`, with `--dry-run` | same message, exit 0 | AH (`--update`) | yes |
| HG-05 | clean, `PATH` without rust-doctor | install | file written; stderr `Warning: `rust-doctor` is not on PATH, and the hook runs it by name.`; exit 0 | none | yes |
| HG-06 | hook with marker and `exec rust-doctor . --yes --staged` | install | `.git/hooks/pre-commit holds an outdated rust-doctor pre-commit hook: pass --update to rewrite it.`, exit 1, file unchanged | AH | yes |
| HG-07 | same | `--update --dry-run` | `Would update .git/hooks/pre-commit: `rust-doctor . --yes --staged` becomes `rust-doctor . --yes --staged --scope lines`.` + `Would write`; exit 0; file unchanged | none | yes |
| HG-08 | same | `--update` | `Updating ...` + `Wrote ...`, exit 0 | AH | yes |
| HG-09 | foreign `#!/bin/sh\nmake lint\n` | install | stdout `... exists and was not written by rust-doctor, so nothing was written.` + `Add this line to it:` + command; exit 1; file unchanged | AH | yes |
| HG-10 | same | `--update` | same stdout, exit 2 | AH | yes |
| HG-11 | same | `--dry-run` | exit 1 | none | yes |
| HG-12 | marker kept, user line `make lint` added, same exec | install / `--update` | treated as outdated (exit 1); `--update` prints "`X` becomes `X`" with identical commands and silently drops the user's line | none | yes (finding F15) |
| HG-13 | `.git/hooks/pre-commit` dangling symlink to an outside dir | install | stderr `rust-doctor: .git/hooks/pre-commit is a symlink, and nothing is written through one.`, exit 2, target dir empty | none (unit for skill/ci) | yes |
| HG-14 | `.git/hooks` symlinked outside | install | `The hooks directory is outside this working tree, so nothing was written.` + line, exit 1 | none | yes |
| HG-15 | `core.hooksPath=.githooks` | install | writes `.githooks/pre-commit`; `.git/hooks/pre-commit` absent | AH | yes |
| HG-16 | `core.hooksPath=<outside dir>` | install | exit 1, outside message, shared dir empty | AH | yes |
| HG-17 | `.cargo-husky/hooks/` and `.pre-commit-config.yaml` | install | husky wins: `Wrote .cargo-husky/hooks/pre-commit` | AH (husky alone) | yes |
| HG-18 | `.pre-commit-config.yaml` | install | `.pre-commit-config.yaml runs this repository's hooks, and rust-doctor never edits it.` + `- repo: local ... entry: rust-doctor . --yes --staged --scope lines ... pass_filenames: false`; exit 0; nothing written; config byte-identical | AH | yes |
| HG-19 | each of `lefthook.yml`, `lefthook.yaml`, `.lefthook.yml`, `.lefthook.yaml` | install | `pre-commit:\n  commands:\n    rust-doctor:\n      run: ...`, exit 0 | AH | yes (one) |
| HG-20 | lefthook | `--dry-run --update` | same snippet, exit 0 | none | yes |
| HG-21 | crate, no git | install | stderr `rust-doctor: not a git repository, so no pre-commit hook was installed.`, exit 2 | AH | yes |
| HG-22 | none | `hook install git /nonexistent/dir` | same, exit 2 | none | yes |
| HG-23 | linked worktree of a clean repo | `hook install git <worktree>` | exit 1, outside-working-tree message (hooks live in the common dir) | none | yes |
| HG-24 | bare repo | install | exit 2, not a git repository | none | yes |
| HG-25 | nested (`crates/x`) | `hook install git <abs>/crates/x --dry-run` | script `exec rust-doctor crates/x --yes --staged --scope lines` | none | yes |
| HG-26 | nested, cwd = repo root | `hook install git crates/x` and `./crates/x` | exit 2 `not a git repository`: relative path resolved twice (`current_dir(ws)` + `-C ws`) | none | yes (finding F1) |
| HG-27 | nested, cwd `crates/x` | `hook install git ..` / `../..` | `..` works by accident (`exec rust-doctor crates`), `../..` fails `not a git repository` | none | yes (finding F1) |
| HG-28 | repo with workspace `my app` | `hook install git "<abs>/my app" --dry-run` | `exec rust-doctor 'my app' --yes --staged --scope lines` | unit `the_hook_runs_a_staged_lines_scan_of_the_workspace` | yes |
| HG-29 | `GIT_DIR`/`GIT_INDEX_FILE` pointing at another repo in env | install | hook lands in the workspace repo (git calls strip them) | none | no |
| HG-30 | every refusal above | `treeHash` | unchanged | partial (AH) | yes |

## 6. `hook install claude|cursor` (`hook-agent-install.test.ts`)

| ID | Setup | Command | Expected observable | Coverage | Verified |
|---|---|---|---|---|---|
| HA-01 | clean | `hook install claude --dry-run` | `Would add a Stop hook to .claude/settings.local.json: {"hooks":[{"command":"rust-doctor hook run claude","timeout":600,"type":"command"}]}`, exit 0, no `.claude` | AH | yes |
| HA-02 | clean | `hook install claude` | `Added a Stop hook ...`; file is pretty JSON + trailing newline `{"hooks":{"Stop":[{"hooks":[{"command":"rust-doctor hook run claude","timeout":600,"type":"command"}]}]}}` | AH | yes |
| HA-03 | after HA-02 | rerun | `The rust-doctor Stop hook is already in .claude/settings.local.json.`, exit 0, bytes unchanged | AH | yes |
| HA-04 | after HA-02 | rerun `--update` | same, exit 0 | none | yes |
| HA-05 | after HA-02 | `--shared` | writes `.claude/settings.json` too; rerun says already | AH | yes |
| HA-06 | our hook + `statusMessage`, top-level `model` | install | still current (extra keys tolerated), exit 0 | none | yes |
| HA-07 | our hook with `timeout: 60` | install | `... is outdated: pass --update to rewrite it.`, exit 1 | AH | yes |
| HA-08 | same | `--update --dry-run` | `Would update the Stop hook in ...: {...60...} becomes {...600...}`, file unchanged | none | yes |
| HA-09 | same | `--update` | `Updated ...`; `statusMessage` and `model` kept | AH | yes |
| HA-10 | command `rust-doctor hook run claude --verbose` | install / `--update` | outdated (exit 1); update resets the command | none | yes |
| HA-11 | foreign Stop matcher group `echo hi` | install | appended as a second group, foreign kept | AH | yes |
| HA-12 | settings `[]`, `{"hooks":[]}`, `{"hooks":{"Stop":{}}}`, `{"hooks":{"Stop":"x"}}`, `"str"`, `null` | install | stderr `rust-doctor: .claude/settings.local.json does not hold the shape its agent reads: nothing was written.`, exit 2 | unit | yes |
| HA-13 | `{"hooks": {,}}` | install | stderr `Could not parse .claude/settings.local.json at line 1, column 12: nothing was written.` (no `rust-doctor:` prefix), exit 2, bytes unchanged | AH | yes |
| HA-14 | empty file | install | parse error line 1 column 0, exit 2 | none | yes |
| HA-15 | `// comment\n{}` | install | parse error line 1 column 1, exit 2 | none | yes |
| HA-16 | `{"a":1,"a":2}` | install | accepted; rewritten with `"a": 2` only | none | yes |
| HA-17 | `{"version":2,"hooks":{"afterFileEdit":[...]}}` (cursor) or keys in non-alphabetical order | install | merge keeps every key but rewrites the file with keys sorted and 2-space indentation | none | yes (finding F16) |
| HA-18 | settings path is a directory | install | `could not be read: Is a directory (os error 21). Nothing was written.`, exit 2 | none | yes |
| HA-19 | settings mode 000 | install | `could not be read: Permission denied (os error 13). Nothing was written.`, exit 2 | none | yes |
| HA-20 | settings mode 444, or `.claude/` mode 555 with no settings | install | stdout `Added a Stop hook ...` then stderr `could not be written: Permission denied (os error 13).`, exit 2 | none | yes (finding F7) |
| HA-21 | settings file is a symlink to an outside file | install | `rust-doctor: .claude/settings.local.json is a symlink, ...`, exit 2, target unchanged | none | yes |
| HA-22 | `.claude` dir is a symlink | install | exit 2, `.claude is a symlink` | AH | no (covered) |
| HA-23 | clean | `hook install cursor --dry-run` / install | `Would add` / `Added a stop hook to .cursor/hooks.json: {"command":"rust-doctor hook run cursor"}`; file `{"hooks":{"stop":[{"command":"rust-doctor hook run cursor"}]},"version":1}` | AH | yes |
| HA-24 | after HA-23 | rerun | `The rust-doctor stop hook is already in .cursor/hooks.json.` | none | yes |
| HA-25 | our cursor entry + `timeout: 5` | install | current, exit 0 | none | yes |
| HA-26 | `{"version":2,"hooks":{"afterFileEdit":[{"command":"x"}]}}` | install | `version` stays 2, `afterFileEdit` kept, `stop` added | none | yes |
| HA-27 | cursor entry `rust-doctor hook run cursor --old` | install / `--update` | exit 1 outdated / `Updated the stop hook in .cursor/hooks.json: {...--old} becomes {...}` | none | yes |
| HA-28 | none | `hook install cursor --shared` | clap `unexpected argument '--shared'`, exit 2 | none | yes |
| HA-29 | none | `hook install claude <nonexistent>/does/not/exist` | exit 0; creates `does/not/exist/.claude/settings.local.json` and every parent | none | yes (finding F5) |
| HA-30 | clean, `PATH` without rust-doctor | install claude, install cursor | file written, stderr not-on-PATH warning, exit 0 | none | yes |
| HA-31 | nested | `hook install claude crates/x` (relative) | works (no git involved), writes `crates/x/.claude/settings.local.json` | none | yes |
| HA-32 | every refusal | `treeHash` | unchanged | partial | yes |

## 7. `hook run claude|cursor` (`hook-run.test.ts`)

Realistic Claude Code Stop payload: `{"session_id":"e2e-session","transcript_path":"<home>/t.jsonl","cwd":"<repo>","permission_mode":"default","hook_event_name":"Stop","stop_hook_active":false}`.
Cursor payload: `{"status":"completed","loop_count":0,"conversation_id":"c","generation_id":"g","hook_event_name":"stop","workspace_roots":["<repo>"]}`.

| ID | Setup | Command | Expected observable | Coverage | Verified |
|---|---|---|---|---|---|
| HR-01 | clean, `CLAUDE_PROJECT_DIR=repo` | `hook run claude` < payload | exit 0, stdout and stderr empty | AH | yes |
| HR-02 | `todo!()` added on line 6 of tracked `src/lib.rs`, unstaged | same | exit 2; stderr exactly `rust-doctor found 1 finding in the lines this turn changed:\n- clippy::todo src/lib.rs:6 `todo` should not be present in production code\nFix them, then rescan: rust-doctor . --scope lines --base HEAD --include-untracked --blocking warning\n` | AH | yes |
| HR-03 | same | payload `stop_hook_active: true` | exit 0 silent; no `CARGO_TARGET_DIR` created | AH | yes |
| HR-04 | same, `CLAUDE_PROJECT_DIR` unset, cwd repo | run | scans `.`, exit 2 | none | yes |
| HR-05 | same, `CLAUDE_PROJECT_DIR=""` | run | scans `.`, exit 2 | none | yes |
| HR-06 | cwd `/`, `CLAUDE_PROJECT_DIR=repo` | run | scans the project dir, exit 2 | none | yes |
| HR-07 | cwd `/`, no `CLAUDE_PROJECT_DIR` | run | stderr `rust-doctor could not scan (discovery: ambiguous-workspace); not blocking this turn.`, exit 0 | none | yes |
| HR-08 | HR-02 state | stdin `not json` | scans, exit 2 | AH (cursor) | yes |
| HR-09 | HR-02 state | empty stdin | scans, exit 2 | none | yes |
| HR-10 | HR-02 state | `{"stop_hook_active":"true"}` (string) | not a guard: scans, exit 2 | unit | yes |
| HR-11 | HR-02 state | 1.1 MB payload starting `{"stop_hook_active": true, "pad": ...}` | stdin cut at 1 MiB, JSON unparsable, guard lost: scans and exits 2 | none | yes (finding F17) |
| HR-12 | HR-02 + `blocking = "none"` | run | still exit 2: the hook forces `--blocking warning` over the config | none | yes |
| HR-13 | HR-02 + `[rules] "clippy::todo" = "off"` | run | exit 0 | none | yes |
| HR-14 | HR-02 + `nope = 1` | run | `rust-doctor could not scan (configuration: config-invalid); not blocking this turn.`, exit 0 | none | yes |
| HR-15 | untracked `src/many.rs` with 12 `todo!()` | run | `found 12 findings`, 10 bullet lines, `- and 2 more`, rescan line; exit 2 | AH (untracked, 1 finding) | yes |
| HR-16 | `todo!()` committed, working tree clean | run | exit 0 | none | yes |
| HR-17 | `git init` without commits | run | `(scope: base-unavailable)`, exit 0 | none | yes |
| HR-18 | crate outside git | run | `(scope: base-unavailable)`, exit 0 | AH | yes |
| HR-19 | `CLAUDE_PROJECT_DIR=<nonexistent>` | run | `(discovery: no-manifest)`, exit 0 | none | yes |
| HR-20 | HR-15 state | `hook run cursor` < completed/0 | stdout one line `{"followup_message":"rust-doctor found 12 findings ..."}`, stderr empty, exit 0 | AH | yes |
| HR-21 | same | `status` `aborted` / `error`, `loop_count: 1` | exit 0, stdout empty, no scan target created | AH | yes |
| HR-22 | same | `{}` | scans, follow-up JSON | AH (malformed) | yes |
| HR-23 | same, `CLAUDE_PROJECT_DIR=/` | `hook run cursor` | ignores it, scans cwd | none | yes |
| HR-24 | finding only in a test module / at `info` severity | run | not counted as blocking (`blocks()` filter), exit 0 | none | no |
| HR-25 | any | run, then `treeHash(repo)` | unchanged: the hook writes only under `CARGO_TARGET_DIR` | none | yes |
| HR-26 | any | stdin is a TTY (pty) | does not wait for input, scans | none | no |
| HR-27 | `CLAUDECODE=1` | run | no skill notice on stderr (notice is scan-CLI only) | none | yes (indirect) |

## 8. `skill install` (`skill-install.test.ts`)

| ID | Setup | Command | Expected observable | Coverage | Verified |
|---|---|---|---|---|---|
| SK-01 | clean | `skill install --dry-run` | `Would write .claude/skills/rust-doctor/SKILL.md`, `Would write .claude/skills/rust-doctor/references/expert-review.md`; tree unchanged | unit | yes |
| SK-02 | clean | `--agent all --dry-run` | six `Would write` lines (`.claude`, `.agents`, `.cursor`); tree unchanged | unit | yes |
| SK-03 | clean | `skill install` | stdout the two relative paths; `SKILL.md` = shipped file with line 4 `<!-- Installed by rust-doctor 0.7.0. Refresh with `rust-doctor skill install --update`. -->`; `expert-review.md` byte-identical to `skills/rust-doctor/references/` | unit | yes |
| SK-04 | after SK-03 | rerun | stderr `rust-doctor: the skill was not installed, .claude/skills/rust-doctor already exists; pass --update to refresh it.`, exit 1 | unit | yes |
| SK-05 | after SK-03 | `--update` | `Replaced rust-doctor 0.7.0 with 0.7.0 in .claude/skills/rust-doctor.` + paths, exit 0 | none | yes |
| SK-06 | after SK-03 | `--update --dry-run` | `Would replace rust-doctor 0.7.0 with 0.7.0 ...` + `Would write` x2 | none | yes |
| SK-07 | stamp changed to 0.1.0 + extra line | `--update` | `Replaced rust-doctor 0.1.0 with 0.7.0 ...`; extra line gone | unit | yes |
| SK-08 | stamp line removed | `--update` | `Replaced a copy that recorded no version with 0.7.0 ...` | none | yes |
| SK-09 | `---\nname: other\n---\nbody\n` | `--update` | `... holds another skill: nothing was written.`, exit 1, bytes unchanged | unit | yes |
| SK-10 | skill dir without `SKILL.md` | `--update` | same foreign refusal, exit 1 | none | yes |
| SK-11 | stamp `0.1\x1b[2J` | `--update` | read as no recorded version (no ESC printed) | unit | yes |
| SK-12 | extra `NOTES.md` in the skill dir | `--update` | kept | none | yes |
| SK-13 | `.claude` copy present | `--agent all` | exit 1 already exists; `.agents` and `.cursor` not created | unit | yes |
| SK-14 | same | `--agent all --update` | claude replaced, the two others created | none | yes |
| SK-15 | after SK-14 | `--agent codex` | exit 1 `.agents/skills/rust-doctor already exists` | none | yes |
| SK-16 | empty dir, no agent on PATH | `--agent detected` | stderr `... no agent was detected: the `claude`, `codex` and `cursor-agent` commands on PATH, and the .claude/, .agents/, .codex/ and .cursor/ directories of the workspace.`, exit 2 | SC | yes |
| SK-17 | `.codex/` dir | `--agent detected --dry-run` | `.agents/skills/rust-doctor/...` | none | yes |
| SK-18 | `.agents/` dir | same | `.agents/...` | none | yes |
| SK-19 | fake `cursor-agent` on PATH | same | `.cursor/...` | none | yes |
| SK-20 | `.cursor/` dir | `--agent detected` | `.cursor/...` written only | SC | no (covered) |
| SK-21 | none | `skill install <nonexistent>/deep` | exit 0, creates the whole tree | none | yes (finding F5) |
| SK-22 | `.claude` symlink | install | `... .claude is a symlink, and nothing is written through one.`, exit 1, target empty | unit | yes |
| SK-23 | `.claude/skills/rust-doctor` symlink | `--update` | exit 1 symlink refusal | none | yes |
| SK-24 | `.claude/skills` mode 555 | install | `rust-doctor: the skill was not installed, permission denied.`, exit 1 (hook and ci exit 2 on a write failure) | none | yes (finding F8) |
| SK-25 | any | install | fake `HOME` untouched (project-level only) | none | yes |
| SK-26 | `CLAUDECODE=1`, no skill | scan text | stderr `rust-doctor: no rust-doctor skill is installed for claude; run `rust-doctor skill install --agent claude`.` | ASN | yes |
| SK-27 | `CODEX_SANDBOX=1` / `CURSOR_AGENT=1` | scan text | same line naming codex / cursor | none | yes |
| SK-28 | `CLAUDECODE=1` with `--json`; `CLAUDECODE=""` | scan | no notice | ASN (json) | yes |

## 9. `setup` (`setup.test.ts`)

`NOAG` PATH = binary dir + cargo + `/usr/bin:/bin`; `ALL` adds fake `claude`, `codex`, `cursor-agent`.

| ID | Setup | Command | Expected observable | Coverage | Verified |
|---|---|---|---|---|---|
| SU-01 | clean, NOAG | `setup` | stderr `rust-doctor: no agent detected, so nothing was set up. Probed ...`, exit 2, tree unchanged | SC | yes |
| SU-02 | clean + `.claude/`, NOAG | `setup --dry-run` | stdout `Detected: claude.`, `Plan:`, two `Would write` skill lines, `Would write .git/hooks/pre-commit:` + script, `Would add a Stop hook ...`, last line `setup never adds a CI workflow: run `rust-doctor ci install` for GitHub Actions.`; tree unchanged | SC (codex) | yes |
| SU-03 | same | `setup` | plan, then `Writing:` with `.claude/skills/...` x2, `Wrote .git/hooks/pre-commit:`, `Added a Stop hook ...`, NO_CI line; exactly 4 new files | SC (codex) | yes |
| SU-04 | after SU-03 | rerun | exit 0; `Would replace rust-doctor 0.7.0 with 0.7.0 ...`, `already installed`, `already in` (twice: plan and write); tree unchanged | SC | yes |
| SU-05 | clean, ALL | `setup --dry-run` then `setup` | `Detected: claude, codex, cursor.`; 3 skills, git hook, `.claude/settings.local.json`, `.cursor/hooks.json` | none | yes |
| SU-06 | `.cursor/` + foreign pre-commit | `setup` | plan prints the foreign message; exit 2 (standalone `hook install git` gives 1); tree unchanged (skills not written) | none | yes |
| SU-07 | crate without git + `.claude/` | `setup` | stderr `not a git repository ...`, exit 2; skill and Stop hook not written either | none | yes |
| SU-08 | foreign `.claude/skills/rust-doctor/SKILL.md` | `setup` | `Detected: claude.`, `Plan:`, stderr foreign refusal, exit 1, tree unchanged | none | yes |
| SU-09 | `.claude/settings.local.json` = `{,}` | `setup` | parse error on stderr, exit 2, tree unchanged | none | yes |
| SU-10 | `.cursor/` + `.pre-commit-config.yaml` | `setup` | snippet printed in plan and in write pass, exit 0, skill + cursor hook written, no pre-commit file | none | yes |
| SU-11 | `.cursor/` + `core.hooksPath` outside | `setup` | outside message, exit 1, tree unchanged | none | yes |
| SU-12 | `.codex/` only | `setup` | `.agents` skill + git hook, no agent hook file | SC (codex on PATH) | yes |
| SU-13 | `.claude/` + outdated rust-doctor pre-commit | `setup` | `Would update ...` then `Updating ...`; exit 0 (setup runs with `update: true`) | none | yes |
| SU-14 | ALL, nonexistent PATH | `setup <missing>` | plan of 3 skills, then `not a git repository`, exit 2, path not created | none | yes |
| SU-15 | none | `setup . extra` | clap error, exit 2 | none | yes |
| SU-16 | nested + `crates/x/.claude`, cwd root | `setup crates/x` | exit 2 `not a git repository` (finding F1); nothing written | none | yes |
| SU-17 | same | `setup <abs>/crates/x --dry-run` | hook `exec rust-doctor crates/x ...`, Stop hook into `crates/x/.claude/` | none | yes |
| SU-18 | any successful setup | `ls .github` | absent | SC (last line) | yes |
| SU-19 | `claude` on PATH but not executable | `setup` | not detected | none | no |

## 10. `ci install` (`ci-install.test.ts`)

| ID | Setup | Command | Expected observable | Coverage | Verified |
|---|---|---|---|---|---|
| CI-01 | clean (`main`) | `ci install --dry-run` | `Would write .github/workflows/rust-doctor.yml:\n\n# Written by rust-doctor ci install; ...`; tree unchanged | CRM | yes |
| CI-02 | clean | `ci install` | stdout exactly `Wrote .github/workflows/rust-doctor.yml\n`, exit 0 | CRM | yes |
| CI-03 | after CI-02 | `Bun.YAML.parse` | `name: Rust Doctor`; `on.push.branches = [main]`; `on.pull_request.types = [opened, synchronize, reopened, ready_for_review]`; `permissions = {contents: read}`; job `inspect`, `runs-on: ubuntu-latest`, `timeout-minutes: 30`, 6 steps; toolchain `'1.97.1'` + `components: clippy`; every `uses` pinned to a 40-hex SHA (checkout, rust-toolchain, rust-cache, upload-artifact); `fetch-depth: 0`, `persist-credentials: false`, `cache-on-failure: true`; `npm install -g rust-doctor@0.7.0`; concurrency group `rust-doctor-${{ github.event.pull_request.number \|\| github.ref }}` | CRM (substring checks) | yes |
| CI-04 | after CI-02 | every `run` block | passes `bash -n`; no `${{` inside any `run` (base ref through env) | none | yes |
| CI-05 | after CI-02 | rerun | stderr `rust-doctor: the workflow was not installed, .github/workflows/rust-doctor.yml already exists, so nothing was written: pass --update to rewrite it.`, exit 1 (hooks answer 0 when already installed) | CRM | yes (finding F11) |
| CI-06 | after CI-02 | rerun `--dry-run` | same refusal, exit 1, no preview | none | yes |
| CI-07 | after CI-02 | `--update` | `Rewrote .github/workflows/rust-doctor.yml`, content identical | CRM | yes |
| CI-08 | after CI-02 | `--update --comment --blocking warning --branch dev --toolchain stable` | `permissions: {contents: read, pull-requests: write}`, 7 steps, step `Comment on the pull request` with `if: always() && github.event_name == 'pull_request'` and env `GH_TOKEN, REPOSITORY, PULL_REQUEST`; `branches: ['dev']`; `toolchain: 'stable'`; baseline args end `--blocking warning)` | CRM | yes |
| CI-09 | after CI-02 | `--update --blocking none` / `error` | baseline args carry it; push args stay `--scope full --blocking none` | none | yes |
| CI-10 | clean | no `--blocking` | baseline args have no `--blocking` (config decides) | CRM | yes |
| CI-11 | none | `--blocking warn` | clap error with tip `warning`, exit 2 | none | yes |
| CI-12 | clean | `--toolchain ""`, `"1.97 x"`, `"1.97.1'$(id)"` | `--toolchain is not a toolchain name.`, exit 2, nothing written | none | yes |
| CI-13 | clean | `--toolchain nightly-2026-07-19` | `toolchain: 'nightly-2026-07-19'` | none | yes |
| CI-14 | clean | `--branch "ma'in"`, `"ma in"`, `$'a\nb'`, `""`, `--branch=-x` | `--branch is not a branch name.`, exit 2 | none | yes |
| CI-15 | clean | `--branch feature/x`, `a#b`, ``a`b$c``, `fé`, `a]b`, `${{github.token}}`, `a..b` | all accepted, written single-quoted; `a..b` is refused by `git check-ref-format`, contrary to the `is_branch_name` doc comment | none | yes |
| CI-16 | ours, `timeout-minutes` hand-edited | `--update` | rewritten, edit lost silently | none | yes |
| CI-17 | marker line removed, or marker on line 2 | `--update` | `... was not written by rust-doctor, so nothing was written.`, exit 1 | CRM | yes |
| CI-18 | ours + leftover `.github/workflows/rust-doctor.yml.rust-doctor` | `--update` | exit 2 `could not write .github/workflows/rust-doctor.yml (entity already exists)` on every run; message names the target, not the staging file | none | yes (finding F18) |
| CI-19 | `.github/workflows/lint.yaml` uses `arthjean/rust-doctor@v1` | `ci install [--update]` | `.github/workflows/lint.yaml already runs rust-doctor, so nothing was written.`, exit 1 | CRM | yes |
| CI-20 | `lint.yaml` only mentions `arthjean/rust-doctor@` in a comment | `ci install` | refused the same way (substring match) | none | yes |
| CI-21 | `renamed.yml` starting with the marker | `ci install` | `renamed.yml already runs rust-doctor`, exit 1 | none | yes |
| CI-22 | detached HEAD, no `main`/`master`/`origin` | `ci install` | `no default branch found: pass --branch <NAME>.`, exit 2, nothing written | CRM | yes |
| CI-23 | default branch `master`, then on `feature` | `ci install --dry-run` | `branches: ['master']` both times | CRM (trunk) | yes |
| CI-24 | `refs/remotes/origin/HEAD -> origin/develop` | same | `branches: ['develop']` | none | yes |
| CI-25 | crate without git / nonexistent path | `ci install --branch main` | `the path is not inside a git repository, and GitHub reads workflows from one.`, exit 2 | CRM | yes |
| CI-26 | `.github` symlink | `ci install` | `.github is a symlink, ...`, exit 2, target empty | CRM | yes |
| CI-27 | dangling symlink at the workflow path | `--update` | `.github/workflows/rust-doctor.yml is a symlink, ...`, exit 2 | none | yes |
| CI-28 | nested, absolute path | `ci install <abs>/crates/x --dry-run` | `workspaces: crates/x`, `rust-doctor crates/x --yes --json` | CRM (src-tauri) | yes |
| CI-29 | nested, relative | `ci install crates/x [--branch main]` | `the path is not inside a git repository`, exit 2 | none | yes (finding F1) |
| CI-30 | repo root mode 555 | `ci install` | `could not write .github/workflows/rust-doctor.yml (permission denied).`, exit 2 | none | yes |
| CI-31 | workspace dir `my app` | `ci install <abs>/my app --branch main` | `the workspace's path from the repository's top is not one a workflow can carry.`, exit 2 | none | yes |
| CI-32 | WF-13 | simulated PR step | see WF-13 | none | yes |
| CI-33 | `--comment` workflow, fake `gh` + real `jq` on PATH | run the comment step twice | round 1: `gh api --paginate .../issues/7/comments` then `-X POST`; round 2 with an existing bot comment id 42: `-X PATCH .../issues/comments/42`; exit 0 both | none | yes |
| CI-34 | same, no `summary.md` / fake `gh` failing POST | comment step | `::notice::rust-doctor wrote no report, so no comment was posted.` exit 0 / `::warning::... read-only token.` exit 0 | none | yes (first) / no (second) |

## 11. `rust-doctor.toml` (`config.test.ts`)

Fixture `cfg`: `src/lib.rs` (todo line 4), `src/generated/mod.rs` (todo), `src/quiet/mod.rs` (todo + `x > u8::MAX`). Read `policy`, `gate`, `scan.ignored`, `diagnostics` from `--json`.

| ID | Config / flags | Expected observable | Coverage | Verified |
|---|---|---|---|---|
| CF-01 | none | `policy.config_file: null`, `blocking {level: error, source: default}`; terminal `Configuration: none loaded; blocking error (default)`; 4 warnings, gate passed, exit 0 | CK | yes |
| CF-02 | empty file / comment only | `config_file: "rust-doctor.toml"`, defaults | unit | yes |
| CF-03 | `blocking = "warning"` | gate failed, 4 blocking, source `config`, exit 1; terminal `Configuration: rust-doctor.toml loaded; blocking warning (config)` | CK | yes |
| CF-04 | same + `--blocking none` | source `request`, passed, exit 0 | CK | yes |
| CF-05 | `blocking = "none"` + todo error + `--blocking error` | failed, 3 blocking, exit 1 | none | yes |
| CF-06 | `[ignore] paths = ["src/gen"]` | does not match `src/generated` (whole segments), nothing ignored | unit | yes |
| CF-07 | `[ignore] paths = ["src/generated/", "src/q*"]` | only `src/lib.rs` reported; `scan.ignored = 3` (diagnostics, not files) | PRC | yes |
| CF-08 | override `src/quiet/**` todo off | todo gone in `src/quiet` only | PRC | yes |
| CF-09 | override `src/quiet` categories correctness error | todo + absurd at `error` in `src/quiet` only, gate failed exit 1 | PRC (off) | yes |
| CF-10 | override `src/**` todo error, then `src/quiet/**` todo off | later table wins | none | yes |
| CF-11 | `[rules] todo = off` + override todo error under `src/quiet/**` | no todo anywhere (an override cannot switch on a rule the plan leaves off) | docs only | yes |
| CF-12 | override todo error + `--rule clippy::todo=warn` | all todo `warning`, source `request-rule` | PRC | yes |
| CF-13 | `[rules] todo = error` + `--category correctness=off` | todo off `request-category` (request category beats config rule) | CK | yes |
| CF-14 | `[categories] correctness = off` + `--rule clippy::todo=error` | todo error `request-rule`, absurd off `config-category` | CK | yes |
| CF-15 | `[categories] correctness = error` + `[rules] todo = off` | todo off `config-rule`, absurd error `config-category` | unit | yes |
| CF-16 | `--rule clippy::todo=off --category correctness=error` | todo off `request-rule`, absurd error `request-category` | unit | yes |
| CF-17 | `--rule todo=warn --rule todo=off` (full ids) | failed report, `policy/duplicate-rule-override`, exit 2 | PC | yes |
| CF-18 | `--rule todo=off` (suffix) | `policy/unknown-rule`, exit 2 (`rules explain` accepts the suffix) | none | yes (finding F19) |
| CF-19 | `[rules] todo = "off"` (suffix) | `configuration/unknown-rule` | none | yes |
| CF-20 | `[rules] "Clippy::Todo"` | `invalid-rule-selector` | CK | yes |
| CF-21 | `[categories] style` | `unknown-category` | CK | yes |
| CF-22 | `[rules] "clippy::todo" = "warning"` | `config-invalid` `Invalid rust-doctor.toml at line 2, column 18.` | unit | yes |
| CF-23 | `blocking = "warn"` | `config-invalid` line 1 column 12 | unit | yes |
| CF-24 | `blockng = ...`, `[ignore] other = 1`, override `level = ...` | `config-invalid` with line/column (1:1, 3:1, 3:1) | CK (top key) | yes |
| CF-25 | `[[overrides]]` without `paths` / with `paths = []` | `config-invalid` / accepted and inert | none | yes |
| CF-26 | `[ignore] paths = ["ok", G]` for G in `{a,b}`, `[ab]`, `!src`, `/abs`, `./src`, `../x`, `src/**x`, `a\\b`, `""`, `.`, `src//gen` | `policy/invalid-glob` `Invalid glob in rust-doctor.toml at ignore.paths[1].`, pattern never echoed, exit 2 | PRC (2) | yes |
| CF-27 | `[[overrides]]` second table bad glob | `overrides[1].paths[0]` | PRC | no (covered) |
| CF-28 | `[structure] cyclomatic_threshold` (snake case) | `config-invalid` line 2 column 1 | none | yes |
| CF-29 | thresholds 0 / 10001 | `config-invalid-threshold` | unit | yes |
| CF-30 | threshold 10000 | accepted | none | yes |
| CF-31 | threshold -1 / `"5"` | `config-invalid` line 2 column 23 | none | yes |
| CF-32 | `cyclomatic-threshold = 12`, `cognitive-threshold = 2` | complexity findings follow the thresholds | STK | no (covered) |
| CF-33 | duplicate `blocking` key | `config-invalid` line 2 column 1 | unit | yes |
| CF-34 | UTF-8 BOM prefix | accepted (`blocking warning (config)`) | none | yes |
| CF-35 | CRLF line endings | accepted | none | yes |
| CF-36 | invalid UTF-8 byte in a comment | `config-invalid-utf8` | CK | yes |
| CF-37 | 65536 bytes / 65537 bytes | exit 0 / `config-too-large` | CK | yes |
| CF-38 | `rust-doctor.toml` symlink to a file outside the workspace | followed and applied | none | yes |
| CF-39 | config is a dir / dangling symlink | `config-not-file` / `config-unreadable` | CK | yes (dir via RL-31) |
| CF-40 | any config failure | failed report: `status: failed`, `policy: null`, `gate.status: not-evaluated`, one error at stage `configuration`, exit 2; terminal `Scan failed: Invalid rust-doctor.toml at line 1, column 1.\n(configuration/config-invalid)` | CK | yes |
| CF-41 | member-dir `rust-doctor.toml` | ignored by the scan | CK | yes (contrast RL-21) |
| CF-42 | git root holds `blocking = "warning"`, workspace in `src-tauri/` | scan of `.`: `config_file: null`, `blocking error (default)`: the root file is silently ignored | none | yes |
| CF-43 | staged scan: config only in the working tree (untracked) vs config staged with a different working-tree copy | the working-tree file decides: untracked `blocking = "warning"` blocks the commit; staged `warning` with working-tree `none` lets it through | none | yes (finding F3) |
| CF-44 | hostile selectors/paths/ESC in config | never in JSON or terminal output | CK | no (covered) |

## 12. Chained workflows (`workflows.test.ts`)

| ID | Setup | Steps | Expected observable | Coverage | Verified |
|---|---|---|---|---|---|
| WF-01 | clean + `.claude/`, NOAG | `setup` then commit a `.gitignore` change | hook prints `No staged Rust change to judge.`, commit succeeds | none | yes |
| WF-02 | after WF-01, no config | stage `todo!()`, `git commit` | commit SUCCEEDS: `Gate passed: blocking error, 0 blocking diagnostic(s)` (every rule defaults to `warn`, blocking defaults to `error`) | none | yes (finding F2) |
| WF-03 | + `blocking = "warning"` staged | commit | refused (git exit 1), hook output on git's stderr holds `clippy::todo` and `Gate failed: blocking warning, 1 blocking diagnostic(s)` | AH | yes |
| WF-04 | same | `git commit --no-verify` | succeeds | none | yes |
| WF-05 | same, PATH without rust-doctor | commit | succeeds, stderr `rust-doctor is not on PATH: this commit was not judged.` | AH | yes |
| WF-06 | `[rules] "clippy::todo" = "error"`, default blocking | commit with new todo | refused, `Policy: base severity warning, effective severity error` | none | yes |
| WF-07 | config todo error, pre-existing todos | commit touching only `README.md` | succeeds (lines scope) | none | yes |
| WF-08 | default config | commit `x > u8::MAX` (P1, caps the score at 65, deny-by-default in Clippy) | commit succeeds, output `Capped at 65/100 by a P1 finding` | none | yes (finding F2) |
| WF-09 | `blocking = "warning"` committed; stage a `todo!()`, then fix the working tree only | commit | refused (correct), but the code frame prints the working-tree line: `>   16 \|     7` under the `todo` message | AH (stderr names the rule only) | yes (finding F10) |
| WF-10 | invalid `rust-doctor.toml` staged | commit | refused; output `Scope: full codebase` then `Scan failed: Invalid rust-doctor.toml ...`: the scope line contradicts the `--staged --scope lines` the hook asked for | none | yes |
| WF-11 | = CF-43 through the installed hook | commit | working-tree config decides | none | yes (finding F3) |
| WF-12 | clean | `hook install claude` → read `.claude/settings.local.json` command → run that exact command with the HR-02 payload → fix → rerun → payload `stop_hook_active: true` | 2, then 0, then 0 silent | AH (pieces) | yes |
| WF-13 | clean, `origin/main` ref, `ci install --blocking warning --comment` committed, branch `feature` adds a todo | extract the `Inspect the workspace` run block with `Bun.YAML`, run it with `RUNNER_TEMP`, `GITHUB_STEP_SUMMARY`, `BASE_REF=main`, `EVENT_NAME=pull_request`; then `BASE_REF=""`, `EVENT_NAME=push` | PR: exit 1, `report.json` `scope.mode = baseline`, `delta.summary.introduced = 1`, summary has `**Against the base:** 1 introduced, 0 fixed.`; push: exit 0, `--blocking none`, full scope | none | yes |
| WF-14 | twoMember | `--json` scan saved → `report markdown` → `why --report` with `PATH=""` | markdown row and `why` hit agree with the live `why` byte for byte | EX (JSON) | yes |
| WF-15 | clean, `CLAUDECODE=1` | scan (missing notice) → `skill install` (silent) → stamp 0.0.1 (stale notice) → `skill install --update` (silent) | notice lines as in SK-26 and the stale form `the claude skill was written by rust-doctor 0.0.1, and this is 0.7.0; run ... --update.` | ASN | partial |
| WF-16 | none | `--version`, installed skill stamp, CI `npm install -g rust-doctor@X`, `Cargo.toml` | all `0.7.0` | none | yes |
| WF-17 | nested `crates/x`, absolute install, `blocking = "warning"` in `crates/x` | commit a todo in `crates/x` | refused; the relative `rust-doctor crates/x --staged --scope lines` from the toplevel works | none | partial (scan verified) |
| WF-18 | `ci install --comment` | WF-13 then CI-33 | sticky comment POST then PATCH | none | yes |
| WF-19 | clean | `setup` then `ci install` then `setup` again | setup never touches `.github`; `ci install` rerun exit 1 | none | yes (pieces) |
| WF-20 | each installer | `--dry-run` everywhere on one repo | single `treeHash` before and after the whole sequence | none | yes (per command) |

---

## 13. Findings (evidence above, IDs cross-referenced)

| # | Severity | Finding | Evidence |
|---|---|---|---|
| F1 | high | Every installer that asks git where the repository is fails on a relative PATH other than `.`: `git_hook::locate`/`toplevel` call `run_git` with `current_dir(ws)` and `-C ws`, so `crates/x` resolves to `crates/x/crates/x`. `hook install git crates/x`, `ci install crates/x`, `setup crates/x` all exit 2 "not a git repository"; the absolute path works. Existing tests always pass absolute paths. | HG-26, HG-27, CI-29, SU-16; `src/git.rs:103-120` and `:242-246` |
| F2 | high (product) | The pre-commit hook `setup` installs blocks nothing by default: every catalogued rule defaults to `warn` and blocking defaults to `error`, so a staged `todo!()` or a P1 correctness finding capping the score at 65 commits cleanly. The Stop hook forces `--blocking warning`; the git hook does not. | WF-02, WF-08 |
| F3 | medium | A `--staged` scan (the pre-commit hook) reads `rust-doctor.toml` from the working tree, not the index, so an untracked or unstaged config decides a commit that does not record it. | CF-43, WF-11 |
| F4 | medium | `rules explain RULE PATH` reads `PATH/rust-doctor.toml` literally, while scans read only the Cargo workspace root's file: a member path reports `warn (default)` where the scan applies `error (config-rule)`, and a member's own file, which scans ignore, is honored. | RL-20, RL-21 |
| F5 | medium | No installer or explainer checks that PATH exists: `rules explain todo /nonexistent` answers `warn (default)` with exit 0; `hook install claude/cursor` and `skill install` on a mistyped path create the whole directory tree and exit 0. | RL-22, HA-29, SK-21 |
| F6 | medium | `why --report` performs no schema check: `{"a":1}` answers "Nothing is reported" with exit 0 and a schema-24 report is read, where `report markdown` refuses both. | WY-12, WY-13 |
| F7 | low | `hook install claude/cursor` prints `Added a Stop hook ...` on stdout before writing; on a read-only file or directory the write then fails (exit 2) with the success line already printed. | HA-20 |
| F8 | low | Write failures exit inconsistently: `skill install` exits 1, `hook install` and `ci install` exit 2. Idempotent reruns too: `hook install` exits 0 "already installed", `ci install` exits 1 and cannot even `--dry-run`; inside `setup` a foreign pre-commit is exit 2, standalone exit 1. | SK-24, CI-05/06, SU-06 |
| F9 | low | `rules explain` and `why` echo argv verbatim, ESC included, to stderr, unlike the redacted `--rule` parser. | CL-10 |
| F10 | low | Staged scans draw code frames from the working tree, so a refused commit can show a fixed line (`> 16 \|     7`) under the staged `todo` finding. | WF-09 |
| F11 | low | (see F8) `ci install` rerun is not idempotent in exit status. | CI-05 |
| F12 | low | `rules explain` lists an override that cannot take effect (rule off by config category, override raising its category) without saying it is inert. | RL-18, CF-11 |
| F13 | low | `why` live and `why --report` disagree on `.gitattributes`-declared generated files ("declared generated" vs "Nothing is reported"), against the module doc "Both answer from the same JSON document". | WY-26, WY-27 |
| F14 | low | `why` on a line silenced by an applied `rust-doctor: allow` directive or an `#[expect]` says only "Nothing is reported"; `--help` promises "how to silence it" and the skill says it names "the directives near it". SKILL.md also says "every directive is listed in `suppressions[]`" right after describing `#[expect]`, which is never listed there. | WY-29, WY-30 |
| F15 | low | `hook install git --update` over a hand-edited hook that keeps the marker drops the user's lines while printing "`X` becomes `X`". | HG-12 |
| F16 | low | Merging into an agent settings file reorders every key alphabetically and reformats it (noisy diff for the shared `.claude/settings.json`). | HA-17 |
| F17 | low | A Stop payload over 1 MiB loses the `stop_hook_active` loop guard (truncated, unparsable, scanned). | HR-11 |
| F18 | low | A leftover `rust-doctor.yml.rust-doctor` staging file makes `ci install --update` fail forever, naming the target file. `--branch a..b` is accepted though git refuses it. `existing_install` matches a comment mentioning the action. | CI-18, CI-15, CI-20 |
| F19 | low | Rule suffixes work in `rules explain` but not in `--rule` or `[rules]`. | CF-18, CF-19 |
| F20 | info | SKILL.md: "Every installer ... never writes over a file it did not create": `hook install claude/cursor` merges into user-owned settings files (stated correctly in `src/hook.rs` docs). `setup` aborts entirely outside git, so the skill and agent hooks, which need no git, cannot be installed through it. | HA-09, SU-07 |

## 14. Proposed split

| File | Scenarios | Notes |
|---|---|---|
| `e2e-helpers.ts` | section 0 | isolated spawn, `treeHash`, fixtures, fake agents, fake `gh`, `Bun.YAML` |
| `cli-surface.test.ts` | CL-01..13 | no cargo, sub-second |
| `rules.test.ts` | RL-01..33 | no cargo |
| `report-markdown.test.ts` | RM-01..22 | one scan fixture, rest static JSON |
| `why.test.ts` | WY-01..35 | live scans of tiny fixtures |
| `hook-git.test.ts` | HG-01..30 | git only, no cargo |
| `hook-agent-install.test.ts` | HA-01..32 | filesystem only |
| `hook-run.test.ts` | HR-01..27 | scans, one target dir per test |
| `skill-install.test.ts` | SK-01..28 | filesystem + notice scans |
| `setup.test.ts` | SU-01..19 | PATH-controlled detection |
| `ci-install.test.ts` | CI-01..34 | YAML + `bash -n` + fake `gh` |
| `config.test.ts` | CF-01..44 | `--json` scans, table-driven |
| `workflows.test.ts` | WF-01..20 | `git commit` through the installed hook, CI step simulation |
