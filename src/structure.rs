//! Structural pass over the enumerated source units.
//!
//! The seven native detectors answer "is this call site wrong?". A structural
//! detector answers a question no per-site lint can reach: how the codebase is
//! shaped, how often the same thing appears, how much of it a reviewer has to
//! hold at once. It reads the units the source kernel already walked and parsed,
//! so the workspace is enumerated once whatever the number of producers.
//!
//! A structural finding is a family, not a site. Observations sharing a
//! normalized key become one diagnostic whose `related` array names every other
//! member, so a pattern repeated forty times costs the report one line instead
//! of forty. That key is also the identity of the finding: it carries no source
//! position, which is what lets a baseline comparison survive an insertion
//! above it.
//!
//! Three rules hold the pass together, and each of them exists because its
//! absence had a cost.
//!
//! One traversal per unit. The four detector families need a handful of node
//! kinds between them, and the pass runs under a wall clock, so the walk that
//! finds them runs once into an `Inventory` every family reads. A walk per
//! family spends the budget once per family over the same tree, and the
//! substring pre-filters that used to hide the fourth one were a heuristic
//! answering a question the walk answers exactly.
//!
//! One set of active rules. Every family asks the same question of the same
//! plan, so [`ActiveRules`] is one set rather than a boolean pair per family.
//! It lives in `policy` rather than here: the catalog already records which
//! producer owns each rule, so the set derives from the plan itself and no
//! producer keeps a second list. The dependency pack and the repository pass
//! read the same one.
//!
//! One way into the family map, and one phase reporting its own partiality.
//! `record_family` is the only writer, so no producer can insert over a family
//! another one is still merging into; and a phase that stops at the deadline
//! says so, because a clock read after the fact cannot tell a pass that stopped
//! from a pass that merely finished late, and calling a complete report partial
//! costs the score its authoritative flag for nothing.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use cargo_metadata::Metadata;
use ra_ap_syntax::ast::{self, HasAttrs};
use ra_ap_syntax::{AstNode, SyntaxKind, SyntaxNode};

use crate::policy::{ActiveRules, PolicyPlan, Producer, RuleDefinition};
use crate::report::{ComplexityFigures, DiagnosticContext};
use crate::source_kernel::{Enumeration, SourceUnit};
use crate::source_text::{SourceSpan, compact, line_starts, source_span};

#[cfg(test)]
mod benchmark;
mod duplication;
mod hotspots;
/// The file length `oversized_unit` reports at. Published rather than kept
/// crate-private because eleven modules assert that their own files hold this
/// bound and none of them restates the number, and three of those modules are
/// compiled into the binary rather than into this library.
pub use hotspots::FILE_LINES;
mod manifest;
mod normalize;
mod suppression;

const FINGERPRINT_DOMAIN: &str = "rust-doctor-structure-fingerprint-v1";

/// Wall clock the whole pass is allowed before it stops and says so.
///
/// The budget the NFR sets is 2 seconds on a workspace of a thousand files, and
/// this is five times that: it is not a target, it is the point past which a
/// partial answer beats a scan that never returns. Crossing it records an error
/// under the `structure` stage, which is what makes the score stop calling
/// itself authoritative.
const TIME_BUDGET: Duration = Duration::from_secs(10);

/// Thresholds the complexity detector reads, overridable through the
/// `[structure]` table of `rust-doctor.toml`.
///
/// The defaults are measured rather than inherited. Over the 1461 functions of
/// this repository on 2026-08-08, they name 9, every one a function its own
/// author would call the hard part, and lowering either bound by five starts
/// naming the ordinary ones. The cognitive bound is also the default Clippy
/// ships for `cognitive_complexity`; SonarSource gates at 15, which fits a
/// linter that restyles the norm, not a hotspot detector that reports the
/// tail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StructureSettings {
    /// Cyclomatic complexity a function reaches before it is reported.
    pub(crate) cyclomatic_threshold: u32,
    /// Cognitive complexity a function reaches before it is reported.
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

/// What the pass did, for the tests that bound what it is allowed to cost.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct StructureCounters {
    /// Functions the duplication pass kept, above the node floor.
    pub(crate) functions: usize,
    /// Distinct canonical forms among them.
    pub(crate) shapes: usize,
    /// Pairs of shapes the near-duplicate pass scored.
    pub(crate) comparisons: usize,
    /// Bytes the pass holds for those functions at its peak.
    pub(crate) retained_bytes: usize,
}

/// One structural family, published as a single diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StructureFinding {
    pub(crate) definition: &'static RuleDefinition,
    pub(crate) message: String,
    pub(crate) package: Option<String>,
    pub(crate) target: Option<String>,
    pub(crate) context: Option<DiagnosticContext>,
    pub(crate) path: String,
    pub(crate) span: SourceSpan,
    /// Every member beyond the first, in workspace-relative sorted order.
    pub(crate) related: Vec<StructureLocation>,
    pub(crate) occurrences: usize,
    /// Identity of the normalized content, as a hexadecimal blake3 digest. It
    /// contains no line and no column, so unrelated edits above the finding
    /// leave it unchanged.
    pub(crate) structure: String,
    /// How alike the members of the family are, in basis points, when the rule
    /// grouped them on a similarity rather than on an equality.
    pub(crate) similarity: Option<u16>,
    /// Cyclomatic and cognitive complexity of the reported function, when the
    /// rule measured them.
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

/// What a detector reads off one parsed unit.
///
/// `key` is what makes two observations the same finding. It never contains a
/// position and it never contains a path, so a family gathers what the whole
/// workspace does, across files, and keeps its identity across edits. A
/// detector that wants a per-file family says so by putting the path in its own
/// key.
///
/// `subject` and `complexity` describe the family the key designates, not the
/// site: every observation of one family carries the same ones, because every
/// subject a detector writes is a function of the key that gathered it.
struct Observation {
    key: String,
    subject: String,
    span: SourceSpan,
    context: Option<DiagnosticContext>,
    complexity: Option<ComplexityFigures>,
}

/// What a family says about itself, as opposed to where its members are.
///
/// The first observation to arrive fixes it, and `record_family` is the only
/// place that happens: the alternative was an emptiness test on the subject,
/// which encoded "the first one wins" as a sentinel a detector could trip by
/// writing an empty message.
#[derive(Debug)]
struct Summary {
    subject: String,
    similarity: Option<u16>,
    complexity: Option<ComplexityFigures>,
}

impl Summary {
    /// A family whose claim is its message, with nothing measured beside it.
    fn of(subject: String) -> Self {
        Self {
            subject,
            similarity: None,
            complexity: None,
        }
    }
}

/// Every rule this pass produces, the union of what the four families declare.
///
/// Activation itself comes from the catalog now, through
/// [`ActiveRules::of`]: what these tables still decide is whether a family is
/// walked at all. `the_pass_produces_every_catalogued_structural_rule`
/// compares the union against the catalog, which is what stops a rule from
/// being published, activated, and then never reached because the family that
/// owns it skipped the unit.
#[cfg(test)]
pub(super) fn rules() -> impl Iterator<Item = &'static RuleDefinition> {
    suppression::RULES
        .into_iter()
        .chain(duplication::RULES)
        .chain(hotspots::RULES)
        .chain(manifest::RULES)
}

/// Whether a unit is one the detectors read at all.
///
/// A unit the parser could not read is skipped whole and said so: a normalized
/// tree built over a recovered region describes the recovery, not the code. A
/// generated file is skipped silently, because its size, its repetition and
/// its exemptions describe the generator and there is nothing its author
/// should act on.
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
        if is_generated(unit.source()) {
            return Self::Generated;
        }
        Self::Yes
    }
}

/// What every family takes from one readable unit.
///
/// Each is asked for its own table first, and every family the same way: a
/// family with nothing left on has nothing to walk for.
fn observe_unit(
    analysed: &Unit<'_>,
    settings: &StructureSettings,
    active: &ActiveRules,
    families: &mut BTreeMap<(&'static str, String), Family>,
    functions: &mut Vec<duplication::Function>,
    uses: &mut manifest::Uses,
) {
    if active.any_of(&suppression::RULES) {
        for (definition, observation) in suppression::observe(analysed, active) {
            record(families, definition.id, analysed.path, observation);
        }
    }
    if active.any_of(&hotspots::RULES) {
        for (definition, observation) in hotspots::observe(analysed, settings, active) {
            record(families, definition.id, analysed.path, observation);
        }
    }
    if active.any_of(&duplication::RULES) {
        // Only the canonical form of each function is retained, never the tree
        // it was read from, so the walk stays linear in memory whatever the
        // size of the workspace.
        functions.extend(duplication::observe(analysed));
    }
    // The two manifest detectors decide after the walk, on the whole
    // workspace: what they take from a unit is what it reaches and what it
    // reads, never a finding.
    manifest::observe(analysed, active, uses);
}

/// Every node of one unit the detectors read, gathered in the traversal they
/// share.
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

/// What a detector knows about the unit under analysis.
struct Unit<'a> {
    tree: ast::SourceFile,
    source: &'a str,
    line_starts: Vec<usize>,
    /// Workspace-relative path, for the detectors whose family is per-file.
    path: &'a str,
    /// Non-production mark the Cargo targets reaching this unit agree on.
    context: Option<DiagnosticContext>,
    /// Packages whose targets reach this unit, for the detectors whose answer
    /// depends on which manifest describes the file.
    packages: Vec<&'a str>,
    /// The nodes every detector of this pass reads, walked once.
    inventory: Inventory,
}

impl<'a> Unit<'a> {
    /// The unit as the detectors read it, built from the unit the source kernel
    /// walked. It is the one place a `Unit` is assembled in a scan, so no caller
    /// can build one that is missing a part of itself.
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

    /// One parsed snippet, for the tests of the detectors. It exists so the
    /// four families do not each keep their own copy of this constructor.
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

/// Members of one family, before it becomes a diagnostic.
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
}

/// The wall clock the pass is running against.
///
/// A workspace far larger than the benchmark must not turn a scan into a hang.
/// Crossing the budget is not a failure of the workspace, it is the pass saying
/// what it did not get to, so it stops, publishes what it collected and records
/// an error, which is what makes the score drop its authoritative flag.
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
) -> StructureScan {
    analyze_within(metadata, enumeration, plan, settings, time_budget())
}

/// The wall-clock budget, overridable through
/// `RUST_DOCTOR_STRUCTURE_TIME_BUDGET_SECS`.
///
/// A wall-clock cutoff makes the findings of a large workspace depend on
/// machine load: the pinned-corpus harness raises it so that two replays of
/// the same revision publish the same observations. The default stays the
/// interactive contract.
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
            &mut families,
            &mut functions,
            &mut uses,
        );
    }

    // The manifest detectors run on what the walk collected, and on the
    // workspace description the walk never reads. A stopped pass leaves them
    // out: an orphan is an absence of reachability, and half a walk cannot tell
    // an unreached file from an unvisited one.
    if active.any_of(&manifest::RULES) && !stopped {
        let found = manifest::findings(metadata, enumeration, &active, &uses, &deadline);
        stopped |= found.stopped;
        for (definition, path, observation) in found.observations {
            record(&mut families, definition.id, &path, observation);
        }
    }

    let mut counters = StructureCounters::default();
    if active.any_of(&duplication::RULES) {
        let grouping = duplication::groups(functions, &active, &deadline);
        counters = StructureCounters {
            functions: grouping.functions,
            shapes: grouping.shapes,
            comparisons: grouping.comparisons,
            retained_bytes: grouping.retained_bytes,
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

/// One observation joins the family its rule and key designate.
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

/// Members join the family its rule and key designate, and the first arrival
/// fixes what the family says about itself.
///
/// This is the only way into the map. A producer that inserted its own families
/// would silently replace what another one had merged into the same key.
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

/// Mark of a family, when every member carries the same one.
///
/// A family straddling production and a test target stays unmarked and keeps
/// weighing on the score: the shipped half of it is still shipped.
///
/// A family whose members are all non-production but of disagreeing kinds is
/// unmarked by the same rule and weighs too, which is a defect and not the case
/// above. It is the family layer, not the unit layer: each member's own context
/// is correct, and the abstention `source_kernel::unanimous` performs over a
/// unit reached by disagreeing traversals is a different rule over a different
/// disagreement. Abstaining is right there, where the unit may still ship, and
/// wrong here, where no member of the family ships under any reading.
///
/// The repair is three lines, taking the mark of the anchor whenever every
/// member carries one, and its cost was measured on 2026-08-22 against the
/// pinned corpus rather than estimated. It is not where it looks. The four
/// agent structural scopes do not move at all: the two families it corrects,
/// both in `vibesql`, are anchored under `examples/` and `benches/`, which
/// `every_reviewed_structural_site_is_production_context` already keeps out of
/// the population the sampling plans draw over, so `observed` stays at 202 and
/// 186. What moves is the healthy population, through one family of `anyhow`
/// spanning `build.rs` and `tests/test_ffi.rs`: it is a reviewed site of
/// `crate_level_allow` and of `unreasoned_allow_attribute`, both of which rest
/// on exactly `MINIMUM_REVIEWED_SITES` sites. Correcting the mark takes each to
/// four and their rates stop being publishable, and both are 10000 basis points
/// in `CORPUS_NOISE`, which is what keeps the report from ranking two rules the
/// corpus measured wrong on every healthy site it looked at.
///
/// What that price is really made of is the raw model reading an absent rate as
/// a perfect one: `expected_repair_value` retains the whole contribution of a
/// rule nothing measured, so withholding a rate promotes the rule rather than
/// demoting it. Laplace smoothing is what removes the price, not a deeper
/// sample, since it ranks four false positives out of four at 8333 basis points
/// instead of at nothing. So the repair is US-017, blocked by US-013 and
/// US-014, and it costs three lines the day those land.
fn unanimous_context(members: &[Member]) -> Option<DiagnosticContext> {
    let mut contexts = members.iter().map(|member| member.context);
    let first = contexts.next().flatten()?;
    contexts.all(|context| context == Some(first)).then_some(first)
}

/// Does a recognized generator header open this file?
///
/// The conventions are the ones generators actually write: the `@generated`
/// marker Meta and prost use, the `DO NOT EDIT` banner protoc and bindgen
/// write, and the "Automatically generated" sentence of older tools. Only the
/// opening lines are read: a file that merely documents these markers, as this
/// one does, is not carrying them as a header.
fn is_generated(source: &str) -> bool {
    source.lines().take(10).any(|line| {
        line.contains("@generated")
            || line.contains("DO NOT EDIT")
            || line.contains("Automatically generated")
            || line.contains("automatically generated")
    })
}

/// Identity of a family: its rule and its normalized key, never its position.
fn structural_hash(rule: &str, key: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    for field in [FINGERPRINT_DOMAIN, rule, key] {
        hasher.update(&(field.len() as u64).to_le_bytes());
        hasher.update(field.as_bytes());
    }
    hasher.finalize().to_hex().to_string()
}

/// The single name a path is written as, when it is one name rather than a
/// qualified path. It is how an attribute and a macro call are both named.
fn single_name(path: &ast::Path) -> Option<String> {
    path.as_single_name_ref().map(|name| name.text().to_string())
}

/// Does this node sit under a `#[cfg(test)]` module or inside a `#[test]`
/// function? Those are not a Cargo target, so no target kind names them, and
/// they are exactly the non-production code a census must not charge for.
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
