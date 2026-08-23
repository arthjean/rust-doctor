# The reports

The wire format, the linear renderer, the interactive report and the code
frame: everything between a producer's finding and a reader.

## The published report

`src/report.rs` is the wire format and nothing else: the request that starts a
scan, the report that comes back, `SCHEMA_VERSION`, and the closed vocabularies
its members draw from. `report/assembly.rs` builds one from an execution,
`report/normalize.rs` turns a producer's finding into one of its diagnostics,
`report/sanitize.rs` takes every published path and home directory out of the
text a scan produced, and the tests sit in `report/tests.rs` and
`report/tests/normalization.rs`.

Four rules hold it together, and each of them replaced something that had a
cost.

One merge, one order. `diagnostics_from_execution` keeps one `BTreeMap` open
across all five producers and sorts once at the end. Three word-for-word
`merge_*` functions used to close it between producers, each rebuilding the map
it had just been handed and re-sorting behind it: the vector was sorted five
times per scan, every diagnostic id was cloned three times over, and the guard
that skips a failed scan was spelled three more times after the `match` above
had already answered it.

Two states, not eight. `Origin` says whether the run compiled a plan and
resolved a scope, or failed before either existed. It used to be an
`Option<&PolicyPlan>`, a `BlockingLevel` and an `Option<ScopeReport>` side by
side: every caller holding a plan passed that plan's own blocking level, and the
single caller without a plan was also the one without a scope.

One escape grammar, in `terminal_text`. This module carried a second, and
neither copy was complete: `terminal_text` advanced two characters past any
`ESC` it did not recognize, so `ESC ( B` left a bare `B` in a frame, and no
sequence was ever cancelled by `CAN` or `SUB`. What separated the two callers
was never the grammar but whether a newline survives it, which is what
`sanitize` and `sanitize_multiline` now name.

No pass-through with a second name. `baseline_report_failure` had a
`baseline_cleanup_failure` in front of it that called it and nothing else, and
`summarize` stood in front of `Summary::from_diagnostics`.

`the_report_holds_the_size_bound_it_publishes` keeps every file of the module
under the 1000 lines `oversized_unit` reports, tests included. This was one file
of 3226 lines, three times over the bound, and the self-scan that named it froze
the defect rather than gating it: a test asserting the crate's largest violation
is a test that fails the day the violation is repaired.

## The linear report

`src/render.rs` is the report every non-interactive run prints: the error type,
the terminal options, the three entry points, the eight section renderers and
the one styled-line primitive they all write through. `src/render/score_header.rs`
is the score block, and each of the two carries its tests in a file of its own.

Five rules hold it together, and each of them replaced something that had a
cost.

The report is its sections, in order. `render_terminal_with_presentation` is
twelve calls and nothing else: a line written straight into the entry point is
a section nobody named, and eight of them are what made it one of the module's
three complexity hotspots. `render_legacy_context` was the other shape of the
same problem, five unrelated sections in one function whose name admitted it,
reading `report.delta` at both ends with three other sections in between.

One width, guaranteed rather than branched on. `MIN_WIDTH` is at least
`score_block::MIN_BLOCK_COLUMNS`, asserted at compile time, and every entry
point normalizes to it, so the score block always fits. That deleted a whole
dimension the module used to pay for and never reach: an optional constructor,
a `drawn` flag threaded back through two modules, and a second single-line
renderer whose bar truncated where the shared `bar_fill` rounds up. The two
questions the geometry answers are now separate: `right_column_width` is total,
`bar_width` is the one that says whether the block fits, and only the
interactive report, which really does draw at forty columns, reads the second.

One frame builder. A frame is the four rows composed under one `Palette`, and
the palette is the only thing separating the counting frames from the scrolling
ones and from the frame the block freezes on. Three builders used to carry the
same loop, two of them identical but for a bolted-in `index == 1`, and the
frozen frame was computed once in each of the two paths that show it.

One view, not two booleans. `GroupView` says whether the report is drawing the
worst group with its first location or every group with every location. It used
to be `top: bool, all_locations: bool`, four combinations for the two that
exist, and a call site read `(.., true, false)`; the limit that pair encoded was
applied through a `Box<dyn Iterator>` allocated to choose between `iter()` and
`iter().take(1)`.

One row, built from its pieces. A `Row` carries the segments it paints
differently, so the score line is never formatted and split back apart on
`/ 100` to find its denominator, nor the branding on `" ("` to find its URL. A
cut now lands inside one piece and leaves the pieces before it whole, which is
what the comment on that code already claimed and what the string round-trip
could not do.

A failed scan is two sections, not twelve. `render_failure` is the scope the
scan was attempted under and the stage that failed, and nothing else, because
every other section counts, tallies, ranks or scores a population no producer
ever produced. All twelve used to run on a failure too: a run that dies in the
toolchain preflight still carries an inventory of the workspace it never
scanned, so the score was built from that count against zero diagnostics, and a
`Scan failed` line was followed by `No issues found.` and a `100 / 100` face.
That retired the `allow_links` dimension of the score block with it, whose one
job was to drop the branding URL from a block that is no longer drawn.

`the_report_holds_the_size_bound_it_reports_for` keeps every file of the module
under the 1000 lines `oversized_unit` reports, tests included. That is why the
two test modules have files of their own: the report has to pass the rule it
prints.

## The interactive report

`src/tui/` is a transposition of React Doctor's Ink application, screen for
screen: the landing score block and its action menu, the split review with the
rule list on the left and the detail on the right, the agent handoff, and the
two GitHub Actions screens. `model.rs` carries the geometry of
`resolve-report-layout.ts` unchanged, so the same terminal size picks the same
split, stacked or compact arrangement the reference picks. `screens/menu.rs` is
the shape the four menu screens share, drawing and reading keys once each;
`screens/viewer.rs` is the split review; `screens.rs` keeps the score block.
`text.rs` composes styled spans and measures them in display columns,
`canvas.rs` owns the terminal, and `tui.rs` is the state machine and the frame
loop. `frames.rs` is what a state looks like and `input.rs` is what a key does
to it: they carry an `impl App` each because one block holding both reached 535
lines, over the five hundred `oversized_unit` reports an impl at, and
`the_interactive_report_holds_the_size_bound_the_report_reports_for` is what
keeps every file of the module under the bound it prints.

Two things the two reports share rather than each transposing. `score_block` is
the score block's model: the faces, the label a score carries, how a value
fills the bar, how much room the block needs and the cadence of its count-up.
`terminal_text` is the sanitizer and the ruler. Both are public on the library
for the binary to reach, and both are public for that reason alone. A second
copy of either is what let the two reports disagree on the bar rounding, on the
guard column, and on which escape sequences a diagnostic could smuggle into a
frame.

Three structural rules hold it together. Each screen carries its own cursor
inside the `View` enum, never a flat set of cursors beside a view tag: a cursor
that outlives the list it indexes is how a menu ends up pointing past its own
actions. Every menu answers to one `screens::input`, so no screen carries its
own hard-coded upper bound. And `canvas.rs` truncates every frame to the
terminal minus its last row and last column, because a row that wraps moves the
cursor away from where the next rewind expects it and corrupts every frame
after; screens still size themselves, but none of them can break the loop.

Three things it deliberately does not do. It asks nothing before the scan, the
way React Doctor's TUI path hard-codes `skipPrompts: true` in
`resolve-tui-scan-scope.ts`: the reader answers questions after seeing findings,
not before. It never takes the alternate screen:
frames are rewritten in place the way Ink does, so the last one survives in the
scrollback. And it never renders on a run that asked for `--json`, `--yes` or
`--verbose`, or on a scan that failed, because those readers are pipes, CI,
agents, and someone who needs an error message the report has no room for. Every
existing test asserts against the linear renderer, which is unchanged.

The only file the tool writes into a scanned workspace is
`.github/workflows/rust-doctor.yml`, only from the CI menu entry, and never over
an existing file (`src/tui/workflow.rs`): the refusal is the creation itself,
`create_new`, rather than a check followed by a write. That workflow installs
the published
launcher, `npm install -g rust-doctor@<version>`, pinned to the version of the
binary that wrote it: the pin comes from `CARGO_PKG_VERSION` rather than a
string in the template, so a release cannot forget to move it, and a generated
gate keeps scanning with the rule set its author saw.

## The code frame

`src/presentation/code_frame.rs` is the only place the tool reads a scanned
file's contents back out to a terminal, so it is where the trust boundary is
enforced one file at a time: the path is decoded and canonicalized, the handle
is opened and then revalidated against the live path by device and inode, the
bytes are refused if they carry a NUL or do not decode, and every escape
sequence is neutralized before a byte reaches a frame. Both reports call
`code_frame`, and `src/presentation/code_frame/tests.rs` carries the tests.

Three rules hold it together, and each of them replaced something that had a
cost.

One reader, walking lines. `read_window` reads line by line to the window it
needs and decodes only the lines it keeps. The byte prefix it replaced capped
the read at eight kilobytes and then asked whether the reported line was in it,
so every finding past roughly the first two hundred lines of a file printed
`Code frame unavailable`, which is most of a file `oversized_unit` reports on at
a thousand. It also cut a character in half whenever a multi-byte one straddled
the cap, and a file that valid reported `InvalidUtf8` for every frame it had,
including the ones on its first line.

Every bound is a budget on work, never on reach. `SCAN_MAX_BYTES` bounds what
one frame may scan, `LINE_MAX_BYTES` what one line may contribute, and
`FRAME_MAX_COLUMNS` what one line may render, and none of the three decides
which line is reachable. That is the distinction the byte prefix collapsed:
a window disguised as a budget answers `Unavailable` for a line it simply never
looked at.

One gutter, published by the frame. `CodeFrame::gutter_width` is the width both
reports lay their rows out from. The linear report used to hard-code four
columns while the interactive one computed its own, so a frame reaching line ten
thousand slid the source row one column right and left the caret row where it
was, in one report and not the other. Four columns are now its floor, not its
ceiling.

`the_frame_holds_the_size_bound_the_report_reports_for` keeps both files of the
module under the 1000 lines `oversized_unit` reports, tests included, for the
reason the rest of the crate holds it: the code that renders the finding has to
pass the rule that raised it.

