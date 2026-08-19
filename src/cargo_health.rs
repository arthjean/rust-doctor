use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use cargo_metadata::{Dependency, DependencyKind, Metadata, Package, TargetKind};
use serde::Deserialize;

use crate::policy::{
    CARGO_DUPLICATE_MAJOR_VERSIONS, CARGO_MISSING_LOCKFILE,
    CARGO_PATH_DEPENDENCY_OUTSIDE_WORKSPACE, CARGO_PERMISSIVE_LINT_TABLE,
    CARGO_PERMISSIVE_RUSTFLAGS, CARGO_RELEASE_DEBUG_SYMBOLS, CARGO_TEST_ONLY_DEPENDENCY,
    CARGO_UNBOUNDED_REGISTRY, CARGO_UNCHECKED_RELEASE_OVERFLOW, CARGO_UNPINNED_GIT,
    ActiveRules, CARGO_UNUSED_DEPENDENCY, PolicyPlan, Producer, RuleDefinition,
};
use crate::source_kernel::references::{self, CrateReferences, Mention};
use crate::source_kernel::Enumeration;
use crate::source_text::{SourceSpan, byte_range_span, line_starts};

/// The two dependency-truth rules, named once: the entry point asks whether
/// either is on, and `dependency_truth_required` asks the same question before
/// the source walk that only they need is started.
const TRUTH_RULES: [&RuleDefinition; 2] =
    [&CARGO_UNUSED_DEPENDENCY, &CARGO_TEST_ONLY_DEPENDENCY];

/// The two rules the release profile carries, named once: the entry point asks
/// whether either is on before reading the root manifest, and
/// `inspect_release_profile` asks per rule from the same set.
const PROFILE_RULES: [&RuleDefinition; 2] = [
    &CARGO_UNCHECKED_RELEASE_OVERFLOW,
    &CARGO_RELEASE_DEBUG_SYMBOLS,
];



/// Same bound for a manifest the lint-table reader re-parses from disk.
const MAX_MANIFEST_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Candidate {
    pub(crate) definition: &'static RuleDefinition,
    pub(crate) message: String,
    pub(crate) package: String,
    pub(crate) manifest_path: Option<String>,
    /// Position of the offending manifest key, when the pack re-read the
    /// manifest itself; the metadata-derived predicates carry none.
    pub(crate) span: Option<SourceSpan>,
}

/// Bounded error of the pack: a closed code and a frozen message, with no
/// path, no environment variable and no escape sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CargoHealthError {
    pub(crate) code: &'static str,
    pub(crate) message: &'static str,
}

#[derive(Debug, Default)]
pub(crate) struct CargoHealthScan {
    pub(crate) candidates: Vec<Candidate>,
    pub(crate) errors: Vec<CargoHealthError>,
    #[allow(dead_code, reason = "read only by the tests that assert the pass did the work")]
    pub(crate) counters: CargoHealthCounters,
}

#[derive(Debug, Default)]
pub(crate) struct CargoHealthCounters {
    pub(crate) dependencies_evaluated: usize,
    pub(crate) unbounded_registry_predicates: usize,
    pub(crate) unpinned_git_predicates: usize,
    pub(crate) resolved_packages: usize,
}




pub(crate) fn inspect(metadata: &Metadata, plan: &PolicyPlan) -> CargoHealthScan {
    let active = ActiveRules::of(plan, Producer::CargoHealth);
    if !active.any() {
        return CargoHealthScan::default();
    }

    let mut scan = CargoHealthScan::default();
    if active.on(&CARGO_PERMISSIVE_LINT_TABLE) {
        inspect_lint_tables(metadata, plan, &mut scan);
    }
    if active.any_of(&PROFILE_RULES) {
        inspect_release_profile(metadata, &active, &mut scan);
    }
    if active.on(&CARGO_PERMISSIVE_RUSTFLAGS) {
        inspect_rustflags(metadata, &mut scan);
    }
    inspect_declarations(metadata, &active, &mut scan);
    inspect_resolution(metadata, &active, &mut scan);
    scan
}

/// Every `[dependencies]` entry of every workspace member, judged on what the
/// manifest declares rather than on what the graph resolved to.
fn inspect_declarations(metadata: &Metadata, active: &ActiveRules, scan: &mut CargoHealthScan) {
    let workspace_root = metadata.workspace_root.as_std_path();
    for package in workspace_packages(metadata) {
        let manifest_path = package
            .manifest_path
            .strip_prefix(&metadata.workspace_root)
            .ok()
            .map(|path| path.as_str().to_owned());
        let mut name = |definition: &'static RuleDefinition, message: String| {
            scan.candidates.push(Candidate {
                definition,
                message,
                package: package.name.to_string(),
                manifest_path: manifest_path.clone(),
                span: None,
            });
        };

        for dependency in &package.dependencies {
            scan.counters.dependencies_evaluated += 1;
            let key = dependency.rename.as_deref().unwrap_or(&dependency.name);
            if active.on(&CARGO_UNBOUNDED_REGISTRY) {
                scan.counters.unbounded_registry_predicates += 1;
                if is_unbounded_registry(dependency) {
                    name(
                        &CARGO_UNBOUNDED_REGISTRY,
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
                    format!("Path dependency \"{key}\" resolves outside the workspace."),
                );
            }
        }
    }
}


/// Do the dependency-truth rules need the source enumeration? The walk is the
/// expensive part of the scan, so execution asks before starting it for these
/// rules alone.
pub(crate) fn dependency_truth_required(plan: &PolicyPlan) -> bool {
    ActiveRules::of(plan, Producer::CargoHealth).any_of(&TRUTH_RULES)
}

/// Judges every `[dependencies]` entry of every workspace member against the
/// references its sources actually make (US-006 and US-007).
///
/// The candidate set is deliberately narrow: only `kind = Normal` entries are
/// judged, never `[dev-dependencies]` or `[build-dependencies]`, an optional
/// entry sits behind a feature the scan cannot evaluate, and a target-gated
/// entry belongs to a platform the scan may not be on. A package whose
/// reference collection is incomplete is skipped entirely, so a parse failure
/// never converts into a wall of wrong findings. Before either rule reports,
/// the textual fallback scans the package's sources for the crate name, which
/// silences the doctest, macro-token and attribute-argument classes.
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
        // A package whose reference collection is incomplete is skipped
        // entirely, so a parse failure never converts into a wall of wrong
        // findings.
        if collected.incomplete(&package.id.repr) {
            continue;
        }
        let judged = Judged::of(package, metadata);
        for dependency in &package.dependencies {
            if let Some((definition, message)) =
                judge_dependency(dependency, &judged, enumeration, collected, &active)
            {
                scan.candidates.push(Candidate {
                    definition,
                    message,
                    package: package.name.to_string(),
                    manifest_path: judged.manifest_path.clone(),
                    span: None,
                });
            }
        }
    }
}

/// What the two rules need to know about the package a dependency is declared
/// in, read once per package rather than once per entry.
struct Judged<'a> {
    id: &'a str,
    manifest_path: Option<String>,
    /// The names it also declares under `[dev-dependencies]`. An entry present
    /// in both tables is not test-only: the normal one is what production
    /// resolves against.
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

/// The finding one `[dependencies]` entry earns, if either rule reaches it.
///
/// The candidate set is deliberately narrow: only `kind = Normal` entries are
/// judged, an optional entry sits behind a feature the scan cannot evaluate,
/// and a target-gated entry belongs to a platform the scan may not be on.
/// Before either rule reports, the textual fallback scans the package's sources
/// for the crate name, which silences the doctest, macro-token and
/// attribute-argument classes.
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

    // A dependency referenced nowhere belongs to the unused rule above, so one
    // manifest entry never produces two findings.
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

/// Shape of the manifest the lint-table reader needs: the `[lints]` table of a
/// member, and the `[workspace.lints]` table a member can inherit. Everything
/// else in the document is ignored without making it invalid.
#[derive(Debug, Default, Deserialize)]
struct ManifestLintDocument {
    lints: Option<LintsSection>,
    workspace: Option<WorkspaceLintSection>,
}

#[derive(Debug, Deserialize)]
struct WorkspaceLintSection {
    lints: Option<LintsSection>,
}

/// A `[lints]` table is either an inheritance marker or per-tool tables. Only
/// the `clippy` tool can name a catalogued rule, so the others are not read.
#[derive(Debug, Default, Deserialize)]
struct LintsSection {
    workspace: Option<bool>,
    clippy: Option<BTreeMap<toml::Spanned<String>, LintSetting>>,
}

/// The two spellings Cargo accepts for a lint level.
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

/// Reports every `[lints.clippy]` entry that sets a catalogued, active rule to
/// `allow`. The tool only judges what it would otherwise have said: a lint the
/// catalog does not carry is not this rule's subject, and neither is a `deny`
/// or a `warn`, which silence nothing.
///
/// A member declaring `[lints] workspace = true` points at the workspace
/// root's `[workspace.lints]`, which is judged once and attributed to the root
/// manifest rather than once per inheriting member.
fn inspect_lint_tables(metadata: &Metadata, plan: &PolicyPlan, scan: &mut CargoHealthScan) {
    let mut judge_root = false;
    for package in workspace_packages(metadata) {
        let document = match read_manifest::<ManifestLintDocument>(
            package.manifest_path.as_std_path(),
            LINT_TABLE_UNPARSEABLE,
        ) {
            Ok(document) => document,
            Err(error) => {
                scan.errors.push(error);
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
            judge_root = true;
            continue;
        }
        let manifest_path = package
            .manifest_path
            .strip_prefix(&metadata.workspace_root)
            .ok()
            .map(|path| path.as_str().to_owned());
        permissive_entries(
            &lints,
            &source,
            plan,
            package.name.as_ref(),
            manifest_path,
            scan,
        );
    }

    if !judge_root {
        return;
    }
    let root_manifest = metadata.workspace_root.as_std_path().join("Cargo.toml");
    match read_manifest::<ManifestLintDocument>(&root_manifest, LINT_TABLE_UNPARSEABLE) {
        Err(error) => scan.errors.push(error),
        Ok(None) => {}
        Ok(Some((source, manifest))) => {
            let Some(lints) = manifest.workspace.and_then(|workspace| workspace.lints) else {
                return;
            };
            permissive_entries(
                &lints,
                &source,
                plan,
                &resolution_owner(metadata),
                Some("Cargo.toml".to_owned()),
                scan,
            );
        }
    }
}

fn permissive_entries(
    lints: &LintsSection,
    source: &str,
    plan: &PolicyPlan,
    package: &str,
    manifest_path: Option<String>,
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
        // Catalogued and active in one check: the plan only knows catalogued
        // identifiers, and an off rule was switched off by the user, not by
        // the manifest.
        if !plan.is_active(&rule) {
            continue;
        }
        scan.candidates.push(Candidate {
            definition: &CARGO_PERMISSIVE_LINT_TABLE,
            message: format!("Manifest lint table sets catalogued rule \"{rule}\" to \"allow\"."),
            package: package.to_owned(),
            manifest_path: manifest_path.clone(),
            span: Some(byte_range_span(key.span(), &starts, source)),
        });
    }
}

/// Reads one shape out of a manifest, bounded exactly like the lockfile: an
/// oversized, unreadable or unparseable file is a closed error, never a partial
/// read. The source comes back with the document because every span the rules
/// publish is a byte range into it.
///
/// The two shapes the pack reads used to have a reader each, identical to the
/// character but for the type they deserialized and one word of the error
/// message. `table` is that word.
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

/// Bounded read shared by every TOML file the pack opens itself: an absent
/// file is an observed fact, an oversized or unreadable one is a closed error,
/// never a partial read. The code distinguishes the manifest from the cargo
/// configuration so the report says which file failed without naming a path.
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

/// Shape of the manifest the profile rules need (US-008): the `[profile.*]`
/// tables of the workspace root, with the byte span of each judged setting.
/// `cargo metadata` publishes no profile, so the root manifest is re-read from
/// disk under the same bound as the lint reader. A virtual workspace manifest
/// with no `[package]` still parses: profiles are all this shape reads.
///
/// `Spanned` yields byte ranges over the read source; the span of a setting is
/// the span of its value, which sits on the line that carries the key, and
/// `byte_range_span` converts it with the same line index the other readers
/// use.
#[derive(Debug, Default, Deserialize)]
struct ManifestProfileDocument {
    profile: Option<BTreeMap<toml::Spanned<String>, ProfileSettings>>,
}

/// The three settings the release-profile rules judge. Every other profile key
/// is ignored without making the document invalid.
#[derive(Debug, Default, Deserialize)]
struct ProfileSettings {
    #[serde(rename = "overflow-checks")]
    overflow_checks: Option<toml::Spanned<TomlScalar>>,
    debug: Option<toml::Spanned<TomlScalar>>,
    strip: Option<toml::Spanned<TomlScalar>>,
}

/// The scalar spellings Cargo accepts for profile settings: `debug` is a
/// boolean, an integer level or a named level, `strip` a boolean or a named
/// mode.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum TomlScalar {
    Truth(bool),
    Level(i64),
    Name(String),
}

/// Judges `[profile.release]` of the workspace root (US-009). Only that table
/// is read: Cargo honors profiles from the root manifest alone, and a profile
/// inheriting from release through `inherits` is not resolved, which the rule
/// help states.
///
/// `unchecked_release_overflow` fires when the workspace produces a binary and
/// the release profile does not set `overflow-checks = true`, including when
/// the manifest carries no `[profile.release]` at all, because Cargo's release
/// default is `false`. `release_debug_symbols` fires only on an explicit
/// `debug = true`, `2` or `"full"` left unstripped, because Cargo's release
/// default is no debug info.
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
            // The span points at the explicit `overflow-checks` value when one
            // is written, at the `[profile.release]` header when the section
            // exists without it, and nowhere when the default is what fires.
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
            });
        }
    }
}

/// Shape of `.cargo/config.toml` the rustflags rule reads: the `[build]` table
/// and every `[target.*]` table, because both apply their flags to every build
/// they cover. Everything else in the document is ignored.
#[derive(Debug, Default, Deserialize)]
struct CargoConfigDocument {
    build: Option<RustflagsCarrier>,
    target: Option<BTreeMap<String, RustflagsCarrier>>,
}

/// The setting is kept as a spanned raw value because cargo accepts two
/// spellings, a list of arguments or one space-separated string, and an
/// untagged enum would lose the span: serde buffers untagged content through a
/// deserializer that cannot carry it. Every argument shares the span of the
/// setting that declares it, which is the line the finding points at.
#[derive(Debug, Default, Deserialize)]
struct RustflagsCarrier {
    rustflags: Option<toml::Spanned<toml::Value>>,
}

/// The flags of one rustflags setting as (argument, byte span) pairs. A shape
/// cargo does not accept, an integer for instance, yields no argument rather
/// than an error: it is cargo's to refuse, not this rule's.
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

/// Reports every workspace-wide rustflag drawn from the closed list of
/// checks-disabling flags (US-010): `--cap-lints allow`, `-A warnings` and
/// `-C overflow-checks=off`, in their separated and joined spellings. Flags
/// outside that list are not this rule's subject. Only the workspace root's
/// `.cargo/config.toml` is read; its absence is silence, not an error.
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
            });
        }
    }
}

/// The closed list of neutralizing flags, matched over an argument sequence.
/// Each hit is returned under its canonical separated spelling so the finding
/// names one flag however it was written.
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

/// `overflow-checks` accepts the codegen spellings of false: `off`, `false`,
/// `no`, `n` and `0`.
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

mod resolution;
use resolution::{inspect_resolution, resolution_owner};

#[cfg(test)]
mod tests;
