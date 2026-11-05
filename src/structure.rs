use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use cargo_metadata::Metadata;
use ra_ap_syntax::ast::{self, HasAttrs};
use ra_ap_syntax::{AstNode, SyntaxKind, SyntaxNode};

use crate::generated::has_generator_header;
use crate::policy::{ActiveRules, PolicyPlan, Producer, RuleDefinition};
use crate::report::{ComplexityFigures, DiagnosticContext};
use crate::source_kernel::{Enumeration, SourceUnit};
use crate::source_text::{SourceSpan, compact, line_starts, source_span};

#[cfg(test)]
mod benchmark;
mod duplication;
mod hotspots;
pub use hotspots::FILE_LINES;
mod manifest;
mod normalize;
mod suppression;

const FINGERPRINT_DOMAIN: &str = "rust-doctor-structure-fingerprint-v1";

const TIME_BUDGET: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StructureSettings {
    pub(crate) cyclomatic_threshold: u32,
    pub(crate) cognitive_threshold: u32,
}

impl Default for StructureSettings {
    fn default() -> Self {
        Self {
            cyclomatic_threshold: 20,
            cognitive_threshold: 25,
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct StructureScan {
    pub(crate) findings: Vec<StructureFinding>,
    pub(crate) errors: Vec<StructureError>,
    #[allow(
        dead_code,
        reason = "read only by the tests that assert the pass held its budget"
    )]
    pub(crate) counters: StructureCounters,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct StructureCounters {
    pub(crate) functions: usize,
    pub(crate) shapes: usize,
    pub(crate) comparisons: usize,
    pub(crate) retained_bytes: usize,
    pub(crate) orphan_abstentions: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StructureFinding {
    pub(crate) definition: &'static RuleDefinition,
    pub(crate) message: String,
    pub(crate) package: Option<String>,
    pub(crate) target: Option<String>,
    pub(crate) context: Option<DiagnosticContext>,
    pub(crate) path: String,
    pub(crate) span: SourceSpan,
    pub(crate) related: Vec<StructureLocation>,
    pub(crate) occurrences: usize,
    pub(crate) structure: String,
    pub(crate) similarity: Option<u16>,
    pub(crate) complexity: Option<ComplexityFigures>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct StructureLocation {
    pub(crate) path: String,
    pub(crate) span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct StructureError {
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

struct Observation {
    key: String,
    subject: String,
    span: SourceSpan,
    context: Option<DiagnosticContext>,
    complexity: Option<ComplexityFigures>,
}

#[derive(Debug)]
struct Summary {
    subject: String,
    similarity: Option<u16>,
    complexity: Option<ComplexityFigures>,
}

impl Summary {
    fn of(subject: String) -> Self {
        Self {
            subject,
            similarity: None,
            complexity: None,
        }
    }
}

#[cfg(test)]
pub(super) fn rules() -> impl Iterator<Item = &'static RuleDefinition> {
    suppression::RULES
        .into_iter()
        .chain(duplication::RULES)
        .chain(hotspots::RULES)
        .chain(manifest::RULES)
}

enum Readable {
    Yes,
    Generated,
    Unparseable(StructureError),
}

impl Readable {
    fn of(unit: &SourceUnit) -> Self {
        if !unit.parses_cleanly() {
            return Self::Unparseable(StructureError {
                code: "parse-error",
                message: format!(
                    "Source path \"{}\" was skipped: the parser could not read it.",
                    unit.relative_path()
                ),
            });
        }
        if has_generator_header(unit.source()) {
            return Self::Generated;
        }
        Self::Yes
    }
}

fn observe_unit(
    analysed: &Unit<'_>,
    settings: &StructureSettings,
    active: &ActiveRules,
    clippy: &ActiveRules,
    families: &mut BTreeMap<(&'static str, String), Family>,
    functions: &mut Vec<duplication::Function>,
    uses: &mut manifest::Uses,
) {
    if active.any_of(&suppression::RULES) {
        for (definition, observation) in suppression::observe(analysed, active, clippy) {
            record(families, definition.id, analysed.path, observation);
        }
    }
    if active.any_of(&hotspots::RULES) {
        for (definition, observation) in hotspots::observe(analysed, settings, active) {
            record(families, definition.id, analysed.path, observation);
        }
    }
    if active.any_of(&duplication::RULES) {
        functions.extend(duplication::observe(analysed));
    }
    manifest::observe(analysed, active, uses);
}

#[derive(Debug, Default)]
struct Inventory {
    attributes: Vec<ast::Attr>,
    functions: Vec<ast::Fn>,
    implementations: Vec<ast::Impl>,
    modules: Vec<ast::Module>,
    macro_calls: Vec<ast::MacroCall>,
}

impl Inventory {
    fn of(tree: &ast::SourceFile) -> Self {
        let mut inventory = Self::default();
        for node in tree.syntax().descendants() {
            match node.kind() {
                SyntaxKind::ATTR => inventory.attributes.extend(ast::Attr::cast(node)),
                SyntaxKind::FN => inventory.functions.extend(ast::Fn::cast(node)),
                SyntaxKind::IMPL => inventory.implementations.extend(ast::Impl::cast(node)),
                SyntaxKind::MODULE => inventory.modules.extend(ast::Module::cast(node)),
                SyntaxKind::MACRO_CALL => inventory.macro_calls.extend(ast::MacroCall::cast(node)),
                _ => {}
            }
        }
        inventory
    }
}

struct Unit<'a> {
    tree: ast::SourceFile,
    source: &'a str,
    line_starts: Vec<usize>,
    path: &'a str,
    context: Option<DiagnosticContext>,
    packages: Vec<&'a str>,
    inventory: Inventory,
}

impl<'a> Unit<'a> {
    fn of(unit: &'a SourceUnit, enumeration: &'a Enumeration) -> Self {
        let tree = unit.tree();
        Self {
            inventory: Inventory::of(&tree),
            tree,
            source: unit.source(),
            line_starts: line_starts(unit.source()),
            path: unit.relative_path(),
            context: unit.context(enumeration.contexts()),
            packages: unit.package_ids().collect(),
        }
    }

    #[cfg(test)]
    fn probe(source: &'a str, path: &'a str) -> Self {
        let tree = ra_ap_syntax::SourceFile::parse(source, ra_ap_syntax::Edition::Edition2024)
            .tree();
        Self {
            inventory: Inventory::of(&tree),
            tree,
            source,
            line_starts: line_starts(source),
            path,
            context: None,
            packages: vec!["probe"],
        }
    }

    fn span(&self, node: &SyntaxNode) -> SourceSpan {
        source_span(node.text_range(), &self.line_starts, self.source)
    }
}

#[derive(Debug)]
struct Family {
    summary: Summary,
    members: Vec<Member>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Member {
    path: String,
    span: SourceSpan,
    context: Option<DiagnosticContext>,
    test_case: bool,
}

struct Deadline {
    start: Instant,
    budget: Duration,
}

impl Deadline {
    fn new(budget: Duration) -> Self {
        Self {
            start: Instant::now(),
            budget,
        }
    }

    fn exceeded(&self) -> bool {
        self.start.elapsed() >= self.budget
    }
}

pub(crate) fn analyze(
    metadata: &Metadata,
    enumeration: &Enumeration,
    plan: &PolicyPlan,
    settings: &StructureSettings,
    cap: Option<Duration>,
) -> StructureScan {
    let budget = cap.map_or_else(time_budget, |cap| time_budget().min(cap));
    analyze_within(metadata, enumeration, plan, settings, budget)
}

fn time_budget() -> Duration {
    std::env::var("RUST_DOCTOR_STRUCTURE_TIME_BUDGET_SECS")
        .ok()
        .and_then(|value| value.parse().ok())
        .map(Duration::from_secs)
        .unwrap_or(TIME_BUDGET)
}

fn analyze_within(
    metadata: &Metadata,
    enumeration: &Enumeration,
    plan: &PolicyPlan,
    settings: &StructureSettings,
    budget: Duration,
) -> StructureScan {
    let active = ActiveRules::of(plan, Producer::Structure);
    if !active.any() {
        return StructureScan::default();
    }
    let clippy = ActiveRules::of(plan, Producer::Clippy);

    let deadline = Deadline::new(budget);
    let mut stopped = false;
    let mut errors = Vec::new();
    let mut families = BTreeMap::<(&'static str, String), Family>::new();
    let mut functions = Vec::new();
    let mut uses = manifest::Uses::default();
    for unit in enumeration.units() {
        if deadline.exceeded() {
            stopped = true;
            break;
        }
        match Readable::of(unit) {
            Readable::Yes => {}
            Readable::Generated => continue,
            Readable::Unparseable(error) => {
                errors.push(error);
                continue;
            }
        }
        observe_unit(
            &Unit::of(unit, enumeration),
            settings,
            &active,
            &clippy,
            &mut families,
            &mut functions,
            &mut uses,
        );
    }

    let mut orphan_abstentions = 0;
    if active.any_of(&manifest::RULES) && !stopped {
        let found = manifest::findings(metadata, enumeration, &active, &uses, &deadline);
        stopped |= found.stopped;
        orphan_abstentions = found.orphan_abstentions;
        for (definition, path, observation) in found.observations {
            record(&mut families, definition.id, &path, observation);
        }
    }

    let mut counters = StructureCounters {
        orphan_abstentions,
        ..StructureCounters::default()
    };
    if active.any_of(&duplication::RULES) {
        let grouping = duplication::groups(functions, &active, &deadline);
        counters = StructureCounters {
            functions: grouping.functions,
            shapes: grouping.shapes,
            comparisons: grouping.comparisons,
            retained_bytes: grouping.retained_bytes,
            orphan_abstentions,
        };
        stopped |= grouping.stopped;
        for group in grouping.groups {
            record_family(
                &mut families,
                group.definition.id,
                group.key,
                group.summary,
                group.members,
            );
        }
    }
    if stopped {
        errors.push(StructureError {
            code: "time-budget",
            message: "Structural analysis stopped at the time budget; results are partial."
                .to_owned(),
        });
    }

    errors.sort();
    errors.dedup();
    StructureScan {
        findings: findings(families, enumeration),
        errors,
        counters,
    }
}

fn record(
    families: &mut BTreeMap<(&'static str, String), Family>,
    rule: &'static str,
    path: &str,
    observation: Observation,
) {
    let member = Member {
        path: path.to_owned(),
        span: observation.span,
        context: observation.context,
        test_case: false,
    };
    record_family(
        families,
        rule,
        observation.key,
        Summary {
            subject: observation.subject,
            similarity: None,
            complexity: observation.complexity,
        },
        [member],
    );
}

fn record_family(
    families: &mut BTreeMap<(&'static str, String), Family>,
    rule: &'static str,
    key: String,
    summary: Summary,
    members: impl IntoIterator<Item = Member>,
) {
    families
        .entry((rule, key))
        .or_insert_with(|| Family {
            summary,
            members: Vec::new(),
        })
        .members
        .extend(members);
}

fn findings(
    families: BTreeMap<(&'static str, String), Family>,
    enumeration: &Enumeration,
) -> Vec<StructureFinding> {
    let packages: BTreeMap<&str, (Option<String>, Option<String>)> = enumeration
        .units()
        .map(|unit| (unit.relative_path(), (unit.package(), unit.target())))
        .collect();
    families
        .into_iter()
        .filter_map(|((rule, key), mut family)| {
            let definition = crate::policy::find(rule)?;
            family.members.sort();
            family.members.dedup();
            let (first, related) = family.members.split_first()?;
            let (package, target) = packages
                .get(first.path.as_str())
                .cloned()
                .unwrap_or_default();
            Some(StructureFinding {
                definition,
                message: family.summary.subject,
                package,
                target,
                context: unanimous_context(&family.members),
                path: first.path.clone(),
                span: first.span,
                related: related
                    .iter()
                    .map(|member| StructureLocation {
                        path: member.path.clone(),
                        span: member.span,
                    })
                    .collect(),
                occurrences: family.members.len(),
                structure: structural_hash(rule, &key),
                similarity: family.summary.similarity,
                complexity: family.summary.complexity,
            })
        })
        .collect()
}

fn unanimous_context(members: &[Member]) -> Option<DiagnosticContext> {
    let mut contexts = members.iter().map(|member| member.context);
    let anchor = contexts.next().flatten()?;
    contexts
        .all(|context| context.is_some())
        .then_some(anchor)
}

fn structural_hash(rule: &str, key: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    for field in [FINGERPRINT_DOMAIN, rule, key] {
        hasher.update(&(field.len() as u64).to_le_bytes());
        hasher.update(field.as_bytes());
    }
    hasher.finalize().to_hex().to_string()
}

fn single_name(path: &ast::Path) -> Option<String> {
    path.as_single_name_ref().map(|name| name.text().to_string())
}

fn test_context(node: &SyntaxNode) -> Option<DiagnosticContext> {
    node.ancestors()
        .any(|ancestor| {
            let gated = ancestor
                .children()
                .filter_map(ast::Attr::cast)
                .any(|attribute| compact(attribute.syntax()) == "#[cfg(test)]");
            let test_function = ast::Fn::cast(ancestor.clone()).is_some_and(|function| {
                function
                    .attrs()
                    .any(|attribute| compact(attribute.syntax()) == "#[test]")
            });
            gated || test_function
        })
        .then_some(DiagnosticContext::Tests)
}

#[cfg(test)]
mod tests;
