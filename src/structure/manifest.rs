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
//! target. The walk is bounded by a directory budget and by the pass's
//! deadline, never leaves the directories a target already roots, and stops at
//! any directory holding another package.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use cargo_metadata::{Metadata, Package};
use ra_ap_syntax::ast;
use ra_ap_syntax::{AstNode, AstToken, SyntaxNode, SyntaxToken};
use serde::Deserialize;

use super::{Active, Deadline, Observation, Unit, is_generated, single_name};
use crate::policy::{
    RuleDefinition, STRUCTURE_ORPHAN_MODULE_FILE, STRUCTURE_UNREFERENCED_FEATURE,
};
use crate::report::DiagnosticContext;
use crate::source_kernel::{
    Enumeration, SourceSpan, byte_range_span, line_starts, workspace_relative,
};

/// The rules this half of the pass produces.
pub(super) const RULES: [&RuleDefinition; 2] = [
    &STRUCTURE_ORPHAN_MODULE_FILE,
    &STRUCTURE_UNREFERENCED_FEATURE,
];

/// Directory entries the orphan walk may visit across the whole workspace.
/// Past it the walk stops: a workspace that large has already answered the
/// question, and the pass runs under a wall clock.
const WALK_LIMIT: usize = 20_000;

/// Bytes a candidate file may reach before the pass declines to read it. It is
/// the limit the source kernel applies to the files it enumerates, so a file no
/// module reaches is not read on terms the reached ones are not.
const CANDIDATE_BYTES_LIMIT: u64 = 8_388_608;

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

/// Reads one unit for both detectors, off the inventory its single traversal
/// collected.
pub(super) fn observe(unit: &Unit<'_>, active: &Active, uses: &mut Uses) {
    let orphans = active.on(&STRUCTURE_ORPHAN_MODULE_FILE);
    let features = active.on(&STRUCTURE_UNREFERENCED_FEATURE);
    if orphans && unit.context == Some(DiagnosticContext::BuildScript) {
        // Every string literal of the file, and only of this file: a package
        // has one build script, so the tokens it writes are worth a walk of
        // their own rather than a bucket every other unit would carry empty.
        for token in significant(unit.tree.syntax()) {
            if let Some(literal) = string_value(&token)
                && literal.ends_with(".rs")
                && let Some(name) = Path::new(&literal).file_name()
            {
                uses.scripted.insert(name.to_string_lossy().into_owned());
            }
        }
    }
    if orphans || features {
        for call in &unit.inventory.macro_calls {
            match call.path().as_ref().and_then(single_name).as_deref() {
                Some("include") if orphans => {
                    if let Some(tree) = call.token_tree()
                        && let Some(literal) = first_string(tree.syntax())
                        && let Some(path) = included_path(unit.path, &literal)
                    {
                        uses.included.insert(path);
                    }
                }
                Some("cfg") if features => {
                    if let Some(tree) = call.token_tree() {
                        read_features(unit, &significant(tree.syntax()), call.syntax(), uses);
                    }
                }
                _ => {}
            }
        }
    }
    if features {
        for attribute in &unit.inventory.attributes {
            if reads_a_cfg(attribute) {
                let node = attribute.syntax();
                read_features(unit, &significant(node), node, uses);
            }
        }
    }
}

/// Does this attribute gate on a `cfg`?
///
/// `#[cfg(...)]` and `#[cfg_attr(...)]` each parse into a meta of their own, so
/// the question is asked of the meta the grammar produced rather than of the
/// token that happens to sit after the bracket. The token-tree form is answered
/// too, because a grammar that stops distinguishing them must not silently stop
/// answering.
fn reads_a_cfg(attribute: &ast::Attr) -> bool {
    match attribute.meta() {
        Some(ast::Meta::CfgMeta(_) | ast::Meta::CfgAttrMeta(_)) => true,
        Some(ast::Meta::TokenTreeMeta(meta)) => matches!(
            meta.path().as_ref().and_then(single_name).as_deref(),
            Some("cfg" | "cfg_attr")
        ),
        _ => false,
    }
}

/// Tokens of a node with the trivia dropped, in written order.
fn significant(node: &SyntaxNode) -> Vec<SyntaxToken> {
    node.descendants_with_tokens()
        .filter_map(|element| element.into_token())
        .filter(|token| !token.kind().is_trivia())
        .collect()
}

/// The value of a string literal token, unescaped.
fn string_value(token: &SyntaxToken) -> Option<String> {
    ast::String::cast(token.clone())
        .and_then(|literal| literal.value().ok().map(|value| value.into_owned()))
}

/// Every `feature = "..."` of one condition, whatever nests it: `any`, `all`
/// and `not` are read through, because what matters here is that the name was
/// written, not the shape of the condition around it.
fn read_features(unit: &Unit<'_>, tokens: &[SyntaxToken], site: &SyntaxNode, uses: &mut Uses) {
    for window in tokens.windows(3) {
        if window[0].text() != "feature" || window[1].text() != "=" {
            continue;
        }
        let Some(feature) = string_value(&window[2]) else {
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
pub(super) struct Findings {
    pub(super) observations: Vec<(&'static RuleDefinition, String, Observation)>,
    /// Did the orphan walk stop at the deadline rather than finish? It is the
    /// one phase of the pass that reads the filesystem, so it is the one that
    /// has to say whether it got to the end.
    pub(super) stopped: bool,
}

pub(super) fn findings(
    metadata: &Metadata,
    enumeration: &Enumeration,
    active: &Active,
    uses: &Uses,
    deadline: &Deadline,
) -> Findings {
    let Ok(workspace_root) = metadata.workspace_root.as_std_path().canonicalize() else {
        return Findings {
            observations: Vec::new(),
            stopped: false,
        };
    };
    let mut observations = Vec::new();
    let mut stopped = false;
    if active.on(&STRUCTURE_ORPHAN_MODULE_FILE) {
        stopped = orphans(
            metadata,
            enumeration,
            uses,
            &workspace_root,
            deadline,
            &mut observations,
        );
    }
    if active.on(&STRUCTURE_UNREFERENCED_FEATURE) {
        let declared = declared_features(metadata);
        observations.extend(unreferenced_declarations(metadata, uses, &workspace_root));
        observations.extend(unknown_references(uses, &declared));
    }
    Findings {
        observations,
        stopped,
    }
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
///
/// Returns whether the walk stopped early.
fn orphans(
    metadata: &Metadata,
    enumeration: &Enumeration,
    uses: &Uses,
    workspace_root: &Path,
    deadline: &Deadline,
    observations: &mut Vec<(&'static RuleDefinition, String, Observation)>,
) -> bool {
    let compiled: BTreeSet<&str> = enumeration
        .units()
        .map(|unit| unit.relative_path())
        .collect();

    let mut budget = WALK_LIMIT;
    let mut stopped = false;
    for package in workspace_packages(metadata) {
        let Some(package_directory) = package.manifest_path.parent() else {
            continue;
        };
        let package_directory = package_directory.as_std_path();

        let mut candidates = BTreeSet::new();
        for root in module_roots(package, package_directory) {
            stopped |= walk(&root, &mut budget, deadline, &mut candidates);
        }
        for candidate in candidates {
            // Reading a candidate is the only unbounded work of this phase, so
            // the deadline is read before every one of them and not only
            // between packages.
            if deadline.exceeded() {
                return true;
            }
            let Some(relative) = workspace_relative(workspace_root, &candidate) else {
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
    stopped
}

/// Every feature name each workspace package declares, its optional
/// dependencies included: activating one of those is what declares it.
fn declared_features(metadata: &Metadata) -> BTreeMap<&str, BTreeSet<&str>> {
    workspace_packages(metadata)
        .map(|package| {
            let names = package
                .features
                .keys()
                .map(String::as_str)
                .chain(optional_dependencies(package))
                .collect();
            (package.id.repr.as_str(), names)
        })
        .collect()
}

/// Features a manifest declares that nothing reads and that do nothing.
fn unreferenced_declarations(
    metadata: &Metadata,
    uses: &Uses,
    workspace_root: &Path,
) -> Vec<(&'static RuleDefinition, String, Observation)> {
    let mut observations = Vec::new();
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
        // `default` is read by Cargo itself, an optional dependency's feature
        // is read by whoever activates the dependency, and a feature that
        // activates something does something whether or not a `cfg` names it.
        // What is left is a feature with no reader and no effect.
        let unreferenced: Vec<&String> = package
            .features
            .iter()
            .filter(|(feature, activates)| {
                feature.as_str() != "default"
                    && !optional.contains(feature.as_str())
                    && activates.is_empty()
                    && !listed.contains(feature.as_str())
                    && !read.contains(feature.as_str())
            })
            .map(|(feature, _)| feature)
            .collect();
        if unreferenced.is_empty() {
            continue;
        }
        let Some(manifest) = workspace_relative(workspace_root, package.manifest_path.as_std_path())
        else {
            continue;
        };
        // The manifest is read once here, and only for a package that has
        // something to report in it.
        let text = readable(package.manifest_path.as_std_path()).unwrap_or_default();
        let spans = declaration_spans(&text);
        for feature in unreferenced {
            observations.push((
                &STRUCTURE_UNREFERENCED_FEATURE,
                manifest.clone(),
                Observation {
                    key: format!("declared|{}|{feature}", package.name),
                    subject: format!(
                        "Package \"{}\" declares feature \"{feature}\", which no cfg reads and which activates nothing.",
                        package.name
                    ),
                    span: spans.get(feature.as_str()).copied().unwrap_or_else(head_of_file),
                    context: None,
                    complexity: None,
                },
            ));
        }
    }
    observations
}

/// Sites reading a feature no manifest compiling them declares.
fn unknown_references(
    uses: &Uses,
    declared: &BTreeMap<&str, BTreeSet<&str>>,
) -> Vec<(&'static RuleDefinition, String, Observation)> {
    uses.features
        .iter()
        .filter(|reference| {
            // Resolution is per package: the manifests that decide are those of
            // the packages whose targets compile the file.
            !reference.packages.is_empty()
                && !reference.packages.iter().any(|package| {
                    declared
                        .get(package.as_str())
                        .is_some_and(|names| names.contains(reference.feature.as_str()))
                })
        })
        .map(|reference| {
            (
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
            )
        })
        .collect()
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

/// The `[features]` table as the parser reads it, for the position of each
/// declaration.
#[derive(Deserialize)]
struct ManifestFeatures {
    features: Option<BTreeMap<toml::Spanned<String>, toml::Value>>,
}

/// Where each feature is declared in its manifest, so a finding points at the
/// entry rather than at the top of the file.
///
/// The positions come from the parser, the way `cargo_health` locates a
/// manifest entry: a scan looking for a `[features]` header misses a table
/// written any other way and then points at the first line of the file as
/// though it said something.
fn declaration_spans(manifest: &str) -> BTreeMap<String, SourceSpan> {
    let Ok(document) = toml::from_str::<ManifestFeatures>(manifest) else {
        return BTreeMap::new();
    };
    let starts = line_starts(manifest);
    document
        .features
        .unwrap_or_default()
        .into_keys()
        .map(|name| {
            let span = byte_range_span(name.span(), &starts, manifest);
            (name.into_inner(), span)
        })
        .collect()
}

/// Where a finding points when the manifest that declares it does not read
/// back.
const fn head_of_file() -> SourceSpan {
    SourceSpan {
        line_start: 1,
        column_start: 1,
        line_end: 1,
        column_end: 1,
    }
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
/// without entering another package. Returns whether the walk stopped early.
fn walk(
    root: &Path,
    budget: &mut usize,
    deadline: &Deadline,
    files: &mut BTreeSet<PathBuf>,
) -> bool {
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        if deadline.exceeded() {
            return true;
        }
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            if *budget == 0 {
                return false;
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
                if name != "target" && !holds_a_package(&entry.path(), budget) {
                    stack.push(entry.path());
                }
            } else if kind.is_file() && name.ends_with(".rs") {
                files.insert(entry.path());
            }
        }
    }
    false
}

/// Is this directory another package, or the place several of them are kept?
/// Either way the files under it answer to a module tree that is not the one
/// being walked, which is what keeps a directory of fixture crates out of the
/// report.
///
/// The probe reads a directory of its own, so it spends the same budget the
/// walk does: a tree of directories that each hold several others costs more
/// listings than it holds entries.
fn holds_a_package(directory: &Path, budget: &mut usize) -> bool {
    if directory.join("Cargo.toml").is_file() {
        return true;
    }
    let Ok(entries) = fs::read_dir(directory) else {
        return false;
    };
    entries.flatten().any(|entry| {
        *budget = budget.saturating_sub(1);
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
        .find_map(|token| string_value(&token))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn both() -> Active {
        Active::of_rules(RULES)
    }

    fn read(source: &str) -> Uses {
        let mut uses = Uses::default();
        observe(&Unit::probe(source, "src/lib.rs"), &both(), &mut uses);
        uses
    }

    /// US-015: every shape a `cfg` writes a feature in is read, and the ones
    /// that are not a feature gate are left alone.
    ///
    /// The order is not asserted: the features reach a report through a keyed
    /// map, so the order they were read in never survives to it.
    #[test]
    fn every_cfg_shape_names_its_feature() {
        let uses = read(
            "#[cfg(feature = \"one\")]\nfn a() {}\n\
             #[cfg(all(unix, any(feature = \"two\", not(feature = \"three\"))))]\nfn b() {}\n\
             #[cfg_attr(feature = \"four\", derive(Debug))]\nstruct C;\n\
             fn d() { if cfg!(feature = \"five\") { } }\n\
             fn e() { let feature = \"six\"; }\n",
        );
        let mut names: Vec<&str> = uses
            .features
            .iter()
            .map(|reference| reference.feature.as_str())
            .collect();
        names.sort_unstable();
        assert_eq!(names, ["five", "four", "one", "three", "two"]);
    }

    /// US-013: a file pulled in with `include!` is compiled without any `mod`
    /// declaration naming it, so the walk must treat it as reached.
    #[test]
    fn an_included_file_is_recorded_as_reached() {
        let mut uses = Uses::default();
        observe(
            &Unit::probe("include!(\"../generated/table.rs\");\n", "src/deep/mod.rs"),
            &both(),
            &mut uses,
        );
        assert_eq!(
            uses.included.iter().map(String::as_str).collect::<Vec<_>>(),
            ["src/generated/table.rs"]
        );
    }

    /// A rule the policy left off reads nothing, so neither detector pays for
    /// the other.
    #[test]
    fn an_inactive_rule_reads_nothing() {
        let mut uses = Uses::default();
        observe(
            &Unit::probe(
                "#[cfg(feature = \"one\")]\nfn a() {}\ninclude!(\"other.rs\");\n",
                "src/lib.rs",
            ),
            &Active::default(),
            &mut uses,
        );
        assert!(uses.features.is_empty());
        assert!(uses.included.is_empty());

        let mut orphans_only = Uses::default();
        observe(
            &Unit::probe(
                "#[cfg(feature = \"one\")]\nfn a() {}\ninclude!(\"other.rs\");\n",
                "src/lib.rs",
            ),
            &Active::of_rules([&STRUCTURE_ORPHAN_MODULE_FILE]),
            &mut orphans_only,
        );
        assert!(orphans_only.features.is_empty());
        assert_eq!(orphans_only.included.len(), 1);
    }

    /// A build script naming a Rust file compiles it without any module
    /// declaration, and only a build script is read that way.
    #[test]
    fn a_build_script_records_the_files_it_names() {
        let source = "fn main() { println!(\"cargo:rerun-if-changed=build/probe.rs\"); }";
        let mut scripted = Unit::probe(source, "build.rs");
        scripted.context = Some(DiagnosticContext::BuildScript);
        let mut uses = Uses::default();
        observe(&scripted, &both(), &mut uses);
        assert_eq!(
            uses.scripted.iter().map(String::as_str).collect::<Vec<_>>(),
            ["probe.rs"]
        );

        let mut library = Uses::default();
        observe(&Unit::probe(source, "src/lib.rs"), &both(), &mut library);
        assert!(library.scripted.is_empty());
    }

    #[test]
    fn a_feature_entry_is_located_in_its_manifest() {
        let manifest = "[package]\nname = \"probe\"\n\n[features]\ndefault = []\nspare = []\n\n[dependencies]\nspare = \"1\"\n";
        let spans = declaration_spans(manifest);
        assert_eq!(
            spans.get("spare").copied(),
            Some(SourceSpan {
                line_start: 6,
                column_start: 1,
                line_end: 6,
                column_end: 6,
            })
        );
        assert!(spans.contains_key("default"));
        // A name the table does not carry has no position, and the caller falls
        // back to the head of the file rather than to a line that says
        // something else.
        assert_eq!(spans.get("absent"), None);
        assert_eq!(
            head_of_file(),
            SourceSpan {
                line_start: 1,
                column_start: 1,
                line_end: 1,
                column_end: 1,
            }
        );
    }

    /// A table the line scan it replaced could not find: the header carries a
    /// comment, and the entries are still located.
    #[test]
    fn a_features_table_is_located_however_it_is_written() {
        let manifest = "[features] # the table this package publishes\nspare = []\n";
        let spans = declaration_spans(manifest);
        assert_eq!(
            spans.get("spare").map(|span| span.line_start),
            Some(2),
            "{spans:?}"
        );
    }

    #[test]
    fn a_manifest_that_does_not_parse_locates_nothing() {
        assert!(declaration_spans("[features\nspare = []").is_empty());
    }

    #[test]
    fn only_a_plain_entry_names_another_feature() {
        assert_eq!(plain_feature("spare"), Some("spare"));
        assert_eq!(plain_feature("dep:serde"), None);
        assert_eq!(plain_feature("serde/derive"), None);
        assert_eq!(plain_feature("serde?/derive"), None);
    }
}
