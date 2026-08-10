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
    CARGO_UNUSED_DEPENDENCY, PolicyPlan, RuleDefinition,
};
use crate::source_kernel::references::{self, CrateReferences, Mention};
use crate::source_kernel::{Enumeration, SourceSpan, byte_range_span, line_starts};

/// Name of the offline resolved graph. `cargo metadata` runs with `--no-deps`,
/// so `metadata.resolve` is always absent: the only graph readable without a
/// registry index and without the network is the lockfile.
const LOCKFILE: &str = "Cargo.lock";

/// Bounds the pack's work on a hostile or gigantic lockfile. Beyond it, the
/// pack abstains instead of loading the file.
const MAX_LOCKFILE_BYTES: u64 = 4 * 1024 * 1024;

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

/// Minimal shape of the lockfile: only the resolved list matters, and all the
/// rest of the document is ignored without making it invalid.
#[derive(Debug, Deserialize)]
struct LockDocument {
    package: Option<Vec<LockPackage>>,
}

#[derive(Debug, Deserialize)]
struct LockPackage {
    name: Option<String>,
    version: Option<String>,
}

/// State of the resolved graph as the pack can observe it offline.
#[derive(Debug, PartialEq, Eq)]
enum Resolution {
    /// No lockfile: this is an observed fact, not an error.
    Absent,
    /// The file exists but its resolution section is unusable.
    Unusable(CargoHealthError),
    /// (name, version) pairs of the resolved graph, in file order.
    Packages(Vec<(String, String)>),
}

pub(crate) fn inspect(metadata: &Metadata, plan: &PolicyPlan) -> CargoHealthScan {
    let unbounded_registry = plan.is_active(CARGO_UNBOUNDED_REGISTRY.id);
    let unpinned_git = plan.is_active(CARGO_UNPINNED_GIT.id);
    let outside_path = plan.is_active(CARGO_PATH_DEPENDENCY_OUTSIDE_WORKSPACE.id);
    let missing_lockfile = plan.is_active(CARGO_MISSING_LOCKFILE.id);
    let duplicate_majors = plan.is_active(CARGO_DUPLICATE_MAJOR_VERSIONS.id);
    let permissive_lints = plan.is_active(CARGO_PERMISSIVE_LINT_TABLE.id);
    let release_profile = plan.is_active(CARGO_UNCHECKED_RELEASE_OVERFLOW.id)
        || plan.is_active(CARGO_RELEASE_DEBUG_SYMBOLS.id);
    let rustflags = plan.is_active(CARGO_PERMISSIVE_RUSTFLAGS.id);
    if !unbounded_registry
        && !unpinned_git
        && !outside_path
        && !missing_lockfile
        && !duplicate_majors
        && !permissive_lints
        && !release_profile
        && !rustflags
    {
        return CargoHealthScan::default();
    }

    let mut scan = CargoHealthScan::default();
    let workspace_root = metadata.workspace_root.as_std_path();

    if permissive_lints {
        inspect_lint_tables(metadata, plan, &mut scan);
    }
    if release_profile {
        inspect_release_profile(metadata, plan, &mut scan);
    }
    if rustflags {
        inspect_rustflags(metadata, &mut scan);
    }

    for package in workspace_packages(metadata) {
        let manifest_path = package
            .manifest_path
            .strip_prefix(&metadata.workspace_root)
            .ok()
            .map(|path| path.as_str().to_owned());

        for dependency in &package.dependencies {
            scan.counters.dependencies_evaluated += 1;
            let key = dependency.rename.as_deref().unwrap_or(&dependency.name);
            if unbounded_registry {
                scan.counters.unbounded_registry_predicates += 1;
            }
            if unbounded_registry && is_unbounded_registry(dependency) {
                scan.candidates.push(Candidate {
                    definition: &CARGO_UNBOUNDED_REGISTRY,
                    message: format!(
                        "Registry dependency \"{key}\" uses an unbounded \"*\" version requirement."
                    ),
                    package: package.name.to_string(),
                    manifest_path: manifest_path.clone(),
                    span: None,
                });
            }
            if unpinned_git {
                scan.counters.unpinned_git_predicates += 1;
            }
            if unpinned_git && is_unpinned_git(dependency) {
                scan.candidates.push(Candidate {
                    definition: &CARGO_UNPINNED_GIT,
                    message: format!(
                        "Git dependency \"{key}\" is not pinned to a full commit revision."
                    ),
                    package: package.name.to_string(),
                    manifest_path: manifest_path.clone(),
                    span: None,
                });
            }
            if outside_path && leaves_workspace(dependency, workspace_root) {
                scan.candidates.push(Candidate {
                    definition: &CARGO_PATH_DEPENDENCY_OUTSIDE_WORKSPACE,
                    message: format!("Path dependency \"{key}\" resolves outside the workspace."),
                    package: package.name.to_string(),
                    manifest_path: manifest_path.clone(),
                    span: None,
                });
            }
        }
    }

    if !missing_lockfile && !duplicate_majors {
        return scan;
    }

    match read_resolution(workspace_root) {
        Resolution::Absent if missing_lockfile => {
            for package in workspace_packages(metadata).filter(|package| produces_a_binary(package))
            {
                scan.candidates.push(Candidate {
                    definition: &CARGO_MISSING_LOCKFILE,
                    // The pack observes the disk, not the version control
                    // index: the message therefore states what is measured,
                    // the missing file, and the rule help carries the fix.
                    message: format!(
                        "Package \"{}\" produces a binary but no {LOCKFILE} sits next to its workspace manifest.",
                        package.name
                    ),
                    package: package.name.to_string(),
                    manifest_path: package
                        .manifest_path
                        .strip_prefix(&metadata.workspace_root)
                        .ok()
                        .map(|path| path.as_str().to_owned()),
                    span: None,
                });
            }
        }
        Resolution::Absent => {}
        Resolution::Unusable(error) => scan.errors.push(error),
        Resolution::Packages(packages) => {
            scan.counters.resolved_packages = packages.len();
            if duplicate_majors {
                let owner = resolution_owner(metadata);
                for (name, versions) in duplicate_major_versions(&packages) {
                    scan.candidates.push(Candidate {
                        definition: &CARGO_DUPLICATE_MAJOR_VERSIONS,
                        message: format!(
                            "Crate \"{name}\" is resolved with incompatible major versions {versions}."
                        ),
                        package: owner.clone(),
                        manifest_path: Some(LOCKFILE.to_owned()),
                        span: None,
                    });
                }
            }
        }
    }

    scan
}

/// Do the dependency-truth rules need the source enumeration? The walk is the
/// expensive part of the scan, so execution asks before starting it for these
/// rules alone.
pub(crate) fn dependency_truth_required(plan: &PolicyPlan) -> bool {
    plan.is_active(CARGO_UNUSED_DEPENDENCY.id) || plan.is_active(CARGO_TEST_ONLY_DEPENDENCY.id)
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
    let unused = plan.is_active(CARGO_UNUSED_DEPENDENCY.id);
    let test_only = plan.is_active(CARGO_TEST_ONLY_DEPENDENCY.id);
    if !unused && !test_only {
        return;
    }

    for package in workspace_packages(metadata) {
        if collected.incomplete(&package.id.repr) {
            continue;
        }
        let manifest_path = package
            .manifest_path
            .strip_prefix(&metadata.workspace_root)
            .ok()
            .map(|path| path.as_str().to_owned());
        let development: BTreeSet<&str> = package
            .dependencies
            .iter()
            .filter(|dependency| dependency.kind == DependencyKind::Development)
            .map(|dependency| dependency.name.as_str())
            .collect();

        for dependency in &package.dependencies {
            if dependency.kind != DependencyKind::Normal
                || dependency.optional
                || dependency.target.is_some()
            {
                continue;
            }
            let key = dependency.rename.as_deref().unwrap_or(&dependency.name);
            let sites = collected.sites(&package.id.repr, &key.replace('-', "_"));

            if !sites.anywhere() {
                if unused
                    && !references::mentioned(
                        enumeration,
                        &package.id.repr,
                        key,
                        Mention::Anywhere,
                    )
                {
                    scan.candidates.push(Candidate {
                        definition: &CARGO_UNUSED_DEPENDENCY,
                        message: format!(
                            "Declared dependency \"{key}\" is referenced by no source of its package."
                        ),
                        package: package.name.to_string(),
                        manifest_path: manifest_path.clone(),
                        span: None,
                    });
                }
                continue;
            }

            // A dependency referenced nowhere belongs to the unused rule
            // above, so one manifest entry never produces two findings.
            if !test_only
                || sites.production
                || development.contains(dependency.name.as_str())
                || references::mentioned(
                    enumeration,
                    &package.id.repr,
                    key,
                    Mention::ProductionOnly,
                )
            {
                continue;
            }
            let message = if sites.test_target {
                format!(
                    "Dependency \"{key}\" is referenced only from test, bench or example code."
                )
            } else {
                format!(
                    "Dependency \"{key}\" is referenced only from an inline #[cfg(test)] module."
                )
            };
            scan.candidates.push(Candidate {
                definition: &CARGO_TEST_ONLY_DEPENDENCY,
                message,
                package: package.name.to_string(),
                manifest_path: manifest_path.clone(),
                span: None,
            });
        }
    }
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
        let document = match read_manifest_lints(package.manifest_path.as_std_path()) {
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
    match read_manifest_lints(&root_manifest) {
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

/// Reads the `[lints]` shape of one manifest, bounded exactly like the
/// lockfile: an oversized, unreadable or unparseable file is a closed error,
/// never a partial read.
fn read_manifest_lints(
    path: &Path,
) -> Result<Option<(String, ManifestLintDocument)>, CargoHealthError> {
    let Some(contents) = read_bounded_toml("manifest-unreadable", path)? else {
        return Ok(None);
    };
    let Ok(document) = toml::from_str::<ManifestLintDocument>(&contents) else {
        return Err(CargoHealthError {
            code: "manifest-invalid",
            message: "a manifest lint table could not be parsed",
        });
    };
    Ok(Some((contents, document)))
}

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

/// Reads the `[profile.*]` shape of the workspace root manifest, bounded and
/// closed exactly like the lint reader.
fn read_manifest_profiles(
    path: &Path,
) -> Result<Option<(String, ManifestProfileDocument)>, CargoHealthError> {
    let Some(contents) = read_bounded_toml("manifest-unreadable", path)? else {
        return Ok(None);
    };
    let Ok(document) = toml::from_str::<ManifestProfileDocument>(&contents) else {
        return Err(CargoHealthError {
            code: "manifest-invalid",
            message: "a manifest profile table could not be parsed",
        });
    };
    Ok(Some((contents, document)))
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
fn inspect_release_profile(metadata: &Metadata, plan: &PolicyPlan, scan: &mut CargoHealthScan) {
    let root_manifest = metadata.workspace_root.as_std_path().join("Cargo.toml");
    let (source, document) = match read_manifest_profiles(&root_manifest) {
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

    if plan.is_active(CARGO_UNCHECKED_RELEASE_OVERFLOW.id)
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

    if plan.is_active(CARGO_RELEASE_DEBUG_SYMBOLS.id)
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

/// The resolved graph belongs to the workspace, not to a member. The diagnostic
/// is therefore attached to the root package when it exists, otherwise to the
/// first member by name order, which stays deterministic on a virtual
/// workspace.
fn resolution_owner(metadata: &Metadata) -> String {
    let root_manifest = metadata.workspace_root.join("Cargo.toml");
    if let Some(root) = workspace_packages(metadata)
        .find(|package| package.manifest_path == root_manifest)
        .or_else(|| workspace_packages(metadata).min_by_key(|package| package.name.to_string()))
    {
        return root.name.to_string();
    }
    metadata
        .workspace_root
        .file_name()
        .unwrap_or_default()
        .to_owned()
}

fn leaves_workspace(dependency: &Dependency, workspace_root: &Path) -> bool {
    dependency
        .path
        .as_ref()
        .is_some_and(|path| !path.as_std_path().starts_with(workspace_root))
}

fn read_resolution(workspace_root: &Path) -> Resolution {
    let path = workspace_root.join(LOCKFILE);
    let Ok(metadata) = fs::metadata(&path) else {
        return Resolution::Absent;
    };
    if !metadata.is_file() || metadata.len() > MAX_LOCKFILE_BYTES {
        return Resolution::Unusable(CargoHealthError {
            code: "lockfile-unreadable",
            message: "the lockfile is not a readable regular file within the published size limit",
        });
    }
    let Ok(contents) = fs::read_to_string(&path) else {
        return Resolution::Unusable(CargoHealthError {
            code: "lockfile-unreadable",
            message: "the lockfile could not be read as UTF-8 text",
        });
    };
    let Ok(document) = toml::from_str::<LockDocument>(&contents) else {
        return Resolution::Unusable(CargoHealthError {
            code: "lockfile-invalid",
            message: "the lockfile is not valid TOML",
        });
    };
    let Some(entries) = document.package else {
        return Resolution::Unusable(CargoHealthError {
            code: "lockfile-resolution-absent",
            message: "the lockfile carries no resolved package section",
        });
    };

    let packages: Vec<_> = entries
        .iter()
        .filter_map(|entry| Some((entry.name.clone()?, entry.version.clone()?)))
        .collect();
    if packages.len() != entries.len() {
        return Resolution::Unusable(CargoHealthError {
            code: "lockfile-resolution-absent",
            message: "the lockfile carries a resolved package without a name or a version",
        });
    }
    Resolution::Packages(packages)
}

/// Two versions of the same crate whose major number differs are not
/// interchangeable: their types do not unify and the binary embeds both copies.
/// Pre-releases and build metadata are ignored, only the major part of the
/// triplet is compared.
fn duplicate_major_versions(packages: &[(String, String)]) -> Vec<(String, String)> {
    let mut majors = BTreeMap::<&str, Vec<&str>>::new();
    for (name, version) in packages {
        let entry = majors.entry(name.as_str()).or_default();
        if !entry.contains(&version.as_str()) {
            entry.push(version.as_str());
        }
    }

    majors
        .into_iter()
        .filter_map(|(name, mut versions)| {
            versions.sort_unstable();
            let distinct: Vec<_> = versions
                .iter()
                .filter_map(|version| major_of(version))
                .collect();
            let mut unique = distinct.clone();
            unique.sort_unstable();
            unique.dedup();
            (unique.len() > 1).then(|| (name.to_owned(), versions.join(", ")))
        })
        .collect()
}

fn major_of(version: &str) -> Option<&str> {
    let major = version
        .split(['+', '-'])
        .next()?
        .split('.')
        .next()
        .filter(|major| !major.is_empty() && major.bytes().all(|byte| byte.is_ascii_digit()))?;
    Some(major)
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use cargo_metadata::semver::VersionReq;
    use serde_json::Value;

    use super::*;
    use crate::policy::{PolicyInput, RuleLevel};

    fn inspect(metadata: &Metadata) -> Vec<Candidate> {
        super::inspect(metadata, &PolicyPlan::default()).candidates
    }

    static NEXT_CARGO_HOME: AtomicUsize = AtomicUsize::new(0);

    fn fixture() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cargo-health/protocol")
    }

    fn protocol_metadata() -> (Metadata, Value) {
        offline_metadata(&fixture(), "protocol")
    }

    fn offline_metadata(fixture: &Path, label: &str) -> (Metadata, Value) {
        let cargo_home = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!(
                "cargo-health-{label}-home-{}-{}",
                std::process::id(),
                NEXT_CARGO_HOME.fetch_add(1, Ordering::Relaxed)
            ));
        if cargo_home.exists() {
            fs::remove_dir_all(&cargo_home).expect("stale protocol Cargo home should be removable");
        }

        let output = Command::new(env!("CARGO"))
            .args([
                "metadata",
                "--format-version",
                "1",
                "--no-deps",
                "--offline",
                "--manifest-path",
                "Cargo.toml",
            ])
            .current_dir(fixture)
            .env("CARGO_HOME", &cargo_home)
            .env("CARGO_NET_OFFLINE", "true")
            .output()
            .expect("Cargo metadata should start");

        if cargo_home.exists() {
            fs::remove_dir_all(&cargo_home).expect("protocol Cargo home should be removable");
        }
        assert!(
            output.status.success(),
            "Cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let json: Value =
            serde_json::from_slice(&output.stdout).expect("metadata should be valid JSON");
        let metadata =
            serde_json::from_value(json.clone()).expect("cargo_metadata should decode the corpus");
        (metadata, json)
    }

    fn fixture_hashes(root: &Path) -> BTreeMap<String, blake3::Hash> {
        fn visit(root: &Path, directory: &Path, hashes: &mut BTreeMap<String, blake3::Hash>) {
            let mut entries: Vec<_> = fs::read_dir(directory)
                .expect("fixture directory should be readable")
                .map(|entry| entry.expect("fixture entry should be readable").path())
                .collect();
            entries.sort();
            for path in entries {
                if path.is_dir() {
                    if path.file_name().is_some_and(|name| name == "target") {
                        continue;
                    }
                    visit(root, &path, hashes);
                } else if path.file_name().is_none_or(|name| name != "Cargo.lock") {
                    let relative = path
                        .strip_prefix(root)
                        .expect("fixture file should be below its root")
                        .to_string_lossy()
                        .into_owned();
                    hashes.insert(
                        relative,
                        blake3::hash(&fs::read(path).expect("fixture file should be readable")),
                    );
                }
            }
        }

        let mut hashes = BTreeMap::new();
        visit(root, root, &mut hashes);
        hashes
    }

    fn dependency<'a>(metadata: &'a Metadata, key: &str) -> &'a Dependency {
        let package = metadata
            .packages
            .first()
            .expect("protocol metadata should contain its member");
        package
            .dependencies
            .iter()
            .find(|dependency| dependency.rename.as_deref().unwrap_or(&dependency.name) == key)
            .expect("protocol dependency should exist")
    }

    #[test]
    fn producer_uses_its_canonical_catalog_entries() {
        let definitions: Vec<_> = PolicyPlan::default()
            .active_rules(crate::policy::Producer::CargoHealth)
            .map(|(definition, _)| definition.id)
            .collect();
        assert_eq!(
            definitions,
            [
                CARGO_DUPLICATE_MAJOR_VERSIONS.id,
                CARGO_MISSING_LOCKFILE.id,
                CARGO_PATH_DEPENDENCY_OUTSIDE_WORKSPACE.id,
                CARGO_PERMISSIVE_LINT_TABLE.id,
                CARGO_PERMISSIVE_RUSTFLAGS.id,
                CARGO_RELEASE_DEBUG_SYMBOLS.id,
                CARGO_TEST_ONLY_DEPENDENCY.id,
                CARGO_UNBOUNDED_REGISTRY.id,
                CARGO_UNCHECKED_RELEASE_OVERFLOW.id,
                CARGO_UNPINNED_GIT.id,
                CARGO_UNUSED_DEPENDENCY.id,
            ]
        );
        assert!(
            definitions
                .iter()
                .filter_map(|id| crate::policy::find(id))
                .filter(|definition| definition.category == "dependencies")
                .count()
                >= 3
        );
    }

    #[test]
    fn semver_oracle_distinguishes_only_the_total_wildcard() {
        let star = VersionReq::parse("*").expect("star should parse");
        assert_eq!(star, VersionReq::STAR);
        assert!(star.comparators.is_empty());

        for requirement in [
            "1.*",
            "1.2.*",
            "1.x",
            "1.X",
            "1",
            "1.2",
            "^1.2.3",
            "=1.2.3",
            ">=1.2,<2",
            "1.2.3-alpha.1",
        ] {
            let requirement =
                VersionReq::parse(requirement).expect("bounded requirement should parse");
            assert_ne!(requirement, VersionReq::STAR);
            assert!(!requirement.comparators.is_empty());
        }
    }

    #[test]
    fn target_protocol_is_offline_complete_and_classifiable() {
        let oracle: Value = serde_json::from_slice(
            &fs::read(fixture().join("oracle.json")).expect("protocol oracle should be readable"),
        )
        .expect("protocol oracle should be valid JSON");
        let cargo_version = Command::new(env!("CARGO"))
            .arg("--version")
            .output()
            .expect("Cargo version should start");
        assert!(cargo_version.status.success());
        assert_eq!(
            String::from_utf8(cargo_version.stdout)
                .expect("Cargo version should be UTF-8")
                .trim(),
            oracle["cargo"]
                .as_str()
                .expect("Cargo oracle should be a string")
        );
        assert_eq!(oracle["cargo_metadata"], "0.23.1");
        assert!(
            include_str!("../Cargo.lock")
                .contains("name = \"cargo_metadata\"\nversion = \"0.23.1\"")
        );

        let (metadata, json) = protocol_metadata();
        assert_eq!(json["version"], oracle["metadata_format"]);
        assert_eq!(metadata.workspace_members.len(), 1);

        let registry = dependency(&metadata, "registry_alias");
        assert_eq!(registry.req, VersionReq::STAR);
        assert!(registry.req.comparators.is_empty());
        assert!(registry.optional);
        assert_eq!(registry.rename.as_deref(), Some("registry_alias"));
        assert_eq!(registry.kind, cargo_metadata::DependencyKind::Normal);
        assert!(
            registry
                .source
                .as_ref()
                .is_some_and(|source| source.repr.starts_with("registry+"))
        );

        let alternative = dependency(&metadata, "registry_alternative");
        assert!(alternative.registry.is_some());
        assert!(
            alternative
                .source
                .as_ref()
                .is_some_and(|source| source.repr.starts_with("registry+"))
        );
        assert!(dependency(&metadata, "path_dependency").path.is_some());
        assert!(
            dependency(&metadata, "development_dependency").kind
                == cargo_metadata::DependencyKind::Development
        );
        assert!(
            dependency(&metadata, "build_dependency").kind == cargo_metadata::DependencyKind::Build
        );
        assert!(dependency(&metadata, "target_dependency").target.is_some());

        for key in [
            "git_default",
            "git_branch",
            "git_tag",
            "git_short",
            "git_non_hex",
        ] {
            let source = &dependency(&metadata, key)
                .source
                .as_ref()
                .expect("Git dependency should expose its source")
                .repr;
            assert!(!has_full_git_revision(source), "{key}");
        }
        let full = &dependency(&metadata, "git_full")
            .source
            .as_ref()
            .expect("full Git dependency should expose its source")
            .repr;
        assert!(has_full_git_revision(full));
        assert!(
            full.contains(
                oracle["full_git_revision"]
                    .as_str()
                    .expect("revision oracle should be a string")
            )
        );
        assert!(!has_full_git_revision(
            "git+https://git.invalid/private.git#0123456789abcdef0123456789abcdef01234567"
        ));
    }

    #[test]
    fn every_declaration_kind_is_evaluated_and_nonmembers_are_excluded() {
        let (mut metadata, _) = protocol_metadata();
        let package = metadata
            .packages
            .first_mut()
            .expect("protocol metadata should contain its member");
        for key in [
            "development_dependency",
            "build_dependency",
            "target_dependency",
        ] {
            dependency_mut(package, key).req = VersionReq::STAR;
        }

        let mut nonmember = package.clone();
        nonmember.id.repr.push_str("-nonmember");
        metadata.packages.push(nonmember);

        let candidates = inspect(&metadata);
        assert_eq!(candidates.len(), 9);
        for key in [
            "registry_alias",
            "development_dependency",
            "build_dependency",
            "target_dependency",
        ] {
            assert!(candidates.iter().any(|candidate| {
                candidate.definition.id == CARGO_UNBOUNDED_REGISTRY.id
                    && candidate.message
                        == format!(
                            "Registry dependency \"{key}\" uses an unbounded \"*\" version requirement."
                        )
            }));
        }
        assert!(candidates.iter().all(|candidate| {
            candidate.package == "cargo-health-protocol"
                && candidate.manifest_path.as_deref() == Some("Cargo.toml")
        }));
    }

    #[test]
    fn precision_matrix_has_exact_positive_and_negative_oracles_without_mutation_or_leaks() {
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cargo-health/precision");
        let before = fixture_hashes(&fixture);
        let oracle: Value = serde_json::from_slice(
            &fs::read(fixture.join("oracle.json")).expect("precision oracle should be readable"),
        )
        .expect("precision oracle should be valid JSON");
        let (metadata, _) = offline_metadata(&fixture, "precision");
        let candidates = inspect(&metadata);
        let positive = oracle["positive"]
            .as_array()
            .expect("positive oracle should be an array");
        let negative_keys = oracle["negative_keys"]
            .as_array()
            .expect("negative oracle should be an array");

        assert_eq!(positive.len(), 10);
        assert_eq!(negative_keys.len(), 12);
        assert_eq!(candidates.len(), positive.len());
        for expected in positive {
            let message = expected["message"]
                .as_str()
                .expect("expected message should be a string");
            let candidate = candidates
                .iter()
                .find(|candidate| candidate.message == message)
                .expect("every positive oracle should produce one candidate");
            let definition = candidate.definition;
            assert_eq!(definition.id, expected["code"]);
            assert_eq!(definition.category, expected["category"]);
            assert_eq!(definition.default_level, RuleLevel::Warn);
            assert_eq!(definition.help, expected["help"]);
            assert_eq!(candidate.package, "cargo-health-precision");
            assert_eq!(
                candidate.manifest_path.as_deref(),
                Some("member/Cargo.toml")
            );
        }

        let member = metadata
            .packages
            .iter()
            .find(|package| package.name == "cargo-health-precision")
            .expect("precision member should be present");
        for key in negative_keys {
            let key = key
                .as_str()
                .expect("negative dependency key should be a string");
            assert!(
                member.dependencies.iter().any(|dependency| dependency
                    .rename
                    .as_deref()
                    .unwrap_or(&dependency.name)
                    == key),
                "negative dependency {key} should be represented in metadata"
            );
            assert!(
                candidates
                    .iter()
                    .all(|candidate| !candidate.message.contains(&format!("\"{key}\""))),
                "negative dependency {key} produced a candidate"
            );
        }

        assert!(
            member
                .dependencies
                .iter()
                .find(|dependency| dependency.rename.as_deref() == Some("registry_optional_alias"))
                .is_some_and(|dependency| dependency.optional)
        );
        assert!(
            member
                .dependencies
                .iter()
                .find(|dependency| dependency.rename.as_deref() == Some("registry_development"))
                .is_some_and(|dependency| {
                    dependency.kind == cargo_metadata::DependencyKind::Development
                })
        );
        assert!(
            member
                .dependencies
                .iter()
                .find(|dependency| dependency.rename.as_deref() == Some("registry_build"))
                .is_some_and(|dependency| {
                    dependency.kind == cargo_metadata::DependencyKind::Build
                })
        );
        assert!(
            member
                .dependencies
                .iter()
                .find(|dependency| dependency.rename.as_deref() == Some("registry_target"))
                .is_some_and(|dependency| dependency.target.is_some())
        );
        assert!(
            metadata
                .packages
                .iter()
                .any(|package| package.name == "cargo-health-empty-member")
        );
        assert!(
            member
                .dependencies
                .iter()
                .find(|dependency| {
                    dependency.rename.as_deref().unwrap_or(&dependency.name) == "path_external"
                })
                .is_some_and(|dependency| dependency.path.is_some())
        );
        assert!(
            candidates
                .iter()
                .all(|candidate| candidate.package != "cargo-health-external-path")
        );

        let observable = candidates
            .iter()
            .map(|candidate| {
                let definition = candidate.definition;
                format!(
                    "{} {} {:?} {} {} {} {}",
                    definition.id,
                    definition.category,
                    definition.default_level,
                    candidate.message,
                    definition.help,
                    candidate.package,
                    candidate.manifest_path.as_deref().unwrap_or_default()
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        for secret in [
            "://",
            "git+",
            "user",
            "secret",
            "?",
            "#",
            "\u{1b}",
            fixture.to_string_lossy().as_ref(),
        ] {
            assert!(!observable.contains(secret), "leaked {secret:?}");
        }
        assert_eq!(fixture_hashes(&fixture), before);
    }

    #[test]
    fn policy_prunes_the_producer_and_each_inactive_predicate() {
        let (metadata, _) = protocol_metadata();
        let all_off = PolicyPlan::default()
            .active_rules(crate::policy::Producer::CargoHealth)
            .map(|(definition, _)| definition.id)
            .fold(PolicyInput::default(), |input, id| {
                input.with_rule(id, RuleLevel::Off)
            });
        let all_off = PolicyPlan::compile(&all_off).expect("policy should compile");
        let scan = super::inspect(&metadata, &all_off);
        assert!(scan.candidates.is_empty());
        assert_eq!(scan.counters.dependencies_evaluated, 0);
        assert_eq!(scan.counters.unbounded_registry_predicates, 0);
        assert_eq!(scan.counters.unpinned_git_predicates, 0);

        let git_off = PolicyInput::default()
            .with_rule(CARGO_UNPINNED_GIT.id, RuleLevel::Off)
            .with_rule(CARGO_DUPLICATE_MAJOR_VERSIONS.id, RuleLevel::Off)
            .with_rule(CARGO_MISSING_LOCKFILE.id, RuleLevel::Off);
        let git_off = PolicyPlan::compile(&git_off).expect("policy should compile");
        let scan = super::inspect(&metadata, &git_off);
        assert!(scan.counters.dependencies_evaluated > 0);
        assert!(scan.counters.unbounded_registry_predicates > 0);
        assert_eq!(scan.counters.unpinned_git_predicates, 0);
        assert!(
            scan.candidates
                .iter()
                .all(|candidate| candidate.definition.id != CARGO_UNPINNED_GIT.id)
        );
    }

    #[test]
    fn empty_dependency_and_unknown_source_inputs_are_safe_and_empty() {
        let (metadata, _) = protocol_metadata();

        let mut no_packages = metadata.clone();
        no_packages.packages.clear();
        assert!(inspect(&no_packages).is_empty());

        let mut no_dependencies = metadata.clone();
        no_dependencies
            .packages
            .first_mut()
            .expect("protocol member should exist")
            .dependencies
            .clear();
        assert!(inspect(&no_dependencies).is_empty());

        let mut unknown_source = metadata;
        let package = unknown_source
            .packages
            .first_mut()
            .expect("protocol member should exist");
        let dependency = package
            .dependencies
            .first_mut()
            .expect("protocol dependency should exist");
        dependency.req = VersionReq::STAR;
        dependency.path = None;
        dependency
            .source
            .as_mut()
            .expect("protocol dependency should expose a source")
            .repr = "future+opaque\u{1b}[31m?credential=secret#fragment".to_owned();
        package.dependencies.truncate(1);
        assert!(inspect(&unknown_source).is_empty());
    }

    #[test]
    fn git_oracle_rejects_duplicate_empty_and_future_queries_without_leaking_sources() {
        for source in [
            "git+https://secret@git.invalid/repo.git?",
            "git+https://secret@git.invalid/repo.git?branch=main",
            "git+https://secret@git.invalid/repo.git?rev=0123456",
            "git+https://secret@git.invalid/repo.git?rev=gggggggggggggggggggggggggggggggggggggggg",
            "git+https://secret@git.invalid/repo.git?rev=0123456789abcdef0123456789abcdef01234567&rev=0123456789abcdef0123456789abcdef01234567",
            "git+https://secret@git.invalid/repo.git?future=0123456789abcdef0123456789abcdef01234567",
        ] {
            assert!(!has_full_git_revision(source));
        }

        let (metadata, _) = protocol_metadata();
        let candidates = inspect(&metadata);
        assert_eq!(candidates.len(), 6);
        assert!(candidates.iter().all(|candidate| {
            let definition = candidate.definition;
            let observable = format!(
                "{} {} {} {} {} {}",
                definition.id,
                definition.category,
                candidate.message,
                definition.help,
                candidate.package,
                candidate.manifest_path.as_deref().unwrap_or_default()
            );
            !observable.contains("git.invalid")
                && !observable.contains("secret")
                && !observable.contains("0123456789abcdef")
        }));
        assert!(candidates.iter().any(|candidate| {
            candidate.definition.id == CARGO_UNBOUNDED_REGISTRY.id
                && candidate.message
                    == "Registry dependency \"registry_alias\" uses an unbounded \"*\" version requirement."
        }));
        assert_eq!(
            candidates
                .iter()
                .filter(|candidate| candidate.definition.id == CARGO_UNPINNED_GIT.id)
                .count(),
            5
        );
    }

    /// US-008: the reader returns the profile values with byte spans that
    /// convert to the same line and column the crate's span helpers publish
    /// elsewhere, and a virtual workspace manifest with no `[package]` still
    /// yields its profiles.
    #[test]
    fn the_profile_reader_returns_values_with_line_accurate_spans() {
        let source = "[workspace]\nmembers = []\n\n[profile.release]\ndebug = 2\nstrip = \"none\"\noverflow-checks = false\n";
        let document: ManifestProfileDocument =
            toml::from_str(source).expect("the virtual manifest should parse");
        let profiles = document.profile.expect("the profiles should be read");
        let (key, settings) = profiles
            .iter()
            .next()
            .expect("the release profile should be read");
        assert_eq!(key.get_ref(), "release");
        let starts = line_starts(source);
        assert_eq!(byte_range_span(key.span(), &starts, source).line_start, 4);

        let debug = settings.debug.as_ref().expect("debug should be read");
        assert!(matches!(debug.get_ref(), TomlScalar::Level(2)));
        assert_eq!(byte_range_span(debug.span(), &starts, source).line_start, 5);

        let strip = settings.strip.as_ref().expect("strip should be read");
        assert!(matches!(strip.get_ref(), TomlScalar::Name(name) if name == "none"));
        assert_eq!(byte_range_span(strip.span(), &starts, source).line_start, 6);

        let checks = settings
            .overflow_checks
            .as_ref()
            .expect("overflow-checks should be read");
        assert!(matches!(checks.get_ref(), TomlScalar::Truth(false)));
        let span = byte_range_span(checks.span(), &starts, source);
        assert_eq!((span.line_start, span.line_end), (7, 7));
    }

    /// US-008: an absent manifest is an observed fact, an oversized one and an
    /// unparseable one are closed errors, never a partial read.
    #[test]
    fn the_profile_reader_bounds_what_it_opens() {
        let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join(format!(
            "cargo-health-profile-reader-{}",
            std::process::id()
        ));
        fs::create_dir_all(&scratch).expect("the scratch directory should be creatable");

        assert!(
            read_manifest_profiles(&scratch.join("absent").join("Cargo.toml"))
                .expect("an absent manifest is not an error")
                .is_none()
        );

        let oversized = scratch.join("oversized.toml");
        fs::write(&oversized, vec![b'#'; MAX_MANIFEST_BYTES as usize + 1])
            .expect("the oversized manifest should be writable");
        let error = read_manifest_profiles(&oversized).expect_err("the bound should refuse");
        assert_eq!(error.code, "manifest-unreadable");

        let invalid = scratch.join("invalid.toml");
        fs::write(&invalid, "[profile.release]\ndebug = 1.5\n")
            .expect("the invalid manifest should be writable");
        let error = read_manifest_profiles(&invalid).expect_err("the shape should refuse");
        assert_eq!(error.code, "manifest-invalid");
        assert!(!error.message.contains('/'));

        fs::remove_dir_all(&scratch).expect("the scratch directory should be removable");
    }

    /// US-010: the closed list matches every published spelling and nothing
    /// else, and each hit is returned under its canonical separated form.
    #[test]
    fn the_closed_rustflags_list_matches_every_spelling_and_nothing_else() {
        let arguments = |flags: &[&'static str]| {
            flags
                .iter()
                .map(|flag| (*flag, 0..flag.len()))
                .collect::<Vec<_>>()
        };
        let matched = |flags: &[&'static str]| {
            neutralizing_rustflags(&arguments(flags))
                .into_iter()
                .map(|(flag, _)| flag)
                .collect::<Vec<_>>()
        };

        assert_eq!(matched(&["--cap-lints", "allow"]), ["--cap-lints allow"]);
        assert_eq!(matched(&["--cap-lints=allow"]), ["--cap-lints allow"]);
        for spelling in [
            ["-A", "warnings"].as_slice(),
            ["-Awarnings"].as_slice(),
            ["--allow", "warnings"].as_slice(),
            ["--allow=warnings"].as_slice(),
        ] {
            assert_eq!(matched(spelling), ["-A warnings"], "{spelling:?}");
        }
        for spelling in [
            ["-C", "overflow-checks=off"].as_slice(),
            ["-Coverflow-checks=false"].as_slice(),
            ["--codegen", "overflow-checks=no"].as_slice(),
            ["--codegen=overflow-checks=0"].as_slice(),
        ] {
            assert_eq!(matched(spelling), ["-C overflow-checks=off"], "{spelling:?}");
        }
        assert_eq!(
            matched(&["-C", "target-cpu=native", "--cap-lints", "allow", "-Awarnings"]),
            ["--cap-lints allow", "-A warnings"]
        );

        for negative in [
            ["-C", "target-cpu=native"].as_slice(),
            ["--cap-lints", "warn"].as_slice(),
            ["--cap-lints"].as_slice(),
            ["-A", "dead_code"].as_slice(),
            ["-D", "warnings"].as_slice(),
            ["-Coverflow-checks=on"].as_slice(),
            ["overflow-checks=off"].as_slice(),
            ["-C"].as_slice(),
        ] {
            assert_eq!(matched(negative), [""; 0], "{negative:?}");
        }
    }

    /// US-009: `debug = "full"` is the named spelling of the level `2`, and a
    /// strip mode that removes debug info silences the finding while
    /// `strip = false` does not.
    #[test]
    fn the_debug_symbols_rule_reads_the_named_level_and_the_strip_modes() {
        let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join(format!(
            "cargo-health-debug-full-{}",
            std::process::id()
        ));
        fs::create_dir_all(&scratch).expect("the scratch directory should be creatable");
        let (mut metadata, _) = protocol_metadata();
        metadata.workspace_root = scratch
            .to_str()
            .expect("the scratch path should be UTF-8")
            .into();

        let observed = |manifest: &str| {
            fs::write(scratch.join("Cargo.toml"), manifest)
                .expect("the scratch manifest should be writable");
            let scan = super::inspect(&metadata, &PolicyPlan::default());
            assert!(scan.errors.is_empty(), "{:?}", scan.errors);
            scan.candidates
                .iter()
                .filter(|candidate| {
                    candidate.definition.id == CARGO_RELEASE_DEBUG_SYMBOLS.id
                })
                .count()
        };

        assert_eq!(observed("[profile.release]\ndebug = \"full\"\nstrip = false\n"), 1);
        assert_eq!(observed("[profile.release]\ndebug = \"full\"\nstrip = \"debuginfo\"\n"), 0);
        assert_eq!(observed("[profile.release]\ndebug = 1\n"), 0);

        fs::remove_dir_all(&scratch).expect("the scratch directory should be removable");
    }

    /// US-010: a cargo configuration the parser cannot read is a bounded
    /// error, never a partial judgement, and the rest of the pack still
    /// reports.
    #[test]
    fn a_malformed_cargo_config_abstains_with_a_bounded_error() {
        let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join(format!(
            "cargo-health-config-invalid-{}",
            std::process::id()
        ));
        fs::create_dir_all(scratch.join(".cargo")).expect("the scratch should be creatable");
        fs::write(scratch.join(".cargo/config.toml"), "[build\nrustflags = [\n")
            .expect("the malformed configuration should be writable");

        let (mut metadata, _) = protocol_metadata();
        metadata.workspace_root = scratch
            .to_str()
            .expect("the scratch path should be UTF-8")
            .into();
        let scan = super::inspect(&metadata, &PolicyPlan::default());
        assert!(
            scan.errors
                .iter()
                .any(|error| error.code == "cargo-config-invalid"),
            "{:?}",
            scan.errors
        );
        assert!(
            scan.candidates
                .iter()
                .all(|candidate| candidate.definition.id != CARGO_PERMISSIVE_RUSTFLAGS.id),
            "a malformed configuration still produced a rustflags finding"
        );
        assert!(
            !scan.candidates.is_empty(),
            "the rest of the pack should still report"
        );

        fs::remove_dir_all(&scratch).expect("the scratch directory should be removable");
    }

    fn dependency_mut<'a>(
        package: &'a mut cargo_metadata::Package,
        key: &str,
    ) -> &'a mut Dependency {
        package
            .dependencies
            .iter_mut()
            .find(|dependency| dependency.rename.as_deref().unwrap_or(&dependency.name) == key)
            .expect("protocol dependency should exist")
    }

    fn resolution_fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/cargo-health/resolution")
            .join(name)
    }

    /// `cargo metadata --no-deps` resolves nothing, so it never writes a
    /// lockfile: the resolution fixtures stay intact.
    fn resolution_scan(name: &str) -> CargoHealthScan {
        let root = resolution_fixture(name);
        let before = fixture_hashes(&root);
        let (metadata, _) = offline_metadata(&root, &format!("resolution-{name}"));
        let scan = super::inspect(&metadata, &PolicyPlan::default());
        assert_eq!(fixture_hashes(&root), before, "{name} was mutated");
        scan
    }

    fn candidates_for(scan: &CargoHealthScan, definition: &RuleDefinition) -> Vec<String> {
        scan.candidates
            .iter()
            .filter(|candidate| candidate.definition.id == definition.id)
            .map(|candidate| candidate.message.clone())
            .collect()
    }

    /// US-076: two major versions of the same crate in the resolved graph are
    /// named, versions included, and a minor divergence is not.
    #[test]
    fn duplicate_major_versions_are_named_with_their_versions() {
        let scan = resolution_scan("duplicate");
        assert!(scan.errors.is_empty(), "{:?}", scan.errors);
        assert_eq!(scan.counters.resolved_packages, 5);

        assert_eq!(
            candidates_for(&scan, &CARGO_DUPLICATE_MAJOR_VERSIONS),
            ["Crate \"shared\" is resolved with incompatible major versions 1.4.2, 2.0.0."]
        );
        let candidate = scan
            .candidates
            .iter()
            .find(|candidate| candidate.definition.id == CARGO_DUPLICATE_MAJOR_VERSIONS.id)
            .expect("the duplicate candidate should exist");
        assert_eq!(candidate.package, "duplicate-resolution");
        assert_eq!(candidate.manifest_path.as_deref(), Some("Cargo.lock"));
        assert_eq!(candidate.definition.category, "dependencies");

        // The fixture also carries two minor versions of the same crate: they
        // stay compatible, so they are not reported.
        assert!(!candidate.message.contains("aligned"));
        assert!(candidates_for(&scan, &CARGO_MISSING_LOCKFILE).is_empty());
    }

    /// US-076: a workspace clean on these criteria produces no diagnostic.
    #[test]
    fn a_clean_resolution_produces_no_candidate() {
        let scan = resolution_scan("clean");
        assert!(scan.candidates.is_empty(), "{:?}", scan.candidates);
        assert!(scan.errors.is_empty(), "{:?}", scan.errors);
        assert_eq!(scan.counters.resolved_packages, 2);
    }

    /// US-076: a binary with no lockfile is reported, naming the package and
    /// its manifest.
    #[test]
    fn a_binary_without_a_lockfile_is_named() {
        let scan = resolution_scan("absent");
        assert!(scan.errors.is_empty(), "{:?}", scan.errors);
        assert_eq!(
            candidates_for(&scan, &CARGO_MISSING_LOCKFILE),
            [
                "Package \"absent-resolution\" produces a binary but no Cargo.lock sits next to its workspace manifest."
            ]
        );
        let candidate = &scan.candidates[0];
        assert_eq!(candidate.package, "absent-resolution");
        assert_eq!(candidate.manifest_path.as_deref(), Some("Cargo.toml"));
        assert!(candidates_for(&scan, &CARGO_DUPLICATE_MAJOR_VERSIONS).is_empty());
    }

    /// US-076: a missing resolution section makes the pack abstain with a
    /// bounded error, and the scan stays usable.
    #[test]
    fn an_unusable_resolution_abstains_with_a_bounded_error() {
        let scan = resolution_scan("unusable");
        assert!(scan.candidates.is_empty(), "{:?}", scan.candidates);
        assert_eq!(
            scan.errors,
            [CargoHealthError {
                code: "lockfile-resolution-absent",
                message: "the lockfile carries no resolved package section",
            }]
        );
        for error in &scan.errors {
            assert!(!error.message.contains('/'));
            assert!(!error.message.contains('\u{1b}'));
        }
    }

    /// US-076: a path dependency that leaves the workspace is reported without
    /// publishing the absolute path it points at.
    #[test]
    fn a_path_dependency_leaving_the_workspace_is_named_without_its_path() {
        let scan = resolution_scan("outside-path");
        assert_eq!(
            candidates_for(&scan, &CARGO_PATH_DEPENDENCY_OUTSIDE_WORKSPACE),
            ["Path dependency \"outside-crate\" resolves outside the workspace."]
        );
        let candidate = &scan.candidates[0];
        assert_eq!(candidate.package, "outside-path");
        assert_eq!(candidate.manifest_path.as_deref(), Some("Cargo.toml"));
        assert!(!candidate.message.contains(env!("CARGO_MANIFEST_DIR")));
        assert!(!candidate.message.contains(".."));
    }

    /// The ordering of major versions is independent of file order and ignores
    /// pre-releases and build metadata.
    #[test]
    fn major_comparison_is_order_independent_and_ignores_prerelease_metadata() {
        let packages = |pairs: &[(&str, &str)]| {
            pairs
                .iter()
                .map(|(name, version)| ((*name).to_owned(), (*version).to_owned()))
                .collect::<Vec<_>>()
        };

        assert_eq!(
            duplicate_major_versions(&packages(&[("a", "2.0.0"), ("a", "1.0.0")])),
            [("a".to_owned(), "1.0.0, 2.0.0".to_owned())]
        );
        assert!(
            duplicate_major_versions(&packages(&[("a", "1.0.0-rc.1"), ("a", "1.0.0+build")]))
                .is_empty()
        );
        assert!(duplicate_major_versions(&packages(&[("a", "1.0.0"), ("b", "2.0.0")])).is_empty());
        assert!(duplicate_major_versions(&packages(&[("a", "1.0.0"), ("a", "1.0.0")])).is_empty());
        // An unreadable version cannot be compared, so it cannot ground a
        // duplication verdict.
        assert!(duplicate_major_versions(&packages(&[("a", "x.0.0"), ("a", "1.0.0")])).is_empty());
        assert_eq!(major_of("10.2.3"), Some("10"));
        assert_eq!(major_of(""), None);
    }
}
