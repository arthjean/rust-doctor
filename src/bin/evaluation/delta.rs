use super::manifest::{hex_digest, read_json, sha256_file, write_json_atomic};
use super::model::{
    CORPUS_SCHEMA_VERSION, CorpusRecord, DELTA_SCHEMA_VERSION, DeltaReport, DiagnosticLabel,
    EvaluationDiagnostic, EvaluationRule, EvidenceApproval, LabelFile, PromotionReview,
    PromotionReviewTemplate, PromotionRuleSample, ReviewLabel, RootState, RuleDelta, RuntimeDelta,
};
use super::{EvalError, Result};
use crate::DeltaArgs;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Component, Path};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct DiagnosticKey {
    repository: String,
    package_root: String,
    rule: String,
    site_id: String,
    baseline_key: String,
}

#[derive(Default)]
struct MutableRuleDelta {
    introduced: usize,
    removed: usize,
    changed: usize,
    baseline_count: usize,
    candidate_count: usize,
    affected_roots: HashSet<(String, String)>,
    affected_repositories: BTreeMap<String, usize>,
}

struct Comparison {
    introduced: Vec<EvaluationDiagnostic>,
    introduced_count: usize,
    removed_count: usize,
    changed_count: usize,
    affected_roots: HashSet<(String, String)>,
    per_rule: BTreeMap<String, MutableRuleDelta>,
}

#[expect(
    clippy::needless_pass_by_value,
    clippy::too_many_lines,
    reason = "the owned Clap command drives one complete comparison and gate transaction"
)]
pub(crate) fn run(args: DeltaArgs) -> Result<()> {
    let baseline = read_ndjson(&args.baseline)?;
    let candidate = read_ndjson(&args.candidate)?;
    validate_pair(&baseline, &candidate)?;
    let baseline_sha256 = sha256_file(&args.baseline)?;
    let candidate_sha256 = sha256_file(&args.candidate)?;
    validate_approval(&args.baseline_approval, &baseline_sha256, "baseline")?;
    let candidate_approval = args
        .candidate_approval
        .as_deref()
        .map(|path| validate_approval(path, &candidate_sha256, "candidate"))
        .transpose()?;
    let approved_replacement = candidate_approval.is_some();
    let baseline_catalog = corpus_catalog(&baseline)?;
    let candidate_catalog = corpus_catalog(&candidate)?;
    let promoted_rules = derive_promoted_rules(baseline_catalog, candidate_catalog);

    let baseline_by_repository = by_repository(&baseline);
    let candidate_by_repository = by_repository(&candidate);
    let repository_pairs: Vec<_> = baseline_by_repository
        .iter()
        .filter_map(|(repository, baseline)| {
            let candidate = candidate_by_repository.get(repository)?;
            Some((*baseline, *candidate))
        })
        .collect();
    let complete_pairs: Vec<_> = repository_pairs
        .iter()
        .copied()
        .filter(|(baseline, candidate)| baseline.complete && candidate.complete)
        .collect();
    let comparison = compare_diagnostics(&repository_pairs);
    let complete_baseline_roots = complete_root_count(&baseline);
    let complete_candidate_roots = complete_root_count(&candidate);
    let total_baseline_roots = total_root_count(&baseline);
    let total_candidate_roots = total_root_count(&candidate);
    let affected_root_percent = percent(
        comparison.affected_roots.len(),
        complete_baseline_roots.max(1),
    );
    let baseline_incomplete = total_baseline_roots.saturating_sub(complete_baseline_roots);
    let candidate_incomplete = total_candidate_roots.saturating_sub(complete_candidate_roots);
    let incomplete_root_change_percentage_points =
        percent(candidate_incomplete, total_candidate_roots.max(1))
            - percent(baseline_incomplete, total_baseline_roots.max(1));
    let mut runtime_deltas = runtime_deltas(&complete_pairs);
    let median_runtime_delta_percent = percentile(
        &mut runtime_deltas
            .iter()
            .map(|delta| delta.delta_percent)
            .collect::<Vec<_>>(),
        50,
    );
    let p95_runtime_delta_percent = percentile(
        &mut runtime_deltas
            .iter()
            .map(|delta| delta.delta_percent)
            .collect::<Vec<_>>(),
        95,
    );
    runtime_deltas.sort_by(|left, right| {
        right
            .delta_percent
            .total_cmp(&left.delta_percent)
            .then_with(|| left.repository.cmp(&right.repository))
    });
    runtime_deltas.truncate(10);

    // The evaluation profile forces candidate rules on in both runs. A rule can
    // therefore become default-enabled without producing an "introduced"
    // delta, so promotion review samples the candidate population itself.
    let promotion_population: Vec<_> = repository_pairs
        .iter()
        .flat_map(|(baseline, candidate)| {
            candidate.diagnostics.iter().filter(|diagnostic| {
                baseline.root_states.get(&diagnostic.package_root) == Some(&RootState::Complete)
                    && candidate.root_states.get(&diagnostic.package_root)
                        == Some(&RootState::Complete)
            })
        })
        .cloned()
        .collect();
    if let Some(path) = args.review_template.as_deref() {
        write_json_atomic(
            path,
            &promotion_review_template(&promotion_population, &promoted_rules, &candidate_sha256),
        )?;
    }
    let labels = args.labels.as_deref().map_or_else(
        || Ok(Vec::new()),
        |path| read_labels(path, &candidate_sha256, candidate_approval.as_ref()),
    )?;
    let promotion_reviews = promotion_reviews(&promotion_population, &promoted_rules, &labels);
    let mut reasons = Vec::new();
    if affected_root_percent > 0.5 && !approved_replacement {
        reasons.push(format!(
            "diagnostic increases affect {affected_root_percent:.3}% of complete roots, above 0.5%"
        ));
    }
    if incomplete_root_change_percentage_points > 0.2 {
        reasons.push(format!(
            "incomplete roots increased by {incomplete_root_change_percentage_points:.3} percentage points, above 0.2"
        ));
    }
    for review in &promotion_reviews {
        if !review.eligible_for_default {
            reasons.push(format!(
                "promoted rule {} lacks a complete acceptable label sample ({}/{}, {:.3}% confirmed false positives)",
                review.rule,
                review.labeled,
                review.sample_size,
                review.false_positive_percent
            ));
        }
    }
    let rule_deltas: Vec<_> = comparison
        .per_rule
        .into_iter()
        .map(|(rule, delta)| RuleDelta {
            catalog_changed: catalog_rule_changed(
                baseline_catalog.get(&rule),
                candidate_catalog.get(&rule),
            ),
            count_growth: delta.candidate_count.saturating_sub(delta.baseline_count),
            affected_root_percent: percent(
                delta.affected_roots.len(),
                complete_baseline_roots.max(1),
            ),
            rule,
            introduced: delta.introduced,
            removed: delta.removed,
            changed: delta.changed,
            affected_repositories: top_affected_repositories(delta.affected_repositories),
        })
        .collect();
    for delta in &rule_deltas {
        if (delta.introduced > 0 || delta.count_growth > 0 || delta.catalog_changed)
            && delta.affected_root_percent > 0.5
            && !approved_replacement
        {
            reasons.push(format!(
                "rule {} changed across {:.3}% of complete roots, above 0.5%",
                delta.rule, delta.affected_root_percent
            ));
        }
    }
    let report = DeltaReport {
        schema_version: DELTA_SCHEMA_VERSION.to_string(),
        baseline_sha256,
        candidate_sha256,
        complete_baseline_roots,
        complete_candidate_roots,
        introduced: comparison.introduced_count,
        removed: comparison.removed_count,
        changed: comparison.changed_count,
        affected_root_percent,
        incomplete_root_change_percentage_points,
        median_runtime_delta_percent,
        p95_runtime_delta_percent,
        top_runtime_regressions: runtime_deltas,
        rule_deltas,
        promotion_reviews,
        blocked: !reasons.is_empty(),
        reasons,
    };
    write_json_atomic(&args.output, &report)?;
    if report.blocked {
        Err(EvalError::GateFailed(report.reasons.join("; ")))
    } else {
        Ok(())
    }
}

fn read_ndjson(path: &Path) -> Result<Vec<CorpusRecord>> {
    let content = std::fs::read_to_string(path)
        .map_err(|error| EvalError::io("cannot read corpus NDJSON", path, error))?;
    let mut records = Vec::new();
    for (index, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let record: CorpusRecord = serde_json::from_str(line).map_err(|error| {
            EvalError::InvalidManifest(format!(
                "{}:{} is not schema-compatible corpus NDJSON: {error}",
                path.display(),
                index + 1
            ))
        })?;
        if record.schema_version != CORPUS_SCHEMA_VERSION {
            return Err(EvalError::InvalidManifest(format!(
                "{}:{} uses corpus schema {}, expected {}",
                path.display(),
                index + 1,
                record.schema_version,
                CORPUS_SCHEMA_VERSION
            )));
        }
        validate_record(&record, path, index + 1)?;
        records.push(record);
    }
    if records.is_empty() {
        return Err(EvalError::InvalidManifest(format!(
            "corpus NDJSON '{}' is empty",
            path.display()
        )));
    }
    Ok(records)
}

fn validate_pair(baseline: &[CorpusRecord], candidate: &[CorpusRecord]) -> Result<()> {
    let baseline_keys = record_keys(baseline)?;
    let candidate_keys = record_keys(candidate)?;
    if baseline_keys != candidate_keys {
        let missing: Vec<_> = baseline_keys.difference(&candidate_keys).take(5).collect();
        let extra: Vec<_> = candidate_keys.difference(&baseline_keys).take(5).collect();
        return Err(EvalError::InvalidManifest(format!(
            "candidate and baseline pins differ; missing {missing:?}, extra {extra:?}"
        )));
    }
    let candidate_by_repository = by_repository(candidate);
    let baseline_profile = corpus_profile(baseline)?;
    let candidate_profile = corpus_profile(candidate)?;
    if baseline_profile != candidate_profile {
        return Err(EvalError::InvalidManifest(
            "candidate and baseline use different evaluation-profile fingerprints".to_string(),
        ));
    }
    for baseline_record in baseline {
        let candidate_record = candidate_by_repository
            .get(baseline_record.repository.as_str())
            .ok_or_else(|| {
                EvalError::InvalidManifest(format!(
                    "candidate omits repository {}",
                    baseline_record.repository
                ))
            })?;
        if baseline_record.expected_roots != candidate_record.expected_roots {
            return Err(EvalError::InvalidManifest(format!(
                "candidate canonical roots differ for {}",
                baseline_record.repository
            )));
        }
        for root in &baseline_record.expected_roots {
            let baseline_state = baseline_record.root_states.get(root).ok_or_else(|| {
                EvalError::InvalidManifest(format!(
                    "baseline omits root state for {}:{root}",
                    baseline_record.repository
                ))
            })?;
            let candidate_state = candidate_record.root_states.get(root).ok_or_else(|| {
                EvalError::InvalidManifest(format!(
                    "candidate omits root state for {}:{root}",
                    candidate_record.repository
                ))
            })?;
            if root_state_rank(*candidate_state) < root_state_rank(*baseline_state) {
                return Err(EvalError::GateFailed(format!(
                    "candidate completeness degraded for {}:{root} from {baseline_state:?} to {candidate_state:?}",
                    baseline_record.repository
                )));
            }
        }
    }
    Ok(())
}

fn validate_record(record: &CorpusRecord, path: &Path, line: usize) -> Result<()> {
    let invalid = |reason: &str| {
        EvalError::InvalidManifest(format!(
            "{}:{line} is not schema-compatible corpus NDJSON: {reason}",
            path.display()
        ))
    };
    if record.repository.is_empty()
        || record.tool_revision.is_empty()
        || record.package_roots.is_empty()
        || record.expected_roots.is_empty()
        || record.evaluation_profile_sha256.len() != 64
        || record.catalog_sha256.len() != 64
        || record.catalog.is_empty()
        || record.tree_digest.is_empty()
        || !(1..=3).contains(&record.attempts)
    {
        return Err(invalid("required identity or attempt fields are invalid"));
    }
    if !matches!(
        record.completeness.as_str(),
        "complete" | "partial" | "incomplete"
    ) || record.complete != (record.completeness == "complete")
    {
        return Err(invalid("completeness fields disagree"));
    }
    let mut roots = HashSet::new();
    for root in &record.package_roots {
        if !roots.insert(root.as_str()) || !safe_relative_path(root) {
            return Err(invalid("package roots must be unique relative paths"));
        }
    }
    if record.complete
        && (record.expected_roots != record.attempted_roots
            || record.attempted_roots != record.reported_roots)
    {
        return Err(invalid(
            "complete records require exact expected, attempted, and reported root equality",
        ));
    }
    let expected: BTreeSet<_> = record.expected_roots.iter().cloned().collect();
    if record.root_states.keys().cloned().collect::<BTreeSet<_>>() != expected {
        return Err(invalid(
            "root states must cover exactly the canonical expected roots",
        ));
    }
    for (root, state) in &record.root_states {
        if *state == RootState::Complete
            && (!record.attempted_roots.contains(root) || !record.reported_roots.contains(root))
        {
            return Err(invalid(
                "complete root states must be both attempted and reported",
            ));
        }
    }
    let catalog_bytes = serde_json::to_vec(&record.catalog)
        .map_err(|_| invalid("catalog cannot be fingerprinted"))?;
    if hex_digest(&catalog_bytes) != record.catalog_sha256 {
        return Err(invalid(
            "catalog fingerprint does not match catalog contents",
        ));
    }
    for diagnostic in &record.diagnostics {
        if diagnostic.repository != record.repository
            || !roots.contains(diagnostic.package_root.as_str())
            || diagnostic.rule.is_empty()
            || diagnostic.site_id.is_empty()
            || diagnostic.baseline_key.is_empty()
            || diagnostic.fingerprint.len() != 64
            || !diagnostic
                .fingerprint
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            return Err(invalid("diagnostic identity is invalid"));
        }
    }
    if record
        .failure_chain
        .iter()
        .any(|failure| !(1..=3).contains(&failure.attempt) || failure.kind.is_empty())
    {
        return Err(invalid("failure chain is invalid"));
    }
    Ok(())
}

fn safe_relative_path(path: &str) -> bool {
    path == "."
        || (!path.is_empty()
            && !path.contains('\\')
            && Path::new(path)
                .components()
                .all(|component| matches!(component, Component::Normal(_))))
}

fn record_keys(records: &[CorpusRecord]) -> Result<BTreeSet<(String, String)>> {
    let mut keys = BTreeSet::new();
    for record in records {
        if record.commit.len() != 40 || !record.commit.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(EvalError::InvalidManifest(format!(
                "repository {} is not pinned to a full commit",
                record.repository
            )));
        }
        let key = (record.repository.clone(), record.commit.clone());
        if !keys.insert(key) {
            return Err(EvalError::InvalidManifest(format!(
                "duplicate corpus record for {} at {}",
                record.repository, record.commit
            )));
        }
    }
    Ok(keys)
}

fn by_repository(records: &[CorpusRecord]) -> HashMap<&str, &CorpusRecord> {
    records
        .iter()
        .map(|record| (record.repository.as_str(), record))
        .collect()
}

fn corpus_profile(records: &[CorpusRecord]) -> Result<&str> {
    let first = records
        .first()
        .ok_or_else(|| EvalError::InvalidManifest("corpus is empty".to_string()))?
        .evaluation_profile_sha256
        .as_str();
    if records
        .iter()
        .any(|record| record.evaluation_profile_sha256 != first)
    {
        return Err(EvalError::InvalidManifest(
            "corpus records disagree on the evaluation-profile fingerprint".to_string(),
        ));
    }
    Ok(first)
}

fn corpus_catalog(records: &[CorpusRecord]) -> Result<&BTreeMap<String, EvaluationRule>> {
    let first = records
        .first()
        .ok_or_else(|| EvalError::InvalidManifest("corpus is empty".to_string()))?;
    if records.iter().any(|record| {
        record.catalog_sha256 != first.catalog_sha256 || record.catalog != first.catalog
    }) {
        return Err(EvalError::InvalidManifest(
            "corpus records disagree on the evaluated rule catalog".to_string(),
        ));
    }
    Ok(&first.catalog)
}

fn derive_promoted_rules(
    baseline: &BTreeMap<String, EvaluationRule>,
    candidate: &BTreeMap<String, EvaluationRule>,
) -> Vec<String> {
    candidate
        .iter()
        .filter(|(rule, state)| {
            state.default_enabled
                && baseline
                    .get(*rule)
                    .is_none_or(|previous| !previous.default_enabled)
        })
        .map(|(rule, _)| rule.clone())
        .collect()
}

fn catalog_rule_changed(
    baseline: Option<&EvaluationRule>,
    candidate: Option<&EvaluationRule>,
) -> bool {
    baseline != candidate
}

fn compare_diagnostics(pairs: &[(&CorpusRecord, &CorpusRecord)]) -> Comparison {
    let mut baseline = BTreeMap::new();
    let mut candidate = BTreeMap::new();
    for (baseline_record, candidate_record) in pairs {
        let eligible_roots: BTreeSet<_> = baseline_record
            .root_states
            .iter()
            .filter_map(|(root, state)| {
                (*state == RootState::Complete
                    && candidate_record.root_states.get(root) == Some(&RootState::Complete))
                .then_some(root.as_str())
            })
            .collect();
        index_diagnostics(baseline_record, &eligible_roots, &mut baseline);
        index_diagnostics(candidate_record, &eligible_roots, &mut candidate);
    }
    let keys: BTreeSet<_> = baseline.keys().chain(candidate.keys()).cloned().collect();
    let mut comparison = Comparison {
        introduced: Vec::new(),
        introduced_count: 0,
        removed_count: 0,
        changed_count: 0,
        affected_roots: HashSet::new(),
        per_rule: BTreeMap::new(),
    };
    for key in keys {
        let mut old = baseline.remove(&key).unwrap_or_default();
        let mut new = candidate.remove(&key).unwrap_or_default();
        let baseline_count = old.len();
        let candidate_count = new.len();
        old.sort_by(|left, right| left.fingerprint.cmp(&right.fingerprint));
        new.sort_by(|left, right| left.fingerprint.cmp(&right.fingerprint));
        let mut old_remaining = Vec::new();
        let mut new_remaining = new;
        for diagnostic in old {
            if let Some(index) = new_remaining
                .iter()
                .position(|candidate| candidate.fingerprint == diagnostic.fingerprint)
            {
                new_remaining.remove(index);
            } else {
                old_remaining.push(diagnostic);
            }
        }
        let changed = old_remaining.len().min(new_remaining.len());
        let removed = old_remaining.len().saturating_sub(changed);
        let introduced = new_remaining.len().saturating_sub(changed);
        comparison.changed_count += changed;
        comparison.removed_count += removed;
        comparison.introduced_count += introduced;
        comparison
            .introduced
            .extend(new_remaining.into_iter().skip(changed));
        if introduced > 0 {
            comparison
                .affected_roots
                .insert((key.repository.clone(), key.package_root.clone()));
        }
        let delta = comparison.per_rule.entry(key.rule).or_default();
        delta.baseline_count += baseline_count;
        delta.candidate_count += candidate_count;
        delta.changed += changed;
        delta.removed += removed;
        delta.introduced += introduced;
        if changed + removed + introduced > 0 {
            delta
                .affected_roots
                .insert((key.repository.clone(), key.package_root.clone()));
            *delta
                .affected_repositories
                .entry(key.repository)
                .or_insert(0) += changed + removed + introduced;
        }
    }
    comparison
}

fn top_affected_repositories(counts: BTreeMap<String, usize>) -> Vec<String> {
    let mut counts: Vec<_> = counts.into_iter().collect();
    counts.sort_by(|(left_name, left_count), (right_name, right_count)| {
        right_count
            .cmp(left_count)
            .then_with(|| left_name.cmp(right_name))
    });
    counts
        .into_iter()
        .map(|(repository, _)| repository)
        .take(10)
        .collect()
}

fn index_diagnostics(
    record: &CorpusRecord,
    eligible_roots: &BTreeSet<&str>,
    index: &mut BTreeMap<DiagnosticKey, Vec<EvaluationDiagnostic>>,
) {
    for diagnostic in record
        .diagnostics
        .iter()
        .filter(|diagnostic| eligible_roots.contains(diagnostic.package_root.as_str()))
    {
        index
            .entry(DiagnosticKey {
                repository: record.repository.clone(),
                package_root: diagnostic.package_root.clone(),
                rule: diagnostic.rule.clone(),
                site_id: diagnostic.site_id.clone(),
                baseline_key: diagnostic.baseline_key.clone(),
            })
            .or_default()
            .push(diagnostic.clone());
    }
}

fn read_labels(
    path: &Path,
    candidate_sha256: &str,
    candidate_approval: Option<&EvidenceApproval>,
) -> Result<Vec<DiagnosticLabel>> {
    let labels: LabelFile = read_json(path)?;
    if labels.schema_version != DELTA_SCHEMA_VERSION || labels.candidate_sha256 != candidate_sha256
    {
        return Err(EvalError::InvalidManifest(format!(
            "labels must use schema {DELTA_SCHEMA_VERSION} and bind the exact candidate SHA-256"
        )));
    }
    validate_approval_value(&labels.approval, candidate_sha256, "label candidate")?;
    if candidate_approval != Some(&labels.approval) {
        return Err(EvalError::InvalidManifest(
            "labels must embed the exact reviewed candidate approval supplied to the gate"
                .to_string(),
        ));
    }
    let mut seen = HashSet::new();
    for label in &labels.labels {
        if label.repository.is_empty()
            || label.package_root.is_empty()
            || label.rule.is_empty()
            || label.site_id.is_empty()
            || label.evidence_fingerprint.is_empty()
            || !seen.insert((
                label.repository.as_str(),
                label.package_root.as_str(),
                label.rule.as_str(),
                label.site_id.as_str(),
                label.evidence_fingerprint.as_str(),
            ))
        {
            return Err(EvalError::InvalidManifest(format!(
                "invalid or duplicate label for {} {}",
                label.rule, label.site_id
            )));
        }
    }
    Ok(labels.labels)
}

fn validate_approval(path: &Path, subject_sha256: &str, subject: &str) -> Result<EvidenceApproval> {
    let approval: EvidenceApproval = read_json(path)?;
    validate_approval_value(&approval, subject_sha256, subject)?;
    Ok(approval)
}

fn validate_approval_value(
    approval: &EvidenceApproval,
    subject_sha256: &str,
    subject: &str,
) -> Result<()> {
    let expected_url = format!(
        "https://github.com/{}/actions/runs/{}/artifacts/{}",
        approval.repository, approval.run_id, approval.artifact_id
    );
    if approval.schema_version != DELTA_SCHEMA_VERSION
        || approval.subject_sha256 != subject_sha256
        || approval.repository.trim().is_empty()
        || !lower_hex(&approval.head_sha, 40)
        || approval.run_id == 0
        || approval.artifact_id == 0
        || approval.artifact_name.trim().is_empty()
        || approval
            .artifact_digest
            .strip_prefix("sha256:")
            .is_none_or(|digest| !lower_hex(digest, 64))
        || approval.artifact_url != expected_url
        || approval.reviewed_by.trim().is_empty()
        || approval.reviewed_at.trim().is_empty()
        || !matches!(
            approval.review_source.as_str(),
            "protected-ci" | "protected-environment"
        )
    {
        return Err(EvalError::InvalidManifest(format!(
            "{subject} approval must bind the exact SHA-256 and GitHub identity of a protected immutable artifact"
        )));
    }
    Ok(())
}

fn lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn promotion_reviews(
    candidate_diagnostics: &[EvaluationDiagnostic],
    promoted_rules: &[String],
    labels: &[DiagnosticLabel],
) -> Vec<PromotionReview> {
    let label_map: HashMap<_, _> = labels
        .iter()
        .map(|label| {
            (
                (
                    label.repository.as_str(),
                    label.package_root.as_str(),
                    label.rule.as_str(),
                    label.site_id.as_str(),
                    label.evidence_fingerprint.as_str(),
                ),
                &label.label,
            )
        })
        .collect();
    let mut reviews = Vec::new();
    for rule in promoted_rules {
        let sample = promotion_sample(candidate_diagnostics, rule);
        let mut labeled = 0usize;
        let mut confirmed = 0usize;
        let mut false_positives = 0usize;
        let mut uncertain = 0usize;
        for diagnostic in &sample {
            if let Some(label) = label_map.get(&(
                diagnostic.repository.as_str(),
                diagnostic.package_root.as_str(),
                rule.as_str(),
                diagnostic.site_id.as_str(),
                diagnostic.fingerprint.as_str(),
            )) {
                labeled += 1;
                match label {
                    ReviewLabel::TruePositive => confirmed += 1,
                    ReviewLabel::FalsePositive => {
                        confirmed += 1;
                        false_positives += 1;
                    }
                    ReviewLabel::Uncertain => uncertain += 1,
                }
            }
        }
        let false_positive_percent = percent(false_positives, confirmed.max(1));
        reviews.push(PromotionReview {
            rule: rule.clone(),
            sample_size: sample.len(),
            labeled,
            false_positive_percent,
            eligible_for_default: !sample.is_empty()
                && labeled == sample.len()
                && uncertain == 0
                && false_positive_percent <= 2.0,
        });
    }
    reviews
}

fn promotion_review_template(
    candidate_diagnostics: &[EvaluationDiagnostic],
    promoted_rules: &[String],
    candidate_sha256: &str,
) -> PromotionReviewTemplate {
    let samples = promoted_rules
        .iter()
        .map(|rule| {
            let labels = promotion_sample(candidate_diagnostics, rule)
                .into_iter()
                .map(|diagnostic| DiagnosticLabel {
                    repository: diagnostic.repository.clone(),
                    package_root: diagnostic.package_root.clone(),
                    rule: diagnostic.rule.clone(),
                    site_id: diagnostic.site_id.clone(),
                    evidence_fingerprint: diagnostic.fingerprint.clone(),
                    label: ReviewLabel::Uncertain,
                })
                .collect::<Vec<_>>();
            PromotionRuleSample {
                rule: rule.clone(),
                sample_size: labels.len(),
                labels,
            }
        })
        .collect();
    PromotionReviewTemplate {
        schema_version: DELTA_SCHEMA_VERSION.to_string(),
        candidate_sha256: candidate_sha256.to_string(),
        samples,
    }
}

fn promotion_sample<'a>(
    candidate_diagnostics: &'a [EvaluationDiagnostic],
    rule: &str,
) -> Vec<&'a EvaluationDiagnostic> {
    let mut sample: Vec<_> = candidate_diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.rule == rule)
        .collect();
    sample.sort_by_key(|diagnostic| {
        hex_digest(
            format!(
                "{}\0{}\0{}\0{}\0{}",
                diagnostic.repository,
                diagnostic.package_root,
                diagnostic.rule,
                diagnostic.site_id,
                diagnostic.fingerprint
            )
            .as_bytes(),
        )
    });
    sample.dedup_by(|left, right| {
        left.repository == right.repository
            && left.package_root == right.package_root
            && left.rule == right.rule
            && left.site_id == right.site_id
            && left.fingerprint == right.fingerprint
    });
    sample.truncate(100);
    sample
}

fn runtime_deltas(pairs: &[(&CorpusRecord, &CorpusRecord)]) -> Vec<RuntimeDelta> {
    pairs
        .iter()
        .map(|(baseline, candidate)| RuntimeDelta {
            repository: baseline.repository.clone(),
            baseline_ms: baseline.duration_ms,
            candidate_ms: candidate.duration_ms,
            delta_percent: runtime_percent_change(baseline.duration_ms, candidate.duration_ms),
        })
        .collect()
}

fn complete_root_count(records: &[CorpusRecord]) -> usize {
    records
        .iter()
        .flat_map(|record| record.root_states.values())
        .filter(|state| **state == RootState::Complete)
        .count()
}

fn total_root_count(records: &[CorpusRecord]) -> usize {
    records.iter().map(|record| record.root_states.len()).sum()
}

const fn root_state_rank(state: RootState) -> u8 {
    match state {
        RootState::NotAttempted => 0,
        RootState::Failed => 1,
        RootState::Incomplete => 2,
        RootState::Complete => 3,
    }
}

fn percentile(values: &mut [f64], percentile: usize) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(f64::total_cmp);
    let index = (values.len() - 1).saturating_mul(percentile).div_ceil(100);
    values[index.min(values.len() - 1)]
}

#[expect(
    clippy::cast_precision_loss,
    reason = "corpus millisecond durations are bounded far below f64 integer precision"
)]
fn runtime_percent_change(baseline_ms: u64, candidate_ms: u64) -> f64 {
    if baseline_ms == 0 {
        0.0
    } else {
        ((candidate_ms as f64 / baseline_ms as f64) - 1.0) * 100.0
    }
}

#[expect(
    clippy::cast_precision_loss,
    reason = "corpus root and label samples are explicitly bounded to small counts"
)]
fn percent(numerator: usize, denominator: usize) -> f64 {
    numerator as f64 * 100.0 / denominator.max(1) as f64
}

#[cfg(test)]
mod tests {
    use super::super::model::{FailureEvent, RootState, SeverityCounts};
    use super::*;

    fn record(repository: &str, diagnostics: Vec<EvaluationDiagnostic>) -> CorpusRecord {
        let catalog = BTreeMap::from([(
            "rule".to_string(),
            EvaluationRule {
                category: "correctness".to_string(),
                default_severity: "warning".to_string(),
                default_enabled: true,
            },
        )]);
        let catalog_sha256 = hex_digest(&serde_json::to_vec(&catalog).unwrap());
        CorpusRecord {
            schema_version: CORPUS_SCHEMA_VERSION.to_string(),
            repository: repository.to_string(),
            commit: "a".repeat(40),
            package_roots: vec![".".to_string()],
            expected_roots: vec![".".to_string()],
            attempted_roots: vec![".".to_string()],
            reported_roots: vec![".".to_string()],
            root_states: BTreeMap::from([(".".to_string(), RootState::Complete)]),
            tool_revision: "test".to_string(),
            binary_sha256: String::new(),
            evaluation_profile_sha256: "b".repeat(64),
            catalog_sha256,
            catalog,
            tree_digest: "tree".to_string(),
            complete: true,
            completeness: "complete".to_string(),
            diagnostic_counts: SeverityCounts::default(),
            per_rule_counts: BTreeMap::new(),
            duration_ms: 100,
            attempts: 1,
            diagnostics,
            failure_chain: Vec::<FailureEvent>::new(),
        }
    }

    fn diagnostic(rule: &str, key: &str, fingerprint: &str) -> EvaluationDiagnostic {
        EvaluationDiagnostic {
            repository: "repo".to_string(),
            package_root: ".".to_string(),
            rule: rule.to_string(),
            site_id: key.to_string(),
            baseline_key: key.to_string(),
            fingerprint: fingerprint.to_string(),
        }
    }

    #[test]
    fn diagnostic_multisets_distinguish_introduced_removed_and_changed() {
        let baseline = record(
            "repo",
            vec![
                diagnostic("rule", "same", "v1"),
                diagnostic("rule", "gone", "v1"),
            ],
        );
        let candidate = record(
            "repo",
            vec![
                diagnostic("rule", "same", "v2"),
                diagnostic("rule", "new", "v1"),
            ],
        );
        let comparison = compare_diagnostics(&[(&baseline, &candidate)]);
        assert_eq!(comparison.introduced_count, 1);
        assert_eq!(comparison.removed_count, 1);
        assert_eq!(comparison.changed_count, 1);
    }

    #[test]
    fn promotion_requires_complete_labels_and_at_most_two_percent_false_positives() {
        let introduced: Vec<_> = (0..100)
            .map(|index| diagnostic("rule", &format!("key-{index}"), "v1"))
            .collect();
        let labels: Vec<_> = introduced
            .iter()
            .map(|diagnostic| DiagnosticLabel {
                repository: diagnostic.repository.clone(),
                package_root: diagnostic.package_root.clone(),
                rule: "rule".to_string(),
                site_id: diagnostic.site_id.clone(),
                evidence_fingerprint: diagnostic.fingerprint.clone(),
                label: if diagnostic.baseline_key == "key-0" {
                    ReviewLabel::FalsePositive
                } else {
                    ReviewLabel::TruePositive
                },
            })
            .collect();
        let reviews = promotion_reviews(&introduced, &["rule".to_string()], &labels);
        assert!(reviews[0].eligible_for_default);
        assert!((reviews[0].false_positive_percent - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn review_template_uses_the_exact_bounded_promotion_sample() {
        let diagnostics: Vec<_> = (0..120)
            .rev()
            .map(|index| diagnostic("rule", &format!("key-{index}"), &format!("{index:064x}")))
            .chain([diagnostic("other-rule", "only", &"f".repeat(64))])
            .collect();
        let promoted_rules = vec!["other-rule".to_string(), "rule".to_string()];
        let template = promotion_review_template(&diagnostics, &promoted_rules, &"c".repeat(64));
        assert_eq!(
            template
                .samples
                .iter()
                .map(|sample| (sample.rule.as_str(), sample.sample_size))
                .collect::<Vec<_>>(),
            [("other-rule", 1), ("rule", 100)]
        );

        let labels: Vec<_> = template
            .samples
            .iter()
            .flat_map(|sample| sample.labels.iter().cloned())
            .map(|mut label| {
                label.label = ReviewLabel::TruePositive;
                label
            })
            .collect();
        let reviews = promotion_reviews(&diagnostics, &promoted_rules, &labels);
        assert!(reviews.iter().all(|review| review.eligible_for_default));
    }

    #[test]
    fn affected_repositories_are_ranked_by_changed_diagnostic_count() {
        let counts = BTreeMap::from([
            ("alphabetical-first".to_string(), 1),
            ("most-affected".to_string(), 4),
        ]);
        assert_eq!(
            top_affected_repositories(counts),
            ["most-affected", "alphabetical-first"]
        );
    }
}
