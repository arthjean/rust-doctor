use std::fs::File;
use std::io::Read;
use std::path::Path;

use ra_ap_syntax::{AstNode, SourceFile, SyntaxKind};

use crate::source_text::line_starts;

const MAX_MANIFEST_BYTES: u64 = 4 * 1024 * 1024;

const MARKER: &str = "rust-doctor:";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Directive {
    pub(crate) path: String,
    pub(crate) line: usize,
    pub(crate) rules: Vec<String>,
    pub(crate) has_reason: bool,
    pub(crate) key: Option<String>,
}

pub(crate) fn in_rust(path: &str, source: &str, tree: &SourceFile) -> Vec<Directive> {
    if !source.contains(MARKER) {
        return Vec::new();
    }
    let starts = line_starts(source);
    tree.syntax()
        .descendants_with_tokens()
        .filter_map(|element| element.into_token())
        .filter(|token| token.kind() == SyntaxKind::COMMENT)
        .filter_map(|token| {
            let body = token.text().strip_prefix("//")?;
            if body.starts_with('/') || body.starts_with('!') {
                return None;
            }
            let offset = usize::from(token.text_range().start());
            directive(path, line_of(&starts, offset), body, None)
        })
        .collect()
}

pub(crate) fn in_toml_file(path: &str, file: &Path) -> Vec<Directive> {
    let Ok(handle) = File::open(file) else {
        return Vec::new();
    };
    let mut bytes = Vec::new();
    if handle
        .take(MAX_MANIFEST_BYTES + 1)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() as u64 > MAX_MANIFEST_BYTES
    {
        return Vec::new();
    }
    String::from_utf8(bytes).map_or_else(|_| Vec::new(), |source| in_toml(path, &source))
}

fn in_toml(path: &str, source: &str) -> Vec<Directive> {
    if !source.contains(MARKER) {
        return Vec::new();
    }
    let lines: Vec<&str> = source.lines().collect();
    toml_comments(source)
        .into_iter()
        .filter_map(|(line, body)| {
            let key = lines.get(line).and_then(|below| declared_key(below));
            directive(path, line, body, key)
        })
        .collect()
}

fn declared_key(line: &str) -> Option<String> {
    let line = line.trim();
    let key = match line.strip_prefix('[') {
        Some(header) => header.split(']').next()?.rsplit('.').next()?,
        None => line.split_once('=')?.0.split('.').next()?,
    };
    let key = key.trim().trim_matches(|quote| quote == '"' || quote == '\'');
    (!key.is_empty()).then(|| key.to_owned())
}

fn toml_comments(source: &str) -> Vec<(usize, &str)> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum State {
        Code,
        Basic,
        Literal,
        MultiBasic,
        MultiLiteral,
    }
    let mut comments = Vec::new();
    let mut state = State::Code;
    let mut line = 1;
    let mut index = 0;
    let bytes = source.as_bytes();
    while let Some(&byte) = bytes.get(index) {
        let rest = bytes.get(index..).unwrap_or_default();
        let mut step = 1;
        match (state, byte) {
            (_, b'\n') => {
                line += 1;
                if matches!(state, State::Basic | State::Literal) {
                    state = State::Code;
                }
            }
            (State::Code, b'#') => {
                let end = source
                    .get(index..)
                    .and_then(|tail| tail.find('\n'))
                    .map_or(source.len(), |offset| index + offset);
                if let Some(body) = source.get(index + 1..end) {
                    comments.push((line, body));
                }
                step = end - index;
            }
            (State::Code, b'"') if rest.starts_with(b"\"\"\"") => (state, step) = (State::MultiBasic, 3),
            (State::Code, b'"') => state = State::Basic,
            (State::Code, b'\'') if rest.starts_with(b"'''") => (state, step) = (State::MultiLiteral, 3),
            (State::Code, b'\'') => state = State::Literal,
            (State::Basic | State::MultiBasic, b'\\') => {
                if rest.get(1) == Some(&b'\n') {
                    line += 1;
                }
                step = 2;
            }
            (State::Basic, b'"') | (State::Literal, b'\'') => state = State::Code,
            (State::MultiBasic, b'"') if rest.starts_with(b"\"\"\"") => (state, step) = (State::Code, 3),
            (State::MultiLiteral, b'\'') if rest.starts_with(b"'''") => (state, step) = (State::Code, 3),
            _ => {}
        }
        index += step.max(1);
    }
    comments
}

fn directive(path: &str, line: usize, body: &str, key: Option<String>) -> Option<Directive> {
    let rest = body.trim_start().strip_prefix(MARKER)?.trim_start();
    let rest = rest.strip_prefix("allow(")?;
    let (list, tail) = rest.split_once(')')?;
    let rules = list
        .split(',')
        .map(str::trim)
        .filter(|rule| !rule.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if rules.is_empty() {
        return None;
    }
    let has_reason = tail
        .trim_start()
        .strip_prefix("--")
        .is_some_and(|reason| !reason.trim().is_empty());
    Some(Directive {
        path: path.to_owned(),
        line,
        rules,
        has_reason,
        key,
    })
}

fn line_of(starts: &[usize], offset: usize) -> usize {
    starts.partition_point(|start| *start <= offset).max(1)
}

#[cfg(test)]
mod tests {
    use ra_ap_syntax::Edition;

    use super::*;

    fn rust(source: &str) -> Vec<Directive> {
        let parse = SourceFile::parse(source, Edition::Edition2024);
        in_rust("src/lib.rs", source, &parse.tree())
    }

    #[test]
    fn a_line_comment_is_a_directive_and_a_string_or_doc_comment_is_not() {
        let found = rust(
            "fn a() {\n    // rust-doctor: allow(rust_doctor::source::dynamic_shell_command, clippy::x) -- fixed argv\n    let s = \"// rust-doctor: allow(rust_doctor::a) -- no\";\n}\n/// rust-doctor: allow(rust_doctor::b) -- doc\n//! rust-doctor: allow(rust_doctor::c) -- doc\n",
        );
        assert_eq!(
            found,
            vec![Directive {
                path: "src/lib.rs".to_owned(),
                line: 2,
                rules: vec![
                    "rust_doctor::source::dynamic_shell_command".to_owned(),
                    "clippy::x".to_owned()
                ],
                has_reason: true,
                key: None,
            }]
        );
    }

    #[test]
    fn a_reason_is_what_follows_two_dashes_and_is_not_blank() {
        let found = rust(
            "// rust-doctor: allow(rust_doctor::a)\n// rust-doctor: allow(rust_doctor::a) --   \n// rust-doctor: allow(rust_doctor::a) -- because\n// rust-doctor: deny(rust_doctor::a) -- no\n// rust-doctor: allow() -- empty\n",
        );
        assert_eq!(
            found
                .iter()
                .map(|directive| (directive.line, directive.has_reason))
                .collect::<Vec<_>>(),
            [(1, false), (2, false), (3, true)]
        );
    }

    #[test]
    fn a_toml_comment_counts_and_a_hash_inside_any_string_form_does_not() {
        let source = "[package]\nname = \"a # rust-doctor: allow(rust_doctor::x) -- no\"\nb = '''\n# rust-doctor: allow(rust_doctor::x) -- no\n'''\nc = \"\"\"\n# rust-doctor: allow(rust_doctor::x) -- no\\\n\"\"\"\n# rust-doctor: allow(rust_doctor::cargo::permissive_lint_table) -- vendored\nd = 'x' # rust-doctor: allow(rust_doctor::y) -- trailing\n";
        let found = in_toml("Cargo.toml", source);
        assert_eq!(
            found
                .iter()
                .map(|directive| (directive.line, directive.rules[0].as_str()))
                .collect::<Vec<_>>(),
            [
                (9, "rust_doctor::cargo::permissive_lint_table"),
                (10, "rust_doctor::y")
            ]
        );
        assert_eq!(found[0].key.as_deref(), Some("d"));
    }

    #[test]
    fn a_manifest_directive_names_the_dependency_key_below_it() {
        let source = "[dependencies]\n# rust-doctor: allow(rust_doctor::a) -- r\nhelper = { path = \"h\" }\n# rust-doctor: allow(rust_doctor::a) -- r\n\"quoted\".workspace = true\n# rust-doctor: allow(rust_doctor::a) -- r\n[target.'cfg(unix)'.dependencies.tabled]\n# rust-doctor: allow(rust_doctor::a) -- r\n\n";
        let keys: Vec<Option<String>> = in_toml("Cargo.toml", source)
            .into_iter()
            .map(|directive| directive.key)
            .collect();
        assert_eq!(
            keys,
            [
                Some("helper".to_owned()),
                Some("quoted".to_owned()),
                Some("tabled".to_owned()),
                None
            ]
        );
    }
}
