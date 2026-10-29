#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! Proofs of EP-003 of the self-scan remediation, from the binary a user runs:
//! a withheld rule caps nothing, the ranking discounts a measured rule once, a
//! duplicate no member requires weighs nothing, versions compare by Cargo
//! compatibility class, inline tests leave the denominator, and schema 24
//! publishes the binding ceilings and the production tallies.
//!
//! Every crate is written fresh under `target/score-model-v6/`, path
//! dependencies only, so each scan runs offline against its own target
//! directory.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicUsize;

use serde_json::Value;

mod support;

static NEXT_WORKSPACE: AtomicUsize = AtomicUsize::new(0);

const DUPLICATE: &str = "rust_doctor::cargo::duplicate_major_versions";

fn manifest(name: &str, version: &str, dependencies: &str) -> String {
    format!(
        "[package]\nname = \"{name}\"\nversion = \"{version}\"\nedition = \"2024\"\npublish = false\n\n\
         [dependencies]\n{dependencies}"
    )
}

/// A fresh workspace holding `files`, its lockfile resolved offline when the
/// root manifest declares dependencies.
fn workspace<P: AsRef<Path>>(files: &[(P, String)]) -> PathBuf {
    let root = support::fresh_workspace("score-model-v6", &NEXT_WORKSPACE);
    for (path, contents) in files {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }
    let lockfile = Command::new(support::resolve_program("cargo"))
        .args(["generate-lockfile", "--offline"])
        .current_dir(&root)
        .env("CARGO_TARGET_DIR", support::scan_target(&root))
        .output()
        .unwrap();
    assert!(lockfile.status.success(), "{lockfile:?}");
    root
}

fn run(root: &Path, arguments: &[&str]) -> Vec<u8> {
    let output = support::rust_doctor()
        .arg(root)
        .arg("--yes")
        .args(support::NO_ADVISORY_DB)
        .args(arguments)
        .env("CARGO_TARGET_DIR", support::scan_target(root))
        .env("CARGO_NET_OFFLINE", "true")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .output()
        .unwrap();
    output.stdout
}

fn json(root: &Path, arguments: &[&str]) -> Value {
    let mut arguments = arguments.to_vec();
    arguments.push("--json");
    serde_json::from_slice(&run(root, &arguments)).unwrap()
}

/// The linear report, its 80-column wrapping joined back.
fn text(root: &Path) -> String {
    String::from_utf8(run(root, &[])).unwrap().replace('\n', " ")
}

fn diagnostics<'a>(report: &'a Value, code: &str) -> Vec<&'a Value> {
    report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|diagnostic| diagnostic["code"] == code)
        .collect()
}

fn single_crate(lib: &str) -> PathBuf {
    workspace(&[
        ("Cargo.toml", manifest("probe", "0.1.0", "\n[workspace]\n")),
        ("src/lib.rs", lib.to_owned()),
    ])
}

/// A library `app` whose manifest declares `dependencies` and whose source is
/// `lib`, beside path crates `(directory, name, version, dependencies)`, each
/// exporting `ID` and excluded from the workspace so that two of them may share
/// a name.
fn with_dependencies(
    dependencies: &str,
    lib: &str,
    crates: &[(&str, &str, &str, &str)],
) -> Vec<(String, String)> {
    let excluded: Vec<String> = crates
        .iter()
        .map(|(directory, ..)| format!("\"{directory}\""))
        .collect();
    let mut files = vec![
        (
            "Cargo.toml".to_owned(),
            manifest(
                "app",
                "0.1.0",
                &format!("{dependencies}\n[workspace]\nexclude = [{}]\n", excluded.join(", ")),
            ),
        ),
        ("src/lib.rs".to_owned(), lib.to_owned()),
    ];
    for (directory, name, version, own) in crates {
        files.push((format!("{directory}/Cargo.toml"), manifest(name, version, own)));
        files.push((
            format!("{directory}/src/lib.rs"),
            "pub const ID: u8 = 0;\n".to_owned(),
        ));
    }
    files
}

/// US-015: one `std::process::exit` in a library function, a rule the corpus
/// judged wrong on every site it showed, caps nothing; one site of an
/// unmeasured `P2` rule still holds reliability at 75.
#[test]
fn a_withheld_rule_caps_nothing_and_an_unmeasured_p2_rule_still_caps() {
    let stop = "pub fn stop(code: i32) {\n    std::process::exit(code);\n}\n";
    // The withheld rule's sites are still charged at the discount, so on the
    // two-kiloline floor a lone site costs a point of density and no ceiling;
    // over eight thousand lines the charge rounds away and reliability is 100.
    let small = single_crate(stop);
    let mut files = vec![
        ("Cargo.toml".to_owned(), manifest("probe", "0.1.0", "\n[workspace]\n")),
        (
            "src/lib.rs".to_owned(),
            (0..9).fold(stop.to_owned(), |lib, module| lib + &format!("pub mod padding{module};\n")),
        ),
    ];
    for module in 0..9 {
        let constants: String = (0..900)
            .map(|index| format!("pub const C{index}: u32 = {index};\n"))
            .collect();
        files.push((format!("src/padding{module}.rs"), constants));
    }
    for (root, reliability) in [(small, 99), (workspace(&files), 100)] {
        let exit = json(&root, &[]);
        let score = &exit["audit"]["score"];
        assert_eq!(diagnostics(&exit, "clippy::exit").len(), 1, "{exit:#}");
        assert_eq!(score["dimensions"]["reliability"], reliability, "{score:#}");
        assert!(score["worst_tier"].is_null(), "{score:#}");
        assert!(score["applied_ceiling"].is_null());
        assert_eq!(score["ceilings"], Value::Array(Vec::new()));
        assert_eq!(score["withheld_rule_ids"][0], "clippy::exit");
    }

    let todo = json(&single_crate("pub fn later() {\n    todo!()\n}\n"), &[]);
    let score = &todo["audit"]["score"];
    assert_eq!(score["dimensions"]["reliability"], 75, "{score:#}");
    assert_eq!(score["worst_tier"], "P2");
    assert_eq!(
        score["ceilings"],
        serde_json::json!([{
            "dimension": "reliability",
            "tier": "P2",
            "ceiling": 75,
            "rule_ids": ["clippy::todo"],
        }])
    );
}

/// US-016: two rules of equal raw relief, `ptr_arg` measured at 3333 basis
/// points and `useless_vec` nobody measured, both `P3` performance. The
/// measured one's charge already carries its discount, so it ranks first;
/// under core-v5 the discount applied twice and the unmeasured rule led. A rule
/// holding a ceiling is still projected for the ceiling it releases.
#[test]
fn a_measured_rule_is_discounted_once_in_the_ranking() {
    let report = json(
        &single_crate(
            "pub fn length(values: &Vec<u8>) -> usize {\n    values.len()\n}\n\n\
             pub fn total() -> u32 {\n    let values = vec![1, 2, 3];\n    values.iter().sum()\n}\n",
        ),
        &[],
    );
    let score = &report["audit"]["score"];
    assert_eq!(
        score["projected_rule_ids"],
        serde_json::json!(["clippy::ptr_arg", "clippy::useless_vec"]),
        "{report:#}"
    );

    let report = json(
        &workspace(&with_dependencies(
            "hb = { path = \"hb1\" }\nright = { path = \"right\" }\n",
            "pub fn app() -> u8 {\n    hb::ID + right::ID\n}\n",
            &[("hb1", "hb", "1.0.0", ""), ("hb2", "hb", "2.0.0", ""), ("right", "right", "0.1.0", "hb = { path = \"../hb2\" }\n")],
        )),
        &[],
    );
    let score = &report["audit"]["score"];
    assert_eq!(score["projected_rule_ids"][0], DUPLICATE, "{report:#}");
    // Under the 75 the P2 ceiling allows: since the core-v6 replay the corpus
    // publishes no rate for the rule, so its one site is charged whole.
    assert_eq!(score["dimensions"]["dependencies"], 72);
    assert_eq!(
        score["projected_after_top_three"], 100,
        "the ceiling the duplicate holds is released whole, not discounted: {score:#}"
    );
}

/// US-017: a duplicate whose every version arrives through a dependency is
/// published at `Info`, weighs nothing, is not projected, and keeps the id it
/// had as a warning.
#[test]
fn a_duplicate_no_member_requires_publishes_at_info_and_keeps_its_id() {
    let root = workspace(&with_dependencies(
        "left = { path = \"left\" }\nright = { path = \"right\" }\n",
        "pub fn app() -> u8 {\n    left::ID + right::ID\n}\n",
        &[
            ("hb1", "hb", "1.0.0", ""),
            ("hb2", "hb", "2.0.0", ""),
            ("left", "left", "0.1.0", "hb = { path = \"../hb1\" }\n"),
            ("right", "right", "0.1.0", "hb = { path = \"../hb2\" }\n"),
        ],
    ));
    let report = json(&root, &[]);
    let found = diagnostics(&report, DUPLICATE);
    assert_eq!(found.len(), 1, "{report:#}");
    let duplicate = found[0];
    assert_eq!(duplicate["severity"], "info");
    assert_eq!(duplicate["base_severity"], "warning");
    assert_eq!(
        duplicate["message"],
        "Crate \"hb\" is resolved with incompatible versions 1.0.0 (through left), 2.0.0 (through right)."
    );
    assert_eq!(
        duplicate["help"],
        "Align the requirement your manifest controls, or bump the dependency the message names so it moves to the version you require."
    );
    let score = &report["audit"]["score"];
    assert_eq!(score["dimensions"]["dependencies"], 100, "{score:#}");
    assert!(
        !score["projected_rule_ids"]
            .as_array()
            .unwrap()
            .contains(&Value::from(DUPLICATE))
    );

    // Asked for at its level, the same finding is a warning under the same id:
    // the identity hashes the base severity, never the published one.
    let restamped = json(&root, &["--rule", &format!("{DUPLICATE}=warn")]);
    let warned = diagnostics(&restamped, DUPLICATE);
    assert_eq!(warned.len(), 1);
    assert_eq!(warned[0]["severity"], "warning");
    assert_eq!(warned[0]["id"], duplicate["id"]);
}

/// US-017 and US-018: a duplicate a member requires directly stays a warning
/// naming the requirement, and `0.59` beside `0.61`, or `0.0.3` beside
/// `0.0.4`, are incompatible, as Cargo compiles them.
#[test]
fn a_direct_duplicate_stays_a_warning_and_zero_dot_versions_compare_by_class() {
    let root = workspace(&with_dependencies(
        "hb = { path = \"hb1\" }\nright = { path = \"right\" }\n\
         winsys = { path = \"w59\" }\nwinsys_next = { package = \"winsys\", path = \"w61\" }\n\
         tiny = { path = \"t3\" }\ntiny_next = { package = \"tiny\", path = \"t4\" }\n",
        "pub fn app() -> u8 {\n    hb::ID + right::ID + winsys::ID + winsys_next::ID + tiny::ID + tiny_next::ID\n}\n",
        &[
            ("hb1", "hb", "1.0.0", ""),
            ("hb2", "hb", "2.0.0", ""),
            ("right", "right", "0.1.0", "hb = { path = \"../hb2\" }\n"),
            ("w59", "winsys", "0.59.0", ""),
            ("w61", "winsys", "0.61.2", ""),
            ("t3", "tiny", "0.0.3", ""),
            ("t4", "tiny", "0.0.4", ""),
        ],
    ));
    let report = json(&root, &[]);
    let mut messages: Vec<(&str, &str)> = diagnostics(&report, DUPLICATE)
        .into_iter()
        .map(|diagnostic| {
            (
                diagnostic["severity"].as_str().unwrap(),
                diagnostic["message"].as_str().unwrap(),
            )
        })
        .collect();
    messages.sort_unstable();
    assert_eq!(
        messages,
        [
            (
                "warning",
                "Crate \"hb\" is resolved with incompatible versions 1.0.0 (required by app), 2.0.0 (through right)."
            ),
            (
                "warning",
                "Crate \"tiny\" is resolved with incompatible versions 0.0.3 (required by app), 0.0.4 (required by app)."
            ),
            (
                "warning",
                "Crate \"winsys\" is resolved with incompatible versions 0.59.0 (required by app), 0.61.2 (required by app)."
            ),
        ],
        "{report:#}"
    );
}

/// US-019: the lines of an outermost test-gated item are not production lines,
/// for the workspace and for each member, and a crate holding nothing else has
/// no score.
#[test]
fn inline_test_gated_items_leave_the_production_lines() {
    let tests = "#[cfg(test)]\nmod tests {\n    use super::*;\n\n    #[test]\n    fn adds() {\n        assert_eq!(add(1, 2), 3);\n    }\n\n    #[test]\n    fn zero() {\n        assert_eq!(add(0, 0), 0);\n    }\n}\n";
    // Nine lines, then a twelve-line test module: twenty-one in all.
    let production = "//! A probe.\n\n/// Adds.\npub fn add(left: u64, right: u64) -> u64 {\n    left + right\n}\n\npub const ONE: u64 = 1;\n\n";
    let lib = format!("{production}#[cfg(test)]\nmod tests {{\n    use super::*;\n\n    #[test]\n    fn adds() {{\n        assert_eq!(add(1, 2), 3);\n    }}\n\n    // The module spans\n    // twelve lines.\n}}\n");
    assert_eq!(lib.lines().count(), 21, "{lib}");
    let report = json(&single_crate(&lib), &[]);
    assert_eq!(report["audit"]["production_lines"], 9, "{report:#}");
    assert!(report["audit"]["score"].is_object());

    let only_tests = single_crate(tests);
    let report = json(&only_tests, &[]);
    assert_eq!(report["audit"]["production_lines"], 0, "{report:#}");
    assert_eq!(report["audit"]["source_files"], 1, "the file count is not this story's");
    assert!(report["audit"]["score"].is_null());
    assert!(
        text(&only_tests)
            .contains("Score unavailable: only tests, benches, examples or build scripts were found."),
    );

    let root = workspace(&[
        (
            "Cargo.toml",
            "[workspace]\nmembers = [\"alpha\", \"beta\"]\nresolver = \"3\"\n".to_owned(),
        ),
        ("alpha/Cargo.toml", manifest("alpha", "0.1.0", "")),
        ("alpha/src/lib.rs", format!("pub fn alpha() {{}}\n{tests}")),
        ("beta/Cargo.toml", manifest("beta", "0.1.0", "")),
        ("beta/src/lib.rs", format!("pub fn beta() {{}}\n\npub fn gamma() {{}}\n{tests}")),
    ]);
    let report = json(&root, &[]);
    let members: Vec<(&str, u64)> = report["audit"]["packages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|member| {
            (
                member["name"].as_str().unwrap(),
                member["production_lines"].as_u64().unwrap(),
            )
        })
        .collect();
    assert_eq!(members, [("alpha", 1), ("beta", 3)], "{report:#}");
    assert_eq!(report["audit"]["production_lines"], 4);
}

/// US-020: the text names each binding ceiling from `ceilings[]`, the overall
/// one and every dimension's, and the share link carries the production
/// counts rather than every occurrence.
#[test]
fn the_ceilings_and_the_production_counts_reach_the_text_and_the_share_link() {
    let adversarial = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/score-credibility/adversarial")
        .canonicalize()
        .unwrap();
    let report = json(&adversarial, &[]);
    let rendered = text(&adversarial);
    let ceilings = report["audit"]["score"]["ceilings"].as_array().unwrap();
    let overall = ceilings
        .iter()
        .find(|ceiling| ceiling["dimension"] == "overall")
        .expect("a P0 finding imposes an overall ceiling");
    assert_eq!(overall["tier"], "P0");
    assert_eq!(overall["ceiling"], 40);
    let named = |ceiling: &Value| {
        ceiling["rule_ids"]
            .as_array()
            .unwrap()
            .iter()
            .take(3)
            .map(|id| id.as_str().unwrap())
            .collect::<Vec<_>>()
            .join(", ")
    };
    assert!(
        rendered.contains(&format!("Capped at 40/100 by a P0 finding: {}", named(overall))),
        "{rendered}"
    );
    let dimensions: Vec<&Value> = ceilings
        .iter()
        .filter(|ceiling| ceiling["dimension"] != "overall")
        .collect();
    assert!(dimensions.iter().any(|ceiling| ceiling["tier"] == "P0"), "{ceilings:?}");
    for ceiling in dimensions {
        let dimension = ceiling["dimension"].as_str().unwrap();
        let mut name = dimension.to_owned();
        name.get_mut(..1).unwrap().make_ascii_uppercase();
        let line = format!(
            "{name} {} ({} ceiling: {})",
            report["audit"]["score"]["dimensions"][dimension],
            ceiling["tier"].as_str().unwrap(),
            named(ceiling)
        );
        assert!(rendered.contains(&line), "{line} missing from {rendered}");
    }

    // A production `todo!` beside a shell command built in an integration
    // test: the test-context finding is published and counted in
    // `occurrences`, and the link counts what weighs.
    let root = workspace(&[
        ("Cargo.toml", manifest("probe", "0.1.0", "\n[workspace]\n")),
        ("src/lib.rs", "pub fn later() {\n    todo!()\n}\n".to_owned()),
        (
            "tests/shell.rs",
            "#[test]\nfn pause() {\n    let secs = std::env::args().count().to_string();\n    \
             let _ = std::process::Command::new(\"sh\").arg(\"-c\").arg(format!(\"sleep {secs}\")).status();\n}\n"
                .to_owned(),
        ),
    ]);
    let report = json(&root, &[]);
    let categories = report["audit"]["categories"].as_array().unwrap();
    let sum = |member: &str, severity: &str| -> u64 {
        categories
            .iter()
            .map(|category| category[member][severity].as_u64().unwrap())
            .sum()
    };
    assert!(
        sum("occurrences", "total") > sum("production", "total"),
        "the test-context finding is published and does not weigh: {report:#}"
    );
    let rendered = text(&root);
    assert!(rendered.contains("Reliability 75 (P2 ceiling: clippy::todo)"), "{rendered}");
    let share = rendered
        .split_whitespace()
        .find(|word| word.starts_with("https://rust-doctor.com/share?"))
        .expect("an authoritative score is shared");
    let counted = |key: &str| {
        share
            .split(['?', '&'])
            .find_map(|pair| pair.strip_prefix(&format!("{key}=")))
            .map_or(0, |count| count.parse::<u64>().unwrap())
    };
    assert_eq!(counted("w"), sum("production", "warnings"), "{share}");
    assert_eq!(counted("e"), sum("production", "errors"), "{share}");
    assert_eq!(counted("i"), sum("production", "info"), "{share}");
}
