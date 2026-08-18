//! Source kernel: one walk of the workspace's Rust sources, and the registry of
//! native detectors that reads it.
//!
//! `walk` loads and parses each reachable file exactly once. Every producer that
//! reads source text then works off that single `Enumeration`: the detectors
//! below, the structural pass, and `references` for the dependency-truth rules.
//! The walk is the expensive part, so it happens here and the producers only
//! read the result.
//!
//! One question runs through the whole module and has exactly one answer:
//! several targets reach the same file, and the file publishes only what all of
//! them agree on. `unanimous` is that answer, and it is the reason a finding in
//! shared code names no package rather than an arbitrary one.

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use ra_ap_syntax::ast::{self, LiteralKind};
use ra_ap_syntax::{AstNode, Edition, SourceFile, SyntaxKind, TextRange};

use crate::policy::{PolicyPlan, RuleDefinition};
use crate::report::DiagnosticContext;
use crate::source_text::{SourceSpan, line_starts, source_span};

mod aliases;
mod detectors;
pub(crate) mod references;
#[cfg(test)]
mod tests;
mod walk;

use aliases::AliasMap;
use detectors::{CrateAliases, DETECTORS, Detection, Detector};

pub(crate) use walk::{enumerate, workspace_relative};

/// The bounds the kernel refuses to work past. A scan is a bounded amount of
/// work on a tree of unknown size, and each of these is a place where an
/// enormous or adversarial workspace would otherwise be unbounded.
#[derive(Debug, Clone, Copy)]
struct Limits {
    /// Largest single file the walk will read.
    file_bytes: u64,
    /// Bytes the walk will read across every file of one scan.
    total_bytes: u64,
    /// Distinct (file, edition) pairs the walk will load.
    units: usize,
    /// Depth of `mod` nesting the walk will follow.
    module_depth: usize,
    /// Bindings one unit's alias map retains before it abstains on everything.
    alias_bindings: usize,
}

const LIMITS: Limits = Limits {
    file_bytes: 8_388_608,
    total_bytes: 268_435_456,
    units: 20_000,
    module_depth: 256,
    alias_bindings: aliases::BINDING_LIMIT,
};

/// A bound the walk enforces, named once.
///
/// The name reaches the report inside a sentence, and no control flow reads it
/// back out of one. What a caller does when a limit is hit is decided by the
/// value it is handed, never by the phrase that was printed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Limit {
    FileBytes,
    TotalBytes,
    SourceUnits,
    ModuleDepth,
    AliasBindings,
}

impl Limit {
    const fn name(self) -> &'static str {
        match self {
            Self::FileBytes => "file-bytes",
            Self::TotalBytes => "total-bytes",
            Self::SourceUnits => "source-units",
            Self::ModuleDepth => "module-depth",
            Self::AliasBindings => "alias-bindings",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Candidate {
    pub(crate) definition: &'static RuleDefinition,
    pub(crate) message: &'static str,
    pub(crate) package: Option<String>,
    pub(crate) target: Option<String>,
    pub(crate) path: String,
    pub(crate) span: SourceSpan,
}

impl Candidate {
    /// Identity of a finding: the same rule, at the same place, saying the same
    /// thing is one finding however many units reached it. Derived from the
    /// candidate rather than assembled beside it, so the two cannot drift.
    fn key(&self) -> CandidateKey {
        CandidateKey {
            code: self.definition.id,
            path: self.path.clone(),
            span: self.span,
            message: self.message,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct CandidateKey {
    code: &'static str,
    path: String,
    span: SourceSpan,
    message: &'static str,
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
    pub(crate) counters: SourceCounters,
}

/// One counter group per phase. Each half is carried whole rather than copied
/// field by field, so a counter added to either reaches the report without a
/// second place to keep in step.
#[derive(Debug, Default)]
pub(crate) struct SourceCounters {
    pub(crate) walk: WalkCounters,
    pub(crate) analysis: AnalysisCounters,
}

/// What the walk did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct WalkCounters {
    pub(crate) files_read: usize,
    pub(crate) bytes_read: u64,
}

/// What the analysis did. The report publishes findings, not how much tree was
/// touched to reach them, so this half is proof for the tests alone.
#[derive(Debug, Default)]
#[allow(dead_code, reason = "read only by the tests that assert the pass did the work")]
pub(crate) struct AnalysisCounters {
    pub(crate) nodes_visited: usize,
    /// Solicitations per rule. The registry carries N detectors, so the
    /// counter is indexed by identifier rather than by named field.
    pub(crate) predicates: BTreeMap<&'static str, usize>,
}

/// A file the walk loaded, identified by its canonical path and the edition it
/// was parsed under. The same file reached from two packages on two editions is
/// two units, because the tree differs.
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

/// Non-production mark of every workspace target, keyed the way `Reachability`
/// names it.
pub(crate) type TargetContexts = BTreeMap<String, Option<DiagnosticContext>>;

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

    /// Package a finding in this unit names, when every target that reaches it
    /// belongs to one.
    pub(crate) fn package(&self) -> Option<String> {
        unanimous(
            self.reachability
                .iter()
                .map(|reach| (&reach.package_id, &reach.package_name)),
        )
        .map(|(_, name)| name.clone())
    }

    /// Target a finding in this unit names, when one target reaches it.
    pub(crate) fn target(&self) -> Option<String> {
        unanimous(
            self.reachability
                .iter()
                .map(|reach| (&reach.target_key, &reach.target_name)),
        )
        .map(|(_, name)| name.clone())
    }

    /// Identifiers of the workspace packages whose targets reach this unit.
    ///
    /// `package` answers what to publish on a finding and abstains when several
    /// packages disagree. This answers the other question, which module tree a
    /// file belongs to, and a file two packages reach belongs to both.
    pub(crate) fn package_ids(&self) -> impl Iterator<Item = &str> {
        self.reachability
            .iter()
            .map(|reach| reach.package_id.as_str())
    }

    /// Non-production context of the unit, when every target that reaches it
    /// agrees on one. A file the library and an integration test both reach is
    /// left unmarked, because silencing shipped code is the expensive mistake.
    pub(crate) fn context(&self, contexts: &TargetContexts) -> Option<DiagnosticContext> {
        unanimous(
            self.reachability
                .iter()
                .map(|reach| contexts.get(&reach.target_key).copied().flatten()),
        )
        .flatten()
    }

    /// Is every target that reaches this unit test, bench or example material,
    /// as Cargo names its kind? This is the authoritative answer, and the one
    /// the dependency rules classify a reference by.
    pub(crate) fn is_test_target(&self, contexts: &TargetContexts) -> bool {
        matches!(
            self.context(contexts),
            Some(DiagnosticContext::Tests | DiagnosticContext::Benchmark | DiagnosticContext::Example)
        )
    }

    /// Does this unit hold test material? Cargo's target kind answers first and
    /// covers `tests/`, `benches/` and `examples/` targets wherever they are
    /// configured to sit; the path convention then covers what no target names
    /// on its own, a `tests` module file the library reaches. A rule that stays
    /// quiet in test code asks this, never a path by itself.
    fn is_test_code(&self, contexts: &TargetContexts) -> bool {
        self.is_test_target(contexts) || path_contains_tests_segment(&self.relative_path)
    }
}

/// Files the workspace reaches, read and parsed once for every producer that
/// analyses source text.
#[derive(Debug, Default)]
pub(crate) struct Enumeration {
    units: BTreeMap<Identity, SourceUnit>,
    tables: BTreeMap<String, CrateAliases>,
    contexts: TargetContexts,
    errors: Vec<SourceError>,
    counters: WalkCounters,
}

impl Enumeration {
    pub(crate) fn units(&self) -> impl Iterator<Item = &SourceUnit> {
        self.units.values()
    }

    pub(crate) const fn contexts(&self) -> &TargetContexts {
        &self.contexts
    }
}

/// Does any producer reading source text still have an active rule? When none
/// does, the workspace is never walked at all.
/// The one value every reacher agrees on, or nothing.
///
/// This is the module's recurring question, and answering it in one place is
/// what keeps the answers consistent: a unit two packages reach names a package
/// only if it is the same package, a candidate two units emit names a target
/// only if it is the same target, and a file is test material only if every
/// target reaching it says so. Disagreement abstains, it never arbitrates.
fn unanimous<T: Eq>(values: impl IntoIterator<Item = T>) -> Option<T> {
    let mut values = values.into_iter();
    let first = values.next()?;
    values.all(|value| value == first).then_some(first)
}

/// The active detectors, grouped by the node kind each one declared. A node is
/// visited once and its kind looked up once, so a registry that grows costs the
/// traversal nothing.
struct Registry {
    by_kind: BTreeMap<SyntaxKind, Vec<&'static Detector>>,
}

impl Registry {
    fn new(detectors: impl IntoIterator<Item = &'static Detector>) -> Self {
        let mut by_kind: BTreeMap<SyntaxKind, Vec<&'static Detector>> = BTreeMap::new();
        for detector in detectors {
            by_kind.entry(detector.node).or_default().push(detector);
        }
        Self { by_kind }
    }

    fn active(plan: &PolicyPlan) -> Self {
        Self::new(
            DETECTORS
                .iter()
                .copied()
                .filter(|detector| plan.is_active(detector.definition.id)),
        )
    }

    fn is_empty(&self) -> bool {
        self.by_kind.is_empty()
    }

    fn for_kind(&self, kind: SyntaxKind) -> &[&'static Detector] {
        self.by_kind.get(&kind).map_or(&[], Vec::as_slice)
    }
}

/// The mutable side of one inspection. `Emitter` carries what a unit is read
/// through, this carries what the reading produces, so a detector or a counter
/// added later widens no analysis signature.
#[derive(Debug, Default)]
struct Findings {
    candidates: BTreeMap<CandidateKey, Candidate>,
    counters: SourceCounters,
    errors: Vec<SourceError>,
}

impl Findings {
    fn into_scan(mut self) -> SourceScan {
        self.errors.sort();
        self.errors.dedup();
        SourceScan {
            candidates: self.candidates.into_values().collect(),
            errors: self.errors,
            counters: self.counters,
        }
    }
}

pub(crate) fn inspect(enumeration: &Enumeration, plan: &PolicyPlan) -> SourceScan {
    inspect_with_limits(enumeration, LIMITS, plan)
}

fn inspect_with_limits(enumeration: &Enumeration, limits: Limits, plan: &PolicyPlan) -> SourceScan {
    let registry = Registry::active(plan);
    if registry.is_empty() {
        return SourceScan::default();
    }

    let mut findings = Findings {
        errors: enumeration.errors.clone(),
        counters: SourceCounters {
            walk: enumeration.counters.clone(),
            ..SourceCounters::default()
        },
        ..Findings::default()
    };
    for unit in enumeration.units.values() {
        analyze_unit(unit, enumeration, &registry, limits, &mut findings);
    }
    findings.into_scan()
}

/// Walks one unit's CST exactly once and solicits every active detector on the
/// nodes of the kind it declared.
fn analyze_unit(
    unit: &SourceUnit,
    enumeration: &Enumeration,
    registry: &Registry,
    limits: Limits,
    findings: &mut Findings,
) {
    let tree = unit.tree();
    let aliases = AliasMap::build(&tree, &unit.error_ranges, limits.alias_bindings);
    if aliases.saturated() {
        push_limit_error(
            &mut findings.errors,
            Limit::AliasBindings,
            limits.alias_bindings,
        );
    }
    let crates = detectors::shared_crate_aliases(
        unit.reachability
            .iter()
            .map(|reach| enumeration.tables.get(&reach.package_id)),
    );
    let context = detectors::Context {
        aliases: &aliases,
        crates: &crates,
        error_ranges: &unit.error_ranges,
        edition: unit.edition,
        test_code: unit.is_test_code(&enumeration.contexts),
    };
    let emitter = Emitter {
        package: unit.package(),
        target: unit.target(),
        path: unit.relative_path(),
        line_starts: line_starts(unit.source()),
        source: unit.source(),
    };

    for node in tree.syntax().descendants() {
        findings.counters.analysis.nodes_visited += 1;
        for detector in registry.for_kind(node.kind()) {
            *findings
                .counters
                .analysis
                .predicates
                .entry(detector.definition.id)
                .or_default() += 1;
            if let Some(detection) = (detector.inspect)(&context, &node) {
                emitter.insert(&mut findings.candidates, detector.definition, &detection);
            }
        }
    }
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
        let candidate = Candidate {
            definition,
            message: detection.message,
            package: self.package.clone(),
            target: self.target.clone(),
            path: self.path.to_owned(),
            span: source_span(detection.range, &self.line_starts, self.source),
        };
        match candidates.entry(candidate.key()) {
            // One finding reached from two units keeps only what both name,
            // which is the same unanimity a unit applies to its own reachers.
            Entry::Occupied(mut occupied) => {
                let existing = occupied.get_mut();
                existing.package =
                    unanimous([existing.package.take(), candidate.package]).flatten();
                existing.target = unanimous([existing.target.take(), candidate.target]).flatten();
            }
            Entry::Vacant(slot) => {
                slot.insert(candidate);
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

fn path_contains_tests_segment(path: &str) -> bool {
    Path::new(path)
        .components()
        .any(|component| component == Component::Normal("tests".as_ref()))
}

fn push_limit_error(errors: &mut Vec<SourceError>, limit: Limit, maximum: impl std::fmt::Display) {
    push_error(
        errors,
        "limit-exceeded",
        format!(
            "Source limit \"{}\" exceeded (maximum {maximum}).",
            limit.name()
        ),
    );
}

fn push_error(errors: &mut Vec<SourceError>, code: &'static str, message: String) {
    errors.push(SourceError { code, message });
}
