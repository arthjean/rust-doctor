# The opt-in rules checked against the corpus (2026-10-29)

US-026 of the self-scan remediation, before `clippy::indexing_slicing` and
`clippy::string_slice` turned opt-in. Every site `tests/corpus.json` adjudicated
for either rule is listed below with its verdict, read from the record at
commit `aa65611`. Both rules were sampled on the healthy population only: the
agent population is scanned with every Clippy rule off.

| Rule | Reviewed | True positives | False positives |
|---|---|---|---|
| `clippy::indexing_slicing` | 40 | 0 | 40 |
| `clippy::string_slice` | 40 | 0 | 40 |

No true positive exists, so US-027 proceeds. The defect the rules exist for
is real all the same: the 0.9.0 self-scan's `string_slice` site at
`src/repo_hygiene/credentials.rs:359` sliced a line at a byte offset one past
a character found by a char predicate, and a multibyte character before `://`
aborted the scan (US-001). Code that slices untrusted text switches them on.

## `clippy::indexing_slicing`

| Repository | Site | Verdict | Provenance |
|---|---|---|---|
| bytes | `src/buf/buf_impl.rs:67` | false positive | unrecorded |
| bytes | `src/buf/buf_impl.rs:1216` | false positive | agent |
| bytes | `src/buf/buf_mut.rs:258` | false positive | agent |
| bytes | `src/buf/iter.rs:115` | false positive | agent |
| bytes | `src/buf/uninit_slice.rs:243` | false positive | agent |
| fd | `src/exec/mod.rs:172` | false positive | agent |
| hexyl | `src/lib.rs:114` | false positive | agent |
| hexyl | `src/lib.rs:558` | false positive | agent |
| log | `src/lib.rs:670` | false positive | unrecorded |
| ripgrep | `crates/core/flags/hiargs.rs:875` | false positive | agent |
| ripgrep | `crates/core/flags/parse.rs:472` | false positive | agent |
| ripgrep | `crates/globset/src/glob.rs:551` | false positive | agent |
| ripgrep | `crates/globset/src/lib.rs:1009` | false positive | agent |
| ripgrep | `crates/ignore/src/gitignore.rs:272` | false positive | agent |
| ripgrep | `crates/ignore/src/pathutil.rs:122` | false positive | agent |
| ripgrep | `crates/ignore/src/types.rs:447` | false positive | agent |
| ripgrep | `crates/matcher/src/interpolate.rs:28` | false positive | unrecorded |
| ripgrep | `crates/matcher/src/interpolate.rs:119` | false positive | agent |
| ripgrep | `crates/matcher/src/lib.rs:454` | false positive | agent |
| ripgrep | `crates/printer/src/color.rs:320` | false positive | agent |
| ripgrep | `crates/printer/src/hyperlink/mod.rs:921` | false positive | agent |
| ripgrep | `crates/printer/src/json.rs:876` | false positive | agent |
| ripgrep | `crates/printer/src/jsont.rs:247` | false positive | agent |
| ripgrep | `crates/printer/src/jsont.rs:268` | false positive | agent |
| ripgrep | `crates/printer/src/standard.rs:1085` | false positive | unrecorded |
| ripgrep | `crates/printer/src/standard.rs:1153` | false positive | agent |
| ripgrep | `crates/printer/src/standard.rs:1276` | false positive | agent |
| ripgrep | `crates/printer/src/util.rs:433` | false positive | agent |
| ripgrep | `crates/printer/src/util.rs:541` | false positive | agent |
| ripgrep | `crates/searcher/src/line_buffer.rs:368` | false positive | agent |
| ripgrep | `crates/searcher/src/line_buffer.rs:547` | false positive | agent |
| ripgrep | `crates/searcher/src/lines.rs:137` | false positive | agent |
| ripgrep | `crates/searcher/src/searcher/core.rs:254` | false positive | unrecorded |
| ripgrep | `crates/searcher/src/searcher/core.rs:511` | false positive | agent |
| ripgrep | `crates/searcher/src/searcher/glue.rs:59` | false positive | agent |
| serde_json | `src/read.rs:422` | false positive | agent |
| serde_json | `src/read.rs:517` | false positive | agent |
| serde_json | `src/read.rs:622` | false positive | agent |
| serde_json | `src/read.rs:1079` | false positive | agent |
| thiserror | `impl/src/expand.rs:149` | false positive | agent |

## `clippy::string_slice`

| Repository | Site | Verdict | Provenance |
|---|---|---|---|
| bytes | `src/bytes.rs:862` | false positive | unrecorded |
| bytes | `src/bytes_mut.rs:1745` | false positive | agent |
| fd | `src/filter/owner.rs:91` | false positive | agent |
| fd | `src/filter/size.rs:46` | false positive | agent |
| fd | `src/fmt/mod.rs:73` | false positive | agent |
| fd | `src/fmt/mod.rs:74` | false positive | agent |
| fd | `src/fmt/mod.rs:76` | false positive | agent |
| fd | `src/fmt/mod.rs:77` | false positive | agent |
| fd | `src/fmt/mod.rs:82` | false positive | unrecorded |
| fd | `src/fmt/mod.rs:88` | false positive | agent |
| fd | `src/fmt/mod.rs:89` | false positive | agent |
| fd | `src/output.rs:96` | false positive | agent |
| fd | `src/output.rs:106` | false positive | agent |
| fd | `src/output.rs:122` | false positive | agent |
| ripgrep | `crates/cli/src/human.rs:82` | false positive | agent |
| ripgrep | `crates/cli/src/human.rs:89` | false positive | agent |
| ripgrep | `crates/core/flags/doc/mod.rs:26` | false positive | unrecorded |
| ripgrep | `crates/core/flags/doc/mod.rs:29` | false positive | agent |
| ripgrep | `crates/core/flags/doc/mod.rs:32` | false positive | agent |
| ripgrep | `crates/core/flags/doc/mod.rs:34` | false positive | agent |
| ripgrep | `crates/globset/src/lib.rs:498` | false positive | agent |
| ripgrep | `crates/ignore/src/dir.rs:1081` | false positive | agent |
| ripgrep | `crates/ignore/src/gitignore.rs:483` | false positive | agent |
| ripgrep | `crates/ignore/src/gitignore.rs:488` | false positive | agent |
| ripgrep | `crates/ignore/src/gitignore.rs:495` | false positive | unrecorded |
| ripgrep | `crates/ignore/src/gitignore.rs:503` | false positive | agent |
| ripgrep | `crates/ignore/src/gitignore.rs:507` | false positive | agent |
| ripgrep | `crates/matcher/src/lib.rs:179` | false positive | agent |
| serde_json | `src/error.rs:503` | false positive | agent |
| serde_json | `src/error.rs:507` | false positive | agent |
| serde_json | `src/error.rs:514` | false positive | agent |
| serde_json | `src/error.rs:523` | false positive | agent |
| serde_json | `src/error.rs:527` | false positive | unrecorded |
| serde_json | `src/value/index.rs:109` | false positive | agent |
| serde_json | `src/value/index.rs:112` | false positive | agent |
| serde_json | `src/value/index.rs:115` | false positive | agent |
| thiserror | `impl/src/fmt.rs:44` | false positive | agent |
| thiserror | `impl/src/fmt.rs:45` | false positive | agent |
| thiserror | `impl/src/fmt.rs:48` | false positive | agent |
| thiserror | `impl/src/fmt.rs:94` | false positive | agent |
