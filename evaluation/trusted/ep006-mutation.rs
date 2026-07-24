use super::{CustomRule, all_custom_rules};
use std::collections::{HashMap, HashSet};
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

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

fn expected_rules() -> Vec<String> {
    let policy: serde_json::Value = serde_json::from_str(include_str!("ep006-trusted-policy.json"))
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

fn conformance_manifest() -> Vec<ConformanceRule> {
    let input = include_str!("ep006-trusted-conformance.txt");
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

fn rule_panics(rule: &dyn CustomRule, input: &[u8]) -> bool {
    panic::catch_unwind(AssertUnwindSafe(|| {
        let Ok(source) = std::str::from_utf8(input) else {
            return;
        };
        let Ok(syntax) = syn::parse_file(source) else {
            return;
        };
        let _ = rule.check_file(&syntax, Path::new("mutation.rs"));
    }))
    .is_err()
}

fn minimize_panicking_input(rule: &dyn CustomRule, mut input: Vec<u8>) -> Vec<u8> {
    let mut chunk = input.len().div_ceil(2);
    while chunk > 0 {
        let mut removed = false;
        let mut start = 0;
        while start < input.len() {
            let end = (start + chunk).min(input.len());
            let mut candidate = input.clone();
            candidate.drain(start..end);
            if rule_panics(rule, &candidate) {
                input = candidate;
                removed = true;
                break;
            }
            start += chunk;
        }
        if !removed {
            chunk /= 2;
        }
    }
    input
}

fn xorshift(mut value: u64) -> u64 {
    value ^= value << 13;
    value ^= value >> 7;
    value ^ (value << 17)
}

fn mutate_source(seed: u64, source: &str) -> Vec<u8> {
    let mut bytes = source.as_bytes().to_vec();
    if bytes.is_empty() {
        bytes.extend_from_slice(b"fn empty() {}\n");
    }
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

fn adversarial_input(iteration: usize, source: &str, seed: u64) -> Vec<u8> {
    match iteration {
        0 => b"fn truncated( {".to_vec(),
        1 => vec![b'f', b'n', b' ', 0xf0, 0x9f, 0x92],
        2 => {
            let mut deep = String::from("fn deep() {");
            deep.push_str(&"{".repeat(64));
            deep.push_str("let _value = 1;");
            deep.push_str(&"}".repeat(64));
            deep.push('}');
            deep.into_bytes()
        }
        3 => b"#[unknown(attribute)] fn attributed() {}".to_vec(),
        _ => mutate_source(seed, source),
    }
}

fn fires(rule: &dyn CustomRule, source: &str, path: &Path) -> bool {
    let syntax = syn::parse_file(source).expect("metamorphic input must remain parseable");
    rule.check_file(&syntax, path)
        .iter()
        .any(|diagnostic| diagnostic.rule == rule.name())
}

fn run_mutations_for_rule(rule: &dyn CustomRule, fixture: &ConformanceRule) {
    const MUTATIONS_PER_RULE: usize = 1_000;
    const MIN_DISTINCT_PARSEABLE: usize = 100;
    let positive = fixture
        .cases
        .iter()
        .find(|case| case.kind == "positive")
        .expect("conformance requires a positive liveness fixture");
    let negative = fixture
        .cases
        .iter()
        .find(|case| case.kind == "negative")
        .expect("conformance requires a negative liveness fixture");
    write_mutation_reproduction(
        stable_seed(rule.name(), u64::MAX),
        positive.source.as_bytes(),
    );
    assert!(
        fires(rule, &positive.source, &positive.path),
        "{} failed its positive liveness oracle",
        rule.name()
    );
    write_mutation_reproduction(
        stable_seed(rule.name(), u64::MAX - 1),
        negative.source.as_bytes(),
    );
    assert!(
        !fires(rule, &negative.source, &negative.path),
        "{} failed its negative liveness oracle",
        rule.name()
    );
    for (iteration, case) in fixture
        .cases
        .iter()
        .filter(|case| matches!(case.kind.as_str(), "positive" | "negative"))
        .enumerate()
    {
        let seed = stable_seed(rule.name(), iteration as u64);
        let source = format!(
            "// rust-doctor-metamorphic-seed:{seed:016x}\n{}",
            case.source
        );
        let expected = case.kind == "positive";
        write_mutation_reproduction(seed, source.as_bytes());
        assert_eq!(
            fires(rule, &source, &case.path),
            expected,
            "metamorphic oracle failed: rule={} seed={seed:#018x} minimized_input={source:?}",
            rule.name()
        );
    }

    let mut parseable = HashSet::new();
    let mut fired = 0usize;
    let mut did_not_fire = 0usize;
    let mut parse_skips = 0usize;
    for iteration in 0..MUTATIONS_PER_RULE {
        let case = &fixture.cases[iteration % fixture.cases.len()];
        let seed = stable_seed(rule.name(), iteration as u64);
        let input = adversarial_input(iteration, &case.source, seed);
        write_mutation_reproduction(seed, &input);
        if rule_panics(rule, &input) {
            let minimized = minimize_panicking_input(rule, input);
            panic!(
                "mutation panic: rule={} seed={seed:#018x} minimized_input={:?}",
                rule.name(),
                String::from_utf8_lossy(&minimized)
            );
        }
        let Ok(source) = std::str::from_utf8(&input) else {
            parse_skips += 1;
            continue;
        };
        let Ok(syntax) = syn::parse_file(source) else {
            parse_skips += 1;
            continue;
        };
        parseable.insert(hex_input(source));
        if rule
            .check_file(&syntax, &case.path)
            .iter()
            .any(|diagnostic| diagnostic.rule == rule.name())
        {
            fired += 1;
        } else {
            did_not_fire += 1;
        }
    }
    assert!(
        parseable.len() >= MIN_DISTINCT_PARSEABLE,
        "{} produced only {} distinct parseable mutations (parse_skips={parse_skips})",
        rule.name(),
        parseable.len()
    );
    assert!(
        parse_skips > 0,
        "{} did not exercise the parse-skip boundary",
        rule.name()
    );
    assert!(
        fired > 0 && did_not_fire > 0,
        "{} mutation oracle was non-live or fired universally: fired={fired}, quiet={did_not_fire}",
        rule.name()
    );
}

fn hex_input(source: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    digest.update(source.as_bytes());
    format!("{:x}", digest.finalize())
}

#[derive(Debug, PartialEq, Eq)]
enum WorkerOutcome {
    Success,
    CaughtPanic(String),
    Abort(String),
    Timeout(String),
}

fn mutation_worker(rule: Option<&str>, mode: Option<&str>, timeout: Duration) -> WorkerOutcome {
    let reproduction_dir = tempfile::tempdir().expect("create mutation reproduction directory");
    let reproduction_path = reproduction_dir.path().join("active-mutation.txt");
    std::fs::write(
        &reproduction_path,
        "seed=0x0000000000000000\ninput=\"synthetic worker boundary\"\n",
    )
    .expect("initialize mutation reproduction");
    let mut command = Command::new(std::env::current_exe().expect("current executable"));
    command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("RUST_DOCTOR_MUTATION_REPRO", &reproduction_path);
    if let Some(rule) = rule {
        command.env("RUST_DOCTOR_MUTATION_RULE", rule);
    }
    if let Some(mode) = mode {
        command.env("RUST_DOCTOR_MUTATION_MODE", mode);
    }
    let mut child = command.spawn().expect("spawn mutation worker");
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().expect("poll mutation worker") {
            let output = child.wait_with_output().expect("collect mutation worker");
            if status.success() {
                return WorkerOutcome::Success;
            }
            let stderr = String::from_utf8_lossy(&output.stderr);
            return if stderr.contains("mutation panic:") {
                WorkerOutcome::CaughtPanic(format!(
                    "{stderr}\n{}",
                    read_mutation_reproduction(&reproduction_path)
                ))
            } else {
                WorkerOutcome::Abort(read_mutation_reproduction(&reproduction_path))
            };
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return WorkerOutcome::Timeout(read_mutation_reproduction(&reproduction_path));
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn write_mutation_reproduction(seed: u64, input: &[u8]) {
    let Ok(path) = std::env::var("RUST_DOCTOR_MUTATION_REPRO") else {
        return;
    };
    std::fs::write(
        path,
        format!(
            "seed={seed:#018x}\ninput={:?}\n",
            String::from_utf8_lossy(input)
        ),
    )
    .expect("persist active mutation reproduction");
}

fn read_mutation_reproduction(path: &Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_else(|error| format!("reproduction unavailable: {error}"))
}

fn mutation_worker_child() {
    match std::env::var("RUST_DOCTOR_MUTATION_MODE").as_deref() {
        Ok("abort") => std::process::abort(),
        Ok("panic") => panic!("mutation panic: synthetic worker boundary"),
        Ok("timeout") => thread::sleep(Duration::from_mins(1)),
        _ => {}
    }
    let selected =
        std::env::var("RUST_DOCTOR_MUTATION_RULE").expect("mutation worker requires a rule ID");
    let fixtures: HashMap<_, _> = conformance_manifest()
        .into_iter()
        .map(|fixture| (fixture.id.clone(), fixture))
        .collect();
    let rule = all_custom_rules()
        .into_iter()
        .find(|rule| rule.name() == selected)
        .expect("selected mutation rule must exist");
    run_mutations_for_rule(rule.as_ref(), &fixtures[&selected]);
}

fn run_parent() {
    let panic = mutation_worker(None, Some("panic"), Duration::from_secs(2));
    assert!(
        matches!(panic, WorkerOutcome::CaughtPanic(message) if message.contains("seed=") && message.contains("input="))
    );
    let abort = mutation_worker(None, Some("abort"), Duration::from_secs(2));
    assert!(
        matches!(abort, WorkerOutcome::Abort(reproduction) if reproduction.contains("seed=") && reproduction.contains("input="))
    );
    let timeout = mutation_worker(None, Some("timeout"), Duration::from_millis(100));
    assert!(
        matches!(timeout, WorkerOutcome::Timeout(reproduction) if reproduction.contains("seed=") && reproduction.contains("input="))
    );

    let expected = expected_rules();
    assert_eq!(expected.len(), 15, "trusted policy must contain 15 rules");
    let expected_set: HashSet<_> = expected.iter().map(String::as_str).collect();
    assert_eq!(
        expected_set.len(),
        expected.len(),
        "trusted policy contains duplicate rule IDs"
    );
    let rules: HashMap<_, _> = all_custom_rules()
        .into_iter()
        .filter(|rule| expected_set.contains(rule.name()))
        .map(|rule| (rule.name(), rule))
        .collect();
    assert_eq!(
        rules.len(),
        expected.len(),
        "candidate does not expose every trusted EP-006 rule"
    );
    let fixtures: HashMap<_, _> = conformance_manifest()
        .into_iter()
        .map(|fixture| (fixture.id.clone(), fixture))
        .collect();
    assert_eq!(
        fixtures.len(),
        expected.len(),
        "trusted fixtures do not cover every EP-006 rule"
    );
    for rule_id in expected {
        let fixture = &fixtures[&rule_id];
        assert!(
            fixture
                .cases
                .iter()
                .filter(|case| case.kind == "positive")
                .count()
                >= 2,
            "{rule_id} requires at least two positive fixtures"
        );
        assert!(
            fixture
                .cases
                .iter()
                .filter(|case| case.kind == "negative")
                .count()
                >= 4,
            "{rule_id} requires at least four negative fixtures"
        );
        let outcome = mutation_worker(Some(&rule_id), None, Duration::from_secs(10));
        assert!(
            matches!(outcome, WorkerOutcome::Success),
            "mutation worker failed for {rule_id}: {outcome:?}"
        );
        println!("mutation passed rule={rule_id} mutations=1000");
    }
}

pub(crate) fn run() -> i32 {
    if std::env::var_os("RUST_DOCTOR_MUTATION_RULE").is_some()
        || std::env::var_os("RUST_DOCTOR_MUTATION_MODE").is_some()
    {
        mutation_worker_child();
    } else {
        run_parent();
    }
    0
}
