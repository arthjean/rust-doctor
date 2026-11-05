use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use cargo_metadata::{Dependency, DependencyKind, Metadata, Package, TargetKind};
use serde::Deserialize;

use crate::policy::{
    CARGO_DUPLICATE_MAJOR_VERSIONS, CARGO_MISSING_LOCKFILE,
    CARGO_PATH_DEPENDENCY_OUTSIDE_WORKSPACE, CARGO_PERMISSIVE_LINT_TABLE,
    CARGO_PERMISSIVE_RUSTFLAGS, CARGO_RELEASE_DEBUG_SYMBOLS, CARGO_TEST_ONLY_DEPENDENCY,
    CARGO_UNBOUNDED_REGISTRY, CARGO_UNCHECKED_RELEASE_OVERFLOW, CARGO_UNMAINTAINED_DEPENDENCY,
    CARGO_UNPINNED_GIT, ActiveRules, CARGO_UNUSED_DEPENDENCY, CARGO_VULNERABLE_DEPENDENCY,
    PolicyPlan, Producer, RuleDefinition,
};
use crate::source_kernel::references::{self, CrateReferences, Mention};
use crate::source_kernel::Enumeration;
use crate::source_text::{SourceSpan, byte_range_span, line_starts};

const TRUTH_RULES: [&RuleDefinition; 2] =
    [&CARGO_UNUSED_DEPENDENCY, &CARGO_TEST_ONLY_DEPENDENCY];

pub(crate) const ADVISORY_RULES: [&RuleDefinition; 2] =
    [&CARGO_VULNERABLE_DEPENDENCY, &CARGO_UNMAINTAINED_DEPENDENCY];

const PROFILE_RULES: [&RuleDefinition; 2] = [
    &CARGO_UNCHECKED_RELEASE_OVERFLOW,
    &CARGO_RELEASE_DEBUG_SYMBOLS,
];

const MAX_MANIFEST_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Candidate {
    pub(crate) definition: &'static RuleDefinition,
    pub(crate) message: String,
    pub(crate) package: Option<String>,
    pub(crate) manifest_path: Option<String>,
    pub(crate) span: Option<SourceSpan>,
    pub(crate) key: Option<String>,
    pub(crate) informational: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CargoHealthError {
    pub(crate) code: &'static str,
    pub(crate) message: &'static str,
}

#[derive(Debug, Default)]
pub(crate) struct CargoHealthScan {
    pub(crate) candidates: Vec<Candidate>,
    pub(crate) errors: Vec<CargoHealthError>,
    pub(crate) advisory: AdvisoryOutcome,
    #[allow(dead_code, reason = "read only by the tests that assert the pass did the work")]
    pub(crate) counters: CargoHealthCounters,
    pub(crate) allows: BTreeSet<(String, String)>,
}

#[derive(Debug, Default)]
pub(crate) struct CargoHealthCounters {
    pub(crate) dependencies_evaluated: usize,
    pub(crate) unbounded_registry_predicates: usize,
    pub(crate) unpinned_git_predicates: usize,
    pub(crate) resolved_packages: usize,
}




pub(crate) fn inspect(
    metadata: &Metadata,
    plan: &PolicyPlan,
    database: Option<&AdvisoryDatabase>,
) -> CargoHealthScan {
    let active = ActiveRules::of(plan, Producer::CargoHealth);
    let mut scan = CargoHealthScan::default();
    let reported = active.on(&CARGO_PERMISSIVE_LINT_TABLE);
    if reported || plan.active_rules(Producer::Clippy).next().is_some() {
        inspect_lint_tables(metadata, plan, reported, &mut scan);
    }
    if !active.any() {
        return scan;
    }
    if active.any_of(&PROFILE_RULES) {
        inspect_release_profile(metadata, &active, &mut scan);
    }
    if active.on(&CARGO_PERMISSIVE_RUSTFLAGS) {
        inspect_rustflags(metadata, &mut scan);
    }
    inspect_declarations(metadata, &active, &mut scan);
    inspect_resolution(metadata, &active, database, &mut scan);
    scan
}

fn inspect_declarations(metadata: &Metadata, active: &ActiveRules, scan: &mut CargoHealthScan) {
    let workspace_root = metadata.workspace_root.as_std_path();
    for package in workspace_packages(metadata) {
        let manifest_path = package
            .manifest_path
            .strip_prefix(&metadata.workspace_root)
            .ok()
            .map(|path| path.as_str().to_owned());
        let declared = DeclaredKeys::read(package);
        for dependency in &package.dependencies {
            scan.counters.dependencies_evaluated += 1;
            let key = dependency.rename.as_deref().unwrap_or(&dependency.name);
            let span = declared.as_ref().and_then(|keys| keys.span(dependency));
            let mut name = |definition: &'static RuleDefinition, key: &str, message: String| {
                scan.candidates.push(Candidate {
                    definition,
                    message,
                    package: Some(package.name.to_string()),
                    manifest_path: manifest_path.clone(),
                    span,
                    key: Some(key.to_owned()),
                    informational: false,
                });
            };
            if active.on(&CARGO_UNBOUNDED_REGISTRY) {
                scan.counters.unbounded_registry_predicates += 1;
                if is_unbounded_registry(dependency) {
                    name(
                        &CARGO_UNBOUNDED_REGISTRY,
                        key,
                        format!(
                            "Registry dependency \"{key}\" uses an unbounded \"*\" version requirement."
                        ),
                    );
                }
            }
            if active.on(&CARGO_UNPINNED_GIT) {
                scan.counters.unpinned_git_predicates += 1;
                if is_unpinned_git(dependency) {
                    name(
                        &CARGO_UNPINNED_GIT,
                        key,
                        format!(
                            "Git dependency \"{key}\" is not pinned to a full commit revision."
                        ),
                    );
                }
            }
            if active.on(&CARGO_PATH_DEPENDENCY_OUTSIDE_WORKSPACE)
                && leaves_workspace(dependency, workspace_root)
            {
                name(
                    &CARGO_PATH_DEPENDENCY_OUTSIDE_WORKSPACE,
                    key,
                    format!("Path dependency \"{key}\" resolves outside the workspace."),
                );
            }
        }
    }
}

pub(crate) fn dependency_truth_required(plan: &PolicyPlan) -> bool {
    ActiveRules::of(plan, Producer::CargoHealth).any_of(&TRUTH_RULES)
}

pub(crate) fn inspect_dependency_truth(
    metadata: &Metadata,
    enumeration: &Enumeration,
    collected: &CrateReferences,
    plan: &PolicyPlan,
    scan: &mut CargoHealthScan,
) {
    let active = ActiveRules::of(plan, Producer::CargoHealth);
    if !active.any_of(&TRUTH_RULES) {
        return;
    }

    for package in workspace_packages(metadata) {
        if collected.incomplete(&package.id.repr) {
            continue;
        }
        let judged = Judged::of(package, metadata);
        let mut declared = None;
        for dependency in &package.dependencies {
            if let Some((definition, message)) =
                judge_dependency(dependency, &judged, enumeration, collected, &active)
            {
                let keys = declared.get_or_insert_with(|| DeclaredKeys::read(package));
                scan.candidates.push(Candidate {
                    definition,
                    message,
                    package: Some(package.name.to_string()),
                    manifest_path: judged.manifest_path.clone(),
                    span: keys.as_ref().and_then(|keys| keys.span(dependency)),
                    key: Some(
                        dependency
                            .rename
                            .as_deref()
                            .unwrap_or(&dependency.name)
                            .to_owned(),
                    ),
                    informational: false,
                });
            }
        }
    }
}

struct Judged<'a> {
    id: &'a str,
    manifest_path: Option<String>,
    development: BTreeSet<&'a str>,
}

impl<'a> Judged<'a> {
    fn of(package: &'a Package, metadata: &Metadata) -> Self {
        Self {
            id: &package.id.repr,
            manifest_path: package
                .manifest_path
                .strip_prefix(&metadata.workspace_root)
                .ok()
                .map(|path| path.as_str().to_owned()),
            development: package
                .dependencies
                .iter()
                .filter(|dependency| dependency.kind == DependencyKind::Development)
                .map(|dependency| dependency.name.as_str())
                .collect(),
        }
    }
}

fn judge_dependency(
    dependency: &Dependency,
    judged: &Judged<'_>,
    enumeration: &Enumeration,
    collected: &CrateReferences,
    active: &ActiveRules,
) -> Option<(&'static RuleDefinition, String)> {
    if dependency.kind != DependencyKind::Normal
        || dependency.optional
        || dependency.target.is_some()
    {
        return None;
    }
    let key = dependency.rename.as_deref().unwrap_or(&dependency.name);
    let sites = collected.sites(judged.id, &key.replace('-', "_"));

    if !sites.anywhere() {
        let unused = active.on(&CARGO_UNUSED_DEPENDENCY)
            && !references::mentioned(enumeration, judged.id, key, Mention::Anywhere);
        return unused.then(|| {
            (
                &CARGO_UNUSED_DEPENDENCY,
                format!(
                    "Declared dependency \"{key}\" is referenced by no source of its package."
                ),
            )
        });
    }

    if !active.on(&CARGO_TEST_ONLY_DEPENDENCY)
        || sites.production
        || judged.development.contains(dependency.name.as_str())
        || references::mentioned(enumeration, judged.id, key, Mention::ProductionOnly)
    {
        return None;
    }
    let message = if sites.test_target {
        format!("Dependency \"{key}\" is referenced only from test, bench or example code.")
    } else {
        format!("Dependency \"{key}\" is referenced only from an inline #[cfg(test)] module.")
    };
    Some((&CARGO_TEST_ONLY_DEPENDENCY, message))
}

#[derive(Debug, Default, Deserialize)]
struct ManifestLintDocument {
    lints: Option<LintsSection>,
    workspace: Option<WorkspaceLintSection>,
}

#[derive(Debug, Deserialize)]
struct WorkspaceLintSection {
    lints: Option<LintsSection>,
}

#[derive(Debug, Default, Deserialize)]
struct LintsSection {
    workspace: Option<bool>,
    clippy: Option<BTreeMap<toml::Spanned<String>, LintSetting>>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum LintSetting {
    Level(String),
    Detailed { level: Option<String> },
}

impl LintSetting {
    fn level(&self) -> Option<&str> {
        match self {
            Self::Level(level) => Some(level),
            Self::Detailed { level } => level.as_deref(),
        }
    }
}

fn inspect_lint_tables(
    metadata: &Metadata,
    plan: &PolicyPlan,
    reported: bool,
    scan: &mut CargoHealthScan,
) {
    let mut inheriting = Vec::new();
    for package in workspace_packages(metadata) {
        let document = match read_manifest::<ManifestLintDocument>(
            package.manifest_path.as_std_path(),
            LINT_TABLE_UNPARSEABLE,
        ) {
            Ok(document) => document,
            Err(error) => {
                if reported {
                    scan.errors.push(error);
                }
                continue;
            }
        };
        let Some((source, manifest)) = document else {
            continue;
        };
        let Some(lints) = manifest.lints else {
            continue;
        };
        if lints.workspace == Some(true) {
            inheriting.push(package.name.to_string());
            continue;
        }
        let manifest_path = package
            .manifest_path
            .strip_prefix(&metadata.workspace_root)
            .ok()
            .map(|path| path.as_str().to_owned());
        let finding = reported.then(|| Finding {
            package: Some(package.name.as_ref()),
            manifest_path,
        });
        let members = [package.name.to_string()];
        permissive_entries(&lints, &source, plan, &members, finding, scan);
    }

    if inheriting.is_empty() {
        return;
    }
    let root_manifest = metadata.workspace_root.as_std_path().join("Cargo.toml");
    match read_manifest::<ManifestLintDocument>(&root_manifest, LINT_TABLE_UNPARSEABLE) {
        Err(error) => {
            if reported {
                scan.errors.push(error);
            }
        }
        Ok(None) => {}
        Ok(Some((source, manifest))) => {
            let Some(lints) = manifest.workspace.and_then(|workspace| workspace.lints) else {
                return;
            };
            let owner = resolution_owner(metadata);
            let finding = reported.then(|| Finding {
                package: owner.as_deref(),
                manifest_path: Some("Cargo.toml".to_owned()),
            });
            permissive_entries(&lints, &source, plan, &inheriting, finding, scan);
        }
    }
}

struct Finding<'a> {
    package: Option<&'a str>,
    manifest_path: Option<String>,
}

fn permissive_entries(
    lints: &LintsSection,
    source: &str,
    plan: &PolicyPlan,
    members: &[String],
    finding: Option<Finding<'_>>,
    scan: &mut CargoHealthScan,
) {
    let Some(clippy) = lints.clippy.as_ref() else {
        return;
    };
    let starts = line_starts(source);
    for (key, setting) in clippy {
        if setting.level() != Some("allow") {
            continue;
        }
        let rule = format!("clippy::{}", key.get_ref());
        if !plan.is_active(&rule) {
            continue;
        }
        if let Some(finding) = &finding {
            scan.candidates.push(Candidate {
                definition: &CARGO_PERMISSIVE_LINT_TABLE,
                message: format!(
                    "Manifest lint table sets catalogued rule \"{rule}\" to \"allow\"."
                ),
                package: finding.package.map(str::to_owned),
                manifest_path: finding.manifest_path.clone(),
                span: Some(byte_range_span(key.span(), &starts, source)),
                key: None,
                informational: false,
            });
        }
        scan.allows.extend(
            members
                .iter()
                .map(|member| (member.clone(), rule.clone())),
        );
    }
}

fn read_manifest<D: serde::de::DeserializeOwned>(
    path: &Path,
    table: &'static str,
) -> Result<Option<(String, D)>, CargoHealthError> {
    let Some(contents) = read_bounded_toml("manifest-unreadable", path)? else {
        return Ok(None);
    };
    let Ok(document) = toml::from_str::<D>(&contents) else {
        return Err(CargoHealthError {
            code: "manifest-invalid",
            message: table,
        });
    };
    Ok(Some((contents, document)))
}

const LINT_TABLE_UNPARSEABLE: &str = "a manifest lint table could not be parsed";
const PROFILE_TABLE_UNPARSEABLE: &str = "a manifest profile table could not be parsed";

fn read_bounded_toml(code: &'static str, path: &Path) -> Result<Option<String>, CargoHealthError> {
    let Ok(file) = fs::metadata(path) else {
        return Ok(None);
    };
    if !file.is_file() || file.len() > MAX_MANIFEST_BYTES {
        return Err(CargoHealthError {
            code,
            message: "a file is not a readable regular file within the published size limit",
        });
    }
    match fs::read_to_string(path) {
        Ok(contents) => Ok(Some(contents)),
        Err(_) => Err(CargoHealthError {
            code,
            message: "a file could not be read as UTF-8 text",
        }),
    }
}

#[derive(Debug, Default, Deserialize)]
struct ManifestProfileDocument {
    profile: Option<BTreeMap<toml::Spanned<String>, ProfileSettings>>,
}

#[derive(Debug, Default, Deserialize)]
struct ProfileSettings {
    #[serde(rename = "overflow-checks")]
    overflow_checks: Option<toml::Spanned<TomlScalar>>,
    debug: Option<toml::Spanned<TomlScalar>>,
    strip: Option<toml::Spanned<TomlScalar>>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum TomlScalar {
    Truth(bool),
    Level(i64),
    Name(String),
}

fn inspect_release_profile(
    metadata: &Metadata,
    active: &ActiveRules,
    scan: &mut CargoHealthScan,
) {
    let root_manifest = metadata.workspace_root.as_std_path().join("Cargo.toml");
    let (source, document) = match read_manifest::<ManifestProfileDocument>(
        &root_manifest,
        PROFILE_TABLE_UNPARSEABLE,
    ) {
        Ok(Some(read)) => read,
        Ok(None) => return,
        Err(error) => {
            scan.errors.push(error);
            return;
        }
    };
    let starts = line_starts(&source);
    let release = document
        .profile
        .as_ref()
        .and_then(|profiles| profiles.iter().find(|(key, _)| key.get_ref() == "release"));
    let owner = resolution_owner(metadata);

    if active.on(&CARGO_UNCHECKED_RELEASE_OVERFLOW)
        && workspace_packages(metadata).any(produces_a_binary)
    {
        let enabled = release.is_some_and(|(_, settings)| {
            settings
                .overflow_checks
                .as_ref()
                .is_some_and(|checks| matches!(checks.get_ref(), TomlScalar::Truth(true)))
        });
        if !enabled {
            let span = release.map(|(key, settings)| {
                settings
                    .overflow_checks
                    .as_ref()
                    .map_or_else(|| key.span(), toml::Spanned::span)
            });
            scan.candidates.push(Candidate {
                definition: &CARGO_UNCHECKED_RELEASE_OVERFLOW,
                message: "Release profile compiles without overflow checks, so integer overflow wraps silently in the shipped binary.".to_owned(),
                package: owner.clone(),
                manifest_path: Some("Cargo.toml".to_owned()),
                span: span.map(|range| byte_range_span(range, &starts, &source)),
                key: None,
                informational: false,
            });
        }
    }

    if active.on(&CARGO_RELEASE_DEBUG_SYMBOLS)
        && let Some((_, settings)) = release
    {
        let full_debug = settings.debug.as_ref().is_some_and(|debug| {
            match debug.get_ref() {
                TomlScalar::Truth(value) => *value,
                TomlScalar::Level(value) => *value == 2,
                TomlScalar::Name(value) => value == "full",
            }
        });
        let stripped = settings.strip.as_ref().is_some_and(|strip| match strip.get_ref() {
            TomlScalar::Truth(value) => *value,
            TomlScalar::Level(_) => false,
            TomlScalar::Name(value) => value == "symbols" || value == "debuginfo",
        });
        if full_debug
            && !stripped
            && let Some(debug) = settings.debug.as_ref()
        {
            scan.candidates.push(Candidate {
                definition: &CARGO_RELEASE_DEBUG_SYMBOLS,
                message: "Release profile ships full debug info unstripped, so absolute build paths travel inside the binary.".to_owned(),
                package: owner,
                manifest_path: Some("Cargo.toml".to_owned()),
                span: Some(byte_range_span(debug.span(), &starts, &source)),
                key: None,
                informational: false,
            });
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct CargoConfigDocument {
    build: Option<RustflagsCarrier>,
    target: Option<BTreeMap<String, RustflagsCarrier>>,
}

#[derive(Debug, Default, Deserialize)]
struct RustflagsCarrier {
    rustflags: Option<toml::Spanned<toml::Value>>,
}

fn rustflag_arguments(setting: &toml::Spanned<toml::Value>) -> Vec<(&str, std::ops::Range<usize>)> {
    let span = setting.span();
    match setting.get_ref() {
        toml::Value::Array(entries) => entries
            .iter()
            .filter_map(toml::Value::as_str)
            .map(|argument| (argument, span.clone()))
            .collect(),
        toml::Value::String(joined) => joined
            .split_whitespace()
            .map(|argument| (argument, span.clone()))
            .collect(),
        _ => Vec::new(),
    }
}

fn inspect_rustflags(metadata: &Metadata, scan: &mut CargoHealthScan) {
    let path = metadata
        .workspace_root
        .as_std_path()
        .join(".cargo")
        .join("config.toml");
    let contents = match read_bounded_toml("cargo-config-unreadable", &path) {
        Ok(Some(contents)) => contents,
        Ok(None) => return,
        Err(error) => {
            scan.errors.push(error);
            return;
        }
    };
    let Ok(document) = toml::from_str::<CargoConfigDocument>(&contents) else {
        scan.errors.push(CargoHealthError {
            code: "cargo-config-invalid",
            message: "a cargo configuration file could not be parsed",
        });
        return;
    };

    let starts = line_starts(&contents);
    let owner = resolution_owner(metadata);
    let carriers = document
        .build
        .iter()
        .chain(document.target.iter().flat_map(BTreeMap::values));
    for setting in carriers.filter_map(|carrier| carrier.rustflags.as_ref()) {
        for (flag, span) in neutralizing_rustflags(&rustflag_arguments(setting)) {
            scan.candidates.push(Candidate {
                definition: &CARGO_PERMISSIVE_RUSTFLAGS,
                message: format!(
                    "Workspace rustflags carry \"{flag}\", which switches a check off for every build."
                ),
                package: owner.clone(),
                manifest_path: Some(".cargo/config.toml".to_owned()),
                span: Some(byte_range_span(span, &starts, &contents)),
                key: None,
                informational: false,
            });
        }
    }
}

fn neutralizing_rustflags(
    arguments: &[(&str, std::ops::Range<usize>)],
) -> Vec<(&'static str, std::ops::Range<usize>)> {
    let mut matched = Vec::new();
    let mut index = 0;
    while index < arguments.len() {
        let (argument, span) = &arguments[index];
        let next = arguments
            .get(index + 1)
            .map(|(argument, _)| *argument)
            .unwrap_or_default();
        let (flag, consumed) = match *argument {
            "--cap-lints" if next == "allow" => (Some("--cap-lints allow"), 2),
            "--cap-lints=allow" => (Some("--cap-lints allow"), 1),
            "-A" | "--allow" if next == "warnings" => (Some("-A warnings"), 2),
            "-Awarnings" | "--allow=warnings" => (Some("-A warnings"), 1),
            "-C" | "--codegen" if disables_overflow_checks(next) => {
                (Some("-C overflow-checks=off"), 2)
            }
            _ => {
                let joined = argument
                    .strip_prefix("-C")
                    .or_else(|| argument.strip_prefix("--codegen="));
                match joined {
                    Some(setting) if disables_overflow_checks(setting) => {
                        (Some("-C overflow-checks=off"), 1)
                    }
                    _ => (None, 1),
                }
            }
        };
        if let Some(flag) = flag {
            matched.push((flag, span.clone()));
        }
        index += consumed;
    }
    matched
}

fn disables_overflow_checks(setting: &str) -> bool {
    setting
        .split_once('=')
        .is_some_and(|(key, value)| {
            key == "overflow-checks" && matches!(value, "off" | "false" | "no" | "n" | "0")
        })
}

fn workspace_packages(metadata: &Metadata) -> impl Iterator<Item = &Package> {
    metadata
        .packages
        .iter()
        .filter(|package| metadata.workspace_members.contains(&package.id))
}

pub(crate) fn workspace_produces_a_binary(metadata: &Metadata) -> bool {
    workspace_packages(metadata).any(produces_a_binary)
}

fn produces_a_binary(package: &Package) -> bool {
    package
        .targets
        .iter()
        .any(|target| target.kind.contains(&TargetKind::Bin))
}


fn leaves_workspace(dependency: &Dependency, workspace_root: &Path) -> bool {
    dependency
        .path
        .as_ref()
        .is_some_and(|path| !path.as_std_path().starts_with(workspace_root))
}




fn is_unbounded_registry(dependency: &Dependency) -> bool {
    dependency.path.is_none()
        && dependency
            .source
            .as_ref()
            .is_some_and(|source| source.repr.starts_with("registry+"))
        && dependency.req == cargo_metadata::semver::VersionReq::STAR
}

fn is_unpinned_git(dependency: &Dependency) -> bool {
    dependency.path.is_none()
        && dependency.source.as_ref().is_some_and(|source| {
            source.repr.starts_with("git+") && !has_full_git_revision(&source.repr)
        })
}

fn has_full_git_revision(source: &str) -> bool {
    let Some((_, query_and_fragment)) = source.split_once('?') else {
        return false;
    };
    let query = query_and_fragment.split('#').next().unwrap_or_default();
    let mut parameters = query.split('&');
    let Some(parameter) = parameters.next() else {
        return false;
    };
    if parameters.next().is_some() {
        return false;
    }
    let Some(("rev", revision)) = parameter.split_once('=') else {
        return false;
    };

    revision.len() == 40 && revision.bytes().all(|byte| byte.is_ascii_hexdigit())
}

mod advisories;
mod keys;
mod resolution;
pub use advisories::{AdvisoryDatabase, AdvisoryDbReport, AdvisorySource};
pub(crate) use advisories::{AdvisoryOutcome, STALE_AFTER_DAYS};
use keys::DeclaredKeys;
pub(crate) use resolution::locked_direct_versions;
use resolution::{inspect_resolution, resolution_owner};

#[cfg(test)]
mod tests;
