use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use cargo_metadata::{Metadata, Package};
use ra_ap_syntax::ast;
use ra_ap_syntax::{AstNode, AstToken, SyntaxNode, SyntaxToken};
use serde::Deserialize;

use super::{Deadline, Observation, Unit, single_name};
use crate::generated::has_generator_header;
use crate::policy::{ActiveRules, 
    RuleDefinition, STRUCTURE_ORPHAN_MODULE_FILE, STRUCTURE_UNREFERENCED_FEATURE,
};
use crate::report::DiagnosticContext;
use crate::source_kernel::{Enumeration, workspace_relative};
use crate::source_text::{SourceSpan, byte_range_span, line_starts};

pub(super) const RULES: [&RuleDefinition; 2] = [
    &STRUCTURE_ORPHAN_MODULE_FILE,
    &STRUCTURE_UNREFERENCED_FEATURE,
];

const WALK_LIMIT: usize = 20_000;

const CANDIDATE_BYTES_LIMIT: u64 = 8_388_608;

#[derive(Debug, Default)]
pub(super) struct Uses {
    included: BTreeSet<String>,
    scripted: BTreeSet<String>,
    features: Vec<Reference>,
}

#[derive(Debug)]
struct Reference {
    feature: String,
    path: String,
    span: SourceSpan,
    context: Option<DiagnosticContext>,
    packages: BTreeSet<String>,
}

pub(super) fn observe(unit: &Unit<'_>, active: &ActiveRules, uses: &mut Uses) {
    let orphans = active.on(&STRUCTURE_ORPHAN_MODULE_FILE);
    let features = active.on(&STRUCTURE_UNREFERENCED_FEATURE);
    if orphans && unit.context == Some(DiagnosticContext::BuildScript) {
        read_scripted_paths(unit, uses);
    }
    if orphans || features {
        for call in &unit.inventory.macro_calls {
            read_macro_call(unit, call, orphans, features, uses);
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

fn read_scripted_paths(unit: &Unit<'_>, uses: &mut Uses) {
    for token in significant(unit.tree.syntax()) {
        if let Some(literal) = string_value(&token)
            && literal.ends_with(".rs")
            && let Some(name) = Path::new(&literal).file_name()
        {
            uses.scripted.insert(name.to_string_lossy().into_owned());
        }
    }
}

fn read_macro_call(
    unit: &Unit<'_>,
    call: &ast::MacroCall,
    orphans: bool,
    features: bool,
    uses: &mut Uses,
) {
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

fn significant(node: &SyntaxNode) -> Vec<SyntaxToken> {
    node.descendants_with_tokens()
        .filter_map(|element| element.into_token())
        .filter(|token| !token.kind().is_trivia())
        .collect()
}

fn string_value(token: &SyntaxToken) -> Option<String> {
    ast::String::cast(token.clone())
        .and_then(|literal| literal.value().ok().map(|value| value.into_owned()))
}

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

pub(super) struct Findings {
    pub(super) observations: Vec<(&'static RuleDefinition, String, Observation)>,
    pub(super) stopped: bool,
    pub(super) orphan_abstentions: usize,
}

pub(super) fn findings(
    metadata: &Metadata,
    enumeration: &Enumeration,
    active: &ActiveRules,
    uses: &Uses,
    deadline: &Deadline,
) -> Findings {
    let Ok(workspace_root) = metadata.workspace_root.as_std_path().canonicalize() else {
        return Findings {
            observations: Vec::new(),
            stopped: false,
            orphan_abstentions: 0,
        };
    };
    let mut observations = Vec::new();
    let mut stopped = false;
    let mut orphan_abstentions = 0;
    if active.on(&STRUCTURE_ORPHAN_MODULE_FILE) {
        (stopped, orphan_abstentions) = orphans(
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
        orphan_abstentions,
    }
}

fn orphans(
    metadata: &Metadata,
    enumeration: &Enumeration,
    uses: &Uses,
    workspace_root: &Path,
    deadline: &Deadline,
    observations: &mut Vec<(&'static RuleDefinition, String, Observation)>,
) -> (bool, usize) {
    let compiled: BTreeSet<&str> = enumeration
        .reached()
        .map(|unit| unit.relative_path())
        .collect();
    let declared_in_macros: BTreeSet<&str> = enumeration
        .reached()
        .filter(|unit| declares_modules_in_a_macro(&unit.tree()))
        .flat_map(|unit| unit.package_ids())
        .collect();

    let mut budget = WALK_LIMIT;
    let mut stopped = false;
    let mut abstentions = 0;
    for package in workspace_packages(metadata) {
        if !enumeration.selects(&package.name) {
            continue;
        }
        let Some(package_directory) = package.manifest_path.parent() else {
            continue;
        };
        let package_directory = package_directory.as_std_path();
        if !enumeration.module_tree_complete(&package.id.repr)
            || declared_in_macros.contains(package.id.repr.as_str())
        {
            abstentions += 1;
            continue;
        }

        let mut candidates = BTreeSet::new();
        for root in module_roots(package, package_directory) {
            stopped |= walk(&root, &mut budget, deadline, &mut candidates);
        }
        for candidate in candidates {
            if deadline.exceeded() {
                return (true, abstentions);
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
            if has_generator_header(&source) {
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
                    context: DiagnosticContext::from_conventional_path(&relative),
                    complexity: None,
                },
            ));
        }
    }
    (stopped, abstentions)
}

fn declares_modules_in_a_macro(tree: &ra_ap_syntax::SourceFile) -> bool {
    tree.syntax()
        .descendants()
        .filter_map(ast::MacroCall::cast)
        .filter_map(|call| call.token_tree())
        .any(|tokens| {
            tokens
                .syntax()
                .descendants_with_tokens()
                .any(|element| element.kind() == ra_ap_syntax::SyntaxKind::MOD_KW)
        })
}

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

fn unreferenced_declarations(
    metadata: &Metadata,
    uses: &Uses,
    workspace_root: &Path,
) -> Vec<(&'static RuleDefinition, String, Observation)> {
    let mut observations = Vec::new();
    for package in workspace_packages(metadata) {
        let optional: BTreeSet<&str> = optional_dependencies(package).collect();
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

fn unknown_references(
    uses: &Uses,
    declared: &BTreeMap<&str, BTreeSet<&str>>,
) -> Vec<(&'static RuleDefinition, String, Observation)> {
    uses.features
        .iter()
        .filter(|reference| {
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

fn plain_feature(entry: &str) -> Option<&str> {
    (!entry.contains('/') && !entry.starts_with("dep:")).then_some(entry)
}

#[derive(Deserialize)]
struct ManifestFeatures {
    features: Option<BTreeMap<toml::Spanned<String>, toml::Value>>,
}

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

const fn head_of_file() -> SourceSpan {
    SourceSpan {
        line_start: 1,
        column_start: 1,
        line_end: 1,
        column_end: 1,
    }
}

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

    fn both() -> ActiveRules {
        ActiveRules::from_rules(RULES)
    }

    fn read(source: &str) -> Uses {
        let mut uses = Uses::default();
        observe(&Unit::probe(source, "src/lib.rs"), &both(), &mut uses);
        uses
    }

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

    #[test]
    fn an_inactive_rule_reads_nothing() {
        let mut uses = Uses::default();
        observe(
            &Unit::probe(
                "#[cfg(feature = \"one\")]\nfn a() {}\ninclude!(\"other.rs\");\n",
                "src/lib.rs",
            ),
            &ActiveRules::default(),
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
            &ActiveRules::from_rules([&STRUCTURE_ORPHAN_MODULE_FILE]),
            &mut orphans_only,
        );
        assert!(orphans_only.features.is_empty());
        assert_eq!(orphans_only.included.len(), 1);
    }

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
