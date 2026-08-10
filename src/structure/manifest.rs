//! Where the manifest and the module tree disagree.
//!
//! The other structural detectors read one file and describe what is in it.
//! These two read the workspace description and the module tree together, and
//! report what neither side alone can see: a source file Cargo never compiles
//! because no `mod` declaration reaches it, and a feature the manifest and the
//! code do not agree about.
//!
//! Both questions are per package. A file belongs to the module tree of the
//! package whose targets reach it, and a feature is declared by one manifest,
//! so a workspace of several members answers them several times rather than
//! once over the union.
//!
//! The orphan walk is the only place the structural pass touches a file the
//! source kernel did not enumerate, and that is exactly the point: an
//! unreachable file is invisible to every producer that starts from a Cargo
//! target. The walk is bounded, never leaves the directories a target already
//! roots, and stops at any directory holding another package.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use cargo_metadata::{Metadata, Package};
use ra_ap_syntax::ast;
use ra_ap_syntax::{AstNode, AstToken, SyntaxKind, SyntaxNode, SyntaxToken};

use super::{Observation, Unit, is_generated};
use crate::policy::{
    PolicyPlan, RuleDefinition, STRUCTURE_ORPHAN_MODULE_FILE, STRUCTURE_UNREFERENCED_FEATURE,
};
use crate::report::DiagnosticContext;
use crate::source_kernel::{Enumeration, SourceSpan};

/// Directory entries the orphan walk may visit across the whole workspace.
/// Past it the walk stops: a workspace that large has already answered the
/// question, and the pass runs under a wall clock.
const WALK_LIMIT: usize = 20_000;

/// Bytes a candidate file may reach before the pass declines to read it. It is
/// the limit the source kernel applies to the files it enumerates, so a file no
/// module reaches is not read on terms the reached ones are not.
const CANDIDATE_BYTES_LIMIT: u64 = 8_388_608;

/// Which of the two manifest detectors the policy leaves on.
#[derive(Debug, Clone, Copy)]
pub(super) struct Active {
    pub(super) orphans: bool,
    pub(super) features: bool,
}

impl Active {
    pub(super) fn of(plan: &PolicyPlan) -> Self {
        Self {
            orphans: plan.is_active(STRUCTURE_ORPHAN_MODULE_FILE.id),
            features: plan.is_active(STRUCTURE_UNREFERENCED_FEATURE.id),
        }
    }

    pub(super) const fn any(self) -> bool {
        self.orphans || self.features
    }
}

/// What the walk of the units tells the two manifest detectors.
#[derive(Debug, Default)]
pub(super) struct Uses {
    /// Workspace-relative paths pulled in with `include!`. Cargo compiles them
    /// without any `mod` declaration naming them, so they are reached even
    /// though the source kernel never enumerated them.
    included: BTreeSet<String>,
    /// File names a build script writes as a literal. A build script that
    /// names a Rust file compiles it, probes with it or copies it, and it does
    /// all three without any module declaration: `thiserror` feeds
    /// `build/probe.rs` to rustc that way. The name is what is kept, because
    /// the path is usually assembled a segment at a time.
    scripted: BTreeSet<String>,
    /// Every `feature = "..."` a `cfg` reads, with the site that reads it.
    features: Vec<Reference>,
}

/// One site where a `cfg` names a feature.
#[derive(Debug)]
struct Reference {
    feature: String,
    path: String,
    span: SourceSpan,
    context: Option<DiagnosticContext>,
    /// Packages whose targets reach the file, and whose manifests therefore
    /// decide whether the feature exists.
    packages: BTreeSet<String>,
}

/// Reads one unit for both detectors, in a single pass over its tree.
///
/// Both detectors look for a word that has to be written for them to have
/// anything to find, so a file whose text does not carry it is never walked.
/// The scan of the text costs a fraction of the traversal it replaces, and on
/// real code it replaces almost all of them: the pass already walks every tree
/// twice, and a third walk for nothing is a quarter of its budget.
pub(super) fn observe(unit: &Unit<'_>, active: Active, uses: &mut Uses) {
    let includes = active.orphans && unit.source.contains("include");
    let features = active.features && unit.source.contains("feature");
    let scripted = active.orphans
        && unit.context == Some(DiagnosticContext::BuildScript)
        && unit.source.contains(".rs");
    if !includes && !features && !scripted {
        return;
    }
    if scripted {
        for token in significant(unit.tree.syntax()) {
            if let Some(literal) = ast::String::cast(token)
                .and_then(|literal| literal.value().ok().map(|value| value.into_owned()))
                && literal.ends_with(".rs")
                && let Some(name) = Path::new(&literal).file_name()
            {
                uses.scripted.insert(name.to_string_lossy().into_owned());
            }
        }
    }
    for node in unit.tree.syntax().descendants() {
        match node.kind() {
            SyntaxKind::MACRO_CALL => {
                let Some(call) = ast::MacroCall::cast(node.clone()) else {
                    continue;
                };
                let name = call
                    .path()
                    .and_then(|path| path.as_single_name_ref())
                    .map(|name| name.text().to_string());
                match name.as_deref() {
                    Some("include") if includes => {
                        if let Some(tree) = call.token_tree()
                            && let Some(literal) = first_string(tree.syntax())
                            && let Some(path) = included_path(unit.path, &literal)
                        {
                            uses.included.insert(path);
                        }
                    }
                    Some("cfg") if features => {
                        if let Some(tree) = call.token_tree() {
                            read_features(unit, &significant(tree.syntax()), &node, uses);
                        }
                    }
                    _ => {}
                }
            }
            // `#[cfg(...)]` and `#[cfg_attr(...)]` each parse into a meta of
            // their own, so the gate is read off the tokens rather than off a
            // variant: what is needed here is the name written first and the
            // literals under it, whatever shape the grammar gives them.
            SyntaxKind::ATTR if features => {
                let tokens = significant(&node);
                let gate = tokens
                    .iter()
                    .skip_while(|token| token.kind() != SyntaxKind::L_BRACK)
                    .nth(1)
                    .map(|token| token.text().to_owned());
                if matches!(gate.as_deref(), Some("cfg" | "cfg_attr")) {
                    read_features(unit, &tokens, &node, uses);
                }
            }
            _ => {}
        }
    }
}

/// Tokens of a node with the trivia dropped, in written order.
fn significant(node: &SyntaxNode) -> Vec<SyntaxToken> {
    node.descendants_with_tokens()
        .filter_map(|element| element.into_token())
        .filter(|token| !token.kind().is_trivia())
        .collect()
}

/// Every `feature = "..."` of one condition, whatever nests it: `any`, `all`
/// and `not` are read through, because what matters here is that the name was
/// written, not the shape of the condition around it.
fn read_features(unit: &Unit<'_>, tokens: &[SyntaxToken], site: &SyntaxNode, uses: &mut Uses) {
    for window in tokens.windows(3) {
        if window[0].text() != "feature" || window[1].text() != "=" {
            continue;
        }
        let Some(feature) = ast::String::cast(window[2].clone())
            .and_then(|literal| literal.value().ok().map(|value| value.into_owned()))
        else {
            continue;
        };
        uses.features.push(Reference {
            feature,
            path: unit.path.to_owned(),
            span: unit.span(site),
            context: super::test_context(site).or(unit.context),
            packages: unit.packages.iter().map(|id| (*id).to_owned()).collect(),
        });
    }
}

/// What the two detectors found, as observations the pass groups like any
/// other. The path of each one is what the finding points at: the file for an
/// orphan, the manifest for a feature nothing reads, the reading site for a
/// feature nothing declares.
pub(super) fn findings(
    metadata: &Metadata,
    enumeration: &Enumeration,
    active: Active,
    uses: &Uses,
) -> Vec<(&'static RuleDefinition, String, Observation)> {
    let Ok(workspace_root) = metadata.workspace_root.as_std_path().canonicalize() else {
        return Vec::new();
    };
    let mut observations = Vec::new();
    if active.orphans {
        observations.extend(orphans(metadata, enumeration, uses, &workspace_root));
    }
    if active.features {
        observations.extend(features(metadata, uses, &workspace_root));
    }
    observations
}

/// Files under a package's own target directories that no module tree reaches.
///
/// The walk is per package, and so is what the finding names: a file answers to
/// the module tree of the package whose directory holds it. What exempts it is
/// not, and cannot be. A member reaching into its neighbour with `#[path =
/// "../other/src/shared.rs"]` compiles that file, so the reach of every package
/// is what says whether Cargo compiles it. Reachability is compared by
/// workspace-relative path, which is unique, so no `mod` declaration of one
/// package ever silences a file of another.
fn orphans(
    metadata: &Metadata,
    enumeration: &Enumeration,
    uses: &Uses,
    workspace_root: &Path,
) -> Vec<(&'static RuleDefinition, String, Observation)> {
    let compiled: BTreeSet<&str> = enumeration
        .units()
        .map(|unit| unit.relative_path())
        .collect();

    let mut budget = WALK_LIMIT;
    let mut observations = Vec::new();
    for package in workspace_packages(metadata) {
        let Some(package_directory) = package.manifest_path.parent() else {
            continue;
        };
        let package_directory = package_directory.as_std_path();

        let mut candidates = BTreeSet::new();
        for root in module_roots(package, package_directory) {
            walk(&root, &mut budget, &mut candidates);
        }
        for candidate in candidates {
            let Some(relative) = relative_path(workspace_root, &candidate) else {
                continue;
            };
            if compiled.contains(relative.as_str())
                || uses.included.contains(&relative)
                || candidate
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| uses.scripted.contains(name))
            {
                continue;
            }
            let Some(source) = readable(&candidate) else {
                continue;
            };
            // A generated file is nobody's writing: the whole pass leaves it
            // alone, and an unreached one is no different.
            if is_generated(&source) {
                continue;
            }
            observations.push((
                &STRUCTURE_ORPHAN_MODULE_FILE,
                relative.clone(),
                Observation {
                    key: format!("orphan|{relative}"),
                    subject: format!(
                        "{relative} is compiled by nothing: no module declaration of package \"{}\" reaches it.",
                        package.name
                    ),
                    span: whole_file(&source),
                    context: None,
                    complexity: None,
                },
            ));
        }
    }
    observations
}

/// Features the manifest and the code disagree about, per package.
fn features(
    metadata: &Metadata,
    uses: &Uses,
    workspace_root: &Path,
) -> Vec<(&'static RuleDefinition, String, Observation)> {
    let mut observations = Vec::new();
    let mut declared: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for package in workspace_packages(metadata) {
        let names = package
            .features
            .keys()
            .map(String::as_str)
            .chain(optional_dependencies(package))
            .collect();
        declared.insert(package.id.repr.as_str(), names);
    }

    for package in workspace_packages(metadata) {
        let optional: BTreeSet<&str> = optional_dependencies(package).collect();
        // A feature named in another feature's list is referenced: turning the
        // first one on is what turning the second one on does.
        let listed: BTreeSet<&str> = package
            .features
            .values()
            .flatten()
            .filter_map(|entry| plain_feature(entry))
            .collect();
        let read: BTreeSet<&str> = uses
            .features
            .iter()
            .filter(|reference| reference.packages.contains(package.id.repr.as_str()))
            .map(|reference| reference.feature.as_str())
            .collect();
        let Some(manifest) = relative_path(workspace_root, package.manifest_path.as_std_path())
        else {
            continue;
        };
        let text = readable(package.manifest_path.as_std_path()).unwrap_or_default();

        for (feature, activates) in &package.features {
            // `default` is read by Cargo itself, an optional dependency's
            // feature is read by whoever activates the dependency, and a
            // feature that activates something does something whether or not a
            // `cfg` names it. What is left is a feature with no reader and no
            // effect.
            if feature == "default"
                || optional.contains(feature.as_str())
                || !activates.is_empty()
                || listed.contains(feature.as_str())
                || read.contains(feature.as_str())
            {
                continue;
            }
            observations.push((
                &STRUCTURE_UNREFERENCED_FEATURE,
                manifest.clone(),
                Observation {
                    key: format!("declared|{}|{feature}", package.name),
                    subject: format!(
                        "Package \"{}\" declares feature \"{feature}\", which no cfg reads and which activates nothing.",
                        package.name
                    ),
                    span: manifest_span(&text, feature),
                    context: None,
                    complexity: None,
                },
            ));
        }
    }

    for reference in &uses.features {
        // Resolution is per package: the manifests that decide are those of the
        // packages whose targets compile the file.
        let known = reference.packages.iter().any(|package| {
            declared
                .get(package.as_str())
                .is_some_and(|names| names.contains(reference.feature.as_str()))
        });
        if known || reference.packages.is_empty() {
            continue;
        }
        observations.push((
            &STRUCTURE_UNREFERENCED_FEATURE,
            reference.path.clone(),
            Observation {
                key: format!("referenced|{}", reference.feature),
                subject: format!(
                    "cfg(feature = \"{}\") reads a feature the package compiling this file does not declare.",
                    reference.feature
                ),
                span: reference.span,
                context: reference.context,
                complexity: None,
            },
        ));
    }
    observations
}

fn workspace_packages(metadata: &Metadata) -> impl Iterator<Item = &Package> {
    metadata
        .packages
        .iter()
        .filter(|package| metadata.workspace_members.contains(&package.id))
}

fn optional_dependencies(package: &Package) -> impl Iterator<Item = &str> {
    package
        .dependencies
        .iter()
        .filter(|dependency| dependency.optional)
        .map(|dependency| {
            dependency
                .rename
                .as_deref()
                .unwrap_or(dependency.name.as_str())
        })
}

/// Name of a feature this entry turns on, when it names one. `dep:serde` and
/// `serde/derive` name a dependency instead, which is an effect rather than a
/// reference to another feature of the same manifest.
fn plain_feature(entry: &str) -> Option<&str> {
    (!entry.contains('/') && !entry.starts_with("dep:")).then_some(entry)
}

/// Directories a package's own targets root, with the redundant ones dropped:
/// a build script roots the package directory, which already contains `src`.
fn module_roots(package: &Package, package_directory: &Path) -> Vec<PathBuf> {
    let roots: BTreeSet<PathBuf> = package
        .targets
        .iter()
        .filter_map(|target| target.src_path.as_std_path().parent())
        .filter(|parent| parent.starts_with(package_directory))
        .map(Path::to_path_buf)
        .collect();
    roots
        .iter()
        .filter(|root| {
            !roots
                .iter()
                .any(|other| other != *root && root.starts_with(other))
        })
        .cloned()
        .collect()
}

/// Every `.rs` file under one root, without following a symbolic link and
/// without entering another package.
fn walk(root: &Path, budget: &mut usize, files: &mut BTreeSet<PathBuf>) {
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            if *budget == 0 {
                return;
            }
            *budget -= 1;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if name.starts_with('.') {
                continue;
            }
            // `file_type` does not follow a symbolic link, so a link to a
            // directory is neither walked nor read.
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                if name != "target" && !holds_a_package(&entry.path()) {
                    stack.push(entry.path());
                }
            } else if kind.is_file() && name.ends_with(".rs") {
                files.insert(entry.path());
            }
        }
    }
}

/// Is this directory another package, or the place several of them are kept?
/// Either way the files under it answer to a module tree that is not the one
/// being walked, which is what keeps a directory of fixture crates out of the
/// report.
fn holds_a_package(directory: &Path) -> bool {
    if directory.join("Cargo.toml").is_file() {
        return true;
    }
    let Ok(entries) = fs::read_dir(directory) else {
        return false;
    };
    entries.flatten().any(|entry| {
        entry.file_type().is_ok_and(|kind| kind.is_dir())
            && entry.path().join("Cargo.toml").is_file()
    })
}

/// Path an `include!` names, relative to the file that writes it.
fn included_path(from: &str, literal: &str) -> Option<String> {
    let directory = Path::new(from).parent()?;
    let joined = directory.join(literal);
    let mut normalized = PathBuf::new();
    for component in joined.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other),
        }
    }
    Some(normalized.to_string_lossy().replace('\\', "/"))
}

fn first_string(tree: &SyntaxNode) -> Option<String> {
    tree.descendants_with_tokens()
        .filter_map(|element| element.into_token())
        .find_map(|token| {
            ast::String::cast(token).and_then(|literal| {
                literal
                    .value()
                    .ok()
                    .map(|value| value.into_owned())
            })
        })
}

fn relative_path(workspace_root: &Path, path: &Path) -> Option<String> {
    let canonical = path.canonicalize().ok()?;
    let relative = canonical.strip_prefix(workspace_root).ok()?;
    let relative = relative.to_string_lossy().replace('\\', "/");
    (!relative.is_empty()).then_some(relative)
}

/// Text of a file the pass is willing to read, or nothing.
fn readable(path: &Path) -> Option<String> {
    let metadata = fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > CANDIDATE_BYTES_LIMIT {
        return None;
    }
    fs::read_to_string(path).ok()
}

fn whole_file(source: &str) -> SourceSpan {
    let last = source.lines().last().unwrap_or_default();
    SourceSpan {
        line_start: 1,
        column_start: 1,
        line_end: source.lines().count().max(1),
        column_end: last.chars().count() + 1,
    }
}

/// Line of the manifest that declares this feature, so the finding points at
/// the entry rather than at the top of the file.
fn manifest_span(manifest: &str, feature: &str) -> SourceSpan {
    let mut inside = false;
    for (index, line) in manifest.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            inside = trimmed == "[features]";
            continue;
        }
        if !inside {
            continue;
        }
        let named = trimmed
            .split_once('=')
            .map(|(name, _)| name.trim().trim_matches('"'));
        if named == Some(feature) {
            return SourceSpan {
                line_start: index + 1,
                column_start: 1,
                line_end: index + 1,
                column_end: line.chars().count() + 1,
            };
        }
    }
    SourceSpan {
        line_start: 1,
        column_start: 1,
        line_end: 1,
        column_end: 1,
    }
}

#[cfg(test)]
mod tests {
    use ra_ap_syntax::{Edition, SourceFile};

    use super::*;
    use crate::source_kernel::line_starts;

    fn unit<'a>(source: &'a str, path: &'a str) -> Unit<'a> {
        Unit {
            tree: SourceFile::parse(source, Edition::Edition2024).tree(),
            source,
            line_starts: line_starts(source),
            path,
            context: None,
            packages: vec!["probe"],
            attributes: Vec::new(),
        }
    }

    const BOTH: Active = Active {
        orphans: true,
        features: true,
    };

    fn read(source: &str) -> Uses {
        let mut uses = Uses::default();
        observe(&unit(source, "src/lib.rs"), BOTH, &mut uses);
        uses
    }

    /// US-015: every shape a `cfg` writes a feature in is read, and the ones
    /// that are not a feature gate are left alone.
    #[test]
    fn every_cfg_shape_names_its_feature() {
        let read = read(
            "#[cfg(feature = \"one\")]\nfn a() {}\n\
             #[cfg(all(unix, any(feature = \"two\", not(feature = \"three\"))))]\nfn b() {}\n\
             #[cfg_attr(feature = \"four\", derive(Debug))]\nstruct C;\n\
             fn d() { if cfg!(feature = \"five\") { } }\n\
             fn e() { let feature = \"six\"; }\n",
        );
        let names: Vec<&str> = read
            .features
            .iter()
            .map(|reference| reference.feature.as_str())
            .collect();
        assert_eq!(names, ["one", "two", "three", "four", "five"]);
    }

    /// US-013: a file pulled in with `include!` is compiled without any `mod`
    /// declaration naming it, so the walk must treat it as reached.
    #[test]
    fn an_included_file_is_recorded_as_reached() {
        let mut uses = Uses::default();
        observe(
            &unit("include!(\"../generated/table.rs\");\n", "src/deep/mod.rs"),
            BOTH,
            &mut uses,
        );
        assert_eq!(
            uses.included.iter().map(String::as_str).collect::<Vec<_>>(),
            ["src/generated/table.rs"]
        );
    }

    #[test]
    fn a_feature_entry_is_located_in_its_manifest() {
        let manifest = "[package]\nname = \"probe\"\n\n[features]\ndefault = []\nspare = []\n\n[dependencies]\nspare = \"1\"\n";
        assert_eq!(
            manifest_span(manifest, "spare"),
            SourceSpan {
                line_start: 6,
                column_start: 1,
                line_end: 6,
                column_end: 11,
            }
        );
        // A name the table does not carry falls back to the head of the file
        // rather than pointing at a line that says something else.
        assert_eq!(
            manifest_span(manifest, "absent"),
            SourceSpan {
                line_start: 1,
                column_start: 1,
                line_end: 1,
                column_end: 1,
            }
        );
    }

    #[test]
    fn only_a_plain_entry_names_another_feature() {
        assert_eq!(plain_feature("spare"), Some("spare"));
        assert_eq!(plain_feature("dep:serde"), None);
        assert_eq!(plain_feature("serde/derive"), None);
        assert_eq!(plain_feature("serde?/derive"), None);
    }
}
