use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const MUTATIONS_PER_RULE: usize = 1_000;
const MIN_DISTINCT_PARSEABLE: usize = 100;
const OUTPUT_LIMIT: u64 = 16 * 1024 * 1024;
const SCAN_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug)]
struct ConformanceCase {
    kind: String,
    path: PathBuf,
    source: String,
}

#[derive(Debug)]
struct ConformanceRule {
    id: String,
    cases: Vec<ConformanceCase>,
}

#[derive(Debug)]
enum ProcessOutcome {
    Exited(ExitStatus),
    TimedOut,
}

fn expected_rules(path: &Path) -> Vec<String> {
    let policy: Value =
        serde_json::from_slice(&std::fs::read(path).expect("read trusted EP-006 promotion policy"))
            .expect("trusted EP-006 policy must be valid JSON");
    policy["rules"]
        .as_array()
        .expect("trusted EP-006 policy must contain rules")
        .iter()
        .map(|entry| {
            entry["rule"]
                .as_str()
                .expect("trusted EP-006 rule must have an ID")
                .to_string()
        })
        .collect()
}

fn conformance_manifest(path: &Path) -> Vec<ConformanceRule> {
    let input = std::fs::read_to_string(path).expect("read trusted EP-006 conformance fixtures");
    let mut rules = Vec::new();
    let mut current_rule: Option<ConformanceRule> = None;
    let mut current_case: Option<ConformanceCase> = None;
    let finish_case = |rule: &mut Option<ConformanceRule>, case: &mut Option<ConformanceCase>| {
        if let Some(case) = case.take() {
            rule.as_mut()
                .expect("a case must follow a rule header")
                .cases
                .push(case);
        }
    };

    for line in input.lines() {
        if let Some(id) = line.strip_prefix("=== ") {
            finish_case(&mut current_rule, &mut current_case);
            if let Some(rule) = current_rule.take() {
                rules.push(rule);
            }
            current_rule = Some(ConformanceRule {
                id: id.trim().to_string(),
                cases: Vec::new(),
            });
            continue;
        }
        if let Some(header) = line.strip_prefix("--- ") {
            finish_case(&mut current_rule, &mut current_case);
            let (kind, path) = header
                .split_once(' ')
                .expect("fixture header must contain a kind and path");
            current_case = Some(ConformanceCase {
                kind: kind.to_string(),
                path: PathBuf::from(path),
                source: String::new(),
            });
            continue;
        }
        if let Some(case) = current_case.as_mut() {
            case.source.push_str(line);
            case.source.push('\n');
        }
    }
    finish_case(&mut current_rule, &mut current_case);
    if let Some(rule) = current_rule {
        rules.push(rule);
    }
    rules
}

fn xorshift(mut value: u64) -> u64 {
    value ^= value << 13;
    value ^= value >> 7;
    value ^ (value << 17)
}

fn mutate_source(seed: u64, source: &str) -> Vec<u8> {
    let mut bytes = format!("// rust-doctor-mutation-seed:{seed:016x}\n{source}").into_bytes();
    let random = xorshift(seed);
    let index = (random as usize) % bytes.len();
    match random % 7 {
        0 => bytes.insert(index, b' '),
        1 => {
            bytes.remove(index);
        }
        2 => bytes.truncate(index),
        3 => {
            let end = (index + 8).min(bytes.len());
            let duplicate = bytes[index..end].to_vec();
            bytes.splice(index..index, duplicate);
        }
        4 => {
            bytes.splice(0..0, b"#[unknown_tool::attribute]\n".iter().copied());
        }
        5 => bytes[index] ^= 0x80,
        _ => bytes.extend_from_slice(b"\nconst _: () = ();\n"),
    }
    bytes
}

fn stable_seed(rule: &str, iteration: u64) -> u64 {
    rule.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x1000_0000_01b3)
    }) ^ iteration.wrapping_mul(0x9e37_79b9_7f4a_7c15)
}

fn write_project_manifest(root: &Path) {
    std::fs::create_dir_all(root.join("src")).expect("create fixture source directory");
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"ep006-oracle-fixture\"\nversion = \"0.0.0\"\nedition = \"2024\"\n",
    )
    .expect("write fixture Cargo manifest");
    std::fs::write(root.join("src/lib.rs"), "pub fn placeholder() {}\n")
        .expect("write fixture library target");
}

fn unique_case_path(index: usize, original: &Path) -> PathBuf {
    let display = original.to_string_lossy();
    if display == "build.rs" {
        PathBuf::from("build.rs")
    } else if display == "src/main.rs" || display.starts_with("src/bin/") {
        PathBuf::from(format!("src/bin/case-{index}.rs"))
    } else if display.starts_with("tests/") {
        PathBuf::from(format!("tests/case-{index}.rs"))
    } else if display.starts_with("benches/") {
        PathBuf::from(format!("benches/case-{index}.rs"))
    } else if display.starts_with("examples/") {
        PathBuf::from(format!("examples/case-{index}.rs"))
    } else {
        PathBuf::from(format!("src/case-{index}.rs"))
    }
}

fn write_case(root: &Path, path: &Path, source: &[u8]) {
    let destination = root.join(path);
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).expect("create fixture case directory");
    }
    std::fs::write(destination, source).expect("write fixture case");
}

fn wait_for_process(child: &mut std::process::Child, timeout: Duration) -> ProcessOutcome {
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().expect("poll supervised process") {
            return ProcessOutcome::Exited(status);
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return ProcessOutcome::TimedOut;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn read_bounded(path: &Path) -> String {
    let metadata = std::fs::metadata(path).expect("read supervised process output metadata");
    assert!(
        metadata.len() <= OUTPUT_LIMIT,
        "supervised output exceeded {OUTPUT_LIMIT} bytes at {}",
        path.display()
    );
    std::fs::read_to_string(path).expect("supervised output must be UTF-8")
}

fn run_candidate(binary: &Path, root: &Path) -> Value {
    let stdout_path = root.join(".ep006-oracle-stdout.json");
    let stderr_path = root.join(".ep006-oracle-stderr.log");
    let stdout = File::create(&stdout_path).expect("create candidate stdout file");
    let stderr = File::create(&stderr_path).expect("create candidate stderr file");
    let mut child = Command::new(binary)
        .arg(root)
        .args([
            "--json-compact",
            "--offline",
            "--no-telemetry",
            "--no-project-config",
            "--evaluation-profile",
            "--max-duration",
            "90",
        ])
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .expect("spawn exact production candidate binary");
    let outcome = wait_for_process(&mut child, SCAN_TIMEOUT);
    let stderr = read_bounded(&stderr_path);
    match outcome {
        ProcessOutcome::Exited(status) => {
            assert!(
                status.success(),
                "production candidate failed with {status}: {stderr}"
            );
        }
        ProcessOutcome::TimedOut => {
            panic!("production candidate exceeded {SCAN_TIMEOUT:?}: {stderr}");
        }
    }
    let report: Value =
        serde_json::from_str(&read_bounded(&stdout_path)).expect("candidate emitted invalid JSON");
    assert_eq!(report["schema_version"], "1.0");
    assert_eq!(report["report_constructed"], true);
    report
}

fn diagnostic_paths(report: &Value, rule: &str) -> HashSet<String> {
    report["diagnostics"]
        .as_array()
        .expect("Report V1 diagnostics must be an array")
        .iter()
        .filter(|diagnostic| diagnostic["rule"].as_str() == Some(rule))
        .filter_map(|diagnostic| diagnostic["location"]["path"].as_str())
        .map(ToString::to_string)
        .collect()
}

fn report_files(report: &Value, field: &str) -> HashSet<String> {
    report["projects"]
        .as_array()
        .expect("Report V1 projects must be an array")
        .iter()
        .flat_map(|project| {
            project[field]
                .as_array()
                .expect("Report V1 project files must be an array")
        })
        .filter_map(Value::as_str)
        .map(ToString::to_string)
        .collect()
}

fn assert_no_rule_failure(report: &Value, rule: &str, reproductions: &HashMap<String, String>) {
    let failures = report["audit"]["analysis_failures"]
        .as_array()
        .expect("Report V1 analysis failures must be an array");
    if let Some(failure) = failures
        .iter()
        .find(|failure| failure["rule"].as_str() == Some(rule))
    {
        let path = failure["path"].as_str().unwrap_or("<unknown>");
        let reproduction = reproductions
            .get(path)
            .map_or("<reproduction unavailable>", String::as_str);
        panic!("rule failure escaped containment: {failure}; reproduction={reproduction}");
    }
}

fn run_liveness_cases(binary: &Path, rule: &ConformanceRule) {
    let positives = rule
        .cases
        .iter()
        .filter(|case| case.kind == "positive")
        .count();
    let negatives = rule
        .cases
        .iter()
        .filter(|case| case.kind == "negative")
        .count();
    assert!(positives >= 2, "{} requires two positive fixtures", rule.id);
    assert!(
        negatives >= 4,
        "{} requires four negative fixtures",
        rule.id
    );

    for (index, case) in rule.cases.iter().enumerate() {
        let root = tempfile::tempdir().expect("create conformance project");
        write_project_manifest(root.path());
        let path = unique_case_path(index, &case.path);
        write_case(root.path(), &path, case.source.as_bytes());
        let report = run_candidate(binary, root.path());
        let fired =
            diagnostic_paths(&report, &rule.id).contains(&path.to_string_lossy().to_string());
        assert_eq!(
            fired,
            case.kind == "positive",
            "production liveness oracle failed: rule={} kind={} path={} source={:?}",
            rule.id,
            case.kind,
            path.display(),
            case.source
        );
    }
}

fn run_mutations(binary: &Path, rule: &ConformanceRule) {
    let root = tempfile::tempdir().expect("create mutation project");
    write_project_manifest(root.path());
    let mutation_root = root.path().join("src/mutations");
    std::fs::create_dir_all(&mutation_root).expect("create mutation source directory");
    let mut distinct_parseable = HashSet::new();
    let mut parseable_paths = HashSet::new();
    let mut reproductions = HashMap::new();

    let mut candidate_index = 0_u64;
    while parseable_paths.len() < MUTATIONS_PER_RULE {
        let case = &rule.cases[candidate_index as usize % rule.cases.len()];
        let seed = stable_seed(&rule.id, candidate_index);
        candidate_index += 1;
        let input = mutate_source(seed, &case.source);
        let Ok(source) = std::str::from_utf8(&input) else {
            continue;
        };
        if syn::parse_file(source).is_err() {
            continue;
        }
        let iteration = parseable_paths.len();
        let path = format!("src/mutations/case-{iteration:04}.rs");
        write_case(root.path(), Path::new(&path), &input);
        reproductions.insert(
            path.clone(),
            format!(
                "seed={seed:#018x} input={:?}",
                String::from_utf8_lossy(&input)
            ),
        );
        let mut digest = Sha256::new();
        digest.update(source.as_bytes());
        distinct_parseable.insert(format!("{:x}", digest.finalize()));
        parseable_paths.insert(path);
    }

    assert!(
        distinct_parseable.len() >= MIN_DISTINCT_PARSEABLE,
        "{} produced only {} distinct parseable mutations",
        rule.id,
        distinct_parseable.len()
    );
    let report = run_candidate(binary, root.path());
    assert_no_rule_failure(&report, &rule.id, &reproductions);
    let planned_paths = report_files(&report, "planned_files");
    assert!(
        reproductions
            .keys()
            .all(|path| planned_paths.contains(path)),
        "{} did not plan every mutation source",
        rule.id
    );
    let analyzed_paths = report_files(&report, "analyzed_files");
    assert!(
        parseable_paths
            .iter()
            .all(|path| analyzed_paths.contains(path)),
        "{} did not analyze every parseable mutation source",
        rule.id
    );
    let fired_paths = diagnostic_paths(&report, &rule.id);
    let fired = parseable_paths
        .iter()
        .filter(|path| fired_paths.contains(*path))
        .count();
    assert!(
        fired > 0 && fired < parseable_paths.len(),
        "{} mutation oracle was non-live or fired universally: fired={fired}, quiet={}",
        rule.id,
        parseable_paths.len().saturating_sub(fired)
    );
    println!(
        "mutation passed rule={} mutations={} parseable={} fired={} quiet={}",
        rule.id,
        MUTATIONS_PER_RULE,
        parseable_paths.len(),
        fired,
        parseable_paths.len() - fired
    );
}

fn run_malformed_boundaries(binary: &Path, rule: &str) {
    let root = tempfile::tempdir().expect("create malformed mutation project");
    write_project_manifest(root.path());
    let malformed: [(&str, &[u8]); 4] = [
        ("src/malformed/truncated.rs", b"fn truncated( {"),
        (
            "src/malformed/invalid-utf8.rs",
            &[b'f', b'n', b' ', 0xf0, 0x9f, 0x92],
        ),
        (
            "src/malformed/unbalanced.rs",
            b"fn unbalanced() { let value = (((1 + 2); }",
        ),
        (
            "src/malformed/invalid-let.rs",
            b"fn invalid_let() { let = value; }",
        ),
    ];
    for (path, source) in malformed {
        write_case(root.path(), Path::new(path), source);
    }
    let report = run_candidate(binary, root.path());
    let failures = report["audit"]["analysis_failures"]
        .as_array()
        .expect("Report V1 analysis failures must be an array");
    for (path, _) in malformed {
        let failure = failures
            .iter()
            .find(|failure| failure["path"].as_str() == Some(path))
            .unwrap_or_else(|| panic!("{rule} did not retain malformed receipt for {path}"));
        assert!(
            matches!(
                failure["kind"].as_str(),
                Some("parse_failed" | "read_failed")
            ),
            "{rule} emitted an invalid malformed receipt: {failure}"
        );
        assert_eq!(failure["rule"], Value::Null);
    }
}

fn synthetic_child() {
    match std::env::var("EP006_ORACLE_SYNTHETIC").as_deref() {
        Ok("panic") => panic!("synthetic oracle panic"),
        Ok("abort") => std::process::abort(),
        Ok("timeout") => thread::sleep(Duration::from_mins(1)),
        _ => {}
    }
}

fn synthetic_outcome(mode: &str, timeout: Duration) -> ProcessOutcome {
    let mut child =
        Command::new(std::env::current_exe().expect("resolve trusted oracle executable"))
            .env("EP006_ORACLE_SYNTHETIC", mode)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn trusted supervisor contract child");
    wait_for_process(&mut child, timeout)
}

fn prove_supervisor_contract() {
    assert!(matches!(
        synthetic_outcome("panic", Duration::from_secs(2)),
        ProcessOutcome::Exited(status) if !status.success()
    ));
    assert!(matches!(
        synthetic_outcome("abort", Duration::from_secs(2)),
        ProcessOutcome::Exited(status) if !status.success()
    ));
    assert!(matches!(
        synthetic_outcome("timeout", Duration::from_millis(100)),
        ProcessOutcome::TimedOut
    ));
}

fn required_arg(value: Option<&OsStr>, name: &str) -> PathBuf {
    value.map(PathBuf::from).unwrap_or_else(|| {
        panic!(
            "usage: ep006-protected-mutation PRODUCTION_BINARY CONFORMANCE_FIXTURES PROMOTION_POLICY (missing {name})"
        )
    })
}

fn main() {
    if std::env::var_os("EP006_ORACLE_SYNTHETIC").is_some() {
        synthetic_child();
        return;
    }
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    let binary = required_arg(arguments.first().map(AsRef::as_ref), "production binary")
        .canonicalize()
        .expect("canonicalize exact production candidate binary");
    let fixture_path = required_arg(arguments.get(1).map(AsRef::as_ref), "conformance fixtures");
    let policy_path = required_arg(arguments.get(2).map(AsRef::as_ref), "promotion policy");
    assert_eq!(
        arguments.len(),
        3,
        "trusted mutation oracle accepts three arguments"
    );

    prove_supervisor_contract();
    let expected = expected_rules(&policy_path);
    assert_eq!(expected.len(), 15, "trusted policy must contain 15 rules");
    let expected_set: HashSet<_> = expected.iter().map(String::as_str).collect();
    assert_eq!(
        expected_set.len(),
        expected.len(),
        "trusted policy contains duplicate rule IDs"
    );
    let fixtures: HashMap<_, _> = conformance_manifest(&fixture_path)
        .into_iter()
        .map(|fixture| (fixture.id.clone(), fixture))
        .collect();
    assert_eq!(
        fixtures.len(),
        expected.len(),
        "trusted fixtures do not cover every EP-006 rule"
    );

    for rule in expected {
        let fixture = fixtures
            .get(&rule)
            .unwrap_or_else(|| panic!("trusted fixtures do not cover {rule}"));
        run_liveness_cases(&binary, fixture);
        run_malformed_boundaries(&binary, &rule);
        run_mutations(&binary, fixture);
    }
}
