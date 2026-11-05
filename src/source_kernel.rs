use std::borrow::Cow;
use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use ra_ap_syntax::ast::{self, LiteralKind};
use ra_ap_syntax::{AstNode, Edition, SourceFile, SyntaxKind, TextRange};

use crate::policy::{PolicyPlan, RuleDefinition};
use crate::report::DiagnosticContext;
use crate::source_text::{SourceSpan, line_starts, source_span};

mod aliases;
mod bindings;
mod detectors;
mod gates;
pub(crate) mod harness;
pub(crate) mod references;
#[cfg(test)]
mod tests;
mod walk;

use aliases::AliasMap;
use detectors::{CrateAliases, DETECTORS, Detection, Detector};

pub(crate) use walk::{enumerate, workspace_relative};

#[derive(Debug, Clone, Copy)]
struct Limits {
    file_bytes: u64,
    total_bytes: u64,
    units: usize,
    module_depth: usize,
    alias_bindings: usize,
}

const LIMITS: Limits = Limits {
    file_bytes: 8_388_608,
    total_bytes: 268_435_456,
    units: 20_000,
    module_depth: 256,
    alias_bindings: aliases::BINDING_LIMIT,
};

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
    pub(crate) message: Cow<'static, str>,
    pub(crate) package: Option<String>,
    pub(crate) target: Option<String>,
    pub(crate) context: Option<DiagnosticContext>,
    pub(crate) path: String,
    pub(crate) span: SourceSpan,
}

impl Candidate {
    fn key(&self) -> CandidateKey {
        CandidateKey {
            code: self.definition.id,
            path: self.path.clone(),
            span: self.span,
            message: self.message.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct CandidateKey {
    code: &'static str,
    path: String,
    span: SourceSpan,
    message: Cow<'static, str>,
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
    pub(crate) not_evaluated: Vec<(&'static str, crate::NotEvaluated)>,
    #[allow(dead_code, reason = "read only by the tests that assert the pass did the work")]
    pub(crate) counters: AnalysisCounters,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct WalkCounters {
    pub(crate) files_read: usize,
    pub(crate) bytes_read: u64,
}

#[derive(Debug, Default)]
#[allow(dead_code, reason = "read only by the tests that assert the pass did the work")]
pub(crate) struct AnalysisCounters {
    pub(crate) nodes_visited: usize,
    pub(crate) predicates: BTreeMap<&'static str, usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Identity {
    path: PathBuf,
    edition: Edition,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Reachability {
    package_id: String,
    package_name: String,
    target_key: String,
    target_name: String,
    test_gated: bool,
}

impl Reachability {
    fn context(&self, contexts: &TargetContexts) -> Option<DiagnosticContext> {
        contexts
            .get(&self.target_key)
            .copied()
            .flatten()
            .or_else(|| self.test_gated.then_some(DiagnosticContext::Tests))
    }
}

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

    pub(crate) fn package(&self) -> Option<String> {
        unanimous(
            self.reachability
                .iter()
                .map(|reach| (&reach.package_id, &reach.package_name)),
        )
        .map(|(_, name)| name.clone())
    }

    pub(crate) fn target(&self) -> Option<String> {
        unanimous(
            self.reachability
                .iter()
                .map(|reach| (&reach.target_key, &reach.target_name)),
        )
        .map(|(_, name)| name.clone())
    }

    pub(crate) fn package_names(&self) -> impl Iterator<Item = &str> {
        self.reachability
            .iter()
            .map(|reach| reach.package_name.as_str())
    }

    pub(crate) fn package_ids(&self) -> impl Iterator<Item = &str> {
        self.reachability
            .iter()
            .map(|reach| reach.package_id.as_str())
    }

    pub(crate) fn context(&self, contexts: &TargetContexts) -> Option<DiagnosticContext> {
        unanimous(self.reachability.iter().map(|reach| reach.context(contexts))).flatten()
    }

    pub(crate) fn is_test_target(&self, contexts: &TargetContexts) -> bool {
        matches!(
            self.context(contexts),
            Some(DiagnosticContext::Tests | DiagnosticContext::Benchmark | DiagnosticContext::Example)
        )
    }

    fn is_test_code(&self, contexts: &TargetContexts) -> bool {
        self.is_test_target(contexts) || path_contains_tests_segment(&self.relative_path)
    }

    fn production_line_count(&self) -> usize {
        let line_starts = line_starts(&self.source);
        let line_of = |offset: usize| line_starts.partition_point(|start| *start <= offset);
        let mut gated = 0;
        let mut last_gated_line = 0;
        let mut preorder = self.tree().syntax().preorder();
        while let Some(event) = preorder.next() {
            let ra_ap_syntax::WalkEvent::Enter(node) = event else {
                continue;
            };
            if ast::Item::cast(node.clone()).is_none() || !detectors::carries_test_attribute(&node) {
                continue;
            }
            preorder.skip_subtree();
            let range = node.text_range();
            let first = line_of(range.start().into()).max(last_gated_line + 1);
            let last = line_of(range.end().into());
            gated += (last + 1).saturating_sub(first);
            last_gated_line = last_gated_line.max(last);
        }
        self.source.lines().count().saturating_sub(gated)
    }
}

#[derive(Debug, Default)]
pub(crate) struct Enumeration {
    units: BTreeMap<Identity, SourceUnit>,
    tables: BTreeMap<String, CrateAliases>,
    contexts: TargetContexts,
    errors: Vec<SourceError>,
    counters: WalkCounters,
    complete: bool,
    excluded: BTreeSet<Identity>,
    selected: Option<BTreeSet<String>>,
    incomplete_trees: BTreeSet<String>,
    versions: Option<gates::DependencyVersions>,
}

impl Enumeration {
    pub(crate) fn units(&self) -> impl Iterator<Item = &SourceUnit> {
        self.scanned().map(|(_, unit)| unit)
    }

    fn scanned(&self) -> impl Iterator<Item = (&Identity, &SourceUnit)> {
        self.units
            .iter()
            .filter(|(identity, _)| !self.excluded.contains(*identity))
    }

    pub(crate) fn reached(&self) -> impl Iterator<Item = &SourceUnit> {
        self.units.values()
    }

    pub(crate) fn narrow(
        &mut self,
        excluded: impl Fn(&SourceUnit) -> bool,
        selected: Option<&BTreeSet<String>>,
    ) {
        self.excluded = self
            .units
            .iter()
            .filter(|(_, unit)| {
                excluded(unit)
                    || selected.is_some_and(|selected| {
                        !unit.package_names().any(|name| selected.contains(name))
                    })
            })
            .map(|(identity, _)| identity.clone())
            .collect();
        self.selected = selected.cloned();
    }

    pub(crate) fn selects(&self, package: &str) -> bool {
        self.selected
            .as_ref()
            .is_none_or(|selected| selected.contains(package))
    }

    pub(crate) fn module_tree_complete(&self, package_id: &str) -> bool {
        !self.incomplete_trees.contains(package_id)
    }

    pub(crate) const fn contexts(&self) -> &TargetContexts {
        &self.contexts
    }

    pub(crate) fn into_measurement(mut self) -> SourceMeasurement {
        let (production_files, production_lines) = self.production_source();
        let packages = self.package_measurements();
        self.errors.sort();
        self.errors.dedup();
        SourceMeasurement {
            files_read: self.counters.files_read,
            production_files,
            production_lines,
            complete: self.complete,
            errors: self.errors,
            packages,
        }
    }

    fn package_measurements(&self) -> BTreeMap<String, PackageMeasurement> {
        let mut per_path: BTreeMap<&Path, (usize, bool, Option<String>)> = BTreeMap::new();
        for (identity, unit) in self.scanned() {
            let package = unit.package();
            let counted = per_path
                .entry(identity.path.as_path())
                .or_insert_with(|| (unit.production_line_count(), false, package.clone()));
            counted.1 |= !unit.is_test_code(&self.contexts);
            if counted.2 != package {
                counted.2 = None;
            }
        }
        let mut packages = BTreeMap::<String, PackageMeasurement>::new();
        for (lines, is_production, package) in per_path.into_values() {
            if let Some(package) = package {
                let measured = packages.entry(package).or_default();
                if is_production {
                    measured.files += 1;
                    measured.production_lines += lines;
                }
            }
        }
        packages
    }

    fn production_source(&self) -> (usize, usize) {
        let mut per_path: BTreeMap<&Path, (usize, bool)> = BTreeMap::new();
        for (identity, unit) in self.scanned() {
            let counted = per_path
                .entry(identity.path.as_path())
                .or_insert_with(|| (unit.production_line_count(), false));
            counted.1 |= !unit.is_test_code(&self.contexts);
        }
        per_path
            .values()
            .filter(|(_, is_production)| *is_production)
            .fold((0, 0), |(files, total), (lines, _)| (files + 1, total + lines))
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct SourceMeasurement {
    pub(crate) files_read: usize,
    pub(crate) production_files: usize,
    pub(crate) production_lines: usize,
    pub(crate) complete: bool,
    pub(crate) errors: Vec<SourceError>,
    pub(crate) packages: BTreeMap<String, PackageMeasurement>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PackageMeasurement {
    pub(crate) files: usize,
    pub(crate) production_lines: usize,
}

fn unanimous<T: Eq>(values: impl IntoIterator<Item = T>) -> Option<T> {
    let mut values = values.into_iter();
    let first = values.next()?;
    values.all(|value| value == first).then_some(first)
}

struct Registry {
    by_kind: BTreeMap<SyntaxKind, Vec<&'static Detector>>,
    gates: Vec<gates::ParsedGates>,
}

impl Registry {
    fn new(detectors: impl IntoIterator<Item = &'static Detector>) -> Self {
        let mut by_kind: BTreeMap<SyntaxKind, Vec<&'static Detector>> = BTreeMap::new();
        let mut gated = BTreeSet::new();
        let mut parsed = Vec::new();
        for detector in detectors {
            by_kind.entry(detector.node).or_default().push(detector);
            if gated.insert(detector.definition.id)
                && let Some(gates) = gates::ParsedGates::of(detector.definition)
            {
                parsed.push(gates);
            }
        }
        Self {
            by_kind,
            gates: parsed,
        }
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

#[derive(Debug, Default)]
struct Findings {
    candidates: BTreeMap<CandidateKey, Candidate>,
    counters: AnalysisCounters,
    errors: Vec<SourceError>,
    gates: BTreeMap<&'static str, gates::Gate>,
}

impl Findings {
    fn into_scan(mut self, registry: &Registry) -> SourceScan {
        self.errors.sort();
        self.errors.dedup();
        let not_evaluated = registry
            .gates
            .iter()
            .filter_map(|gated| match self.gates.get(gated.rule) {
                Some(gates::Gate::Open) => None,
                Some(gates::Gate::Unsupported) => {
                    Some((gated.rule, crate::NotEvaluated::CrateVersionUnsupported))
                }
                Some(gates::Gate::Absent) | None => Some((gated.rule, crate::NotEvaluated::CrateAbsent)),
            })
            .collect();
        SourceScan {
            candidates: self.candidates.into_values().collect(),
            errors: self.errors,
            not_evaluated,
            counters: self.counters,
        }
    }

    fn record_gate(&mut self, rule: &'static str, gate: gates::Gate) {
        let rank = |gate: gates::Gate| match gate {
            gates::Gate::Absent => 0,
            gates::Gate::Unsupported => 1,
            gates::Gate::Open => 2,
        };
        let entry = self.gates.entry(rule).or_insert(gate);
        if rank(gate) > rank(*entry) {
            *entry = gate;
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

    let mut findings = Findings::default();
    for unit in enumeration.units() {
        analyze_unit(unit, enumeration, &registry, limits, &mut findings);
    }
    findings.into_scan(&registry)
}

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
    let bindings = bindings::Resolver::default();
    let context = detectors::Context {
        aliases: &aliases,
        bindings: &bindings,
        crates: &crates,
        error_ranges: &unit.error_ranges,
        edition: unit.edition,
        test_code: unit.is_test_code(&enumeration.contexts),
        resolved: std::cell::RefCell::default(),
    };
    let emitter = Emitter {
        package: unit.package(),
        target: unit.target(),
        context: unit.context(&enumeration.contexts),
        path: unit.relative_path(),
        line_starts: line_starts(unit.source()),
        source: unit.source(),
    };

    let mut closed = BTreeSet::new();
    for gated in &registry.gates {
        let gate = enumeration.versions.as_ref().map_or(gates::Gate::Open, |versions| {
            let reaching: Vec<_> = unit.package_ids().map(|id| versions.get(id)).collect();
            gated.evaluate(&reaching)
        });
        findings.record_gate(gated.rule, gate);
        if gate != gates::Gate::Open {
            closed.insert(gated.rule);
        }
    }

    for node in tree.syntax().descendants() {
        findings.counters.nodes_visited += 1;
        for detector in registry.for_kind(node.kind()) {
            if closed.contains(detector.definition.id) {
                continue;
            }
            *findings
                .counters
                .predicates
                .entry(detector.definition.id)
                .or_default() += 1;
            if let Some(detection) = (detector.inspect)(&context, &node) {
                emitter.insert(&mut findings.candidates, detector.definition, &detection, &node);
            }
        }
    }
}

struct Emitter<'a> {
    package: Option<String>,
    target: Option<String>,
    context: Option<DiagnosticContext>,
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
        node: &ra_ap_syntax::SyntaxNode,
    ) {
        if detection.range.is_empty() {
            return;
        }
        let candidate = Candidate {
            definition,
            message: detection.message.clone(),
            package: self.package.clone(),
            target: self.target.clone(),
            context: self.context.or_else(|| {
                detectors::under_test_attribute(node).then_some(DiagnosticContext::Tests)
            }),
            path: self.path.to_owned(),
            span: source_span(detection.range, &self.line_starts, self.source),
        };
        match candidates.entry(candidate.key()) {
            Entry::Occupied(mut occupied) => {
                let existing = occupied.get_mut();
                existing.package =
                    unanimous([existing.package.take(), candidate.package]).flatten();
                existing.target = unanimous([existing.target.take(), candidate.target]).flatten();
                existing.context = unanimous([existing.context.take(), candidate.context]).flatten();
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
