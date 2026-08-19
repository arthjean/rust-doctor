//! Which findings a branch introduced, which it inherited, and which it fixed.
//!
//! A baseline comparison is a multiset pairing between two independent
//! scans, and the only thing it may not do is guess. Two diagnostics are the
//! same finding when the code under them is the same code, so the identity this
//! module builds is evidence first: the normalized source excerpt the span
//! covers, hashed with the rule and the message. What a report publishes is not
//! enough on its own, since a line number moves on every edit above it and a
//! message states counts that the next commit changes.
//!
//! Four rules hold it together.
//!
//! A candidate is a diagnostic, not a row beside one. `Candidate` borrows the
//! `Diagnostic` it speaks for, so the pairing cannot drift out of step with
//! the report it is computed from. The two index-parallel slices it replaced
//! were passed to the matcher as four separate arguments, kept aligned by
//! nothing but the reader's attention, and every one of the ten places that
//! walked them indexed a slice a length mismatch would have panicked on, in a
//! crate that denies `panic`.
//!
//! Every pass is named, and the pass says what a match on it means. The four
//! passes used to be four calls carrying six anonymous closures, two of them
//! identical word for word, and a trailing positional `bool` announcing that the
//! match was a move. Nothing tied that flag to the key: the key of the last
//! pass omits the path, and `cross_file` said so a second time, so the count
//! the report publishes could disagree with the pairing that produced it.
//! The count is now the length of what the moved pass returned.
//!
//! A bound is a budget on work, never a filter on meaning. `SOURCE_BYTES_BUDGET`
//! bounds what one comparison may read, `PROOF_BYTES_BUDGET` what it may
//! normalize, `SOURCE_FILE_BYTES_LIMIT` and `PROOF_BYTES_LIMIT` what one file
//! and one excerpt may contribute, and none of them decides which finding is
//! matchable: a diagnostic whose evidence is out of budget falls back to its
//! message rather than disappearing. The two budgets used to share one constant
//! named for neither.
//!
//! The stage a failure names is this one. Refusing a comparison over
//! `DIAGNOSTIC_LIMIT` diagnostics used to be reported with the git baseline's
//! own failure, so a run that hit the diagnostic ceiling published
//! `stage: "baseline"` and told the reader their snapshot exceeded a limit,
//! which was true of nothing.

use std::collections::{BTreeMap, VecDeque};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::internal_error::InternalError;
use crate::policy::Producer;
use crate::report::{Diagnostic, DiagnosticSource, DiagnosticSpan};
use crate::workspace_path;

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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct DeltaFingerprintV1([u8; 32]);

/// What identifies a diagnostic when no proof could be read for it.
///
/// It borrows the diagnostic rather than copying it: a key lives for one pass
/// and the diagnostics outlive every pass, so the four passes used to clone the
/// message of every finding on both sides for nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct FallbackKey<'a> {
    source: DiagnosticSource,
    code: Option<&'a str>,
    message: &'a str,
}

/// One diagnostic and the strongest identity that could be built for it.
#[derive(Debug)]
struct Candidate<'a> {
    diagnostic: &'a Diagnostic,
    fingerprint: Option<DeltaFingerprintV1>,
}

impl<'a> Candidate<'a> {
    fn new(diagnostic: &'a Diagnostic) -> Self {
        Self {
            diagnostic,
            fingerprint: structural_identity(diagnostic).map(structural_fingerprint),
        }
    }

    fn path(&self) -> Option<&'a str> {
        self.diagnostic.path.as_deref()
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

/// Where every line of a source file starts.
///
/// A span reports a 1-based line and a 1-based character column, and turning
/// that into a byte offset is the only arithmetic the evidence needs. Building
/// the index once per file answers every span of that file, which is what the
/// sorted, deduplicated position set and the peekable state machine it replaced
/// did at three functions and a helper mutating two collections at once.
struct LineIndex(Vec<usize>);

impl LineIndex {
    fn of(source: &str) -> Self {
        Self(
            std::iter::once(0)
                .chain(source.match_indices('\n').map(|(index, _)| index + 1))
                .collect(),
        )
    }

    /// Byte offset of a reported position, absent when the file does not reach
    /// it.
    ///
    /// A line owns its terminator, so the column of the `\n` is the last one a
    /// line answers. The position one past the final character of the file is
    /// answered too, because that is what a span covering the last line of a
    /// file with no trailing newline reports.
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

/// Reads the scanned sources one comparison is allowed to read, and stamps the
/// candidates it could build a proof for.
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
            // A diagnostic that already carries an identity of its own needs no
            // excerpt, and reading one for it would spend the evidence budget on
            // a fingerprint it is not going to use.
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
        // Opening a path that is not a regular file is not merely useless:
        // opening a named pipe blocks in the call itself, before any check on
        // the handle could reject it.
        let metadata = fs::symlink_metadata(&path).ok()?;
        let length = usize::try_from(metadata.len()).ok()?;
        if !metadata.is_file()
            || length > SOURCE_FILE_BYTES_LIMIT
            || self.source_bytes_read.checked_add(length)? > SOURCE_BYTES_BUDGET
        {
            return None;
        }
        self.source_bytes_read = self.source_bytes_read.checked_add(length)?;

        // Canonicalizing and then opening has a replacement race, and the code
        // frame closes it the same way: open the checked path, then confirm the
        // handle and the live path still identify one file inside the workspace.
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

/// The identity a structural finding already publishes, when the diagnostic is
/// one.
///
/// A structural finding is a family, and the identity the structural pass
/// computes for it is the normalized content of that family: no span, no path,
/// no measured count. That is what a pairing needs here, and it is what the
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

/// The excerpt reduced to its words, so reindenting a block does not read as
/// rewriting it.
///
/// The budget is checked as the excerpt is built rather than measured first and
/// built again: an excerpt that overruns answers `None` either way, and the
/// caller has already bounded the input at `PROOF_BYTES_LIMIT`.
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

/// Same file, same proof: the finding did not move and the code under it did
/// not change.
fn same_path_stable<'a>(
    candidate: &Candidate<'a>,
) -> Option<(Option<&'a str>, DeltaFingerprintV1)> {
    Some((candidate.path(), candidate.fingerprint?))
}

/// Same file, same message, from a side that has no proof to offer.
fn same_path_unproven<'a>(candidate: &Candidate<'a>) -> Option<(Option<&'a str>, FallbackKey<'a>)> {
    candidate
        .fingerprint
        .is_none()
        .then(|| (candidate.path(), candidate.fallback()))
}

/// Same file, same message, from a side that does carry a proof.
fn same_path_proven<'a>(candidate: &Candidate<'a>) -> Option<(Option<&'a str>, FallbackKey<'a>)> {
    candidate
        .fingerprint
        .is_some()
        .then(|| (candidate.path(), candidate.fallback()))
}

/// Same file, same message, whatever proof the side carries.
fn same_path_fallback<'a>(candidate: &Candidate<'a>) -> Option<(Option<&'a str>, FallbackKey<'a>)> {
    Some((candidate.path(), candidate.fallback()))
}

/// Same proof, any file: the finding moved.
fn moved_stable(candidate: &Candidate<'_>) -> Option<DeltaFingerprintV1> {
    candidate.fingerprint
}

fn match_candidates(baseline: &[Candidate<'_>], current: &[Candidate<'_>]) -> DeltaReport {
    let mut matching = Matching::new(baseline, current);

    // A finding whose file and whose proof are both unchanged is the same
    // finding. Nothing weaker is consulted while a pairing this strong is
    // available, which is what keeps a copied line from consuming the original.
    matching.pass(same_path_stable, same_path_stable);
    // From here at least one side has no proof, so the message is all there is.
    // The proofless baselines are spent first, which reserves a baseline that
    // does carry a proof for a current that carries one too.
    matching.pass(same_path_unproven, same_path_proven);
    matching.pass(same_path_fallback, same_path_unproven);
    // Same proof in another file: the finding moved. It runs last, so a message
    // match on the original file wins over a proof match elsewhere. That is a
    // product decision rather than a consequence, and
    // `a_message_match_on_the_original_file_wins_over_a_moved_proof` is the
    // input that puts the two in competition.
    let moved = matching.pass(moved_stable, moved_stable);

    debug_assert!(
        moved.iter().all(|&(baseline_index, current_index)| {
            baseline.get(baseline_index).map(Candidate::path)
                != current.get(current_index).map(Candidate::path)
        }),
        "same-path stable candidates must be exhausted before cross-file matching"
    );
    matching.into_report(moved.len())
}

/// The state of one pairing: which baseline candidates are spent, and which
/// baseline each current candidate was paired with.
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

    /// Pairs what is still free on the key the two sides answer on, in index
    /// order, and returns the pairs it made.
    ///
    /// A side that answers `None` sits the pass out. Both sides are walked in
    /// index order and equal keys queue, so the same two scans always produce
    /// the same pairing.
    fn pass<Key: Ord>(
        &mut self,
        baseline_key: impl Fn(&Candidate<'d>) -> Option<Key>,
        current_key: impl Fn(&Candidate<'d>) -> Option<Key>,
    ) -> Vec<(usize, usize)> {
        let (baseline, current) = (self.baseline, self.current);
        let mut available = BTreeMap::<Key, VecDeque<usize>>::new();
        for (index, candidate) in baseline.iter().enumerate() {
            if self.consumed.get(index).is_some_and(|consumed| !consumed)
                && let Some(key) = baseline_key(candidate)
            {
                available.entry(key).or_default().push_back(index);
            }
        }

        let mut made = Vec::new();
        for (current_index, candidate) in current.iter().enumerate() {
            if self.matched.get(current_index).is_none_or(Option::is_some) {
                continue;
            }
            let Some(key) = current_key(candidate) else {
                continue;
            };
            let Some(baseline_index) = available.get_mut(&key).and_then(VecDeque::pop_front) else {
                continue;
            };
            if let Some(consumed) = self.consumed.get_mut(baseline_index) {
                *consumed = true;
            }
            if let Some(matched) = self.matched.get_mut(current_index) {
                *matched = Some(baseline_index);
            }
            made.push((baseline_index, current_index));
        }
        made
    }

    fn into_report(self, cross_file_matches: usize) -> DeltaReport {
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
            },
            introduced,
            pre_existing,
            fixed,
        }
    }
}

#[cfg(test)]
mod tests;
