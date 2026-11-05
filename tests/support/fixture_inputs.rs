use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

static REBUILT: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

pub(crate) fn comment_inputs() -> toml::Table {
    fs::read_to_string(fixtures().join("comment-inputs.toml"))
        .expect("tests/fixtures/comment-inputs.toml should be readable")
        .parse()
        .expect("tests/fixtures/comment-inputs.toml should parse")
}

pub(crate) fn fixture_dir(relative: &str) -> PathBuf {
    let source = fixtures().join(relative);
    let prefix = format!("{relative}/");
    let inputs = comment_inputs();
    let files: Vec<(&str, &toml::Value)> = inputs
        .iter()
        .filter_map(|(file, lines)| file.strip_prefix(&prefix).map(|rest| (rest, lines)))
        .collect();
    if files.is_empty() {
        return source;
    }
    let destination = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/fixtures-with-comments")
        .join(relative.replace('/', "--"));
    let mut rebuilt = REBUILT.lock().unwrap_or_else(PoisonError::into_inner);
    if !rebuilt.iter().any(|done| done == relative) {
        if destination.exists() {
            fs::remove_dir_all(&destination).expect("a stale rebuilt fixture should be removable");
        }
        copy(&source, &destination);
        for (file, lines) in files {
            let lines = lines.as_table().expect("a fixture file should map lines to comments");
            restore(&destination.join(file), lines);
        }
        rebuilt.push(relative.to_owned());
    }
    destination
}

fn copy(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("a rebuilt fixture directory should be creatable");
    for entry in fs::read_dir(source).expect("a fixture directory should be readable") {
        let entry = entry.expect("a fixture entry should be readable");
        let target = destination.join(entry.file_name());
        if entry.path().is_dir() {
            if entry.file_name() != "target" {
                copy(&entry.path(), &target);
            }
        } else {
            fs::copy(entry.path(), target).expect("a fixture file should be copyable");
        }
    }
}

fn restore(path: &Path, lines: &toml::Table) {
    let tracked = fs::read_to_string(path).expect("a file the comment inputs name should exist");
    let ending = if tracked.contains("\r\n") { "\r\n" } else { "\n" };
    let mut rows: Vec<String> = tracked
        .split('\n')
        .map(|row| row.trim_end_matches('\r').to_owned())
        .collect();
    for (number, text) in lines {
        let text = text.as_str().expect("a comment input should be a string");
        let row = number
            .parse::<usize>()
            .ok()
            .and_then(|number| number.checked_sub(1))
            .and_then(|index| rows.get_mut(index))
            .expect("a comment input should name a line of its file");
        assert!(
            text.starts_with(row.as_str()),
            "{}:{number} holds code its comment input does not extend",
            path.display()
        );
        row.clear();
        row.push_str(text);
    }
    fs::write(path, rows.join(ending)).expect("a rebuilt fixture file should be writable");
}
