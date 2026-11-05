#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::string_slice))]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use rust_doctor::{OVERSIZED_UNIT_FILE_LINES, SCHEMA_VERSION, catalog};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn read(relative: &str) -> String {
    fs::read_to_string(repo().join(relative))
        .unwrap_or_else(|_| unreachable!("{relative} should be readable"))
}

fn flat(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn bare(token: &str) -> &str {
    token.trim_matches(|character: char| {
        !character.is_ascii_alphanumeric() && character != '_' && character != ':'
    })
}

fn standing_documents() -> Vec<(String, String)> {
    let manifest: serde_json::Value =
        serde_json::from_str(&read("docs/doc-budgets.json")).expect("the manifest should be JSON");
    let budgets = manifest
        .get("budgets")
        .and_then(serde_json::Value::as_object)
        .expect("the manifest should carry a `budgets` object");
    let documents: Vec<(String, String)> = budgets
        .keys()
        .map(|path| (path.clone(), flat(&read(path))))
        .collect();
    assert!(
        documents.len() > 1,
        "the manifest should budget more than one document"
    );
    documents
}

fn documents_under_contract() -> Vec<(String, String)> {
    let mut documents = standing_documents();
    documents.push(("README.md".to_owned(), flat(&read("README.md"))));
    documents
}

fn numbers_before(text: &str, phrase: &str) -> Vec<u64> {
    text.match_indices(phrase)
        .filter_map(|(index, _)| {
            let reversed: String = text[..index]
                .chars()
                .rev()
                .take_while(char::is_ascii_digit)
                .collect();
            reversed.chars().rev().collect::<String>().parse().ok()
        })
        .collect()
}

fn numbers_after(text: &str, phrase: &str) -> Vec<u64> {
    text.match_indices(phrase)
        .filter_map(|(index, _)| {
            text[index + phrase.len()..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse()
                .ok()
        })
        .collect()
}

fn quoted_after(text: &str, marker: &str) -> Vec<String> {
    text.match_indices(marker)
        .map(|(index, _)| {
            text[index + marker.len()..]
                .chars()
                .take_while(|character| *character != '"')
                .collect()
        })
        .collect()
}

fn every_number_before(phrase: &str, expected: u64) -> usize {
    let mut stated = 0;
    for (path, text) in documents_under_contract() {
        for found in numbers_before(&text, phrase) {
            assert_eq!(
                found, expected,
                "{path} states `{found}{phrase}`, and the binary says {expected}"
            );
            stated += 1;
        }
    }
    stated
}

fn spelled(value: usize) -> &'static str {
    const WORDS: [&str; 21] = [
        "zero",
        "one",
        "two",
        "three",
        "four",
        "five",
        "six",
        "seven",
        "eight",
        "nine",
        "ten",
        "eleven",
        "twelve",
        "thirteen",
        "fourteen",
        "fifteen",
        "sixteen",
        "seventeen",
        "eighteen",
        "nineteen",
        "twenty",
    ];
    WORDS
        .get(value)
        .copied()
        .expect("a count the documents spell out should be under twenty-one")
}

fn capitalized(word: &str) -> String {
    let mut characters = word.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().collect::<String>() + characters.as_str(),
        None => String::new(),
    }
}

fn declared_constant(relative: &str, name: &str) -> u64 {
    let source = read(relative);
    let marker = format!("{name}: usize = ");
    let start = source
        .find(&marker)
        .unwrap_or_else(|| unreachable!("{relative} should declare `{name}`"))
        + marker.len();
    source[start..]
        .chars()
        .take_while(|character| character.is_ascii_digit() || *character == '_')
        .filter(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .expect("a line bound should be an integer")
}

fn rust_sources(directory: &Path) -> Vec<PathBuf> {
    let mut sources = Vec::new();
    let mut pending = vec![directory.to_path_buf()];
    while let Some(current) = pending.pop() {
        let entries = fs::read_dir(&current).expect("a source directory should be readable");
        for entry in entries {
            let path = entry.expect("a directory entry should be readable").path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                sources.push(path);
            }
        }
    }
    sources
}

fn size_bound_tests() -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for path in rust_sources(&repo().join("src")) {
        let source = fs::read_to_string(&path).expect("a source file should be readable");
        for line in source.lines() {
            let Some(rest) = line.trim_start().strip_prefix("fn ") else {
                continue;
            };
            let name = rest.split('(').next().unwrap_or_default();
            if !name.contains("size_bound") {
                continue;
            }
            assert!(
                name.starts_with("the_") && name.contains("_holds_the_size_bound"),
                "{} declares `{name}`, and the documents name the family `the_X_holds_the_size_bound`",
                path.display()
            );
            names.insert(name.to_owned());
        }
    }
    names
}

#[test]
fn the_catalog_size_the_documents_state_is_the_catalog_the_binary_ships() {
    let rules = catalog().len() as u64;
    let stated: usize = [
        " rules across",
        " rules are declared",
        " catalogued rules",
        " curated rules",
        " declarations",
    ]
    .iter()
    .map(|phrase| every_number_before(phrase, rules))
    .sum();
    assert!(
        stated >= 3,
        "the documents should state the size of the catalog, and {stated} of them do"
    );
}

#[test]
fn the_producer_counts_the_map_states_are_the_prefixes_the_catalog_carries() {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for entry in catalog() {
        let segments: Vec<&str> = entry.id.split("::").collect();
        let prefix = if let ["rust_doctor", producer, _] = segments.as_slice() {
            format!("rust_doctor::{producer}")
        } else {
            segments.first().copied().unwrap_or_default().to_owned()
        };
        *counts.entry(prefix).or_default() += 1;
    }

    let map = flat(&read("AGENTS.md"));
    let producers = format!("across {} producers", spelled(counts.len()));
    assert!(
        map.contains(&producers),
        "AGENTS.md should state the catalog spreads `{producers}`"
    );
    for (prefix, count) in &counts {
        let claim = format!("`{prefix}::*` ({count}");
        assert!(
            map.contains(&claim),
            "AGENTS.md should state `{claim}`, the rules the catalog carries under that prefix"
        );
    }
    let clippy = counts
        .get("clippy")
        .copied()
        .expect("the catalog should carry Clippy lints");
    every_number_before(" curated lints", clippy as u64);
}

#[test]
fn the_schema_version_the_documents_state_is_the_one_the_binary_publishes() {
    let published = u64::from(SCHEMA_VERSION);
    let mut stated = 0;
    for (path, text) in standing_documents() {
        for found in numbers_after(&text, "currently ") {
            assert_eq!(
                found, published,
                "{path} states the report schema as {found}, and the binary publishes {published}"
            );
            stated += 1;
        }
    }
    assert_eq!(
        stated, 1,
        "the schema version belongs in exactly one standing document, written `currently <n>`"
    );

    let home = "docs/subsystems/reporting.md";
    let ledger = flat(&read(home));
    assert!(
        ledger.contains(&format!("v{published} ")),
        "{home} should name what v{published} added to the report"
    );
    let next = published + 1;
    assert!(
        !ledger.contains(&format!("v{next} ")),
        "{home} names a v{next} the binary does not publish"
    );
}

#[test]
fn the_bounds_the_documents_state_are_the_bounds_the_structure_pass_reports_at() {
    let file_lines = OVERSIZED_UNIT_FILE_LINES as u64;
    let stated = every_number_before(" lines `oversized_unit` reports", file_lines);
    assert!(
        stated >= 2,
        "the documents should state the file bound `oversized_unit` reports at, and {stated} do"
    );

    let impl_lines = declared_constant("src/structure/hotspots.rs", "IMPL_LINES");
    let claim = format!("under the {impl_lines} it reports at");
    assert!(
        flat(&read("AGENTS.md")).contains(&claim),
        "AGENTS.md should hold every `impl` block `{claim}`"
    );
}

#[test]
fn the_size_bound_tests_the_documents_count_are_the_tests_the_source_carries() {
    let tests = size_bound_tests();
    let counted = spelled(tests.len());
    let family = "`the_X_holds_the_size_bound` tests";

    let map = format!("{} {family}", capitalized(counted));
    assert!(
        flat(&read("AGENTS.md")).contains(&map),
        "AGENTS.md should count `{map}`, one per module"
    );
    let home = format!("The {counted} {family}");
    assert!(
        flat(&read("docs/testing.md")).contains(&home),
        "docs/testing.md should count `{home}`"
    );

    for (path, text) in standing_documents() {
        for token in text.split_whitespace().map(bare) {
            if token.contains("size_bound") && token != "the_X_holds_the_size_bound" {
                assert!(
                    tests.contains(token),
                    "{path} names `{token}`, which no module of `src/` declares"
                );
            }
        }
    }
}

#[test]
fn the_workflows_the_map_counts_all_pin_the_toolchain_it_names() {
    let directory = repo().join(".github/workflows");
    let mut workflows: Vec<PathBuf> = fs::read_dir(&directory)
        .expect("the workflow directory should be readable")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|extension| extension == "yml"))
        .collect();
    workflows.sort();

    let map = flat(&read("AGENTS.md"));
    let counted = format!("{} workflows settle", capitalized(spelled(workflows.len())));
    assert!(
        map.contains(&counted),
        "AGENTS.md should count `{counted}`, the workflows on disk"
    );

    let marker = "pins toolchain ";
    let start = map
        .find(marker)
        .expect("AGENTS.md should name the toolchain its workflows pin")
        + marker.len();
    let toolchain: String = map[start..]
        .chars()
        .take_while(|character| character.is_ascii_digit() || *character == '.')
        .collect();
    let toolchain = toolchain.trim_end_matches('.').to_owned();
    assert!(
        !toolchain.is_empty(),
        "AGENTS.md should name a toolchain version after `{marker}`"
    );
    assert!(
        flat(&read("docs/ci-workflows.md")).contains(&toolchain),
        "docs/ci-workflows.md should name the same toolchain {toolchain} the map does"
    );

    let manifest = read("Cargo.toml");
    let msrv = quoted_after(&manifest, "rust-version = \"")
        .first()
        .cloned()
        .expect("Cargo.toml should declare a rust-version");
    assert!(
        map.contains(&format!("rustc {msrv} or later")),
        "AGENTS.md should state the crate needs `rustc {msrv} or later`"
    );

    for path in &workflows {
        let text = fs::read_to_string(path).expect("a workflow should be readable");
        let pinned = quoted_after(&text, "toolchain: \"");
        assert!(
            pinned.contains(&toolchain),
            "{} pins no toolchain {toolchain}, which the map says every workflow does",
            path.display()
        );
        for version in &pinned {
            assert!(
                *version == toolchain || *version == msrv,
                "{} pins toolchain {version}, which is neither {toolchain} nor the declared MSRV {msrv}",
                path.display()
            );
        }
    }
}

#[test]
fn every_checkout_drops_its_persisted_credentials() {
    let directory = repo().join(".github/workflows");
    let mut checkouts = 0;
    for entry in fs::read_dir(&directory).expect("the workflow directory should be readable") {
        let path = entry.expect("a workflow entry should be readable").path();
        if !path.extension().is_some_and(|extension| extension == "yml") {
            continue;
        }
        let text = fs::read_to_string(&path).expect("a workflow should be readable");
        let lines: Vec<&str> = text.lines().collect();
        for (index, line) in lines.iter().enumerate() {
            if !line.trim_start().starts_with("- uses: actions/checkout@") {
                continue;
            }
            checkouts += 1;
            let indent = line.len() - line.trim_start().len() + 2;
            let step: Vec<&str> = lines[index + 1..]
                .iter()
                .take_while(|next| {
                    next.trim().is_empty() || next.len() - next.trim_start().len() >= indent
                })
                .copied()
                .collect();
            assert!(
                step.iter().any(|next| next.trim() == "persist-credentials: false"),
                "{}:{} checks out without `persist-credentials: false`",
                path.display(),
                index + 1
            );
        }
    }
    assert!(checkouts > 0, "no checkout step was found");
}

fn release_pin_violation(line: &str) -> Option<&'static str> {
    fn exact_version(value: &str) -> bool {
        let parts: Vec<&str> = value.split('.').collect();
        parts.len() == 3
            && parts
                .iter()
                .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
    }

    let trimmed = line.trim_start().trim_start_matches("- ").trim();
    if let Some(action) = trimmed.strip_prefix("uses:") {
        let action = action.trim();
        if action.starts_with("./") {
            return None;
        }
        let Some((_, pin)) = action.split_once('@') else {
            return Some("a `uses:` without a ref");
        };
        let (commit, comment) = pin.split_once(' ').unwrap_or((pin, ""));
        if commit.len() != 40
            || !commit
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Some("a `uses:` whose ref is not a 40-hex commit");
        }
        if !comment.trim().is_empty() {
            return Some("a pinned `uses:` with text after its commit");
        }
        return None;
    }
    if let Some((_, spec)) = trimmed.split_once("npm@") {
        let spec = spec.split_whitespace().next().unwrap_or_default();
        if !exact_version(spec) {
            return Some("an `npm@` spec that is not an exact x.y.z");
        }
    }
    if let Some(version) = trimmed.strip_prefix("node-version:")
        && !exact_version(version.trim().trim_matches('"'))
    {
        return Some("a `node-version` that is not an exact x.y.z");
    }
    None
}

#[test]
fn release_yml_binds_every_program_by_commit_or_exact_version() {
    let release = read(".github/workflows/release.yml");
    let violations: Vec<String> = release
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            release_pin_violation(line).map(|violation| {
                format!("release.yml:{} holds {violation}: {}", index + 1, line.trim())
            })
        })
        .collect();
    assert!(violations.is_empty(), "{}", violations.join("\n"));
    assert!(
        release.lines().any(|line| line.trim_start().starts_with("- uses:")),
        "release.yml should hold `uses:` steps"
    );
    for sample in ["npm install -g npm@11.21.1", "node-version: \"22.13.0\""] {
        assert_eq!(release_pin_violation(sample), None, "{sample} binds exactly");
    }
    assert_eq!(
        release_pin_violation("      - uses: actions/checkout@fbc6f3992d24b796d5a048ff273f7fcc4a7b6c09"),
        None
    );
}

#[test]
fn the_release_pin_predicate_rejects_every_mutable_form() {
    for sample in [
        "      - uses: actions/checkout@v5",
        "      - uses: dtolnay/rust-toolchain@master",
        "      - uses: actions/checkout@fbc6f3992d24b796d5a048ff273f7fcc4a7b6c0",
        "      - uses: actions/checkout@fbc6f3992d24b796d5a048ff273f7fcc4a7b6c09 # v5.1.0",
        "      - uses: actions/checkout@fbc6f3992d24b796d5a048ff273f7fcc4a7b6c09 #",
        "      - uses: actions/checkout",
        "        run: npm install -g npm@^11.5.1",
        "        run: npm install -g npm@11",
        "          node-version: \"22\"",
        "          node-version: 22.x",
    ] {
        assert!(
            release_pin_violation(sample).is_some(),
            "{sample} should be rejected as mutable"
        );
    }
}

#[test]
fn the_network_promise_names_cargo_s_downloads_and_how_to_forbid_them() {
    let help = std::process::Command::new(env!("CARGO_BIN_EXE_rust-doctor"))
        .arg("--help")
        .output()
        .unwrap();
    for (name, text) in [
        ("README.md", read("README.md")),
        ("skills/rust-doctor/SKILL.md", read("skills/rust-doctor/SKILL.md")),
        ("--help", String::from_utf8(help.stdout).unwrap()),
    ] {
        let text = flat(&text);
        for promise in [
            "opens no network connection",
            "Cargo downloads missing dependencies as `cargo build` does",
            "CARGO_NET_OFFLINE=true",
        ] {
            assert!(text.contains(promise), "{name} does not say {promise:?}");
        }
        for broken in ["no network,", "reaches no network"] {
            assert!(!text.contains(broken), "{name} still says {broken:?}");
        }
    }
}

fn workflow_jobs(text: &str) -> Vec<(String, Vec<&str>)> {
    let mut jobs: Vec<(String, Vec<&str>)> = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        if !line.starts_with(' ') && !line.trim().is_empty() && !line.starts_with('#') {
            inside = line == "jobs:";
            continue;
        }
        if !inside {
            continue;
        }
        let key = line.strip_prefix("  ").and_then(|rest| rest.strip_suffix(':'));
        match key {
            Some(id) if !id.starts_with([' ', '#']) => jobs.push((id.to_owned(), Vec::new())),
            _ => {
                if let Some((_, lines)) = jobs.last_mut() {
                    lines.push(line);
                }
            }
        }
    }
    jobs
}

fn job_steps<'a>(lines: &[&'a str]) -> Vec<Vec<&'a str>> {
    let mut steps: Vec<Vec<&'a str>> = Vec::new();
    for line in lines {
        if line.starts_with("      - ") {
            steps.push(vec![line]);
        } else if line.starts_with("       ")
            || line.trim().is_empty()
            || line.trim_start().starts_with('#')
        {
            if let Some(step) = steps.last_mut() {
                step.push(line);
            }
        } else if !steps.is_empty() {
            break;
        }
    }
    steps
}

fn restores_a_cache(line: &str) -> bool {
    let trimmed = line.trim_start().trim_start_matches("- ").trim();
    trimmed.strip_prefix("uses:").is_some_and(|action| {
        let action = action.trim();
        action.starts_with("Swatinem/rust-cache@") || action.starts_with("actions/cache@")
    })
}

fn cargo_commands(line: &str) -> Vec<String> {
    let trimmed = line.trim();
    if trimmed.starts_with('#') {
        return Vec::new();
    }
    let mut commands = Vec::new();
    let mut rest = trimmed;
    while let Some(at) = rest.find("cargo ") {
        let bounded = rest[..at]
            .chars()
            .next_back()
            .is_none_or(|before| !before.is_ascii_alphanumeric() && before != '_' && before != '-');
        let after = &rest[at..];
        if bounded {
            let end = after.find(['|', ')', ';', '&']).unwrap_or(after.len());
            commands.push(after[..end].trim().to_owned());
        }
        rest = &after["cargo ".len()..];
    }
    commands
}

fn oidc_job_violations(text: &str) -> (usize, Vec<String>) {
    let preamble = text.split("\njobs:").next().unwrap_or_default();
    let granted_to_all = preamble.lines().any(|line| line.trim() == "id-token: write");
    let mut oidc_jobs = 0;
    let mut violations = Vec::new();
    for (id, lines) in workflow_jobs(text) {
        if !granted_to_all && !lines.iter().any(|line| line.trim() == "id-token: write") {
            continue;
        }
        oidc_jobs += 1;
        for line in &lines {
            if restores_a_cache(line) {
                violations.push(format!("job `{id}` restores a cache: {}", line.trim()));
            }
            for command in cargo_commands(line) {
                if !command.starts_with("cargo metadata --no-deps")
                    && command != "cargo publish --locked --no-verify"
                {
                    violations.push(format!("job `{id}` runs `{command}`"));
                }
            }
        }
    }
    (oidc_jobs, violations)
}

#[test]
fn no_release_job_holding_an_oidc_identity_compiles_or_restores_a_cache() {
    let (oidc_jobs, violations) = oidc_job_violations(&read(".github/workflows/release.yml"));
    assert!(violations.is_empty(), "{}", violations.join("\n"));
    assert_eq!(oidc_jobs, 2, "release.yml should hold the npm and crates.io OIDC jobs");
}

#[test]
fn the_oidc_job_check_rejects_a_compiling_or_cached_job() {
    let job = |body: &str| {
        format!(
            "permissions:\n  contents: read\njobs:\n  crates:\n    permissions:\n      id-token: write\n    steps:\n{body}\n"
        )
    };
    for body in [
        "      - run: cargo publish --locked --dry-run",
        "      - run: cargo build --release --locked",
        "      - run: echo x && cargo test",
        "      - uses: Swatinem/rust-cache@d1527c8f67ef9af357290475d4af4b0fab8f12e0 # v2.9.3",
        "      - uses: actions/cache@0400d5f644dc74513175e3cd8d07132dd4860809 # v4.2.4",
    ] {
        let (oidc_jobs, violations) = oidc_job_violations(&job(body));
        assert_eq!(oidc_jobs, 1);
        assert!(!violations.is_empty(), "{body} should be refused in an OIDC job");
    }
    for body in [
        "      - run: version=\"$(cargo metadata --no-deps --format-version 1 | jq -r .x)\"",
        "      - run: cargo publish --locked --no-verify",
        "      - run: |\n          # cargo build would compile here\n          true",
    ] {
        assert_eq!(oidc_job_violations(&job(body)).1, Vec::<String>::new(), "{body}");
    }
    let unprivileged = "jobs:\n  verify:\n    steps:\n      - run: cargo publish --locked --dry-run\n";
    assert_eq!(oidc_job_violations(unprivileged), (0, Vec::new()));
    let inherited =
        "permissions:\n  id-token: write\njobs:\n  verify:\n    steps:\n      - run: cargo build\n";
    assert_eq!(oidc_job_violations(inherited).0, 1);
    assert_eq!(oidc_job_violations(inherited).1.len(), 1);
}

#[test]
fn the_crate_is_verified_on_every_run_and_published_only_from_a_tag() {
    let release = read(".github/workflows/release.yml");
    let jobs = workflow_jobs(&release);
    let job = |id: &str| {
        jobs.iter()
            .find(|(name, _)| name == id)
            .map(|(_, lines)| lines.clone())
            .unwrap_or_else(|| unreachable!("release.yml should hold the `{id}` job"))
    };

    let verify = job("crates-verify");
    assert!(verify.iter().any(|line| line.trim() == "contents: read"));
    assert!(!verify.iter().any(|line| line.contains("id-token")));
    assert!(!verify.iter().any(|line| line.trim_start().starts_with("if:")));
    assert!(verify.iter().any(|line| line.trim() == "run: cargo publish --locked --dry-run"));
    assert!(verify.iter().any(|line| line.trim() == "persist-credentials: false"));

    let crates = job("crates");
    assert!(
        crates
            .iter()
            .any(|line| line.trim_start().starts_with("needs:") && line.contains("crates-verify")),
        "the `crates` job should need `crates-verify`"
    );
    let steps = job_steps(&crates);
    let heads: Vec<String> = steps
        .iter()
        .map(|step| {
            let head = step[0].trim().trim_start_matches("- ");
            head.split('@').next().unwrap_or(head).to_owned()
        })
        .collect();
    assert_eq!(
        heads,
        [
            "uses: actions/checkout",
            "uses: dtolnay/rust-toolchain",
            "name: Resolve the release version",
            "uses: rust-lang/crates-io-auth-action",
            "name: Publish",
        ]
    );
    for step in &steps[3..] {
        assert!(
            step.iter().any(|line| line.trim().starts_with("if: github.ref_type == 'tag'")),
            "{} should run only from a tag",
            step[0].trim()
        );
    }
}

#[test]
fn release_yml_restores_no_cache() {
    let release = read(".github/workflows/release.yml");
    let restoring: Vec<String> = release
        .lines()
        .enumerate()
        .filter(|(_, line)| restores_a_cache(line))
        .map(|(index, line)| format!("release.yml:{} {}", index + 1, line.trim()))
        .collect();
    assert!(restoring.is_empty(), "{}", restoring.join("\n"));
    for job in ["build", "build-windows", "launcher", "crates"] {
        assert!(
            workflow_jobs(&release).iter().any(|(id, _)| id == job),
            "release.yml should hold the `{job}` job"
        );
    }
    assert!(restores_a_cache(
        "      - uses: Swatinem/rust-cache@d1527c8f67ef9af357290475d4af4b0fab8f12e0 # v2.9.3"
    ));
    assert!(restores_a_cache("      - uses: actions/cache@v4"));
    assert!(!restores_a_cache("# No job here restores a rust-cache entry"));
}

fn fenced_lines(text: &str) -> Vec<&str> {
    let mut inside = false;
    let mut lines = Vec::new();
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            inside = !inside;
        } else if inside {
            lines.push(line.split(" #").next().unwrap_or_default());
        }
    }
    lines
}

fn newest_release_note() -> String {
    let directory = repo().join(".github/releases");
    fs::read_dir(&directory)
        .expect("the release notes should be readable")
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter_map(|name| {
            let version: Vec<u64> = name
                .strip_prefix('v')?
                .strip_suffix(".md")?
                .split('.')
                .map(str::parse)
                .collect::<Result<_, _>>()
                .ok()?;
            Some((version, name))
        })
        .max()
        .map(|(_, name)| format!(".github/releases/{name}"))
        .expect("there should be a release note")
}

#[test]
fn every_advertised_cargo_install_is_locked() {
    let newest = newest_release_note();
    let mut advertised = 0;
    for (path, text) in [
        ("README.md".to_owned(), read("README.md")),
        (newest.clone(), read(&newest)),
    ] {
        for line in fenced_lines(&text) {
            for command in cargo_commands(line) {
                if !command.starts_with("cargo install rust-doctor") {
                    continue;
                }
                advertised += 1;
                assert!(
                    command.split_whitespace().any(|word| word == "--locked"),
                    "{path} advertises `{command}` without `--locked`"
                );
            }
        }
    }
    assert!(
        advertised >= 2,
        "README.md and {newest} should each advertise `cargo install rust-doctor --locked`"
    );
    assert_eq!(
        fenced_lines("```bash\ncargo install rust-doctor # --locked\n```"),
        ["cargo install rust-doctor"],
        "a trailing comment does not lock an install"
    );
}

#[test]
fn install_yml_installs_unlocked_from_the_registry_and_from_this_commit() {
    let workflow = read(".github/workflows/install.yml");
    let installs: Vec<String> = workflow.lines().flat_map(cargo_commands).collect();
    for expected in [
        "cargo install rust-doctor --root \"$RUNNER_TEMP/published\"",
        "cargo install --path . --root \"$RUNNER_TEMP/head\"",
    ] {
        assert!(
            installs.iter().any(|command| command == expected),
            "install.yml should run `{expected}`, found {installs:?}"
        );
    }
    assert!(
        !installs.iter().any(|command| command.contains("--locked")),
        "install.yml should install without `--locked`, found {installs:?}"
    );
    assert!(
        workflow.contains("CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS: allow"),
        "install.yml should resolve `head` the way a registry install does"
    );
}
