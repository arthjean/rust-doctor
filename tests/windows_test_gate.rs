#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

const ROOT: &str = env!("CARGO_MANIFEST_DIR");
const LEDGER: &str = "tests/windows-known-failures.txt";
const CLASSES: [&str; 5] = ["path-form", "separator", "unix-only", "toolchain", "wall-clock"];

static NEXT: AtomicUsize = AtomicUsize::new(0);

const RECORDED: &str = "\
   Compiling rust-doctor v0.10.0 (C:\\dev\\rust-doctor)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 1m 02s
     Running unittests src\\lib.rs (target\\debug\\deps\\rust_doctor-0123abcd.exe)

running 3 tests
test walk::tests::a_verbatim_root_is_walked ... ok
test baseline::tests::a_snapshot_is_swept ... FAILED
test structure::benchmark::the_pass_meets_its_bound ... ignored, measured by hand

failures:

---- baseline::tests::a_snapshot_is_swept stdout ----
     Running tests\\captured.rs (target\\debug\\deps\\captured-0000.exe)
test captured::from_a_child_cargo ... FAILED
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
error: could not compile `fixture` (lib) due to 1 previous error

failures:
    baseline::tests::a_snapshot_is_swept

test result: FAILED. 1 passed; 1 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.20s

error: test failed, to rerun pass `--lib`
     Running unittests src/main.rs (target/debug/deps/rust_doctor-4567ef01)

running 1 test
test tui::tests::the_landing_renders ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s

     Running tests\\advisory_database.rs (target\\debug\\deps\\advisory_database-89ab.exe)

running 2 tests
test a_crlf_database_reads_every_advisory ... \x1b[32mok\x1b[0m
\x1b[2KCompiling dependencies...\x1b[2KRunning native passes...\x1b[2Ktest the_home_is_found_through_userprofile ... \x1b[31mFAILED\x1b[0m

test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.50s

   Doc-tests rust_doctor

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

error: 2 targets failed:
    `--lib`
    `--test advisory_database`
";

const LEDGER_OF_RECORD: &str = "\
lib baseline::tests::a_snapshot_is_swept unix-only
advisory_database the_home_is_found_through_userprofile path-form
";

fn bash() -> PathBuf {
    if !cfg!(windows) {
        return PathBuf::from("bash");
    }
    let output = Command::new("git").arg("--exec-path").output().expect("git starts");
    let exec_path = PathBuf::from(String::from_utf8(output.stdout).unwrap().trim());
    exec_path
        .ancestors()
        .map(|directory| directory.join("bin").join("bash.exe"))
        .find(|candidate| candidate.is_file())
        .expect("Git for Windows ships the bash the gate runs under")
}

fn slashed(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn compare(output: &str, status: i32, ledger: &str) -> Output {
    let scratch = Path::new(ROOT)
        .join("target")
        .join("windows-test-gate")
        .join(format!("{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::SeqCst)));
    fs::create_dir_all(&scratch).unwrap();
    fs::write(scratch.join("output"), output).unwrap();
    fs::write(scratch.join("ledger"), ledger).unwrap();
    let result = Command::new(bash())
        .arg(slashed(&Path::new(ROOT).join("scripts/windows-test-gate.sh")))
        .arg("--compare")
        .arg(slashed(&scratch.join("output")))
        .arg(status.to_string())
        .arg(slashed(&scratch.join("ledger")))
        .output()
        .expect("bash starts");
    fs::remove_dir_all(&scratch).unwrap();
    result
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn section(output: &Output, heading: &str) -> Vec<String> {
    stdout(output)
        .lines()
        .skip_while(|line| *line != heading)
        .skip(1)
        .take_while(|line| line.starts_with("  "))
        .map(|line| line.trim().to_owned())
        .collect()
}

#[test]
fn a_run_whose_failures_are_the_ledger_is_green() {
    let output = compare(RECORDED, 101, LEDGER_OF_RECORD);
    assert_eq!(output.status.code(), Some(0), "{}", stdout(&output));
    assert!(stdout(&output).contains("green, 2 failures"), "{}", stdout(&output));
}

#[test]
fn a_crlf_ledger_and_a_crlf_output_compare_as_their_lf_forms() {
    let output = compare(
        &RECORDED.replace('\n', "\r\n"),
        101,
        &LEDGER_OF_RECORD.replace('\n', "\r\n"),
    );
    assert_eq!(output.status.code(), Some(0), "{}", stdout(&output));
}

#[test]
fn a_failure_the_ledger_does_not_list_is_a_new_failure() {
    let output = compare(RECORDED, 101, "lib baseline::tests::a_snapshot_is_swept unix-only\n");
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        section(&output, "new failure:"),
        ["advisory_database the_home_is_found_through_userprofile"]
    );
    assert!(section(&output, "passes now, remove it:").is_empty());
    assert!(section(&output, "unknown test:").is_empty());
}

#[test]
fn a_ledger_entry_that_passes_or_is_ignored_must_leave_the_ledger() {
    let ledger = format!(
        "{LEDGER_OF_RECORD}main tui::tests::the_landing_renders separator\nlib structure::benchmark::the_pass_meets_its_bound wall-clock\n"
    );
    let output = compare(RECORDED, 101, &ledger);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        section(&output, "passes now, remove it:"),
        [
            "main tui::tests::the_landing_renders separator",
            "lib structure::benchmark::the_pass_meets_its_bound wall-clock"
        ]
    );
    assert!(section(&output, "new failure:").is_empty());
}

#[test]
fn a_ledger_entry_no_binary_ran_is_an_unknown_test() {
    let ledger = format!(
        "{LEDGER_OF_RECORD}lib walk::tests::a_test_since_renamed path-form\nretired a_test_of_a_removed_binary toolchain\n"
    );
    let output = compare(RECORDED, 101, &ledger);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        section(&output, "unknown test:"),
        [
            "lib walk::tests::a_test_since_renamed path-form",
            "retired a_test_of_a_removed_binary toolchain"
        ]
    );
}

#[test]
fn a_build_error_exits_with_cargo_s_status_and_prints_no_comparison() {
    let failed_build = "   Compiling rust-doctor v0.10.0\nerror[E0425]: cannot find value `x`\nerror: could not compile `rust-doctor` (lib test) due to 1 previous error\n";
    let output = compare(failed_build, 101, LEDGER_OF_RECORD);
    assert_eq!(output.status.code(), Some(101));
    assert!(stdout(&output).is_empty(), "{}", stdout(&output));

    let refused_lockfile = "error: the lock file C:\\dev\\rust-doctor\\Cargo.lock needs to be updated but --locked was passed to prevent this\n";
    assert_eq!(compare(refused_lockfile, 101, LEDGER_OF_RECORD).status.code(), Some(101));
}

#[test]
fn a_binary_that_dies_before_its_summary_turns_the_gate_red() {
    let crashed = RECORDED.replace(
        "test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.50s\n",
        "error: test failed, to rerun pass `--test advisory_database`\nCaused by: process didn't exit successfully (exit code: 0xc00000fd, STATUS_STACK_OVERFLOW)\n",
    );
    let output = compare(&crashed, 101, LEDGER_OF_RECORD);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(section(&output, "did not finish:"), ["advisory_database"]);
}

#[test]
fn a_failure_whose_line_a_child_process_split_still_turns_the_gate_red() {
    let split = RECORDED.replace(
        "test baseline::tests::a_snapshot_is_swept ... FAILED\n",
        "test baseline::tests::a_snapshot_is_swept ... \x1b[2KCompiling dependencies...\nFAILED\n",
    );
    let output = compare(&split, 101, "advisory_database the_home_is_found_through_userprofile path-form
");
    assert_eq!(output.status.code(), Some(1), "{}", stdout(&output));
    assert_eq!(
        section(&output, "failures the summary counts but no test line names:"),
        ["lib counts 1 failed, names 0"]
    );
}

#[test]
fn a_should_panic_test_is_compared_by_its_name() {
    let should_panic = RECORDED.replace(
        "test baseline::tests::a_snapshot_is_swept ... FAILED\n",
        "test baseline::tests::a_snapshot_is_swept - should panic ... FAILED\n",
    );
    let output = compare(&should_panic, 101, LEDGER_OF_RECORD);
    assert_eq!(output.status.code(), Some(0), "{}", stdout(&output));

    let unlisted = compare(&should_panic, 101, "advisory_database the_home_is_found_through_userprofile path-form\n");
    assert_eq!(unlisted.status.code(), Some(1));
    assert_eq!(section(&unlisted, "new failure:"), ["lib baseline::tests::a_snapshot_is_swept"]);
}

#[test]
fn an_entry_outside_the_closed_classes_is_refused() {
    for entry in [
        "lib baseline::tests::a_snapshot_is_swept flaky",
        "lib baseline::tests::a_snapshot_is_swept",
        "lib baseline::tests::a_snapshot_is_swept unix-only extra",
    ] {
        let ledger = format!("{entry}\nadvisory_database the_home_is_found_through_userprofile path-form\n");
        let output = compare(RECORDED, 101, &ledger);
        assert_eq!(output.status.code(), Some(1), "{entry}");
        assert!(stdout(&output).contains("malformed ledger entry"), "{entry}");
    }
}

fn ledger_entries() -> Vec<(String, String, String)> {
    fs::read_to_string(Path::new(ROOT).join(LEDGER))
        .expect("the ledger is committed")
        .lines()
        .map(|line| {
            let fields: Vec<&str> = line.split(' ').collect();
            assert_eq!(fields.len(), 3, "{LEDGER}: `{line}` is not `binary test class`");
            (fields[0].to_owned(), fields[1].to_owned(), fields[2].to_owned())
        })
        .collect()
}

#[test]
fn the_ledger_holds_sorted_unique_entries_of_the_closed_classes_and_none_of_path_form() {
    let entries = ledger_entries();
    let mut seen = BTreeSet::new();
    for (binary, test, class) in &entries {
        assert!(CLASSES.contains(&class.as_str()), "{binary} {test}: class `{class}` is not one of {CLASSES:?}");
        assert_ne!(class, "path-form", "{binary} {test}: a path-form failure is fixed through src/path_form.rs, never listed");
        assert!(seen.insert((binary.clone(), test.clone())), "{binary} {test} is listed twice");
    }
    let mut sorted = entries.clone();
    sorted.sort();
    assert_eq!(entries, sorted, "{LEDGER} is kept sorted");
}

#[test]
fn the_ledger_size_the_testing_document_states_is_the_ledger_on_disk() {
    let entries = ledger_entries().len();
    assert!(entries < 103, "the ledger must hold fewer than the 103 failures measured at 0.10.0");
    let testing = fs::read_to_string(Path::new(ROOT).join("docs/testing.md")).unwrap();
    let stated = format!("`{LEDGER}` holds {entries} entries");
    assert!(
        testing.split_whitespace().collect::<Vec<_>>().join(" ").contains(&stated),
        "docs/testing.md should state `{stated}`"
    );
}
