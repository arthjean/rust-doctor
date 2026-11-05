use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::internal_error::InternalError;
use crate::policy::Producer;
use crate::report::{Diagnostic, DiagnosticSource, DiagnosticSpan};
use crate::workspace_path;

mod diff_map;

pub(crate) use diff_map::{DiffMap, NOTICE_CODE as LINE_MAP_NOTICE};

const STAGE: &str = "delta";
pub(crate) const FINGERPRINT_VERSION: u8 = 1;
pub(crate) const DIAGNOSTIC_LIMIT: usize = 50_000;
const PROOF_BYTES_LIMIT: usize = 65_536;
const SOURCE_FILE_BYTES_LIMIT: usize = 8 * 1024 * 1024;
const SOURCE_BYTES_BUDGET: usize = 64 * 1024 * 1024;
const PROOF_BYTES_BUDGET: usize = 64 * 1024 * 1024;
const FINGERPRINT_DOMAIN: &str = "rust-doctor-delta-fingerprint-v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeltaReport {
    pub fingerprint_version: u8,
    pub base_diagnostics: usize,
    pub current_diagnostics: usize,
    pub introduced: Vec<String>,
    pub pre_existing: Vec<DeltaMatch>,
    pub fixed: Vec<Diagnostic>,
    pub summary: DeltaSummary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeltaMatch {
    pub current_id: String,
    pub baseline_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DeltaSummary {
    pub introduced: usize,
    pub pre_existing: usize,
    pub fixed: usize,
    pub cross_file_matches: usize,
    pub rule_count_matches: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct DeltaFingerprintV1([u8; 32]);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct FallbackKey<'a> {
    source: DiagnosticSource,
    code: Option<&'a str>,
    message: &'a str,
}

#[derive(Debug)]
struct Candidate<'a> {
    diagnostic: &'a Diagnostic,
    fingerprint: Option<DeltaFingerprintV1>,
    place: Option<&'a str>,
    line: usize,
}

impl<'a> Candidate<'a> {
    fn new(diagnostic: &'a Diagnostic) -> Self {
        Self {
            diagnostic,
            fingerprint: structural_identity(diagnostic).map(structural_fingerprint),
            place: diagnostic.path.as_deref(),
            line: diagnostic.span.as_ref().map_or(0, |span| span.line_start),
        }
    }

    fn path(&self) -> Option<&'a str> {
        self.diagnostic.path.as_deref()
    }

    fn place_through(&mut self, map: &'a DiffMap) {
        if let Some(path) = self.path() {
            let current = map.current_path(path);
            self.place = Some(current);
            self.line = map.mapped_line(current, self.line);
        }
    }

    fn fallback(&self) -> FallbackKey<'a> {
        FallbackKey {
            source: self.diagnostic.source,
            code: self.diagnostic.code.as_deref(),
            message: self.diagnostic.message.as_str(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct SourcePosition {
    line: usize,
    column: usize,
}

struct LineIndex(Vec<usize>);

impl LineIndex {
    fn of(source: &str) -> Self {
        Self(
            std::iter::once(0)
                .chain(source.match_indices('\n').map(|(index, _)| index + 1))
                .collect(),
        )
    }

    fn offset(&self, source: &str, position: SourcePosition) -> Option<usize> {
        let start = *self.0.get(position.line.checked_sub(1)?)?;
        let next_line = self.0.get(position.line).copied();
        let line = source.get(start..next_line.unwrap_or(source.len()))?;
        let column = position.column.checked_sub(1)?;
        match line.char_indices().nth(column) {
            Some((offset, _)) => start.checked_add(offset),
            None if next_line.is_none() && column == line.chars().count() => Some(source.len()),
            None => None,
        }
    }
}

struct EvidenceLoader {
    root: Option<PathBuf>,
    source_bytes_read: usize,
    proof_bytes: usize,
}

impl EvidenceLoader {
    fn new(root: &Path) -> Self {
        Self {
            root: root.canonicalize().ok(),
            source_bytes_read: 0,
            proof_bytes: 0,
        }
    }

    fn populate<'a>(&mut self, candidates: &mut [Candidate<'a>]) {
        let Some(root) = self.root.clone() else {
            return;
        };
        let mut by_path = BTreeMap::<&'a str, Vec<usize>>::new();
        for (index, candidate) in candidates.iter().enumerate() {
            if candidate.fingerprint.is_none()
                && candidate.diagnostic.span.is_some()
                && let Some(path) = candidate.path()
            {
                by_path.entry(path).or_default().push(index);
            }
        }

        for (path, indexes) in by_path {
            let Some(source) = self.read_source(&root, path) else {
                continue;
            };
            let lines = LineIndex::of(&source);
            for index in indexes {
                let Some(diagnostic) = candidates.get(index).map(|candidate| candidate.diagnostic)
                else {
                    continue;
                };
                let Some(span) = diagnostic.span.as_ref() else {
                    continue;
                };
                let Some(proof) = self.proof(&source, &lines, span) else {
                    continue;
                };
                let fingerprint = stable_fingerprint(diagnostic, &proof);
                if let Some(candidate) = candidates.get_mut(index) {
                    candidate.fingerprint = Some(fingerprint);
                }
            }
        }
    }

    fn read_source(&mut self, root: &Path, logical_path: &str) -> Option<String> {
        let relative = workspace_path::decode_normalized_relative(logical_path)?;
        let path = root.join(relative).canonicalize().ok()?;
        if !path.starts_with(root) {
            return None;
        }
        let metadata = fs::symlink_metadata(&path).ok()?;
        let length = usize::try_from(metadata.len()).ok()?;
        if !metadata.is_file()
            || length > SOURCE_FILE_BYTES_LIMIT
            || self.source_bytes_read.checked_add(length)? > SOURCE_BYTES_BUDGET
        {
            return None;
        }
        self.source_bytes_read = self.source_bytes_read.checked_add(length)?;

        let file = File::open(&path).ok()?;
        let opened = file.metadata().ok()?;
        let revalidated = path.canonicalize().ok()?;
        if !opened.is_file()
            || !revalidated.starts_with(root)
            || !workspace_path::same_file(&opened, &fs::metadata(&revalidated).ok()?)
        {
            return None;
        }

        let mut source = String::with_capacity(length);
        file.take(SOURCE_FILE_BYTES_LIMIT as u64)
            .read_to_string(&mut source)
            .ok()?;
        (source.len() == length).then_some(source)
    }

    fn proof(&mut self, source: &str, lines: &LineIndex, span: &DiagnosticSpan) -> Option<String> {
        let remaining = PROOF_BYTES_BUDGET.checked_sub(self.proof_bytes)?;
        if remaining == 0 {
            return None;
        }
        let proof = extract_proof(source, lines, span, remaining)?;
        self.proof_bytes = self.proof_bytes.checked_add(proof.len())?;
        Some(proof)
    }
}

pub(crate) type Evidence = Vec<Option<[u8; 32]>>;

#[cfg(test)]
pub(crate) fn compute(
    baseline: &[Diagnostic],
    current: &[Diagnostic],
    baseline_root: &Path,
    current_root: &Path,
) -> Result<DeltaReport, InternalError> {
    if baseline.len() > DIAGNOSTIC_LIMIT {
        return Err(limit_exceeded());
    }
    let evidence = evidence(baseline, baseline_root);
    compute_with(baseline, &evidence, current, current_root, &DiffMap::default())
}

pub(crate) fn evidence(diagnostics: &[Diagnostic], root: &Path) -> Evidence {
    if diagnostics.len() > DIAGNOSTIC_LIMIT {
        return vec![None; diagnostics.len()];
    }
    candidates(diagnostics, root)
        .iter()
        .map(|candidate| candidate.fingerprint.map(|fingerprint| fingerprint.0))
        .collect()
}

pub(crate) fn compute_with<'a>(
    baseline: &'a [Diagnostic],
    baseline_evidence: &[Option<[u8; 32]>],
    current: &'a [Diagnostic],
    current_root: &Path,
    map: &'a DiffMap,
) -> Result<DeltaReport, InternalError> {
    if baseline.len() > DIAGNOSTIC_LIMIT || current.len() > DIAGNOSTIC_LIMIT {
        return Err(limit_exceeded());
    }
    let baseline_candidates = baseline
        .iter()
        .zip(baseline_evidence.iter().chain(std::iter::repeat(&None)))
        .map(|(diagnostic, fingerprint)| {
            let mut candidate = Candidate::new(diagnostic);
            candidate.fingerprint = fingerprint.map(DeltaFingerprintV1);
            candidate.place_through(map);
            candidate
        })
        .collect::<Vec<_>>();
    let current_candidates = candidates(current, current_root);
    Ok(match_candidates(&baseline_candidates, &current_candidates))
}

fn limit_exceeded() -> InternalError {
    InternalError::new(
        STAGE,
        "delta-limit-exceeded",
        format!("Baseline comparison exceeds {DIAGNOSTIC_LIMIT} diagnostics on one side."),
    )
}

fn candidates<'a>(diagnostics: &'a [Diagnostic], root: &Path) -> Vec<Candidate<'a>> {
    let mut candidates = diagnostics.iter().map(Candidate::new).collect::<Vec<_>>();
    EvidenceLoader::new(root).populate(&mut candidates);
    candidates
}

fn structural_identity(diagnostic: &Diagnostic) -> Option<&str> {
    let definition = crate::policy::find(diagnostic.code.as_deref()?)?;
    matches!(definition.producer, Producer::Structure).then_some(diagnostic.id.as_str())
}

fn structural_fingerprint(identity: &str) -> DeltaFingerprintV1 {
    let mut hasher = blake3::Hasher::new();
    hash_field(&mut hasher, FINGERPRINT_DOMAIN.as_bytes());
    hash_field(&mut hasher, b"structure");
    hash_field(&mut hasher, identity.as_bytes());
    DeltaFingerprintV1(*hasher.finalize().as_bytes())
}

fn stable_fingerprint(diagnostic: &Diagnostic, proof: &str) -> DeltaFingerprintV1 {
    let mut hasher = blake3::Hasher::new();
    hash_field(&mut hasher, FINGERPRINT_DOMAIN.as_bytes());
    hash_field(&mut hasher, diagnostic.source.as_str().as_bytes());
    match diagnostic.code.as_deref() {
        Some(code) => {
            hasher.update(&[1]);
            hash_field(&mut hasher, code.as_bytes());
        }
        None => {
            hasher.update(&[0]);
        }
    }
    hash_field(&mut hasher, diagnostic.message.as_bytes());
    hash_field(&mut hasher, proof.as_bytes());
    DeltaFingerprintV1(*hasher.finalize().as_bytes())
}

fn hash_field(hasher: &mut blake3::Hasher, value: &[u8]) {
    hasher.update(&(value.len() as u64).to_le_bytes());
    hasher.update(value);
}

fn extract_proof(
    source: &str,
    lines: &LineIndex,
    span: &DiagnosticSpan,
    remaining_budget: usize,
) -> Option<String> {
    let start = lines.offset(
        source,
        SourcePosition {
            line: span.line_start,
            column: span.column_start,
        },
    )?;
    let end = lines.offset(
        source,
        SourcePosition {
            line: span.line_end,
            column: span.column_end,
        },
    )?;
    if start > end || end.checked_sub(start)? > PROOF_BYTES_LIMIT {
        return None;
    }
    normalize_proof(source.get(start..end)?, remaining_budget)
}

fn normalize_proof(source: &str, remaining_budget: usize) -> Option<String> {
    let bound = remaining_budget.min(PROOF_BYTES_LIMIT);
    let mut normalized = String::with_capacity(source.len().min(bound));
    for segment in source.split_whitespace() {
        let separator = usize::from(!normalized.is_empty());
        if normalized.len() + separator + segment.len() > bound {
            return None;
        }
        if separator == 1 {
            normalized.push(' ');
        }
        normalized.push_str(segment);
    }
    (!normalized.is_empty()).then_some(normalized)
}

fn same_path_stable<'a>(
    candidate: &Candidate<'a>,
) -> Option<(Option<&'a str>, DeltaFingerprintV1)> {
    Some((candidate.place, candidate.fingerprint?))
}

fn same_path_unproven<'a>(candidate: &Candidate<'a>) -> Option<(Option<&'a str>, FallbackKey<'a>)> {
    candidate
        .fingerprint
        .is_none()
        .then(|| (candidate.place, candidate.fallback()))
}

fn same_path_proven<'a>(candidate: &Candidate<'a>) -> Option<(Option<&'a str>, FallbackKey<'a>)> {
    candidate
        .fingerprint
        .is_some()
        .then(|| (candidate.place, candidate.fallback()))
}

fn same_path_fallback<'a>(candidate: &Candidate<'a>) -> Option<(Option<&'a str>, FallbackKey<'a>)> {
    Some((candidate.place, candidate.fallback()))
}

fn same_rule<'a>(
    candidate: &Candidate<'a>,
) -> Option<(Option<&'a str>, DiagnosticSource, Option<&'a str>)> {
    Some((
        candidate.place,
        candidate.diagnostic.source,
        candidate.diagnostic.code.as_deref(),
    ))
}

fn moved_stable(candidate: &Candidate<'_>) -> Option<DeltaFingerprintV1> {
    candidate.fingerprint
}

fn match_candidates(baseline: &[Candidate<'_>], current: &[Candidate<'_>]) -> DeltaReport {
    let mut matching = Matching::new(baseline, current);

    matching.pass(same_path_stable, same_path_stable);
    matching.pass(same_path_unproven, same_path_proven);
    matching.pass(same_path_fallback, same_path_unproven);
    let moved = matching.pass(moved_stable, moved_stable);
    debug_assert!(
        moved.iter().all(|&(baseline_index, current_index)| {
            baseline.get(baseline_index).map(|candidate| candidate.place)
                != current.get(current_index).map(|candidate| candidate.place)
        }),
        "same-place stable candidates must be exhausted before cross-file matching"
    );
    let counted = matching.pass(same_rule, same_rule);
    matching.into_report(counted.len())
}

struct Matching<'a, 'd> {
    baseline: &'a [Candidate<'d>],
    current: &'a [Candidate<'d>],
    consumed: Vec<bool>,
    matched: Vec<Option<usize>>,
}

impl<'a, 'd> Matching<'a, 'd> {
    fn new(baseline: &'a [Candidate<'d>], current: &'a [Candidate<'d>]) -> Self {
        Self {
            baseline,
            current,
            consumed: vec![false; baseline.len()],
            matched: vec![None; current.len()],
        }
    }

    fn pass<Key: Ord>(
        &mut self,
        baseline_key: impl Fn(&Candidate<'d>) -> Option<Key>,
        current_key: impl Fn(&Candidate<'d>) -> Option<Key>,
    ) -> Vec<(usize, usize)> {
        let (baseline, current) = (self.baseline, self.current);
        let mut groups = BTreeMap::<Key, (Vec<(usize, usize)>, BTreeSet<(usize, usize)>)>::new();
        for (index, candidate) in baseline.iter().enumerate() {
            if self.consumed.get(index).is_some_and(|consumed| !consumed)
                && let Some(key) = baseline_key(candidate)
            {
                groups.entry(key).or_default().0.push((candidate.line, index));
            }
        }
        for (index, candidate) in current.iter().enumerate() {
            if self.matched.get(index).is_some_and(Option::is_none)
                && let Some(key) = current_key(candidate)
                && let Some((_, free)) = groups.get_mut(&key)
            {
                free.insert((candidate.line, index));
            }
        }

        let mut made = Vec::new();
        for (mut bases, mut free) in groups.into_values() {
            bases.sort_unstable();
            for (line, baseline_index) in bases {
                let Some(nearest) = nearest(&free, line) else {
                    break;
                };
                free.remove(&nearest);
                let current_index = nearest.1;
                if let Some(consumed) = self.consumed.get_mut(baseline_index) {
                    *consumed = true;
                }
                if let Some(matched) = self.matched.get_mut(current_index) {
                    *matched = Some(baseline_index);
                }
                made.push((baseline_index, current_index));
            }
        }
        made
    }

    fn into_report(self, rule_count_matches: usize) -> DeltaReport {
        let cross_file_matches = self
            .current
            .iter()
            .zip(&self.matched)
            .filter(|(candidate, matched)| {
                matched
                    .and_then(|index| self.baseline.get(index))
                    .is_some_and(|base| base.path() != candidate.path())
            })
            .count();
        let introduced = self
            .current
            .iter()
            .zip(&self.matched)
            .filter(|(_, matched)| matched.is_none())
            .map(|(candidate, _)| candidate.diagnostic.id.clone())
            .collect::<Vec<_>>();
        let pre_existing = self
            .current
            .iter()
            .zip(&self.matched)
            .filter_map(|(candidate, matched)| {
                Some(DeltaMatch {
                    current_id: candidate.diagnostic.id.clone(),
                    baseline_id: self.baseline.get((*matched)?)?.diagnostic.id.clone(),
                })
            })
            .collect::<Vec<_>>();
        let fixed = self
            .baseline
            .iter()
            .zip(&self.consumed)
            .filter(|(_, consumed)| !**consumed)
            .map(|(candidate, _)| candidate.diagnostic.clone())
            .collect::<Vec<_>>();

        DeltaReport {
            fingerprint_version: FINGERPRINT_VERSION,
            base_diagnostics: self.baseline.len(),
            current_diagnostics: self.current.len(),
            summary: DeltaSummary {
                introduced: introduced.len(),
                pre_existing: pre_existing.len(),
                fixed: fixed.len(),
                cross_file_matches,
                rule_count_matches,
            },
            introduced,
            pre_existing,
            fixed,
        }
    }
}

fn nearest(free: &BTreeSet<(usize, usize)>, line: usize) -> Option<(usize, usize)> {
    let below = free.range(..(line, 0)).next_back().copied();
    let above = free.range((line, 0)..).next().copied();
    match (below, above) {
        (Some(below), Some(above)) => {
            Some(if line - below.0 <= above.0 - line { below } else { above })
        }
        (below, above) => below.or(above),
    }
}

#[cfg(test)]
mod tests;
