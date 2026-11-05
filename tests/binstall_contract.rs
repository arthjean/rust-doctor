#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

fn read(path: &str) -> String {
    let text = fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(path));
    assert!(text.is_ok(), "{path} should be readable: {text:?}");
    text.unwrap()
}

fn jobs(text: &str) -> Vec<(String, Vec<&str>)> {
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
        match line.strip_prefix("  ").and_then(|rest| rest.strip_suffix(':')) {
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

fn job<'a>(jobs: &'a [(String, Vec<&'a str>)], id: &str) -> &'a [&'a str] {
    let found = jobs.iter().find(|(name, _)| name == id);
    assert!(found.is_some(), "release.yml should hold the `{id}` job");
    &found.unwrap().1
}

fn pair(platform: &str, target: &str) -> (String, String) {
    (platform.trim().to_owned(), target.trim().to_owned())
}

fn built_pairs() -> BTreeSet<(String, String)> {
    let release = read(".github/workflows/release.yml");
    let jobs = jobs(&release);
    let mut pairs = BTreeSet::new();
    let mut platform: Option<&str> = None;
    for line in job(&jobs, "build") {
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix("- platform:") {
            platform = Some(value);
        } else if let Some(value) = trimmed.strip_prefix("target:")
            && let Some(platform) = platform.take()
        {
            pairs.insert(pair(platform, value));
        }
    }
    let windows = job(&jobs, "build-windows");
    let value = |key: &str| {
        windows
            .iter()
            .find_map(|line| line.trim().strip_prefix(key))
            .map(str::trim)
            .unwrap_or_default()
    };
    let platform = value("name: rust-doctor-");
    let target = value("targets:");
    assert!(
        !platform.is_empty() && !target.is_empty(),
        "build-windows should name its artifact and its target"
    );
    pairs.insert(pair(platform, target));
    pairs
}

fn packed_pairs() -> BTreeSet<(String, String)> {
    read("scripts/pack-github-release.sh")
        .lines()
        .skip_while(|line| line.trim() != "PLATFORMS=(")
        .skip(1)
        .take_while(|line| line.trim() != ")")
        .filter_map(|line| {
            let (platform, target) = line.trim().trim_matches('"').split_once(' ')?;
            Some(pair(platform, target))
        })
        .collect()
}

fn launcher_platforms() -> BTreeSet<String> {
    read("npm/rust-doctor/lib/launcher.js")
        .lines()
        .skip_while(|line| !line.contains("PLATFORM_PACKAGES = Object.freeze({"))
        .skip(1)
        .take_while(|line| !line.trim().starts_with("})"))
        .filter_map(|line| Some(line.trim().split_once(':')?.0.trim_matches('"').to_owned()))
        .collect()
}

#[test]
fn every_built_binary_is_packed_and_every_packed_target_is_built() {
    let built = built_pairs();
    assert_eq!(built.len(), 5, "release.yml should build five binaries: {built:?}");
    assert_eq!(
        packed_pairs(),
        built,
        "scripts/pack-github-release.sh should pack exactly what release.yml builds"
    );
    let platforms: BTreeSet<String> = built.into_iter().map(|(platform, _)| platform).collect();
    assert_eq!(
        launcher_platforms(),
        platforms,
        "the launcher should declare the platforms release.yml builds"
    );
}

#[test]
fn cargo_toml_points_binstall_at_the_archives_the_script_lays_out() {
    let manifest: toml::Table = toml::from_str(&read("Cargo.toml")).unwrap();
    let package = manifest["package"].as_table().unwrap();
    assert_eq!(package["name"].as_str(), Some("rust-doctor"));
    assert_eq!(
        package["repository"].as_str(),
        Some("https://github.com/arthjean/rust-doctor"),
        "`{{ repo }}` resolves from `repository`, so it has to name the repository that releases"
    );
    let bins = manifest["bin"].as_array().unwrap();
    assert_eq!(bins.len(), 1);
    assert_eq!(bins[0]["name"].as_str(), Some("rust-doctor"));

    let binstall = package["metadata"]["binstall"].as_table().unwrap();
    assert_eq!(
        binstall["pkg-url"].as_str(),
        Some("{ repo }/releases/download/v{ version }/{ name }-{ target }.tar.gz")
    );
    assert_eq!(binstall["bin-dir"].as_str(), Some("{ name }-{ target }/{ bin }{ binary-ext }"));
    assert_eq!(binstall["pkg-fmt"].as_str(), Some("tgz"));
    let disabled: Vec<&str> = binstall["disabled-strategies"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(toml::Value::as_str)
        .collect();
    assert_eq!(
        disabled,
        ["quick-install"],
        "no binary built outside release.yml, and a target the release lacks still compiles"
    );

    let script = read("scripts/pack-github-release.sh");
    for layout in [
        "root=\"rust-doctor-$target\"",
        "binary=rust-doctor.exe",
        "\"$output/$root.tar.gz\"",
        "expected=\"$root/ $root/$binary $root/LICENSE-APACHE $root/LICENSE-MIT \"",
    ] {
        assert!(
            script.contains(layout),
            "scripts/pack-github-release.sh should lay archives out as Cargo.toml names them: {layout}"
        );
    }
}

#[test]
fn the_archives_are_proven_before_any_registry_publishes_and_after_the_release() {
    let release = read(".github/workflows/release.yml");
    let jobs = jobs(&release);
    let runs = |id: &str, command: &str| job(&jobs, id).iter().any(|line| line.contains(command));

    assert!(runs("archives", "bash scripts/pack-github-release.sh artifacts github-release"));
    assert!(runs("archives", "bash scripts/binstall-proof.sh local github-release"));
    for id in ["publish", "crates", "github-release"] {
        let needs = job(&jobs, id)
            .iter()
            .find(|line| line.trim_start().starts_with("needs:"))
            .copied()
            .unwrap_or_default();
        assert!(needs.contains("archives"), "`{id}` should need the proven archives: {needs}");
    }

    assert!(runs("github-release", "gh release create \"$tag\" --draft"));
    assert!(runs("github-release", "gh release edit \"$tag\" --draft=false --latest"));
    assert!(
        !job(&jobs, "github-release")
            .iter()
            .any(|line| line.contains("--clobber") && !line.contains("\"${archives[@]}\" --clobber")),
        "only a draft's archives may be replaced"
    );

    assert!(runs("binstall", "needs: [github-release]"));
    assert!(runs("binstall", "if: github.ref_type == 'tag'"));
    assert!(runs("binstall", "bash scripts/binstall-proof.sh registry"));
    for id in ["archives", "binstall"] {
        assert!(
            !job(&jobs, id).iter().any(|line| line.contains("id-token")
                || line.trim() == "contents: write"),
            "`{id}` runs a downloaded cargo-binstall, so it holds no write permission"
        );
    }

    let proof = read("scripts/binstall-proof.sh");
    let digest = proof
        .lines()
        .find_map(|line| line.strip_prefix("BINSTALL_SHA256="))
        .unwrap_or_default();
    assert!(
        digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "scripts/binstall-proof.sh should pin cargo-binstall by its SHA-256, found {digest:?}"
    );
    assert!(proof.contains("sha256sum -c"), "the pinned digest should be checked");
    assert!(
        proof.contains("pack-github-release.sh\" --targets"),
        "the proof should resolve every target the script packs"
    );
}
