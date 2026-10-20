# rust-doctor interactive report: end-to-end scenario catalog

Binary under test: `target/release/rust-doctor` built from HEAD `b5389f7` (2026-10-19). Every run was driven under tmux 3.7c on
copies placed in `ws/` of this directory, never on the live repository. 166 scenarios in 13 files: 148 verified empirically
(a few partially, as marked), 18 from code reading or existing coverage only.
The harness used for verification is `scripts/harness.py` (the shape of pf's `tmux-helpers.ts`: one session per scenario, an
observer `/bin/sh` that writes the exit status, `remain-on-exit on`, `pipe-pane` to a raw byte log, `capture-pane` polling).
Scripts `scripts/s01.py` to `scripts/s23.py` are the evidence; each one prints the grids quoted below.

## 1. Conventions every test file shares

### 1.1 Process environment

Run each scenario under `env -i` so nothing from the developer's shell leaks in:

`HOME=$HOME PATH=<profile> LANG=C.UTF-8 TERM=xterm-256color CARGO_TARGET_DIR=<absolute> RD_CAPTURE=<absolute>`

None of the 26 `NON_INTERACTIVE_MARKERS` (`src/terminal_profile.rs:19-54`) and no `CI`, `NO_COLOR`, `COLUMNS`. The developer's own
shell carries `CLAUDECODE=1`, so a harness that only `env -u`s a few names is fragile; `env -i` plus an allow list is not.

`CARGO_TARGET_DIR` must be absolute: a relative value is resolved by Cargo against the scanned workspace and creates
`<workspace>/<relative>` inside it (observed: `ws/repo/tgt/repo` appeared after one relative run, making `git status` dirty).

### 1.2 PATH profiles

| Profile | Content | Use |
|---|---|---|
| `P.tc` | symlinks only: `cargo rustc cargo-clippy clippy-driver rustup git cc gcc ld` (rustup proxies dispatch on argv[0]) | no agent, no clipboard command, no `xdg-open`: the SSH case |
| `P.fake` | `fake/` then `P.tc`. `fake/` holds `claude`, `codex`, `cursor-agent` (write `argc`, `payload`, `cwd`, `tty` into `$RD_CAPTURE`, like `write_agent` at `tests/local_cli_experience.rs:458`), `wl-copy` (stores stdin; `RD_CLIP_MODE=fail` exits 1, `hang` sleeps 5 s), `xdg-open` (logs args; `RD_OPEN_MODE=fail` exits 3) | every Enter on Copy, Open guide, or an agent |
| `P.fake9` | a `codex` that prints and exits 9, then `P.tc` | agent exit status does not leak |
| `P.real` | `~/.cargo/bin:/usr/local/bin:/usr/bin` | read-only screens only. On this machine it resolves the real `claude`, `codex`, `wl-copy`, `xclip` and `xdg-open`: Enter on Copy or an agent would overwrite the real clipboard, open a browser, or start a real agent |

Note `cc` must be linked to `/usr/bin/cc` explicitly: in Arthur's zsh `cc` is an alias of `claude`, so `which cc` returns an alias string.

### 1.3 Workspaces (all copies, `target/` excluded)

| Name | Source | Facts (verified with `--yes --json`) |
|---|---|---|
| `todo` | `tests/fixtures/kernel-contract/todo`, no git | 94 Great, projected 100, 1 rule (`clippy::todo` x3), complete, exit 0, no CI entry |
| `todo-git` | `todo` + `git init -b main` + commit | 90 Great, projected 100, 2 rules (adds `rust_doctor::repo::unignored_build_output`), CI entry offered |
| `todo-nocommit` | `todo` + `git init -b main`, no commit | CI entry offered, install fails: no default branch |
| `todo-otherwf` | `todo-git` + `.github/workflows/scan.yml` with `uses: arthjean/rust-doctor@v0` | CI entry still offered, install fails: already installed |
| `todo-rogh` | `todo-git` + `chmod 555 .github/workflows` | install fails: permission denied |
| `todo-skill` | `todo` + `rust-doctor skill install <ws> --agent claude` | handoff payload without the skill-install line |
| `clean` | `tests/fixtures/projects/clean`, no git | 100 Great, 0 findings, complete, exit 0, no action at all |
| `denied` | `kernel-contract/denied` | 94 "Core partial", 1 error, incomplete, exit 1 |
| `partial` | `kernel-contract/partial-failure` | 94 Core partial, 3 rules (error, warning, info), incomplete, exit 1 |
| `broken-member` | `scan-failures/broken-member` (virtual workspace a, b, c) | 94 Core partial, project name = directory, exit 1 |
| `no-rust` | `local-cli-experience/no-rust` | no score, 0 findings, incomplete, exit 1 |
| `tests-only` | crafted: empty `src/lib.rs`, `tests/pending.rs` with `todo!()` | no score ("only tests, benches..."), 0 findings, exit 0 |
| `uncatalogued` | `score-reasons/uncatalogued` | 65 Needs work, projected 100, includes "Compiler notes (not scored)" |
| `sleeping` | `scan-failures/sleeping-build` (build.rs sleeps 120 s) | the slow scan for SIGINT and `--max-duration` |
| `bad-manifest` | `scan-failures/bad-manifest` | status failed, exit 2 |
| `parent1` | empty dir containing `todo` as `inner/` | discovery notice on stderr |
| `repo` | `git clone --no-hardlinks` of the repository (`--local` fails across devices: "Lien physique inter-peripherique invalide") | 65 Needs work, projected 94, 12 rules, 174 diagnostics, 236 occurrences (184 warning + 52 info), CI entry offered |

The same fixture scores 94 without git and 90 inside a fresh git repository (no `.gitignore`, so `unignored_build_output` fires).
Every test must pin the git state of its workspace.

### 1.4 Stable text markers

| Screen | Wait on | Notes |
|---|---|---|
| Landing with actions | `↑/↓ move · enter select · q quit` | absent during the count-up, which draws no menu |
| Landing without actions | `q quit` plus `No issues found` or `Score unavailable` | the viewer says `q to quit`, so `q quit` alone is unambiguous only on menus |
| Viewer (split, stacked) | `enter copy context` and `issue N/M` | in compact layouts the hint is truncated (`enter copy cont…` at 80x23), wait on `issue 1/` |
| CI setup | `Add Rust Doctor to GitHub Actions?` | starts with a cyan `?` span |
| CI recommendation | `Add Rust Doctor to GitHub Actions first` | hint ends `esc skip · q quit` |
| Handoff | `Choose how to continue` | |
| Final state | pane dead + exit file | `#{cursor_flag}` of a dead pane is meaningless; assert cursor restore on the pipe-pane byte log |

Match on `capture-pane -p` text, never on raw bytes: every span boundary inserts SGR codes (`T1` notes the same).

### 1.5 Timings measured (warm target, this machine)

| Run | Hint visible | Stable (0.4 s quiet) |
|---|---|---|
| `no-rust` (no score, no animation) | 0.12 s | 0.53 s |
| `todo`, `NO_COLOR=1` | 0.11 s | 0.5 s |
| `repo`, `NO_COLOR=1` | 0.85 s | 1.25 s |
| `clean`, `denied`, `partial` (count-up, no projection) | 2.09 to 2.13 s | 2.50 to 2.54 s |
| `todo`, `uncatalogued` (count-up + projection) | 2.65 s | 3.07 s |
| `repo` (count-up + projection) | 3.02 s | 3.43 s |
| cold `repo` scan (`--json`, empty target) | 5.7 s total | |

Count-up = 41 frames 50 ms apart (`score_block::COUNT_UP_FRAME_COUNT` 40) then 16 projection frames 35 ms apart; `uncatalogued`
wrote 58 `ESC[J` (41 + 16 + the first menu frame). Key-to-frame latency on `repo` split: 10.5 ms; lone Esc: 3 ms. Idle CPU while
waiting for a key: 0 ticks over 3 s.

## 2. Existing PTY coverage (`tests/local_cli_experience.rs`, `mod tty`)

| Ref | Test |
|---|---|
| T1 | `a_terminal_run_opens_the_interactive_report_on_its_score_and_its_menu` |
| T2 | `reviewing_the_issues_shows_a_detail_and_esc_returns_to_the_next_action` |
| T3 | `verbose_keeps_the_linear_report_on_a_terminal` |
| T4 | `yes_under_a_terminal_prints_no_animation_frames` |
| T5 | `an_agent_marker_keeps_the_linear_report_on_a_terminal` (CLAUDECODE, GITHUB_ACTIONS, CI=yes) |
| T6 | `a_workspace_with_uncommitted_changes_scans_the_full_codebase_without_asking` |
| T7 | `q_escape_and_ctrl_c_all_leave_the_report_with_the_scan_exit_code` (exit 0 only) |
| T8 | `the_report_uses_stdin_and_stdout_when_stderr_is_redirected` |
| T9 | `a_prompt_no_clipboard_takes_is_printed_and_the_scan_exit_code_holds` |
| T10 | `codex_inherits_one_payload_its_cwd_and_the_terminal` |
| T11 | `packed_wrapper_tty_handoff_contract_when_requested` (opt-in via `RD_PACKED_WRAPPER`) |

All eleven run on the live repo's `kernel("todo")` fixture at a fixed 100-column PTY with no row count set, so none of them
exercises layouts, resize, the CI screens, the clipboard success path, signals other than a keyboard Ctrl-C, or a large report.
Unit tests in `src/tui/tests.rs`, `src/tui/screens/*.rs`, `src/terminal_profile.rs` and `src/handoff/tests.rs` cover the
state machine and the profile resolution without a terminal; they are cited as "unit" below.

## 3. Scenarios

Columns: **Setup** (workspace, env delta, window), **Keys** (tmux key names; `-H` for raw bytes), **Expected**, **Covered**
(existing test or `-`), **Determinism**, **V** (verified empirically: Y, or N for code reading only).

### 3.1 `tui-gating.test.ts`: when the interactive report opens

| ID | Setup | Keys | Expected | Covered | Determinism | V |
|---|---|---|---|---|---|---|
| GATE-01 | todo, baseline, 100x40 | wait, `q` | landing hint appears; linear text `All 3 occurrences` absent; exit 0 | T1 | - | Y |
| GATE-02 | todo, `--json` | none | JSON on stdout, no SGR, no menu, exit 0 | - | output is large: assert pane dead + JSON parse of `capture-pane -S -` | Y |
| GATE-03 | todo, `--yes` | none | linear report (`Scope: full codebase`, score block), no `ESC[nA`, exit 0 | T4 | - | Y |
| GATE-04 | todo, `--verbose` | none | linear, every occurrence listed, no menu, exit 0 | T3 | - | Y |
| GATE-05 | todo, each of the 26 markers `=1` (parametrized) | none | linear report, exit 0 | T5 (3 of 26), unit `every_marker_closes_the_report_and_an_empty_one_counts_as_unset` | one session per marker; reuse a warm target | Y (CLAUDECODE, GIT_DIR, CI=1) |
| GATE-06 | todo, each marker set to empty string | `q` | landing opens | unit | - | Y (`CLAUDECODE=`) |
| GATE-07 | todo, `CI=false`, `CI=0`, `CI=FALSE`, `CI=` | `q` | landing opens | unit | - | Y (`CI=false`) |
| GATE-08 | todo, `CI=1`, `true`, `yes`, `off`, `no` | none | linear | T5 (`CI=yes`) | - | Y (`CI=1`) |
| GATE-09 | todo, `TERM=dumb` | none | linear, no SGR, width 80 (`Help:` line wraps at 80 in a 100-column pane) | unit | - | Y |
| GATE-10 | todo, `TERM` unset | `q` | landing opens | - | `console` still sees a TTY | Y |
| GATE-11 | todo, `</dev/null` | none | linear, still colored (stdout is the terminal) | - | wrap the command in `bash -c` | Y |
| GATE-12 | todo, `\| cat` | none | linear, no SGR, rows of at most 80 columns | non-PTY test at `local_cli_experience.rs:~170` | exit status is cat's unless `pipefail` | Y |
| GATE-13 | todo, `2>/dev/null` or `2>file` | `q` | landing opens, keys work; progress lines land in the file once per phase (`Running native passes...`, `Compiling dependencies...`, `Linting kernel-todo (1/1)`) | T8 | - | Y |
| GATE-14 | bad-manifest; nonexistent path | none | linear `Scan failed: ...` (`(metadata/cargo-metadata)`, `(discovery/no-manifest)`), exit 2, no menu | - | - | Y |
| GATE-15 | denied, partial, broken-member, `sleeping --max-duration 1` | `q` | landing opens despite `status: incomplete` | - | - | Y |
| GATE-16 | sleeping, Ctrl-C during the scan | `C-c` | no menu: `Scan failed: The scan was interrupted by a signal. (clippy/interrupted)`, exit 130 | - | see SIG-01 | Y |
| GATE-17 | git repo with an uncommitted change, no `--scope` | `q` | no `Choose what to scan`, no `Uncommitted changes`, file unchanged | T6 | - | N (covered) |
| GATE-18 | todo via `rust-doctor inspect <path>` | `q` | landing opens (alias shares arguments) | unit `root_and_historical_subcommand_share_arguments` | - | N |

### 3.2 `tui-startup.test.ts`: the landing per workspace kind, and the count-up

| ID | Setup | Keys | Expected | Covered | Determinism | V |
|---|---|---|---|---|---|---|
| LAND-01 | todo, 100x40 | wait | `┌─────┐  94 / 100 Great  ·  kernel-todo`, face `◠ ◠`/`▽`, `Rust Doctor (https://rust-doctor.com)`, `Potential score 100 after priority fixes +6`, `❯ Review 1 issue`, `› Hand off to an agent`, no CI entry, 13 rows used Corrected 2026-10-20: the frame is 11 rows, the key hint last. | T1 (partial markers) | no `.git` in the copy | Y |
| LAND-02 | clean | wait, then `Enter j k Down G` | `100 / 100 Great  ·  clean`, `✔ No issues found. Nice work.`, hint `q quit` alone, no `❯`; keys change nothing; Esc exits 0 | - | - | Y |
| LAND-03 | todo-git `--scope files`, no change | wait | `✔ No issues found. Nice work.` + `❯ Add to GitHub Actions (Recommended)` with its three justification rows as the only action, hint with arrows | - | requires a clean worktree | Y |
| LAND-04 | repo, 100x40 | wait | `65 / 100 Needs work  ·  rust-doctor`, face `• •`/`─`, `Potential score 94 after priority fixes +29`, `❯ Review 12 issues`, `› Add to GitHub Actions (Recommended)`, `› Hand off to an agent` Corrected 2026-10-20: asserted by shape only (a score, a face, then Review, GitHub Actions and the handoff in that order); the values move with every commit of this repository (FR-12). | - | scores drift with every commit of the repo: pin the clone to a SHA or assert shapes, not numbers | Y |
| LAND-05 | denied | wait, `q` | `94 / 100 Core partial`, two yellow `Score is partial: ...` lines (stage failed; packages did not compile), `⚠ clippy and execution checks did not complete: results are incomplete.`, no Potential line, exit 1 Corrected 2026-10-20: since US-009 a scan that linted no package has no score: `Score unavailable: the scan could not score, since no package was linted.`, `1 finding · kernel-denied`, the same warning, no face and no Potential line, exit 1. | unit `the_landing_screen_says_why_a_score_is_partial` | - | Y |
| LAND-06 | partial; broken-member | wait | as LAND-05 with `Review 3 issues` / `Review 2 issues`; broken-member titled by directory (`· broken-member`) Corrected 2026-10-20: partial is unscored since US-009 (`Score unavailable: ...`, `5 findings · partial-failure`, `Review 4 issues`); broken-member keeps `94 / 100 Core partial  ·  broken-member`, its two `Score is partial` lines and `Review 2 issues`. | - | - | Y |
| LAND-07 | no-rust | wait, `q` | no face: `Rust Doctor (https://rust-doctor.com)`, `Score unavailable: no Rust files were analyzed.`, `0 findings · no-rust`, `⚠ No issues detected, but execution checks did not complete.`, hint `q quit`, hint at 0.12 s, exit 1 | unit (unscored) | - | Y |
| LAND-08 | tests-only | wait | `Score unavailable: only tests, benches, examples or build scripts were found.`, `0 findings · tests-only`, `✔ No issues found. Nice work.` | unit | - | Y |
| LAND-09 | uncatalogued | wait | `65 / 100 Needs work`, `+35` projection | - | - | Y |
| LAND-10 | parent1 (path holds no manifest) | wait | first row `No Cargo workspace at this path: scanning the one in inner/.` stays above the score block, frame intact below it | - | - | Y |
| LAND-11 | sleeping `--max-duration 1` | wait, `q` | `Score is partial: the scan hit its --max-duration limit. Raise the limit or narrow the scope.`, `⚠ clippy checks did not complete`, exit 2 | - | 1 s budget + animation: hint at 3.0 s | Y |
| LAND-12 | uncatalogued, color | poll every 10 ms | sampled score values strictly increase from 0 to 65 (`0, 5, 9, 14, ... 64, 65`); no `❯` while counting; `▓` gain grows after the count | - | sample, do not assert exact frames | Y |
| LAND-13 | uncatalogued, pipe-pane | wait | 58 `ESC[J` in the byte log before the first key (41 + 16 + 1) | - | exact count is stable: fixed frame constants | Y |
| LAND-14 | todo, `NO_COLOR=1` | wait | no count-up: hint at scan time (0.11 s), score shown at its final value on the first frame | - | - | Y |
| LAND-15 | no-rust (no score) | wait | no count-up even in color | - | - | Y |
| LAND-16 | denied (no projection) | wait | bar has no `▓`; hint at about 2.1 s (count-up only) Corrected 2026-10-20: run on broken-member, since denied has no score since US-009 and draws no count-up; its count-up frames carry no `▓`. | - | - | Y |
| LAND-17 | subdirectory `todo/src` | wait | landing for `kernel-todo` (Cargo walks up); no discovery line | - | - | Y |
| LAND-18 | `--scope baseline` on a repo with a new finding | wait | landing counts only the delta's new findings | - | needs a base commit | N |

### 3.3 `tui-keybindings.test.ts`: menus and every key

| ID | Setup | Keys | Expected | Covered | Determinism | V |
|---|---|---|---|---|---|---|
| KEY-01 | repo, landing | `Down Down Down Up Up Up Up` | cursor clamps at both ends; `❯` moves; `j`/`k` equal to arrows Corrected 2026-10-20: run on todo-git, whose three entries clamp the same way, so the row asserts nothing about this repository. | unit `a_menu_cursor_never_leaves_the_entries_it_indexes` | - | Y |
| KEY-02 | todo-git, landing | `Down` | the CI entry reveals `Scan every pull request...`, `Baseline scope on a pull request...`, `https://github.com/arthjean/rust-doctor#github-actions` only while selected | unit `the_menu_points_at_the_selection_and_gaps_every_entry` | - | Y |
| KEY-03 | todo, landing | `Enter` on each entry | Review opens the viewer; Hand off opens the handoff (no CI) | T2 | - | Y |
| KEY-04 | any, every screen | `q` | exits with the scan's code from landing, viewer, CI, recommendation, handoff | unit `quitting_works_from_every_screen`, T7 (landing) | - | Y (landing, viewer, CI, handoff) |
| KEY-05 | any, every screen | `C-c` (byte 0x03 in raw mode) | same as `q` | T7 (landing) | race with tcsetattr only during the count-up: send after the hint | Y |
| KEY-06 | landing | `Escape` | quits with the scan code | T7 | - | Y |
| KEY-07 | todo landing | `C-z C-d Q Space BSpace F1 C-l` | nothing changes, process alive (Ctrl-Z does not suspend: ISIG is off in raw mode) | - | - | Y |
| KEY-08 | todo viewer | `Escape` | back to landing in 3 ms, cursor on the next action (Hand off when no CI) | T2, unit `escaping_the_issue_list_lands_on_what_to_do_next` | send Esc alone (KEY-10) | Y |
| KEY-09 | repo viewer | `Escape` | landing cursor on `Add to GitHub Actions (Recommended)` with justification visible | unit | - | Y |
| KEY-10 | todo viewer | `-H 1b 0d`, `-H 1b 6a`, `-H 1b 1b` | each pair in one write is swallowed whole: still in the viewer (see BUG-12) | - | the reason tests must send Esc alone and wait for the frame | Y |
| KEY-11 | todo landing | `-H 1b 4f 42` (SS3 Down), `-H 1b 5b 48` (Home) | ignored; only CSI arrows move | - | - | Y |
| KEY-12 | todo viewer | `-H 1b 5b 32 30 30 7e 6a 71 1b 5b 32 30 31 7e` (bracketed paste of `jq`) | the paste markers are ignored, `q` quits, exit 0 | - | TUI never enables bracketed paste | Y |
| KEY-13 | repo viewer | `-H` twenty `6a` in one write | every key processed: `issue 12/12` | - | - | Y |

### 3.4 `tui-issues-viewer.test.ts`: content, navigation, counters

| ID | Setup | Keys | Expected | Covered | Determinism | V |
|---|---|---|---|---|---|---|
| VIEW-01 | todo, 100x40 | `Enter` | `Bugs`, `› ⚠ Todo ×3`, divider `────`, `⚠ Todo ×3`, `Bugs · warning · src/lib.rs:4`, `Impact Real users hit panics...`, `Why \`todo\` should not be present in production code`, frame `> 4 │     todo!()` + carets, `Fix Replace todo! with...`, `3 findings  0 errors  3 warnings  ·  issue 1/1`, `0 issues unread` | T2 (Impact, Why, issue 1/, unread) | - | Y |
| VIEW-02 | repo, 120x40 | `j j Down k Up Up Up` | `issue 2/12, 3, 4, 3, 2, 1, 1`; headers skipped; clamp at 1 Corrected 2026-10-20: run on the `many` fixture (seven rules, one site each), since stepping needs a known number of issues and this repository's moves with every commit (FR-12). | - | - | Y |
| VIEW-03 | repo | `G`, `j`, `g`, `g` | `issue 12/12`, stays 12, `gg` returns to 1; `g` then `j` cancels the chord Corrected 2026-10-20: run on `many`, for the same reason as VIEW-02; the positions are read from the status bar. | - | - | Y |
| VIEW-04 | repo, 120x40 | `PageDown PageDown PageUp` | 12/12, 12/12, 1/12 (list height 34 exceeds the list) Corrected 2026-10-20: run on `many`, whose whole list fits one page at 120x40 whatever this repository holds. | - | page size = list height of the layout | Y |
| VIEW-05 | repo | `Enter`, `j j Down`, `G` | unread counter `11, 10, 9, 8`, then 7; markers `•` unread, two spaces read, `›` selected Corrected 2026-10-20: run on `many` (counter 6, 5, 4, 3, then 2), since the counts are this repository's otherwise (FR-12). | unit `the_status_bar_counts_sites...` | - | Y |
| VIEW-06 | repo | `End Home Left Tab x` | no change | - | - | Y |
| VIEW-07 | repo | `Enter` on landing | categories in audit order Security, Bugs, Dependencies, Maintainability; `ℹ Print stderr ×26`, `ℹ Print stdout ×26` under Maintainability. Corrected 2026-10-20: the self-scan names no rule (FR-12), so the headers follow `audit.categories` of the same report, one row per rule, `ℹ` exactly for a rule whose every site is info | - | repo-dependent | Y |
| VIEW-08 | uncatalogued | `Enter` | `Compiler notes (not scored)` heading last, with `Must use candidate` and `Unused imports` | unit `build_entries` | - | Y |
| VIEW-09 | repo, first issue | `Enter` | `Hardcoded credential` detail: every code-frame row prints `<redacted>`, location `tasks/prd-react-doctor-gap-closure.md:697`. Corrected 2026-10-20: US-012 removed that self-scan finding, so the subject is the `mini` git scenario, its access key at `src/creds.rs:2` masked in the frame and absent from every byte | unit (masking) | the match lives in a tracked task file: repo-dependent | Y |
| VIEW-10 | denied, 120x21 | `Enter` | `› ✖ Todo`, `1 finding  1 error  0 warnings  ·  issue 1/1` (singular) | - | - | Y |
| VIEW-11 | repo, 100x40 (stacked, list 15 rows) | `G`, then `g g` | after `G` the `Security` header scrolls off; after `gg` it never comes back (BUG-09) | - | - | Y |
| VIEW-12 | repo | `Enter` | status bar `236 findings  0 errors  236 warnings` although 52 occurrences are info (BUG-02) | - | - | Y |
| VIEW-13 | workspace with a test-site finding | `Enter` | meta row ends `· test`; site list labels the context. Corrected 2026-10-20: the label is `· tests` (`DiagnosticContext::Tests`), seen on a crate holding a scored `todo!()` and an interpolated shell command under `tests/` | unit `a_test_site_is_labeled...` | needs a fixture with a scored production finding and a test site | N |
| VIEW-14 | repo, color | `Enter` | `Rule guide:` carries an OSC 8 link (`ESC]8;;https://rust-doctor.com/rules/...`) | - | assert on the byte log | Y |
| VIEW-15 | workspace with a file deleted after the scan | `Enter` | detail keeps every section but the frame; no error text | unit (code reading) | delete between scan and render is racy: use a fixture whose span points outside the file. Corrected 2026-10-20: the frame is read when the detail renders, so deleting the file once the landing is up is deterministic | N |

### 3.5 `tui-layout.test.ts`: geometry matrix

| ID | Setup | Keys | Expected | Covered | Determinism | V |
|---|---|---|---|---|---|---|
| LAY-01 | repo, 200x50 | `Enter` | split: divider `│` at column 79, list column 77, detail right; 49 rows used; widest row 194 | unit `a_wide_terminal_splits...` | - | Y |
| LAY-02 | repo, 120x40 | `Enter` | split with a 45-column list: viewer header truncated `rust-doc…` (expected in a split), detail wraps at 70 | - | - | Y |
| LAY-03 | repo, 119x30 | wait | landing bar 50 cells, no truncation (stacked geometry) | - | - | Y |
| LAY-04 | repo, 100x40 | `Enter` | stacked: header, blank, list 15 rows, divider, detail capped at 15 rows, 2-row status Corrected 2026-10-20: the stacked list holds every entry up to 15 rows (`src/tui/model.rs` `resolve_layout`), so the row expects the smaller of 15 and the report's own entries. | unit `a_narrow_terminal_stacks...` | - | Y |
| LAY-05 | repo, 80x24 and 60x24 | `Enter` | stacked with a 3-row list; second status row truncated with `…` at 60 | - | - | Y |
| LAY-06 | repo, 80x23 | `Enter` | compact: header + 16 list rows + one status row `... issue 1/12  ·  ↑/↓ move · enter copy…` | - | 23 rows is the compact threshold | Y |
| LAY-07 | denied, 120x21 | `Enter` | compact although 120 wide (`rows < 22`), full single-row status | - | - | Y |
| LAY-08 | repo, 40x10 | `Enter` | compact with header (9 rows); 24x8: compact without header | - | - | Y |
| LAY-09 | todo, 20x2, 5x3, 1x1 | `Enter j Escape q` | no crash; 20x2 shows `94 / 100 Great  · …`; 1x1 shows nothing; exit 0 | - | - | Y |
| LAY-10 | todo, 300x80 | wait | bar capped at 50 cells | - | - | Y |
| LAY-11 | every size above | after every frame | no row reaches the last column (widest 59/60, 78/80, 116/120, 194/200), rows used at most `rows - 1` | unit `no_screen_overflows_the_terminal_it_was_resolved_for` | assert on `capture-pane` | Y |
| LAY-12 | repo, 120x22, 125x30 vs 119x30, 130x30, 165x30 | wait | BUG-01: landing header truncated at 120 to about 128 columns (`rust-doc…`, `Rust Doctor (https://rust-doctor…`, `+…`) and bar 33 at 120 vs 50 at 119; full bar again from 165 | - | - | Y |
| LAY-13 | repo, 80x12, 40x10, 24x8 | `Down Down` | BUG-07: the landing is cut at `rows - 1` without scrolling: at 40x10 and 24x8 the selected entry is off screen, at 80x12 the hint is gone | - | - | Y |
| LAY-14 | todo-git, 60x30 | `Down` | CI justification rows truncated with `…` rather than wrapped | unit `a_label_longer_than_the_terminal_is_cut...` | - | Y |
| LAY-15 | todo-otherwf, 100x30 | CI install | failure notice truncated at the frame width (`...so nothing was wr…`). Corrected 2026-10-20: offering that entry is tui/BUG-10, so the subject is a read-only `.github/workflows` at 60x30, whose permission notice is cut the same way | - | - | Y |

### 3.6 `tui-resize.test.ts`

| ID | Setup | Keys | Expected | Covered | Determinism | V |
|---|---|---|---|---|---|---|
| RSZ-01 | repo viewer 120x40 | `resize-window 80 24`, capture, then `j` | before the key: tmux-reflowed stale frame (rows wrapped mid-word, no score face); after `j`: clean stacked frame, one face, one status | - | BUG-08: always send a key after a resize | Y |
| RSZ-02 | then 60x20 | resize, `j` | compact frame, one face | - | - | Y |
| RSZ-03 | then 200x50 | resize, capture, `j` | before the key three score faces are visible (reflowed history); after `j` exactly one Corrected 2026-10-20: run on `many`, since the expected position needs at least three issues. | - | - | Y |
| RSZ-04 | then 120x40, 100x30 | resize, `j` | split, then stacked, one face each Corrected 2026-10-20: run on `many`, as RSZ-03. | - | - | Y |
| RSZ-05 | landing 100x30 then 130x30 | resize, `k` | landing header re-laid at the split-column width (bar 37). Corrected 2026-10-20: the bar width is tui/BUG-01; the contract is that the resized landing equals one opened at 130x30 | - | - | Y |
| RSZ-06 | uncatalogued 120x40, color | resize to 70x30 at 0.5 s into the count-up | the remaining frames use the new geometry; final landing clean, one face | - | animation repaints without a key | Y |
| RSZ-07 | CI and handoff screens | resize, key | menus re-truncate to the new width | - | - | N |
| RSZ-08 | repo viewer at the last issue | shrink to 60x20, `k` | selection stays visible after the viewport is reapplied Corrected 2026-10-20: run on `many`, since the position after `k` needs at least two issues. | unit `the_viewport_follows_the_selection_in_both_directions` | - | Y (partial) |

### 3.7 `tui-clipboard.test.ts`: Enter in the viewer copies one rule

| ID | Setup | Keys | Expected | Covered | Determinism | V |
|---|---|---|---|---|---|---|
| CLIP-01 | repo, `P.fake`, 120x40 | `Enter`, `G`, `Enter` | detail shows `✓ Copied issue context`; fake `wl-copy` called with no argument; payload `Fix exactly one Rust Doctor rule in rust-doctor:`, `Affected sites:` with 8 sites then `- +18 more sites`, `Learn more: https://rust-doctor.com/rules/clippy%3A%3Aprint_stdout`, ends `Verify with \`rust-doctor . --yes --verbose\`...` | unit (payload) | never on `P.real` | Y |
| CLIP-02 | todo, `P.fake`, 80x23 and 100x23 | `Enter`, `Enter` | compact: the status row's key hints become `✓ Copied issue context · esc back · q to quit`. Corrected 2026-10-20: at 80 columns the row is cut after the feedback (`✓ Copied issue context · e…`); only 100 columns shows the whole hint | - | wait on `issue 1/` | Y |
| CLIP-03 | todo, `P.fake`, 100x40 | `Enter`, `Enter` | BUG-03: clipboard written, but no feedback anywhere on screen (stacked detail capped at 15 rows) | - | - | Y |
| CLIP-04 | todo, `P.tc`, 120x40 | `Enter`, `Enter` | `Couldn't copy issue context. Press Enter to try again.` | - | - | Y |
| CLIP-05 | todo, `P.tc`, 80x23 | `Enter`, `Enter` | `Copy failed · enter retry` in the status row | - | - | Y |
| CLIP-06 | todo, `P.fake`, `RD_CLIP_MODE=fail`, 100x23 | `Enter`, `Enter` | `Copy failed` | unit `non_zero_agent_and_clipboard_fallback_are_typed` | - | Y |
| CLIP-07 | todo, `P.fake`, `RD_CLIP_MODE=hang`, 100x23 | `Enter`, `Enter`, `q` | `Copy failed` after 2.01 s (the 2 s `CLIPBOARD_TIMEOUT`); the `q` typed during the hang is processed right after, exit 0; no leftover `wl-copy` | - | timing assertion: 1.9 to 3 s | Y |
| CLIP-08 | repo, `P.fake` | `Enter`, `Enter`, `j` | feedback cleared by the move | - | - | Y |
| CLIP-09 | todo, `P.tc` | `Enter`, `Enter`, `Enter` | retry re-runs the copy, same failure | - | - | Y |
| CLIP-10 | denied, `P.fake` | `Enter`, `Enter` | payload line `ERROR Bugs: Todo (clippy::todo, ×1)` | unit | - | Y |
| CLIP-11 | repo, `P.fake`, last issue | `G`, `Enter` | payload line `WARN Maintainability: Print stdout` for an info rule (BUG-02, second face) | - | - | Y |
| CLIP-12 | `P.fake` with only `xclip`/`xsel` fakes | `Enter`, `Enter` | `xclip -selection clipboard`, `xsel --clipboard --input`; order wl-copy, xclip, xsel | unit | - | N |
| CLIP-13 | hostile fixture (message with escape sequences) | `Enter`, `Enter` | payload carries no control character, at most 12 KiB | unit `the_issue_prompt_sanitizes_report_text...` | - | N |

### 3.8 `tui-ci-workflow.test.ts`: the GitHub Actions screens

Each scenario diffs `git status --porcelain --untracked-files=all` of the copy before and after.

| ID | Setup | Keys | Expected | Covered | Determinism | V |
|---|---|---|---|---|---|---|
| CI-01 | todo (no .git), todo-git, todo-git after install | wait | entry absent, present, absent (`ci::can_install`: `.git` exists and no `rust-doctor.yml`) | unit | - | Y |
| CI-02 | todo-git | `Down Enter` | `? Add Rust Doctor to GitHub Actions?`, the 3 justification rows, `❯ Yes, add the workflow`, `› Open the GitHub Actions guide`, hint `↑/↓ move · enter select · esc back · q quit` | - | - | Y |
| CI-03 | todo-git, `P.fake` | `Down Enter Down Enter` | `✓ Opened the GitHub Actions guide in your browser`; fake `xdg-open` got `https://github.com/arthjean/rust-doctor#github-actions`; porcelain empty | - | never on `P.real` (opens a browser) | Y |
| CI-04 | todo-git, `P.fake`, `RD_OPEN_MODE=fail` | same | `Couldn't open a browser. Visit https://github.com/arthjean/rust-doctor#github-actions` | - | - | Y |
| CI-05 | todo-git, `P.tc` (no xdg-open) | same | same failure notice | - | - | Y |
| CI-06 | todo-git | `Down Enter Escape` | landing, cursor on the CI entry, justification visible | unit | - | Y |
| CI-07 | todo-git, stderr on the terminal | `Down Enter Enter` | TUI exits at once, exit 0; `Added .github/workflows/rust-doctor.yml.` printed under the last frame; porcelain `?? .github/workflows/rust-doctor.yml`; file starts `# Written by rust-doctor ci install`, `branches: ['main']` | unit `installing_the_workflow_leaves_the_landing_pointing_at_a_real_entry` | delete `.github` after | Y |
| CI-08 | same, stderr to a file | same | the confirmation is only in the file; the last frame is the unchanged CI screen (BUG-13) | - | - | Y |
| CI-09 | todo-nocommit | `Down Enter Enter` | stays: `Could not add the workflow: no default branch found: pass --branch <NAME>`; porcelain unchanged (BUG-10) | unit `a_workflow_that_could_not_be_written_reports_it...` | - | Y |
| CI-10 | todo-otherwf | same | `Could not add the workflow: .github/workflows/scan.yml already runs rust-doctor, so nothing was wr…` (BUG-10) | - | - | Y |
| CI-11 | todo-rogh | same | `Could not add the workflow: could not write .github/workflows/rust-doctor.yml (permission denied)` | - | `chmod 555`; tests must not run as root | Y |
| CI-12 | `.github` a symlink | same | `... is a symlink, and nothing is written through one` | - | - | N |
| CI-13 | workspace nested in a git repo (no `.git` at its root) | wait | no CI entry | T1/T2 implicitly (live repo fixture) | - | N |
| CI-14 | todo-git, after a failed install | `Down Enter` (Open guide) | the guide notice replaces the failure notice | - | - | N |

### 3.9 `tui-handoff.test.ts`: recommendation, agents, prompt

| ID | Setup | Keys | Expected | Covered | Determinism | V |
|---|---|---|---|---|---|---|
| HO-01 | todo-git, `P.fake` | `Down Down Enter` | `Add Rust Doctor to GitHub Actions first`, `❯ Add to GitHub Actions first (Recommended)`, `› Continue without GitHub Actions`, hint `... esc skip · q quit` | - | - | Y |
| HO-02 | same | `Escape` | handoff menu, nothing written | - | - | Y |
| HO-03 | same | `Down Enter` | handoff menu, nothing written | - | - | Y |
| HO-04 | same | `Enter` | workflow written, handoff menu; then `Escape`: landing without the CI entry, cursor on `Hand off to an agent`; `Enter` goes straight to the handoff | unit | delete `.github` after | Y |
| HO-05 | todo-nocommit, todo-otherwf, todo-rogh | `Down Down Enter Enter` | failure notice under the recommendation, stays; `Escape` skips to the handoff | unit | - | Y |
| HO-06 | todo, `P.fake` | `Down Enter` | `Choose how to continue`, `❯ Claude Code`, `› Codex`, `› Cursor`, `› Copy prompt` | - | agent list comes from PATH | Y |
| HO-07 | todo, `P.fake9`; `P.tc` | `Down Enter` | `❯ Codex`, `› Copy prompt`; `❯ Copy prompt` + `Paste into any agent or edit it first` | - | - | Y |
| HO-08 | todo-git, handoff | `Escape` | landing, cursor on `Hand off to an agent` | unit | - | Y |
| HO-09 | todo-git, `P.fake` | `Down Down Enter Escape Enter` (first agent) | TUI exits; `claude.argc` 1; `claude.cwd` = workspace; payload starts `The rust-doctor skill is not installed for this agent: run \`rust-doctor skill install --agent claude\` first.`, contains `Audit: 90/100 Great (authoritative).`, `Target: ... projected 100/100.`, `Validate with: rust-doctor . --yes`; `FAKE-AGENT claude RAN` under the frame; exit 0 | T10 (codex) | - | Y |
| HO-10 | todo, `P.fake`, stderr on the terminal | handoff `Down Enter` | `codex.tty` = `yyy`; with stderr redirected `yyn` | T10 | - | Y |
| HO-11 | todo-skill, `P.fake` | handoff `Enter` | payload's first line is `Fix the highest-priority Rust Doctor findings...` (no skill line) | - | - | Y |
| HO-12 | denied, `P.fake9` | handoff `Enter` | agent prints and exits 9; rust-doctor exits 1 (scan code) | unit | - | Y |
| HO-13 | todo, agent `#!/nonexistent/interpreter` | handoff `Enter` | `Warning: could not launch ...` on stderr, prompt between `---- Agent prompt ----` rulers, scan exit code | T9 | - | N (covered) |
| HO-14 | todo-git, `P.fake`, after HO-04 install | `Copy prompt` | stderr `Added .github/workflows/rust-doctor.yml.` then `Prompt copied to clipboard.`; clipboard holds the prompt (1229 bytes) | - | - | Y |
| HO-15 | todo, `P.tc` | handoff `Enter` | stderr `Clipboard unavailable: no clipboard command on PATH. The prompt follows.`; stdout two rulers around the prompt (`Validate with: rust-doctor . --yes`) | T9 | prompt wraps at the pane width in `capture-pane`; join lines or read the byte log | Y |
| HO-16 | todo, `--rule`, `--category`, `--blocking`, `--package`, `--scope files --base HEAD` | handoff, Copy | `Validate with:` repeats every flag, shell-quoted, ending `--yes` | unit `rescan_command_preserves_policy_overrides` | - | N |
| HO-17 | workspace whose top-three groups exceed 12 KiB | handoff | prompt refused: `Handoff prompt exceeds the 12 KiB limit.` on stderr | unit | hard to craft a fixture | N |

### 3.10 `tui-interrupt-recovery.test.ts`: signals by phase

| ID | Setup | Keys / signal | Expected | Covered | Determinism | V |
|---|---|---|---|---|---|---|
| SIG-01 | sleeping | wait `Compiling dependencies...` (0.07 s), `C-c` | exit 130 within 0.03 s; `Scan failed: The scan was interrupted by a signal. (clippy/interrupted)`; no menu; no surviving build script (`pgrep`) | - | fixture sleeps 120 s, so the window is wide | Y |
| SIG-02 | sleeping | `C-c C-c` | exit 130 immediately, nothing printed (second signal `_exit`s). Corrected 2026-10-20: since US-010 two signals within 500 ms are one delivery, so the pair ends as SIG-01 does, with one `interrupted` failure line; the `_exit` path needs a second press after the window | - | - | Y |
| SIG-03 | sleeping | `kill -TERM` during the scan | as SIG-01 | unit | - | N |
| SIG-04 | denied, color | `C-c` 0.3 s into the count-up | BUG-05: exit 130 instead of 1; frame frozen mid-count (`36 / 100`), `^C` echoed | - | send after `/ 100` appears and before the hint | Y |
| SIG-05 | denied | `C-c` after the hint | exit 1 (scan code); byte log ends with `ESC[?25h` after the single `ESC[?25l` | T7 (exit 0 only) | - | Y |
| SIG-06 | denied | `kill -TERM <pid>` while waiting for a key | BUG-06: exit 130; no `ESC[?25h` after `ESC[?25l`; in an interactive bash the cursor stays hidden (`cursor_flag` 0); the tty was `-icanon -echo` at kill time. Corrected 2026-10-20: tui/BUG-06 is fixed by US-010, so exit 130 with an `ESC[?25h` after the last `ESC[?25l`; the loop may repaint one frame after the restore, which never hides the cursor | - | find the pid with `pgrep -n -f 'release/rust-doctor ws/'` | Y |
| SIG-07 | denied | `kill -INT <pid>` while waiting | same as SIG-06. Corrected 2026-10-20: exit 130, cursor shown (US-010) | - | - | Y |
| SIG-08 | denied | `kill -HUP <pid>` | exit 129, cursor left hidden. Corrected 2026-10-20: US-010 handles SIGHUP as SIGTERM, so exit 130 with the cursor shown | - | - | Y |
| SIG-09 | todo, `P.fake` with a long-running fake agent | `kill -INT` while the agent runs | agent and rust-doctor exit; scan exit or 130. Corrected 2026-10-20: automated with a terminal Ctrl-C, which reaches the agent's foreground group; observed exit 130 | - | - | N |

### 3.11 `tui-terminal-hygiene.test.ts`: bytes on the wire

| ID | Setup | Keys | Expected | Covered | Determinism | V |
|---|---|---|---|---|---|---|
| HYG-01 | repo, color | `q` | SGR present; OSC 8 links (236 `ESC]8;;` in one landing session); exactly one `ESC[?25l` (after the count-up) and one `ESC[?25h` (at exit); no `ESC[?1049h` (no alternate screen); the last frame survives in the scrollback | - | read the pipe-pane log | Y |
| HYG-02 | todo, `NO_COLOR=1` | `q` | no SGR, no OSC 8, no count-up, landing still opens | unit | - | Y |
| HYG-03 | todo, `NO_COLOR=` (empty) | `q` | BUG-11: color and animation off as if set. Corrected 2026-10-20: tui/BUG-11 is fixed by US-014, so color and the count-up stay on | - | - | Y |
| HYG-04 | any | after each frame | every row ends with `ESC[K`, every frame with `ESC[J`, rewinds as `ESC[<n>A` | - | - | Y |
| HYG-05 | repo, many keys | after each key | exactly one score face on screen | - | - | Y |
| HYG-06 | uncatalogued | `j q` typed 0.3 s into the count-up | BUG-04: `jq` echoed into the frame; after the count-up `j` moves the menu and `q` quits, actions the reader never saw | - | - | Y |
| HYG-07 | uncatalogued | `Down Enter` typed during the count-up | BUG-04: the echoed newline shifts the rewind; final screen shows a stale `25 / 100` header row above `Choose how to continue`. Automated as a tui/BUG-04 `test.failing` row expecting one clean landing | - | - | Y |
| HYG-08 | hostile fixture | `Enter` | workspace text with ESC/OSC never reaches the frame | unit `a_span_never_carries_an_escape_sequence_into_a_frame` | - | N |

### 3.12 `tui-exit-codes.test.ts`

| ID | Setup | Path out | Expected exit | Covered | V |
|---|---|---|---|---|---|
| EXIT-01 | clean, todo, repo | `q`, `Escape`, `C-c` | 0 | T7 | Y |
| EXIT-02 | denied, partial, broken-member, no-rust | `q` | 1 | - | Y |
| EXIT-03 | todo `--blocking warning` | `q` | 1 | - | Y |
| EXIT-04 | todo `--warning-exit-code 7` | `q` | 7 | - | Y |
| EXIT-05 | sleeping `--max-duration 1` | `q` | 2 | - | Y |
| EXIT-06 | bad-manifest, nonexistent path | none (no TUI) | 2 | - | Y |
| EXIT-07 | todo-git | CI `Yes` | 0 | - | Y |
| EXIT-08 | denied | agent launch (agent exits 9) | 1 | - | Y |
| EXIT-09 | sleeping | `C-c` during scan | 130 | - | Y |
| EXIT-10 | denied | `C-c` during count-up | 130 (BUG-05) Corrected 2026-10-20: run on broken-member (scored, exit 1), since denied has no score and no count-up since US-009; still 130, the open BUG-05. | - | Y |
| EXIT-11 | denied | `kill -TERM` / `-HUP` in report | 130 / 129 Corrected 2026-10-20: 130 for both since US-010 (FR-08), and the terminal is given back (`stty -a` after the exit shows `icanon echo`, the byte log shows the cursor last); a run can still exit with the scan code instead, one in twelve here and two in five pinned to one CPU (tui/BUG-14). | - | Y |
| EXIT-12 | any | `Interactive report unavailable` (tui::run I/O error) | scan code; a handoff error after a 0 scan gives 2 Corrected 2026-10-20: provoked by a stdin terminal lent by a second pane and hung up under the read: `Interactive report unavailable: Input/output error (os error 5)`, exit = scan code (1 on denied). The handoff-error half is HO-17 (prompt over 12 KiB). | - | N |

### 3.13 `tui-performance.test.ts`

| ID | Setup | Measure | Budget suggested | V |
|---|---|---|---|---|
| PERF-01 | each fixture of 1.5 | time to hint and to a 0.4 s stable grid | the table in 1.5 + 50%. Corrected 2026-10-20: since US-009 `denied` and `partial` publish no score, so they draw no count-up and take the `no-rust` budget (hint 0.09 to 0.11 s measured) | Y |
| PERF-02 | repo split | `k` to `issue 11/12` repaint | under 100 ms (measured 10.5 ms) | Y |
| PERF-03 | uncatalogued | count-up duration | 2.0 to 3.2 s (41 x 50 ms + 16 x 35 ms + scan) | Y |
| PERF-04 | uncatalogued | frames before first key | 58 `ESC[J` | Y |
| PERF-05 | repo, idle 3 s | CPU ticks of the process | 0 (blocking read, no polling) | Y |
| PERF-06 | repo | bytes written for the landing session | about 36 KB; regression guard | Y |

## 4. Bugs and oddities, with evidence and cause

| ID | Observation (verified) | Cause |
|---|---|---|
| BUG-01 | Landing header truncated at 120x40 (`rust-doc…`, `Rust Doctor (https://rust-doctor…`, `+…`, widest row 44) but complete at 80 and 200; bar 50 at 119x30, 33 at 120x22, 37 at 130, 50 again at 165. Confirms the earlier report. | `frames.rs` `landing_frame` passes `layout.score_header_width`; `model.rs` `resolve_layout` sets it to `list_column_width` (45 at 120) whenever the arrangement is Split (at least 120x22), a split the landing never draws. |
| BUG-02 | Status bar `236 findings  0 errors  236 warnings` on the repo; the linear report says `All 236 occurrences across 174 findings` and `52 info`. The issue payload labels the info rule `WARN Maintainability: Print stdout`. Confirms the earlier report. | `viewer.rs` `status_bar` sums `row.site_count` (occurrences) under the word "findings" and buckets everything not `is_error()` as a warning; `input.rs` `copy_issue_context` passes `is_error` only, and `build_issue_prompt` maps false to `WARN`. |
| BUG-03 | In the stacked layout (100x40), Enter copies (fake clipboard received the payload) but neither `✓ Copied issue context` nor the failure line appears. | `viewer.rs` `detail_pane` appends the feedback after every section, then `Stacked` keeps `detail.take(layout.detail_height)` (15): any detail longer than 15 rows hides it. Split hides it too once the detail exceeds the column. |
| BUG-04 | Keys typed during the count-up are echoed (`jq`, `^[[B`) and replayed once the menu appears; an echoed Enter shifts every later rewind and leaves a stale `25 / 100` row above the next screen. | `tui.rs` `reveal_score` runs in cooked mode with ECHO (the comment at `App::run` says so); raw mode starts only in `read_key`, and pending input is never flushed. |
| BUG-05 | Ctrl-C during the count-up exits 130 with the frame frozen at `36 / 100`, where Ctrl-C a moment later exits with the scan code (1). | ISIG is still on, so SIGINT reaches `execution::interrupt::on_signal`; with `ACTIVE == 0` the watcher thread calls `std::process::exit(130)`. |
| BUG-06 | `kill -TERM`/`-INT` while the report waits exits 130 and never writes `ESC[?25h`; in a bash pane the cursor stays hidden after the prompt returns (`cursor_flag` 0). The tty was raw (`-icanon -echo`) at kill time; bash restored termios, a shell that does not would be left raw. `-HUP`: 129, same leak. | `interrupt.rs` `watch` exits the process from its own thread; `Canvas::restore` and the termios restore inside `console::read_key_raw` never run. |
| BUG-07 | At 40x10 and 24x8, `Down Down` moves the cursor onto `Hand off to an agent`, which is not on screen; at 80x12 the key hint is gone. | `Canvas::draw` keeps the first `rows - 1` lines; menus never scroll (`screens/menu.rs`). |
| BUG-08 | After `resize-window` the stale, reflowed frame stays until a key arrives (three score faces visible after growing to 200x50). | No SIGWINCH handling: the loop only repaints after `read_key` returns. Any key repairs it. |
| BUG-09 | In a scrolled list (repo, 100x40), after `G` then `gg` the `Security` header never returns. | `move_to(0)` lands on the first selectable entry (index 1) and `visible_start` only guarantees the selection is visible. |
| BUG-10 | The CI entry is labeled Recommended where the install is certain to fail: another workflow already runs rust-doctor (`todo-otherwf`), or the repository has no commit (`todo-nocommit`), whose notice says `pass --branch <NAME>`, an option the TUI cannot take. Notices truncate (`...so nothing was wr…` at 100 columns). | `ci::can_install` checks only `.git` and `rust-doctor.yml`, while `install_default` also refuses on `existing_install` and `BranchUndetected`; `menu` truncates every line to the frame width. |
| BUG-11 | `NO_COLOR=` (empty) turns color and the count-up off. no-color.org: only a non-empty value counts. | `terminal_profile.rs` `resolve`: `variable("NO_COLOR").is_none()`, while the agent markers treat empty as unset. |
| BUG-12 | `ESC` followed by any byte in the same read (`ESC CR`, `ESC j`, `ESC ESC`) is swallowed whole; SS3 arrows (`ESC O B`) are ignored; text pasted with `q` in it quits. | `console` 0.15.11 `read_key_raw` parses the pending bytes as one unknown escape sequence; the TUI enables neither DECCKM handling nor bracketed paste. |
| BUG-13 | `Yes, add the workflow` exits the report with no in-frame confirmation; with stderr redirected the terminal shows nothing about the file just written. | `input.rs` `handle_ci` returns `Outcome::Quit` on success; `main.rs` reports `Added ...` with `eprintln!`. |
| ODD-01 | The same fixture scores 94 or 90 depending on whether it is a git repository. | `rust_doctor::repo::unignored_build_output` needs git. Test data issue, not a defect. |
| ODD-02 | Linear report prints `Security: 0 errors, 1 warnings` (plural with 1). | Outside the TUI; noted while comparing BUG-02. |
| ODD-03 | A relative `CARGO_TARGET_DIR` writes inside the scanned workspace. | Cargo semantics; a harness constraint. |

## 5. Determinism checklist for the TypeScript suite

1. `env -i` with an allow list (1.1). Absolute `CARGO_TARGET_DIR` per workspace, warmed once in `beforeAll` with `--yes --json`.
2. Materialize every workspace by copy (`rsync --exclude target`), set its git state explicitly, and pin `repo` to a SHA or assert shapes only.
3. tmux: `-S <short relative socket>`, `window-size manual`, `escape-time 0`, `remain-on-exit on`, `status off`, explicit `-x/-y`; kill the server in `afterAll`.
4. Wait on the hint, not on `/ 100`, unless the test is about the count-up. `NO_COLOR=1` removes 2 to 2.6 s per test where color is not under test.
5. Send `Escape` alone and wait for the next frame before any further key (BUG-12). Send arrows with `send-keys Down`, which writes one CSI sequence.
6. After every `resize-window`, send a key before asserting (BUG-08).
7. Never press Enter on Copy, Open guide, Yes, or an agent with `P.real`. Assert writes with `git status --porcelain --untracked-files=all` and reset the copy after.
8. Assert cursor restore and escape hygiene on the `pipe-pane` byte log, never on `#{cursor_flag}` of a dead pane.
9. Prompt and clipboard payloads: read the files the fakes wrote, not the wrapped `capture-pane` text.
10. Timings: allow 50% headroom over 1.5 and run performance tests serially.
