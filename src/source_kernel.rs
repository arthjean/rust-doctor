use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use cargo_metadata::Metadata;
use ra_ap_syntax::ast::{self, HasAttrs, HasName, LiteralKind};
use ra_ap_syntax::{AstNode, Edition, SourceFile, SyntaxNode, TextRange};

use crate::policy::{PolicyPlan, Producer, RuleDefinition};
use crate::report::DiagnosticContext;

mod aliases;
mod detectors;

use aliases::AliasMap;
use detectors::{CrateAliases, DETECTORS, Detection, Detector};

const FILE_BYTES_LIMIT: u64 = 8_388_608;
const TOTAL_BYTES_LIMIT: u64 = 268_435_456;
const UNIT_LIMIT: usize = 20_000;
const MODULE_DEPTH_LIMIT: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Candidate {
    pub(crate) definition: &'static RuleDefinition,
    pub(crate) message: &'static str,
    pub(crate) package: Option<String>,
    pub(crate) target: Option<String>,
    pub(crate) path: String,
    pub(crate) span: SourceSpan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct SourceSpan {
    pub(crate) line_start: usize,
    pub(crate) column_start: usize,
    pub(crate) line_end: usize,
    pub(crate) column_end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct SourceError {
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

#[derive(Debug, Default)]
pub(crate) struct SourceScan {
    pub(crate) candidates: Vec<Candidate>,
    pub(crate) errors: Vec<SourceError>,
    #[allow(dead_code, reason = "read only by the tests that assert the pass did the work")]
    pub(crate) counters: SourceCounters,
}

#[derive(Debug, Default)]
pub(crate) struct SourceCounters {
    pub(crate) files_read: usize,
    pub(crate) files_parsed: usize,
    pub(crate) bytes_read: u64,
    pub(crate) nodes_visited: usize,
    /// Solicitations per rule. The registry carries N detectors, so the
    /// counter is indexed by identifier rather than by named field.
    pub(crate) predicates: BTreeMap<&'static str, usize>,
}

impl SourceCounters {
    #[cfg(test)]
    fn solicitations(&self, id: &str) -> usize {
        self.predicates.get(id).copied().unwrap_or_default()
    }
}

#[derive(Debug, Clone, Copy)]
struct Limits {
    file_bytes: u64,
    total_bytes: u64,
    units: usize,
    module_depth: usize,
    alias_bindings: usize,
}

const LIMITS: Limits = Limits {
    file_bytes: FILE_BYTES_LIMIT,
    total_bytes: TOTAL_BYTES_LIMIT,
    units: UNIT_LIMIT,
    module_depth: MODULE_DEPTH_LIMIT,
    alias_bindings: aliases::BINDING_LIMIT,
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Identity {
    path: PathBuf,
    edition: Edition,
}

/// Identity of a unit's reach. It carries only the identity of the package and
/// of the target: crate aliases live in a table indexed by package, so adding
/// a detector aimed at another crate does not widen this shared structure.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Reachability {
    package_id: String,
    package_name: String,
    target_key: String,
    target_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct WorkItem {
    lexical_path: PathBuf,
    edition: Edition,
    module_directory: PathBuf,
    reachability: Reachability,
    depth: usize,
}

#[derive(Debug)]
pub(crate) struct SourceUnit {
    source: String,
    parse: ra_ap_syntax::Parse<SourceFile>,
    edition: Edition,
    error_ranges: Vec<TextRange>,
    relative_path: String,
    reachability: BTreeSet<Reachability>,
    traversals: BTreeSet<(Reachability, PathBuf)>,
}

impl SourceUnit {
    pub(crate) fn relative_path(&self) -> &str {
        &self.relative_path
    }

    pub(crate) fn tree(&self) -> SourceFile {
        self.parse.tree()
    }

    pub(crate) fn parses_cleanly(&self) -> bool {
        self.error_ranges.is_empty()
    }

    pub(crate) fn source(&self) -> &str {
        &self.source
    }

    pub(crate) fn package(&self) -> Option<String> {
        unique_package(&self.reachability)
    }

    pub(crate) fn target(&self) -> Option<String> {
        unique_target(&self.reachability)
    }

    /// Non-production context of the unit, when every target that reaches it
    /// agrees on one. A file the library and an integration test both reach is
    /// left unmarked, because silencing shipped code is the expensive mistake.
    pub(crate) fn context(
        &self,
        contexts: &BTreeMap<String, Option<DiagnosticContext>>,
    ) -> Option<DiagnosticContext> {
        let mut marks = self
            .reachability
            .iter()
            .map(|reach| contexts.get(&reach.target_key).copied().flatten());
        let first = marks.next().flatten()?;
        marks.all(|mark| mark == Some(first)).then_some(first)
    }
}

/// Files the workspace reaches, read and parsed once for every producer that
/// analyses source text. The walk is the expensive part, so it happens here and
/// the producers only read the result.
#[derive(Debug, Default)]
pub(crate) struct Enumeration {
    units: BTreeMap<Identity, SourceUnit>,
    tables: BTreeMap<String, CrateAliases>,
    contexts: BTreeMap<String, Option<DiagnosticContext>>,
    errors: Vec<SourceError>,
    counters: SourceCounters,
}

impl Enumeration {
    pub(crate) fn units(&self) -> impl Iterator<Item = &SourceUnit> {
        self.units.values()
    }

    pub(crate) const fn contexts(&self) -> &BTreeMap<String, Option<DiagnosticContext>> {
        &self.contexts
    }
}

/// Does any producer reading source text still have an active rule? When none
/// does, the workspace is never walked at all.
pub(crate) fn enumeration_required(plan: &PolicyPlan) -> bool {
    [Producer::SourceKernel, Producer::Structure]
        .into_iter()
        .any(|producer| plan.active_rules(producer).next().is_some())
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct CandidateKey {
    code: &'static str,
    path: String,
    span: SourceSpan,
    message: &'static str,
}

pub(crate) fn enumerate(metadata: &Metadata) -> Enumeration {
    enumerate_with_limits(metadata, LIMITS)
}

fn enumerate_with_limits(metadata: &Metadata, limits: Limits) -> Enumeration {
    let workspace_root = match metadata.workspace_root.as_std_path().canonicalize() {
        Ok(root) => root,
        Err(_) => {
            return Enumeration {
                errors: vec![SourceError {
                    code: "read-failed",
                    message: "Workspace source root could not be resolved.".to_owned(),
                }],
                ..Enumeration::default()
            };
        }
    };
    let mut errors = Vec::new();
    let mut queue = source_roots(metadata, &mut errors);
    let mut units = BTreeMap::<Identity, SourceUnit>::new();
    let mut counters = SourceCounters::default();

    while let Some(work) = queue.pop_first() {
        if work.depth > limits.module_depth {
            push_limit_error(&mut errors, "module-depth", limits.module_depth);
            continue;
        }

        if !lexically_within_workspace(&workspace_root, &work.lexical_path) {
            let path = safe_lexical_path(&workspace_root, &work.lexical_path);
            push_error(
                &mut errors,
                "path-outside-workspace",
                format!("Source path \"{path}\" resolves outside the workspace."),
            );
            continue;
        }

        let canonical = match work.lexical_path.canonicalize() {
            Ok(path) => path,
            Err(_) => {
                let path = safe_lexical_path(&workspace_root, &work.lexical_path);
                push_error(
                    &mut errors,
                    "read-failed",
                    format!("Source path \"{path}\" could not be resolved."),
                );
                continue;
            }
        };
        if !canonical.starts_with(&workspace_root) {
            let path = safe_lexical_path(&workspace_root, &work.lexical_path);
            push_error(
                &mut errors,
                "path-outside-workspace",
                format!("Source path \"{path}\" resolves outside the workspace."),
            );
            continue;
        }

        let identity = Identity {
            path: canonical.clone(),
            edition: work.edition,
        };
        if !units.contains_key(&identity) {
            if units.len() >= limits.units {
                push_limit_error(&mut errors, "source-units", limits.units);
                break;
            }
            let error_count = errors.len();
            let loaded = load_unit(
                &identity,
                &workspace_root,
                &limits,
                &mut counters,
                &mut errors,
            );
            let Some(unit) = loaded else {
                if errors[error_count..].iter().any(|error| {
                    error.code == "limit-exceeded" && error.message.contains("total-bytes")
                }) {
                    break;
                }
                continue;
            };
            units.insert(identity.clone(), unit);
        }

        let traversal = (work.reachability.clone(), work.module_directory.clone());
        let Some(unit) = units.get_mut(&identity) else {
            continue;
        };
        unit.reachability.insert(work.reachability.clone());
        if !unit.traversals.insert(traversal) {
            continue;
        }

        let requests = module_requests(unit, &work, &workspace_root, &mut errors);
        queue.extend(requests);
    }

    Enumeration {
        units,
        tables: crate_alias_tables(metadata),
        contexts: target_contexts(metadata),
        errors,
        counters,
    }
}

pub(crate) fn inspect(enumeration: &Enumeration, plan: &PolicyPlan) -> SourceScan {
    inspect_with_limits(enumeration, LIMITS, plan)
}

fn inspect_with_limits(
    enumeration: &Enumeration,
    limits: Limits,
    plan: &PolicyPlan,
) -> SourceScan {
    let detectors: Vec<&'static Detector> = DETECTORS
        .iter()
        .copied()
        .filter(|detector| plan.is_active(detector.definition.id))
        .collect();
    if detectors.is_empty() {
        return SourceScan::default();
    }

    let mut errors = enumeration.errors.clone();
    let mut counters = SourceCounters {
        files_read: enumeration.counters.files_read,
        files_parsed: enumeration.counters.files_parsed,
        bytes_read: enumeration.counters.bytes_read,
        ..SourceCounters::default()
    };
    let mut candidates = BTreeMap::<CandidateKey, Candidate>::new();
    for unit in enumeration.units.values() {
        analyze_unit(
            unit,
            &detectors,
            &enumeration.tables,
            &limits,
            &mut candidates,
            &mut counters,
            &mut errors,
        );
    }
    errors.sort();
    errors.dedup();

    SourceScan {
        candidates: candidates.into_values().collect(),
        errors,
        counters,
    }
}

fn source_roots(metadata: &Metadata, errors: &mut Vec<SourceError>) -> BTreeSet<WorkItem> {
    let mut roots = BTreeSet::new();
    for package in metadata
        .packages
        .iter()
        .filter(|package| metadata.workspace_members.contains(&package.id))
    {
        let Some(edition) = syntax_edition(package.edition.to_string().as_str()) else {
            push_error(
                errors,
                "parse-error",
                format!(
                    "Package \"{}\" uses unsupported Rust edition \"{}\".",
                    package.name, package.edition
                ),
            );
            continue;
        };
        for (target_index, target) in package.targets.iter().enumerate() {
            let path = target.src_path.as_std_path().to_path_buf();
            let module_directory = path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| path.clone());
            roots.insert(WorkItem {
                lexical_path: path,
                edition,
                module_directory,
                reachability: Reachability {
                    package_id: package.id.repr.clone(),
                    package_name: package.name.to_string(),
                    target_key: format!("{}:{target_index}", package.id.repr),
                    target_name: target.name.clone(),
                },
                depth: 0,
            });
        }
    }
    roots
}

fn crate_alias_tables(metadata: &Metadata) -> BTreeMap<String, CrateAliases> {
    metadata
        .packages
        .iter()
        .filter(|package| metadata.workspace_members.contains(&package.id))
        .map(|package| (package.id.repr.clone(), detectors::crate_aliases(package)))
        .collect()
}

/// Non-production mark of every workspace target, keyed the way `Reachability`
/// names it. The kind comes from Cargo, so a producer never guesses a context
/// from a path.
fn target_contexts(metadata: &Metadata) -> BTreeMap<String, Option<DiagnosticContext>> {
    metadata
        .packages
        .iter()
        .filter(|package| metadata.workspace_members.contains(&package.id))
        .flat_map(|package| {
            package
                .targets
                .iter()
                .enumerate()
                .map(move |(target_index, target)| {
                    let kinds: Vec<String> =
                        target.kind.iter().map(ToString::to_string).collect();
                    (
                        format!("{}:{target_index}", package.id.repr),
                        DiagnosticContext::from_target_kinds(&kinds),
                    )
                })
        })
        .collect()
}

fn syntax_edition(edition: &str) -> Option<Edition> {
    match edition {
        "2015" => Some(Edition::Edition2015),
        "2018" => Some(Edition::Edition2018),
        "2021" => Some(Edition::Edition2021),
        "2024" => Some(Edition::Edition2024),
        _ => None,
    }
}

fn load_unit(
    identity: &Identity,
    workspace_root: &Path,
    limits: &Limits,
    counters: &mut SourceCounters,
    errors: &mut Vec<SourceError>,
) -> Option<SourceUnit> {
    let relative_path = relative_path(workspace_root, &identity.path);
    let metadata = match fs::metadata(&identity.path) {
        Ok(metadata) if metadata.is_file() => metadata,
        Ok(_) => {
            push_error(
                errors,
                "read-failed",
                format!("Source path \"{relative_path}\" is not a regular file."),
            );
            return None;
        }
        Err(_) => {
            push_error(
                errors,
                "read-failed",
                format!("Source path \"{relative_path}\" could not be inspected."),
            );
            return None;
        }
    };
    if metadata.len() > limits.file_bytes {
        push_limit_error(errors, "file-bytes", limits.file_bytes);
        return None;
    }
    if counters.bytes_read.saturating_add(metadata.len()) > limits.total_bytes {
        push_limit_error(errors, "total-bytes", limits.total_bytes);
        return None;
    }

    let file = match File::open(&identity.path) {
        Ok(file) => file,
        Err(_) => {
            push_error(
                errors,
                "read-failed",
                format!("Source path \"{relative_path}\" could not be opened."),
            );
            return None;
        }
    };
    let remaining = limits.total_bytes.saturating_sub(counters.bytes_read);
    let read_limit = limits.file_bytes.min(remaining).saturating_add(1);
    let mut bytes = Vec::with_capacity(metadata.len().min(read_limit) as usize);
    if file.take(read_limit).read_to_end(&mut bytes).is_err() {
        push_error(
            errors,
            "read-failed",
            format!("Source path \"{relative_path}\" could not be read."),
        );
        return None;
    }
    if bytes.len() as u64 > limits.file_bytes {
        push_limit_error(errors, "file-bytes", limits.file_bytes);
        return None;
    }
    if counters.bytes_read.saturating_add(bytes.len() as u64) > limits.total_bytes {
        push_limit_error(errors, "total-bytes", limits.total_bytes);
        return None;
    }
    let source = match String::from_utf8(bytes) {
        Ok(source) => source,
        Err(_) => {
            push_error(
                errors,
                "read-failed",
                format!("Source path \"{relative_path}\" is not valid UTF-8."),
            );
            return None;
        }
    };
    counters.files_read += 1;
    counters.bytes_read += source.len() as u64;

    let parse = SourceFile::parse(&source, identity.edition);
    counters.files_parsed += 1;
    let parse_errors = parse.errors();
    if !parse_errors.is_empty() {
        push_error(
            errors,
            "parse-error",
            format!(
                "Source path \"{relative_path}\" contains {} parse errors.",
                parse_errors.len()
            ),
        );
    }

    Some(SourceUnit {
        source,
        parse,
        edition: identity.edition,
        error_ranges: parse_errors
            .into_iter()
            .map(|error| error.range())
            .collect(),
        relative_path,
        reachability: BTreeSet::new(),
        traversals: BTreeSet::new(),
    })
}

fn module_requests(
    unit: &SourceUnit,
    work: &WorkItem,
    workspace_root: &Path,
    errors: &mut Vec<SourceError>,
) -> Vec<WorkItem> {
    let tree = unit.parse.tree();
    let mut requests = Vec::new();
    for module in tree.syntax().descendants().filter_map(ast::Module::cast) {
        if module.item_list().is_some()
            || intersects_errors(module.syntax().text_range(), &unit.error_ranges)
        {
            continue;
        }
        let Some(name) = module.name().map(|name| name.text().to_string()) else {
            continue;
        };
        let mut base = work.module_directory.clone();
        let mut inline_ancestors: Vec<_> = module
            .syntax()
            .ancestors()
            .skip(1)
            .filter_map(ast::Module::cast)
            .filter(|ancestor| ancestor.item_list().is_some())
            .filter_map(|ancestor| ancestor.name().map(|name| name.text().to_string()))
            .collect();
        inline_ancestors.reverse();
        for ancestor in inline_ancestors {
            base.push(ancestor);
        }

        let candidates = match path_attribute(&module) {
            PathAttribute::Absent => vec![
                base.join(format!("{name}.rs")),
                base.join(&name).join("mod.rs"),
            ],
            PathAttribute::Literal(path) => vec![base.join(path)],
            PathAttribute::Invalid => {
                push_error(
                    errors,
                    "module-not-found",
                    format!(
                        "Module \"{name}\" declared in \"{}\" has no supported literal path.",
                        unit.relative_path
                    ),
                );
                continue;
            }
        };
        let mut confined = Vec::new();
        for path in candidates {
            if lexically_within_workspace(workspace_root, &path) {
                confined.push(path);
            } else {
                let path = safe_lexical_path(workspace_root, &path);
                push_error(
                    errors,
                    "path-outside-workspace",
                    format!("Source path \"{path}\" resolves outside the workspace."),
                );
            }
        }
        if confined.is_empty() {
            continue;
        }
        let existing: Vec<_> = confined
            .iter()
            .filter(|path| path_exists(path))
            .cloned()
            .collect();
        if existing.len() > 1 {
            push_error(
                errors,
                "module-ambiguous",
                format!(
                    "Module \"{name}\" declared in \"{}\" has both supported file forms.",
                    unit.relative_path
                ),
            );
            continue;
        }
        let Some(path) = existing.into_iter().next() else {
            push_error(
                errors,
                "module-not-found",
                format!(
                    "Module \"{name}\" declared in \"{}\" could not be resolved.",
                    unit.relative_path
                ),
            );
            continue;
        };
        let resolved_path = path.canonicalize().unwrap_or_else(|_| path.clone());
        let module_directory = module_directory_for_file(&resolved_path);
        requests.push(WorkItem {
            lexical_path: path,
            edition: work.edition,
            module_directory,
            reachability: work.reachability.clone(),
            depth: work.depth + 1,
        });
    }
    requests
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PathAttribute {
    Absent,
    Literal(String),
    Invalid,
}

fn path_attribute(module: &ast::Module) -> PathAttribute {
    let mut paths = module.attrs().filter_map(|attribute| {
        let meta = attribute.meta()?;
        if meta.simple_name().as_deref() != Some("path") {
            return None;
        }
        Some(match meta {
            ast::Meta::KeyValueMeta(key_value) => key_value.expr().and_then(literal_string),
            _ => None,
        })
    });
    match (paths.next(), paths.next()) {
        (None, _) => PathAttribute::Absent,
        (Some(Some(path)), None) => PathAttribute::Literal(path),
        _ => PathAttribute::Invalid,
    }
}

fn path_exists(path: &Path) -> bool {
    match path.symlink_metadata() {
        Ok(_) => true,
        Err(error) => error.kind() != std::io::ErrorKind::NotFound,
    }
}

fn module_directory_for_file(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or(path);
    if path.file_name().and_then(|name| name.to_str()) == Some("mod.rs") {
        parent.to_path_buf()
    } else {
        path.file_stem()
            .map(|stem| parent.join(stem))
            .unwrap_or_else(|| parent.to_path_buf())
    }
}

/// Walks the CST exactly once and solicits every active detector on the nodes
/// of the kind it declared.
fn analyze_unit(
    unit: &SourceUnit,
    detectors: &[&'static Detector],
    tables: &BTreeMap<String, CrateAliases>,
    limits: &Limits,
    candidates: &mut BTreeMap<CandidateKey, Candidate>,
    counters: &mut SourceCounters,
    errors: &mut Vec<SourceError>,
) {
    let tree = unit.parse.tree();
    let aliases = AliasMap::build(&tree, &unit.error_ranges, limits.alias_bindings);
    if aliases.saturated() {
        push_limit_error(errors, "alias-bindings", limits.alias_bindings);
    }
    let crates = detectors::shared_crate_aliases(
        unit.reachability
            .iter()
            .map(|reach| tables.get(&reach.package_id)),
    );
    let context = detectors::Context {
        aliases: &aliases,
        crates: &crates,
        error_ranges: &unit.error_ranges,
        edition: unit.edition,
        test_path: path_contains_tests_segment(&unit.relative_path),
    };
    let emitter = Emitter {
        package: unique_package(&unit.reachability),
        target: unique_target(&unit.reachability),
        path: &unit.relative_path,
        line_starts: line_starts(&unit.source),
        source: &unit.source,
    };

    for node in tree.syntax().descendants() {
        counters.nodes_visited += 1;
        for detector in detectors
            .iter()
            .filter(|detector| detector.node == node.kind())
        {
            *counters
                .predicates
                .entry(detector.definition.id)
                .or_default() += 1;
            if let Some(detection) = (detector.inspect)(&context, &node) {
                emitter.insert(candidates, detector.definition, &detection);
            }
        }
    }
}

fn literal_string(expression: ast::Expr) -> Option<String> {
    let ast::Expr::Literal(literal) = expression else {
        return None;
    };
    let LiteralKind::String(string) = literal.kind() else {
        return None;
    };
    string.value().ok().map(|value| value.into_owned())
}

/// Emission context of a unit. It carries what a candidate must take back from
/// its unit, so adding a detector adds no parameter.
struct Emitter<'a> {
    package: Option<String>,
    target: Option<String>,
    path: &'a str,
    line_starts: Vec<usize>,
    source: &'a str,
}

impl Emitter<'_> {
    fn insert(
        &self,
        candidates: &mut BTreeMap<CandidateKey, Candidate>,
        definition: &'static RuleDefinition,
        detection: &Detection,
    ) {
        if detection.range.is_empty() {
            return;
        }
        let span = source_span(detection.range, &self.line_starts, self.source);
        let key = CandidateKey {
            code: definition.id,
            path: self.path.to_owned(),
            span,
            message: detection.message,
        };
        let candidate = Candidate {
            definition,
            message: detection.message,
            package: self.package.clone(),
            target: self.target.clone(),
            path: self.path.to_owned(),
            span,
        };
        match candidates.get_mut(&key) {
            Some(existing) => {
                merge_context(&mut existing.package, candidate.package);
                merge_context(&mut existing.target, candidate.target);
            }
            None => {
                candidates.insert(key, candidate);
            }
        }
    }
}

fn merge_context(existing: &mut Option<String>, incoming: Option<String>) {
    if existing.as_ref() != incoming.as_ref() {
        *existing = None;
    }
}

fn unique_package(reachability: &BTreeSet<Reachability>) -> Option<String> {
    let packages: BTreeSet<_> = reachability
        .iter()
        .map(|reach| (&reach.package_id, &reach.package_name))
        .collect();
    if packages.len() == 1 {
        packages.into_iter().next().map(|(_, name)| name.clone())
    } else {
        None
    }
}

fn unique_target(reachability: &BTreeSet<Reachability>) -> Option<String> {
    let targets: BTreeSet<_> = reachability
        .iter()
        .map(|reach| (&reach.target_key, &reach.target_name))
        .collect();
    if targets.len() == 1 {
        targets.into_iter().next().map(|(_, name)| name.clone())
    } else {
        None
    }
}

/// Text of a node with every trivia token dropped, so an attribute written
/// across three lines compares equal to the same attribute written on one.
pub(crate) fn compact(node: &SyntaxNode) -> String {
    node.descendants_with_tokens()
        .filter_map(|element| element.into_token())
        .filter(|token| !token.kind().is_trivia())
        .map(|token| token.text().to_string())
        .collect()
}

fn intersects_errors(range: TextRange, errors: &[TextRange]) -> bool {
    let start = u32::from(range.start());
    let end = u32::from(range.end());
    errors.iter().any(|error| {
        let error_start = u32::from(error.start());
        let error_end = u32::from(error.end());
        if error_start == error_end {
            error_start >= start && error_start <= end
        } else {
            error_start < end && error_end > start
        }
    })
}

pub(crate) fn line_starts(source: &str) -> Vec<usize> {
    std::iter::once(0)
        .chain(
            source
                .bytes()
                .enumerate()
                .filter_map(|(index, byte)| (byte == b'\n').then_some(index + 1)),
        )
        .collect()
}

pub(crate) fn source_span(range: TextRange, line_starts: &[usize], source: &str) -> SourceSpan {
    let (line_start, column_start) = source_position(range.start().into(), line_starts, source);
    let (line_end, column_end) = source_position(range.end().into(), line_starts, source);
    SourceSpan {
        line_start,
        column_start,
        line_end,
        column_end,
    }
}

fn source_position(offset: usize, line_starts: &[usize], source: &str) -> (usize, usize) {
    let bounded = offset.min(source.len());
    let line_index = line_starts.partition_point(|start| *start <= bounded) - 1;
    let column = source[line_starts[line_index]..bounded].chars().count() + 1;
    (line_index + 1, column)
}

fn path_contains_tests_segment(path: &str) -> bool {
    Path::new(path)
        .components()
        .any(|component| component == Component::Normal("tests".as_ref()))
}

fn relative_path(workspace_root: &Path, path: &Path) -> String {
    path.strip_prefix(workspace_root)
        .ok()
        .map(|relative| relative.to_string_lossy().replace('\\', "/"))
        .filter(|relative| !relative.is_empty())
        .unwrap_or_else(|| ".".to_owned())
}

fn safe_lexical_path(workspace_root: &Path, path: &Path) -> String {
    path.strip_prefix(workspace_root)
        .ok()
        .filter(|relative| {
            relative
                .components()
                .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
        })
        .map(|relative| relative.to_string_lossy().replace('\\', "/"))
        .filter(|relative| !relative.is_empty())
        .or_else(|| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "<source>".to_owned())
}

fn lexically_within_workspace(workspace_root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(workspace_root) else {
        return false;
    };
    let mut depth = 0_usize;
    for component in relative.components() {
        match component {
            Component::Normal(_) => depth += 1,
            Component::CurDir => {}
            Component::ParentDir if depth > 0 => depth -= 1,
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return false,
        }
    }
    true
}

fn push_limit_error(errors: &mut Vec<SourceError>, name: &'static str, maximum: impl ToString) {
    push_error(
        errors,
        "limit-exceeded",
        format!(
            "Source limit \"{name}\" exceeded (maximum {}).",
            maximum.to_string()
        ),
    );
}

fn push_error(errors: &mut Vec<SourceError>, code: &'static str, message: String) {
    errors.push(SourceError { code, message });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::{
        PolicyInput, Producer, RuleLevel, RuleTier, SOURCE_DISABLED_TLS, SOURCE_DYNAMIC_SHELL,
        STRUCTURE_DUPLICATE_FUNCTION_BODY, STRUCTURE_NEAR_DUPLICATE_FUNCTION_BODY,
        STRUCTURE_UNREASONED_ALLOW,
    };
    use cargo_metadata::MetadataCommand;
    use ra_ap_syntax::{SyntaxKind, SyntaxNode};

    fn inspect(metadata: &Metadata) -> SourceScan {
        inspect_for_plan(metadata, &PolicyPlan::default())
    }

    /// Reproduces the gate `execution` applies: the workspace is walked only
    /// when a producer reading source text still has an active rule, so a
    /// pruned policy proves an absence of IO rather than an empty result.
    fn inspect_for_plan(metadata: &Metadata, plan: &PolicyPlan) -> SourceScan {
        if !enumeration_required(plan) {
            return SourceScan::default();
        }
        super::inspect(&enumerate(metadata), plan)
    }

    fn inspect_with_limits(metadata: &Metadata, limits: Limits) -> SourceScan {
        super::inspect_with_limits(
            &enumerate_with_limits(metadata, limits),
            limits,
            &PolicyPlan::default(),
        )
    }

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/source-kernel")
            .join(name)
    }

    fn metadata(name: &str) -> Metadata {
        let manifest = fixture(name).join("Cargo.toml");
        let mut command = MetadataCommand::new();
        command
            .manifest_path(manifest)
            .no_deps()
            .other_options(["--offline".to_owned(), "--locked".to_owned()]);
        command.exec().unwrap()
    }

    static SILENT_RULE: RuleDefinition = RuleDefinition {
        id: "rust_doctor::source::silent_probe",
        category: "security",
        producer: Producer::SourceKernel,
        default_level: RuleLevel::Warn,
        tier: RuleTier::P3,
        help: "Synthetic registry proof.",
    };

    /// A registered detector that never emits: it proves that a silent
    /// detector costs one solicitation and nothing else.
    static SILENT: Detector = Detector {
        definition: &SILENT_RULE,
        node: SyntaxKind::SOURCE_FILE,
        inspect: silent,
    };

    const REGISTERED: [&Detector; 2] = DETECTORS;

    fn silent(_: &detectors::Context<'_>, _: &SyntaxNode) -> Option<Detection> {
        None
    }

    /// Synthetic unit: the registry is exercised without touching the disk.
    fn synthetic_unit(source: &str) -> SourceUnit {
        let parse = SourceFile::parse(source, Edition::Edition2024);
        SourceUnit {
            source: source.to_owned(),
            error_ranges: parse.errors().iter().map(|error| error.range()).collect(),
            parse,
            edition: Edition::Edition2024,
            relative_path: "src/lib.rs".to_owned(),
            reachability: BTreeSet::from([Reachability {
                package_id: "synthetic".to_owned(),
                package_name: "synthetic".to_owned(),
                target_key: "synthetic:0".to_owned(),
                target_name: "synthetic".to_owned(),
            }]),
            traversals: BTreeSet::new(),
        }
    }

    fn analyze(
        source: &str,
        detectors: &[&'static Detector],
    ) -> (Vec<&'static str>, SourceCounters) {
        let (candidates, counters, _) = analyze_with_limits(source, detectors, LIMITS);
        (candidates, counters)
    }

    fn analyze_with_limits(
        source: &str,
        detectors: &[&'static Detector],
        limits: Limits,
    ) -> (Vec<&'static str>, SourceCounters, Vec<SourceError>) {
        let unit = synthetic_unit(source);
        let tables = BTreeMap::from([(
            "synthetic".to_owned(),
            CrateAliases::from([("http_client".to_owned(), "reqwest".to_owned())]),
        )]);
        let mut candidates = BTreeMap::new();
        let mut counters = SourceCounters::default();
        let mut errors = Vec::new();
        analyze_unit(
            &unit,
            detectors,
            &tables,
            &limits,
            &mut candidates,
            &mut counters,
            &mut errors,
        );
        let codes = candidates
            .into_values()
            .map(|candidate| candidate.definition.id)
            .collect();
        (codes, counters, errors)
    }

    #[test]
    fn pinned_parser_supports_all_target_editions_and_recoverable_errors() {
        let valid = "fn main() { let value = 1; }";
        for edition in [
            Edition::Edition2015,
            Edition::Edition2018,
            Edition::Edition2021,
            Edition::Edition2024,
        ] {
            let parse = SourceFile::parse(valid, edition);
            assert!(parse.errors().is_empty());
            assert_eq!(
                usize::from(parse.tree().syntax().text_range().len()),
                valid.len()
            );
        }

        let recoverable = "fn valid() {}\nfn broken( {\nfn after() {}";
        let parse = SourceFile::parse(recoverable, Edition::Edition2024);
        let errors = parse.errors();
        assert!(!errors.is_empty());
        assert_eq!(
            usize::from(parse.tree().syntax().text_range().len()),
            recoverable.len()
        );
        assert!(errors.iter().all(|error| {
            usize::from(error.range().start()) <= recoverable.len()
                && usize::from(error.range().end()) <= recoverable.len()
        }));
    }

    #[test]
    fn producer_uses_the_two_canonical_catalog_entries() {
        let definitions: Vec<_> = PolicyPlan::default()
            .active_rules(Producer::SourceKernel)
            .map(|(definition, _)| definition.id)
            .collect();
        assert_eq!(
            definitions,
            [SOURCE_DISABLED_TLS.id, SOURCE_DYNAMIC_SHELL.id]
        );
    }

    #[test]
    fn policy_prunes_source_io_and_each_inactive_predicate() {
        let metadata = metadata("precision");
        let all_off = PolicyInput::default()
            .with_rule(SOURCE_DISABLED_TLS.id, RuleLevel::Off)
            .with_rule(SOURCE_DYNAMIC_SHELL.id, RuleLevel::Off)
            .with_rule(STRUCTURE_UNREASONED_ALLOW.id, RuleLevel::Off)
            .with_rule(STRUCTURE_DUPLICATE_FUNCTION_BODY.id, RuleLevel::Off)
            .with_rule(STRUCTURE_NEAR_DUPLICATE_FUNCTION_BODY.id, RuleLevel::Off);
        let all_off = PolicyPlan::compile(&all_off).expect("policy should compile");
        assert!(!enumeration_required(&all_off));
        let scan = inspect_for_plan(&metadata, &all_off);
        assert!(scan.candidates.is_empty());
        assert!(scan.errors.is_empty());
        assert_eq!(scan.counters.files_read, 0);
        assert_eq!(scan.counters.files_parsed, 0);
        assert_eq!(scan.counters.bytes_read, 0);
        assert_eq!(scan.counters.nodes_visited, 0);
        assert!(scan.counters.predicates.is_empty());

        let shell_off = PolicyInput::default().with_rule(SOURCE_DYNAMIC_SHELL.id, RuleLevel::Off);
        let shell_off = PolicyPlan::compile(&shell_off).expect("policy should compile");
        let scan = inspect_for_plan(&metadata, &shell_off);
        assert!(scan.counters.files_read > 0);
        assert!(scan.counters.solicitations(SOURCE_DISABLED_TLS.id) > 0);
        assert_eq!(scan.counters.solicitations(SOURCE_DYNAMIC_SHELL.id), 0);
        assert!(
            scan.candidates
                .iter()
                .all(|candidate| candidate.definition.id != SOURCE_DYNAMIC_SHELL.id)
        );

        let tls_off = PolicyInput::default().with_rule(SOURCE_DISABLED_TLS.id, RuleLevel::Off);
        let tls_off = PolicyPlan::compile(&tls_off).expect("policy should compile");
        let scan = inspect_for_plan(&metadata, &tls_off);
        assert_eq!(scan.counters.solicitations(SOURCE_DISABLED_TLS.id), 0);
        assert!(scan.counters.solicitations(SOURCE_DYNAMIC_SHELL.id) > 0);
        assert!(
            scan.candidates
                .iter()
                .all(|candidate| candidate.definition.id != SOURCE_DISABLED_TLS.id)
        );
    }

    /// The imported form and the fully qualified form go through the same
    /// mechanism, and an undecidable provenance silences the detector.
    #[test]
    fn detectors_resolve_written_paths_through_the_alias_map() {
        let both = "use std::process::Command;
fn run(user: &str) {
    let _ = Command::new(\"sh\").arg(\"-c\").arg(format!(\"echo {user}\"));
    let _ = std::process::Command::new(\"sh\").arg(\"-c\").arg(format!(\"echo {user}\"));
}";
        assert_eq!(
            analyze(both, &REGISTERED).0,
            [SOURCE_DYNAMIC_SHELL.id, SOURCE_DYNAMIC_SHELL.id]
        );

        let renamed = "use http_client::Client;
fn build() {
    let _ = Client::builder().danger_accept_invalid_certs(true);
}";
        assert_eq!(analyze(renamed, &REGISTERED).0, [SOURCE_DISABLED_TLS.id]);

        for abstained in [
            "use std::process::*;
fn run(user: &str) { let _ = Command::new(\"sh\").arg(\"-c\").arg(format!(\"echo {user}\")); }",
            "fn run(user: &str) {
    struct Command;
    let _ = Command::new(\"sh\").arg(\"-c\").arg(format!(\"echo {user}\"));
}",
            "use unknown_crate::Client;
fn build() { let _ = Client::builder().danger_accept_invalid_certs(true); }",
            "fn build() { let _ = other_alias::Client::builder().danger_accept_invalid_certs(true); }",
        ] {
            assert!(analyze(abstained, &REGISTERED).0.is_empty(), "{abstained}");
        }
    }

    /// The walk is single and independent of the registry: the number of
    /// visited nodes depends neither on the number of detectors nor on their
    /// order, and a silent detector does not change the result.
    #[test]
    fn the_registry_shares_one_traversal_and_stays_order_independent() {
        let source = "use std::process::Command;
fn run(user: &str) {
    let _ = Command::new(\"sh\").arg(\"-c\").arg(format!(\"echo {user}\"));
    let _ = http_client::Client::builder().danger_accept_invalid_certs(true);
}";
        let nodes = SourceFile::parse(source, Edition::Edition2024)
            .tree()
            .syntax()
            .descendants()
            .count();
        let method_calls = SourceFile::parse(source, Edition::Edition2024)
            .tree()
            .syntax()
            .descendants()
            .filter(|node| node.kind() == SyntaxKind::METHOD_CALL_EXPR)
            .count();

        let (expected, counters) = analyze(source, &REGISTERED);
        assert_eq!(expected, [SOURCE_DISABLED_TLS.id, SOURCE_DYNAMIC_SHELL.id]);
        assert_eq!(counters.nodes_visited, nodes);
        for detector in REGISTERED {
            assert_eq!(
                counters.solicitations(detector.definition.id),
                method_calls,
                "{}",
                detector.definition.id
            );
        }

        let permuted = analyze(source, &[REGISTERED[1], REGISTERED[0]]);
        assert_eq!(permuted.0, expected);
        assert_eq!(permuted.1.nodes_visited, nodes);

        let single = analyze(source, &[REGISTERED[0]]);
        assert_eq!(single.1.nodes_visited, nodes);
        assert_eq!(single.1.solicitations(SOURCE_DYNAMIC_SHELL.id), 0);

        // A registered detector that emits nothing leaves the result unchanged.
        let with_silent = analyze(source, &[REGISTERED[0], &SILENT, REGISTERED[1]]);
        assert_eq!(with_silent.0, expected);
        assert_eq!(with_silent.1.nodes_visited, nodes);
        assert_eq!(with_silent.1.solicitations(SILENT_RULE.id), 1);
    }

    /// A unit whose alias map saturates is still analysed, but no provenance
    /// is decidable in it any more.
    #[test]
    fn a_saturated_alias_map_reports_a_bounded_error_and_abstains() {
        let source = "use std::process::Command;
use std::io::Write;
fn run(user: &str) { let _ = Command::new(\"sh\").arg(\"-c\").arg(format!(\"echo {user}\")); }";
        let limits = Limits {
            alias_bindings: 1,
            ..LIMITS
        };
        let (candidates, _, errors) = analyze_with_limits(source, &REGISTERED, limits);

        assert!(candidates.is_empty());
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, "limit-exceeded");
        assert!(errors[0].message.contains("alias-bindings"));
    }

    #[test]
    fn corpus_follows_modules_once_and_emits_only_closed_predicates() {
        let scan = inspect(&metadata("precision"));
        assert!(scan.errors.is_empty(), "{:?}", scan.errors);
        assert_eq!(scan.counters.files_read, 13);
        assert_eq!(scan.counters.files_parsed, 13);
        assert_eq!(scan.candidates.len(), 13, "{:?}", scan.candidates);
        assert!(
            scan.candidates
                .iter()
                .all(|candidate| candidate.package.as_deref() == Some("source-kernel-app"))
        );
        assert!(scan.candidates.iter().any(|candidate| {
            candidate.definition.id == SOURCE_DYNAMIC_SHELL.id
                && candidate.path == "app/src/shared.rs"
                && candidate.target.is_none()
        }));
        assert_eq!(
            scan.candidates
                .iter()
                .filter(|candidate| candidate.definition.id == SOURCE_DISABLED_TLS.id)
                .count(),
            7
        );
        assert!(
            scan.candidates
                .iter()
                .all(|candidate| candidate.path != "app/src/ignored.rs")
        );
        assert!(
            scan.candidates
                .iter()
                .all(|candidate| !candidate.path.contains("/tests/"))
        );
    }

    #[test]
    fn partial_failures_are_private_deduplicated_and_preserve_valid_findings() {
        let scan = inspect(&metadata("errors"));
        let codes: Vec<_> = scan.errors.iter().map(|error| error.code).collect();
        assert_eq!(
            codes,
            [
                "module-ambiguous",
                "module-not-found",
                "parse-error",
                "parse-error",
                "path-outside-workspace",
                "path-outside-workspace",
            ]
        );
        assert_eq!(scan.candidates.len(), 2, "{:?}", scan.candidates);
        assert_eq!(scan.counters.files_parsed, 5);
        assert!(
            scan.candidates
                .iter()
                .all(|candidate| candidate.definition.id == SOURCE_DYNAMIC_SHELL.id)
        );
        assert!(
            scan.candidates
                .iter()
                .all(|candidate| candidate.path != "src/intersected.rs")
        );
        let rendered = format!("{:?}", scan.errors);
        assert!(!rendered.contains(env!("CARGO_MANIFEST_DIR")));
        assert!(!rendered.contains("secret"));
        assert!(!rendered.contains("fn invalid"));
    }

    #[test]
    fn existing_and_missing_roots_outside_the_workspace_are_never_read() {
        let existing = inspect(&metadata("external-root"));
        assert_eq!(existing.counters.files_read, 0);
        assert!(existing.candidates.is_empty());
        assert_eq!(existing.errors.len(), 1);
        assert_eq!(existing.errors[0].code, "path-outside-workspace");
        assert!(
            !existing.errors[0]
                .message
                .contains(env!("CARGO_MANIFEST_DIR"))
        );

        let mut missing_metadata = metadata("external-root");
        let missing = missing_metadata
            .workspace_root
            .join("../missing-external-main.rs");
        missing_metadata.packages[0].targets[0].src_path = missing;
        let missing = inspect(&missing_metadata);
        assert_eq!(missing.counters.files_read, 0);
        assert!(missing.candidates.is_empty());
        assert_eq!(missing.errors.len(), 1);
        assert_eq!(missing.errors[0].code, "path-outside-workspace");
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_outside_the_workspace_are_rejected_before_reading() {
        use std::os::unix::fs::symlink;

        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!("source-kernel-symlink-{}", std::process::id()));
        if root.exists() {
            fs::remove_dir_all(&root).unwrap();
        }
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"symlink-proof\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        fs::write(root.join("src/lib.rs"), "mod escape;\n").unwrap();
        symlink(fixture("outside.rs"), root.join("src/escape.rs")).unwrap();

        let mut command = MetadataCommand::new();
        command
            .manifest_path(root.join("Cargo.toml"))
            .no_deps()
            .other_options(["--offline".to_owned()]);
        let scan = inspect(&command.exec().unwrap());

        assert_eq!(scan.counters.files_read, 1);
        assert_eq!(scan.counters.files_parsed, 1);
        assert_eq!(scan.errors.len(), 1);
        assert_eq!(scan.errors[0].code, "path-outside-workspace");
        assert!(!scan.errors[0].message.contains(env!("CARGO_MANIFEST_DIR")));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn invalid_utf8_is_a_private_read_failure() {
        let mut metadata = metadata("precision");
        let path = metadata.workspace_root.join(format!(
            "target/source-kernel-invalid-{}.rs",
            std::process::id()
        ));
        let package = metadata
            .packages
            .iter_mut()
            .find(|package| package.name.as_str() == "source-kernel-app")
            .unwrap();
        let mut target = package.targets[0].clone();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, [0xff, 0xfe]).unwrap();
        target.src_path = path.clone();
        package.targets = vec![target];

        let scan = inspect(&metadata);
        fs::remove_file(path).unwrap();

        assert_eq!(scan.counters.files_read, 1);
        assert_eq!(scan.counters.files_parsed, 1);
        assert_eq!(scan.errors.len(), 1);
        assert_eq!(scan.errors[0].code, "read-failed");
        assert!(!scan.errors[0].message.contains(env!("CARGO_MANIFEST_DIR")));
    }

    #[test]
    fn target_editions_do_not_override_the_package_edition() {
        let mut metadata = metadata("precision");
        let package = metadata
            .packages
            .iter_mut()
            .find(|package| package.name.as_str() == "source-kernel-app")
            .unwrap();
        let mut edition_2018 = package.targets[0].clone();
        edition_2018.name = "edition-2018".to_owned();
        edition_2018.edition = cargo_metadata::Edition::E2018;
        let mut edition_2024 = edition_2018.clone();
        edition_2024.name = "edition-2024".to_owned();
        edition_2024.edition = cargo_metadata::Edition::E2024;
        package.targets = vec![edition_2018, edition_2024];

        let scan = inspect(&metadata);

        assert!(scan.errors.is_empty(), "{:?}", scan.errors);
        assert_eq!(scan.counters.files_read, 11);
        assert_eq!(scan.counters.files_parsed, 11);
        assert_eq!(scan.candidates.len(), 13);
        assert!(
            scan.candidates
                .iter()
                .all(|candidate| candidate.target.is_none())
        );
    }

    #[test]
    fn unsupported_package_editions_fail_closed() {
        let mut metadata = metadata("precision");
        let package = metadata
            .packages
            .iter_mut()
            .find(|package| package.name.as_str() == "source-kernel-app")
            .unwrap();
        package.edition = cargo_metadata::Edition::_E2027;

        let scan = inspect(&metadata);

        assert!(scan.candidates.is_empty());
        assert_eq!(scan.errors.len(), 1);
        assert_eq!(scan.errors[0].code, "parse-error");
        assert!(scan.errors[0].message.contains("unsupported Rust edition"));
    }

    #[test]
    fn file_total_unit_and_depth_limits_stop_the_required_work() {
        let metadata = metadata("precision");
        for (limits, name) in [
            (
                Limits {
                    file_bytes: 1,
                    ..LIMITS
                },
                "file-bytes",
            ),
            (
                Limits {
                    total_bytes: 1,
                    ..LIMITS
                },
                "total-bytes",
            ),
            (Limits { units: 0, ..LIMITS }, "source-units"),
            (
                Limits {
                    module_depth: 0,
                    ..LIMITS
                },
                "module-depth",
            ),
            (
                Limits {
                    alias_bindings: 1,
                    ..LIMITS
                },
                "alias-bindings",
            ),
        ] {
            let scan = inspect_with_limits(&metadata, limits);
            assert_eq!(
                scan.errors
                    .iter()
                    .filter(|error| {
                        error.code == "limit-exceeded" && error.message.contains(name)
                    })
                    .count(),
                1
            );
        }
    }

    #[test]
    fn unicode_columns_are_scalar_based_and_end_exclusive() {
        let source = "fn main() { let _ = \"é\"; true }";
        let start = source.find("true").unwrap();
        let end = start + "true".len();
        let span = source_span(
            TextRange::new((start as u32).into(), (end as u32).into()),
            &line_starts(source),
            source,
        );
        assert_eq!(span.line_start, 1);
        assert_eq!(span.line_end, 1);
        assert_eq!(span.column_start, source[..start].chars().count() + 1);
        assert_eq!(span.column_end, source[..end].chars().count() + 1);
    }

    #[test]
    fn twenty_metadata_reachability_permutations_are_identical() {
        let mut metadata = metadata("precision");
        let package_index = metadata
            .packages
            .iter()
            .position(|package| package.name.as_str() == "source-kernel-app")
            .unwrap();
        let root = metadata.packages[package_index].targets[0].clone();
        let targets: Vec<_> = (0..5)
            .map(|index| {
                let mut target = root.clone();
                target.name = format!("target-{index}");
                target
            })
            .collect();
        let mut order = [0, 1, 2, 3, 4];
        let mut expected = None;
        for _ in 0..20 {
            metadata.packages[package_index].targets =
                order.iter().map(|index| targets[*index].clone()).collect();
            let scan = inspect(&metadata);
            let observation = format!(
                "{:?}|{:?}|{:?}",
                scan.candidates, scan.errors, scan.counters
            );
            match expected.as_ref() {
                Some(expected) => assert_eq!(&observation, expected),
                None => expected = Some(observation),
            }
            assert!(next_permutation(&mut order));
        }
    }

    #[test]
    #[ignore = "manual probe for the five explicitly approved pinned repositories"]
    fn pinned_real_world_evaluation_probe() {
        let manifest = std::env::var_os("RUST_DOCTOR_EVALUATION_MANIFEST")
            .map(PathBuf::from)
            .expect("RUST_DOCTOR_EVALUATION_MANIFEST must name an approved manifest");
        let mut command = MetadataCommand::new();
        command
            .manifest_path(manifest)
            .no_deps()
            .other_options(["--offline".to_owned()]);
        let scan = inspect(&command.exec().expect("approved metadata should load"));
        let mut counts = BTreeMap::from([
            (SOURCE_DISABLED_TLS.id, 0_usize),
            (SOURCE_DYNAMIC_SHELL.id, 0_usize),
        ]);
        for candidate in &scan.candidates {
            *counts.entry(candidate.definition.id).or_default() += 1;
        }
        let mut errors = BTreeMap::<&str, usize>::new();
        for error in &scan.errors {
            *errors.entry(error.code).or_default() += 1;
        }
        let findings: Vec<_> = scan
            .candidates
            .iter()
            .map(|candidate| {
                serde_json::json!({
                    "code": candidate.definition.id,
                    "package": candidate.package,
                    "target": candidate.target,
                    "path": candidate.path,
                    "span": {
                        "line_start": candidate.span.line_start,
                        "column_start": candidate.span.column_start,
                        "line_end": candidate.span.line_end,
                        "column_end": candidate.span.column_end,
                    }
                })
            })
            .collect();
        println!(
            "RUST_DOCTOR_SOURCE_EVALUATION={}",
            serde_json::json!({
                "source_pairs": scan.counters.files_parsed,
                "files_read": scan.counters.files_read,
                "bytes_parsed": scan.counters.bytes_read,
                "counts": counts,
                "source_errors": errors,
                "findings": findings,
            })
        );
    }

    fn next_permutation(values: &mut [usize]) -> bool {
        let Some(pivot) = (0..values.len() - 1)
            .rev()
            .find(|index| values[*index] < values[*index + 1])
        else {
            return false;
        };
        let successor = (pivot + 1..values.len())
            .rev()
            .find(|index| values[*index] > values[pivot])
            .unwrap();
        values.swap(pivot, successor);
        values[pivot + 1..].reverse();
        true
    }
}
