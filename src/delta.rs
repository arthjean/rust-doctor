use std::collections::{BTreeMap, VecDeque};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use serde::Serialize;

use crate::internal_error::InternalError;
use crate::policy::Producer;
use crate::report::{Diagnostic, DiagnosticSpan};

const STAGE: &str = "delta";
pub(crate) const FINGERPRINT_VERSION: u8 = 1;
pub(crate) const DIAGNOSTIC_LIMIT: usize = 50_000;
const PROOF_BYTES_LIMIT: usize = 65_536;
const SOURCE_FILE_BYTES_LIMIT: usize = 8 * 1024 * 1024;
const EVIDENCE_BYTES_LIMIT: usize = 64 * 1024 * 1024;
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
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct DeltaFingerprintV1([u8; 32]);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct FallbackKey {
    source: String,
    code: Option<String>,
    message: String,
}

#[derive(Debug)]
struct Candidate {
    path: Option<String>,
    fallback: FallbackKey,
    fingerprint: Option<DeltaFingerprintV1>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct SourcePosition {
    line: usize,
    column: usize,
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

    fn populate(&mut self, diagnostics: &[Diagnostic], candidates: &mut [Candidate]) {
        let Some(root) = self.root.clone() else {
            return;
        };
        let mut diagnostics_by_path = BTreeMap::<String, Vec<usize>>::new();
        for (index, diagnostic) in diagnostics.iter().enumerate() {
            // A diagnostic that already carries an identity of its own needs no
            // excerpt, and reading one for it would spend the evidence budget on
            // a fingerprint it is not going to use.
            if diagnostic.span.is_some()
                && candidates
                    .get(index)
                    .is_some_and(|candidate| candidate.fingerprint.is_none())
                && let Some(path) = diagnostic.path.as_ref()
            {
                diagnostics_by_path
                    .entry(path.clone())
                    .or_default()
                    .push(index);
            }
        }

        for (path, indexes) in diagnostics_by_path {
            let Some(source) = self.read_source(&root, &path) else {
                continue;
            };
            let positions = requested_positions(diagnostics, &indexes);
            let offsets = resolve_positions(&source, &positions);
            for index in indexes {
                let Some(span) = diagnostics[index].span.as_ref() else {
                    continue;
                };
                let Some(proof) = self.proof(&source, span, &offsets) else {
                    continue;
                };
                candidates[index].fingerprint =
                    Some(stable_fingerprint(&diagnostics[index], &proof));
            }
        }
    }

    fn read_source(&mut self, root: &Path, logical_path: &str) -> Option<String> {
        let relative = Path::new(logical_path);
        if relative.is_absolute()
            || !relative
                .components()
                .all(|component| matches!(component, Component::Normal(_)))
        {
            return None;
        }
        let path = root.join(relative).canonicalize().ok()?;
        if !path.starts_with(root) {
            return None;
        }
        let metadata = fs::metadata(&path).ok()?;
        let length = usize::try_from(metadata.len()).ok()?;
        if !metadata.is_file()
            || length > SOURCE_FILE_BYTES_LIMIT
            || self.source_bytes_read.checked_add(length)? > EVIDENCE_BYTES_LIMIT
        {
            return None;
        }
        self.source_bytes_read += length;
        let mut source = String::with_capacity(length);
        File::open(path)
            .ok()?
            .take(SOURCE_FILE_BYTES_LIMIT as u64)
            .read_to_string(&mut source)
            .ok()?;
        (source.len() == length).then_some(source)
    }

    fn proof(
        &mut self,
        source: &str,
        span: &DiagnosticSpan,
        offsets: &BTreeMap<SourcePosition, usize>,
    ) -> Option<String> {
        let remaining = EVIDENCE_BYTES_LIMIT.checked_sub(self.proof_bytes)?;
        if remaining == 0 {
            return None;
        }
        let proof = extract_proof(source, span, offsets, remaining)?;
        self.proof_bytes = self.proof_bytes.checked_add(proof.len())?;
        Some(proof)
    }
}

pub(crate) fn compute(
    baseline: &[Diagnostic],
    current: &[Diagnostic],
    baseline_root: &Path,
    current_root: &Path,
) -> Result<DeltaReport, InternalError> {
    if baseline.len() > DIAGNOSTIC_LIMIT || current.len() > DIAGNOSTIC_LIMIT {
        return Err(limit_exceeded());
    }

    let baseline_candidates = candidates(baseline, baseline_root);
    let current_candidates = candidates(current, current_root);
    Ok(match_candidates(
        baseline,
        current,
        &baseline_candidates,
        &current_candidates,
    ))
}

/// The comparison refuses more diagnostics than it can pair, under its own
/// stage.
///
/// It used to answer with the git baseline's own failure, so a run that hit the
/// diagnostic ceiling published `stage: "baseline"` with
/// `baseline-limit-exceeded` and told the reader their git snapshot exceeded a
/// limit, which was true of nothing: the snapshot is fine, and nothing in this
/// module shells out to git at all.
fn limit_exceeded() -> InternalError {
    InternalError::new(
        STAGE,
        "delta-limit-exceeded",
        format!("Baseline comparison exceeds {DIAGNOSTIC_LIMIT} diagnostics on one side."),
    )
}

fn candidates(diagnostics: &[Diagnostic], root: &Path) -> Vec<Candidate> {
    let mut candidates = diagnostics
        .iter()
        .map(|diagnostic| Candidate {
            path: diagnostic.path.clone(),
            fallback: fallback_key(diagnostic),
            fingerprint: structural_identity(diagnostic).map(structural_fingerprint),
        })
        .collect::<Vec<_>>();
    EvidenceLoader::new(root).populate(diagnostics, &mut candidates);
    candidates
}

/// The identity a structural finding already publishes, when the diagnostic is
/// one.
///
/// A structural finding is a family, and the identity the structural pass
/// computes for it is the normalized content of that family: no span, no path,
/// no measured count. That is what an appariement needs here, and it is what the
/// message and the source excerpt below cannot give, because every structural
/// message states a number the next edit moves: the line count of a file, the
/// occurrence count of a clone family, the two complexity figures of a hotspot.
/// Matched on those, a finding older than the branch reads as introduced by it,
/// and the one it replaced reads as fixed.
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

fn fallback_key(diagnostic: &Diagnostic) -> FallbackKey {
    FallbackKey {
        source: diagnostic.source.to_string(),
        code: diagnostic.code.clone(),
        message: diagnostic.message.clone(),
    }
}

fn stable_fingerprint(diagnostic: &Diagnostic, proof: &str) -> DeltaFingerprintV1 {
    let mut hasher = blake3::Hasher::new();
    hash_field(&mut hasher, FINGERPRINT_DOMAIN.as_bytes());
    hash_field(&mut hasher, diagnostic.source.to_string().as_bytes());
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

fn requested_positions(diagnostics: &[Diagnostic], indexes: &[usize]) -> Vec<SourcePosition> {
    let mut positions = indexes
        .iter()
        .filter_map(|index| diagnostics[*index].span.as_ref())
        .flat_map(|span| {
            [
                SourcePosition {
                    line: span.line_start,
                    column: span.column_start,
                },
                SourcePosition {
                    line: span.line_end,
                    column: span.column_end,
                },
            ]
        })
        .collect::<Vec<_>>();
    positions.sort_unstable();
    positions.dedup();
    positions
}

fn resolve_positions(
    source: &str,
    requested: &[SourcePosition],
) -> BTreeMap<SourcePosition, usize> {
    let mut offsets = BTreeMap::new();
    let mut requested = requested.iter().copied().peekable();
    let mut current = SourcePosition { line: 1, column: 1 };

    for (offset, character) in source.char_indices() {
        record_offset(current, offset, &mut requested, &mut offsets);
        if requested.peek().is_none() {
            break;
        }
        if character == '\n' {
            current.line += 1;
            current.column = 1;
        } else {
            current.column += 1;
        }
    }
    record_offset(current, source.len(), &mut requested, &mut offsets);
    offsets
}

fn record_offset<I: Iterator<Item = SourcePosition>>(
    current: SourcePosition,
    offset: usize,
    requested: &mut std::iter::Peekable<I>,
    offsets: &mut BTreeMap<SourcePosition, usize>,
) {
    while requested.peek().is_some_and(|position| *position < current) {
        requested.next();
    }
    if requested
        .peek()
        .is_some_and(|position| *position == current)
    {
        offsets.insert(current, offset);
        requested.next();
    }
}

fn extract_proof(
    source: &str,
    span: &DiagnosticSpan,
    offsets: &BTreeMap<SourcePosition, usize>,
    remaining_budget: usize,
) -> Option<String> {
    let start = *offsets.get(&SourcePosition {
        line: span.line_start,
        column: span.column_start,
    })?;
    let end = *offsets.get(&SourcePosition {
        line: span.line_end,
        column: span.column_end,
    })?;
    if start > end || end - start > PROOF_BYTES_LIMIT {
        return None;
    }
    normalize_proof(source.get(start..end)?, remaining_budget)
}

fn normalize_proof(source: &str, remaining_budget: usize) -> Option<String> {
    let mut normalized_length = 0_usize;
    for segment in source.split_whitespace() {
        let separator = usize::from(normalized_length != 0);
        normalized_length = normalized_length
            .checked_add(separator)?
            .checked_add(segment.len())?;
        if normalized_length > remaining_budget || normalized_length > PROOF_BYTES_LIMIT {
            return None;
        }
    }
    if normalized_length == 0 {
        return None;
    }

    let mut normalized = String::with_capacity(normalized_length);
    for segment in source.split_whitespace() {
        if !normalized.is_empty() {
            normalized.push(' ');
        }
        normalized.push_str(segment);
    }
    Some(normalized)
}

fn match_candidates(
    baseline: &[Diagnostic],
    current: &[Diagnostic],
    baseline_candidates: &[Candidate],
    current_candidates: &[Candidate],
) -> DeltaReport {
    let mut consumed = vec![false; baseline.len()];
    let mut matches = vec![None; current.len()];

    match_indexed(
        baseline_candidates,
        current_candidates,
        &mut consumed,
        &mut matches,
        |candidate| {
            candidate
                .fingerprint
                .clone()
                .map(|fingerprint| (candidate.path.clone(), fingerprint))
        },
        |candidate| {
            candidate
                .fingerprint
                .clone()
                .map(|fingerprint| (candidate.path.clone(), fingerprint))
        },
        false,
    );
    match_indexed(
        baseline_candidates,
        current_candidates,
        &mut consumed,
        &mut matches,
        |candidate| {
            candidate
                .fingerprint
                .is_none()
                .then(|| (candidate.path.clone(), candidate.fallback.clone()))
        },
        |candidate| {
            candidate
                .fingerprint
                .is_some()
                .then(|| (candidate.path.clone(), candidate.fallback.clone()))
        },
        false,
    );
    match_indexed(
        baseline_candidates,
        current_candidates,
        &mut consumed,
        &mut matches,
        |candidate| Some((candidate.path.clone(), candidate.fallback.clone())),
        |candidate| {
            candidate
                .fingerprint
                .is_none()
                .then(|| (candidate.path.clone(), candidate.fallback.clone()))
        },
        false,
    );
    match_indexed(
        baseline_candidates,
        current_candidates,
        &mut consumed,
        &mut matches,
        |candidate| candidate.fingerprint.clone(),
        |candidate| candidate.fingerprint.clone(),
        true,
    );

    let introduced = current
        .iter()
        .zip(&matches)
        .filter(|(_, matched)| matched.is_none())
        .map(|(diagnostic, _)| diagnostic.id.clone())
        .collect::<Vec<_>>();
    let pre_existing = current
        .iter()
        .zip(&matches)
        .filter_map(|(diagnostic, matched)| {
            matched.map(|(baseline_index, _)| DeltaMatch {
                current_id: diagnostic.id.clone(),
                baseline_id: baseline[baseline_index].id.clone(),
            })
        })
        .collect::<Vec<_>>();
    let fixed = baseline
        .iter()
        .zip(&consumed)
        .filter(|(_, consumed)| !**consumed)
        .map(|(diagnostic, _)| diagnostic.clone())
        .collect::<Vec<_>>();
    let cross_file_matches = matches
        .iter()
        .filter(|matched| matched.is_some_and(|(_, cross_file)| cross_file))
        .count();

    DeltaReport {
        fingerprint_version: FINGERPRINT_VERSION,
        base_diagnostics: baseline.len(),
        current_diagnostics: current.len(),
        summary: DeltaSummary {
            introduced: introduced.len(),
            pre_existing: pre_existing.len(),
            fixed: fixed.len(),
            cross_file_matches,
        },
        introduced,
        pre_existing,
        fixed,
    }
}

fn match_indexed<Key: Ord>(
    baseline: &[Candidate],
    current: &[Candidate],
    consumed: &mut [bool],
    matches: &mut [Option<(usize, bool)>],
    baseline_key: impl Fn(&Candidate) -> Option<Key>,
    current_key: impl Fn(&Candidate) -> Option<Key>,
    cross_file: bool,
) {
    let mut available = BTreeMap::<Key, VecDeque<usize>>::new();
    for (index, candidate) in baseline.iter().enumerate() {
        if !consumed[index]
            && let Some(key) = baseline_key(candidate)
        {
            available.entry(key).or_default().push_back(index);
        }
    }
    for (current_index, current_candidate) in current.iter().enumerate() {
        if matches[current_index].is_some() {
            continue;
        }
        if let Some(key) = current_key(current_candidate)
            && let Some(baseline_index) = available.get_mut(&key).and_then(VecDeque::pop_front)
        {
            debug_assert!(
                !cross_file || baseline[baseline_index].path != current_candidate.path,
                "same-path stable candidates must be exhausted before cross-file matching"
            );
            consumed[baseline_index] = true;
            matches[current_index] = Some((baseline_index, cross_file));
        }
    }
}

#[cfg(test)]
mod tests;
