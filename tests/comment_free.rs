#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::fs;
use std::path::Path;
use std::process::Command;

use ra_ap_syntax::{AstNode, Edition, SourceFile, SyntaxKind, SyntaxToken};

const ROOT: &str = env!("CARGO_MANIFEST_DIR");

fn tracked_files() -> Vec<String> {
    let output = Command::new("git")
        .args(["ls-files", "-z"])
        .current_dir(ROOT)
        .output()
        .expect("git ls-files starts");
    assert!(output.status.success(), "git ls-files failed in {ROOT}");
    String::from_utf8(output.stdout)
        .expect("every tracked path is UTF-8")
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(str::to_owned)
        .collect()
}

fn line_of(source: &str, token: &SyntaxToken) -> usize {
    let offset = usize::from(token.text_range().start());
    source.bytes().take(offset).filter(|byte| *byte == b'\n').count() + 1
}

fn rust_comments(path: &str, source: &str) -> Vec<String> {
    SourceFile::parse(source, Edition::Edition2024)
        .tree()
        .syntax()
        .descendants_with_tokens()
        .filter_map(|element| element.into_token())
        .filter(|token| matches!(token.kind(), SyntaxKind::COMMENT | SyntaxKind::SHEBANG))
        .map(|token| {
            let first = token.text().lines().next().unwrap_or_default().to_owned();
            format!("{path}:{}: {first}", line_of(source, &token))
        })
        .collect()
}

fn is_line_comment(body: &str, markers: &[&str]) -> bool {
    let pinned = body.trim_start_matches("- ").starts_with("uses:") && body.contains(" #");
    pinned || markers.iter().any(|marker| body.starts_with(marker))
}

fn line_comments(path: &str, source: &str, markers: &[&str]) -> Vec<String> {
    source
        .lines()
        .enumerate()
        .filter(|(index, line)| {
            (*index == 0 && line.starts_with("#!")) || is_line_comment(line.trim_start(), markers)
        })
        .map(|(index, line)| format!("{path}:{}: {}", index + 1, line.trim()))
        .collect()
}

fn comments_in(path: &str) -> Vec<String> {
    let file = Path::new(path);
    let extension = file.extension().and_then(|extension| extension.to_str()).unwrap_or_default();
    let name = file.file_name().and_then(|name| name.to_str()).unwrap_or_default();
    let markers: &[&str] = match extension {
        "rs" => &[],
        "ts" | "js" | "mjs" | "cjs" => &["//", "/*"],
        "yml" | "yaml" | "toml" | "sh" => &["#"],
        _ if name == ".gitignore" => &["#"],
        _ => return Vec::new(),
    };
    let Ok(source) = fs::read_to_string(Path::new(ROOT).join(path)) else {
        return Vec::new();
    };
    if extension == "rs" {
        rust_comments(path, &source)
    } else {
        line_comments(path, &source, markers)
    }
}

#[test]
fn the_repository_carries_no_comment() {
    let files = tracked_files();
    assert!(files.iter().any(|path| path == "src/lib.rs"), "the walk missed src/lib.rs");
    let found: Vec<String> = files.iter().flat_map(|path| comments_in(path)).collect();
    assert!(
        found.is_empty(),
        "comments in the code, which reads without them:\n{}",
        found.join("\n")
    );
}

#[test]
fn every_comment_form_is_reported_and_a_string_is_not() {
    let source = "// SAFETY: one\nfn f() {}\n/// doc\nfn h() {\n    let _x = \"// kept\"; // trailing\n}\n/* block */\n";
    assert_eq!(
        rust_comments("a.rs", source),
        vec!["a.rs:1: // SAFETY: one", "a.rs:3: /// doc", "a.rs:5: // trailing", "a.rs:7: /* block */"]
    );
    assert_eq!(
        rust_comments("b.rs", "#!/usr/bin/env run-cargo-script\n#![allow(dead_code)]\nfn f() {}\n"),
        vec!["b.rs:1: #!/usr/bin/env run-cargo-script"]
    );
    let script = "#!/bin/sh\n# note\necho '# kept'\n";
    assert_eq!(
        line_comments("a.sh", script, &["#"]),
        vec!["a.sh:1: #!/bin/sh", "a.sh:2: # note"]
    );
    assert_eq!(
        line_comments("a.js", "#!/usr/bin/env node\nconst s = \"#!\";\n", &["//", "/*"]),
        vec!["a.js:1: #!/usr/bin/env node"]
    );
    let workflow = "steps:\n  - uses: actions/checkout@0123 # v5\n  - uses: actions/cache@4567\n";
    assert_eq!(
        line_comments("a.yml", workflow, &["#"]),
        vec!["a.yml:2: - uses: actions/checkout@0123 # v5"]
    );
    assert_eq!(
        rust_comments("c.rs", "// rust-doctor: allow(a) -- b\nfn f() {}\n"),
        vec!["c.rs:1: // rust-doctor: allow(a) -- b"]
    );
    assert_eq!(
        line_comments("Cargo.toml", "[package]\n# rust-doctor: allow(a) -- b\n", &["#"]),
        vec!["Cargo.toml:2: # rust-doctor: allow(a) -- b"]
    );
}

#[test]
fn every_comment_input_extends_a_line_its_fixture_leaves_for_it() {
    let fixtures = Path::new(ROOT).join("tests/fixtures");
    let inputs: toml::Table = fs::read_to_string(fixtures.join("comment-inputs.toml"))
        .unwrap()
        .parse()
        .unwrap();
    assert!(!inputs.is_empty(), "tests/fixtures/comment-inputs.toml holds no input");
    for (file, lines) in &inputs {
        let tracked = fs::read_to_string(fixtures.join(file)).unwrap();
        let rows: Vec<&str> = tracked.lines().collect();
        let lines = lines.as_table().unwrap();
        assert!(!lines.is_empty(), "{file} lists no line");
        for (number, text) in lines {
            let row = number
                .parse::<usize>()
                .ok()
                .and_then(|number| number.checked_sub(1))
                .and_then(|index| rows.get(index))
                .copied();
            let rest = row.and_then(|row| text.as_str().and_then(|text| text.strip_prefix(row)));
            assert!(
                rest.map(str::trim_start)
                    .is_some_and(|rest| rest.starts_with("//") || rest.starts_with('#')),
                "{file}:{number} should extend the tracked line {row:?} with a comment"
            );
        }
    }
}
