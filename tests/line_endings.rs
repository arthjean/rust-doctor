#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;

use rust_doctor::{Diagnostic, InspectRequest, RuleLevel, RuleOverride, Status, catalog, inspect};

mod support;

static NEXT_WORKSPACE: AtomicUsize = AtomicUsize::new(0);

const SHELL: &str = "rust_doctor::source::dynamic_shell_command";

const LIB: &str = "//! Probe.\n\n/// Text whose literal spans two lines.\npub fn banner() -> &'static str {\n    \"first\nsecond\"\n}\n\n/// Runs `user` through a shell.\npub fn run(user: &str) {\n    let _ = std::process::Command::new(\"sh\")\n        .arg(\"-c\")\n        .arg(format!(\"run {user}\"));\n}\n";

fn workspace(lib: &str) -> PathBuf {
    let root = support::fresh_workspace("line-endings", &NEXT_WORKSPACE);
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"probe\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n\n[workspace]\n",
    )
    .unwrap();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/lib.rs"), lib).unwrap();
    root
}

fn diagnostics(lib: &str) -> Vec<Diagnostic> {
    let request = catalog()
        .iter()
        .filter(|entry| entry.id.starts_with("clippy::"))
        .fold(InspectRequest::new(workspace(lib)), |request, entry| {
            request.with_rule_override(RuleOverride::new(entry.id, RuleLevel::Off))
        });
    let report = inspect(request);
    assert_eq!(report.status, Status::Complete, "{:?}", report.errors);
    report.diagnostics
}

#[test]
fn a_crlf_or_byte_order_marked_file_reads_as_the_lf_file_rustc_compiles() {
    let lf = diagnostics(LIB);
    assert!(
        lf.iter().any(|diagnostic| diagnostic.code.as_deref() == Some(SHELL)),
        "the LF file should report {SHELL}: {lf:?}"
    );

    let crlf = LIB.replace('\n', "\r\n");
    assert_eq!(diagnostics(&crlf), lf, "a CRLF checkout of the same file");
    let marked = format!("\u{feff}{crlf}");
    assert_eq!(diagnostics(&marked), lf, "the same file behind a byte order mark");
}
