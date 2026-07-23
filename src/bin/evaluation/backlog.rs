use crate::evaluation::{EvalError, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;

const FIELD_COUNT: usize = 20;

#[derive(Debug, Deserialize)]
struct Backlog {
    schema_version: String,
    rubric: Rubric,
    sources: Vec<Source>,
    fields: Vec<String>,
    selected_top_20: Vec<String>,
    candidates: Vec<Vec<Value>>,
}

#[derive(Debug, Deserialize)]
struct Rubric {
    formula: String,
    range: [i64; 2],
    tie_breaker: String,
}

#[derive(Debug, Deserialize)]
struct Source {
    id: String,
    kind: String,
    url: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct RankedCandidate {
    id: String,
    title: String,
    disposition: String,
    score: i64,
}

pub(crate) fn run(manifest: &Path, json: bool) -> Result<()> {
    let ranked = load_and_validate(manifest)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&ranked).map_err(|source| EvalError::Json {
                path: manifest.to_path_buf(),
                source,
            })?
        );
    } else {
        for (position, candidate) in ranked.iter().enumerate() {
            println!(
                "{:>2}. {} ({}, score {})",
                position + 1,
                candidate.id,
                candidate.disposition,
                candidate.score
            );
        }
    }
    Ok(())
}

fn load_and_validate(manifest: &Path) -> Result<Vec<RankedCandidate>> {
    let content = std::fs::read_to_string(manifest)
        .map_err(|source| EvalError::io("read rule backlog", manifest, source))?;
    let backlog: Backlog = serde_json::from_str(&content).map_err(|source| EvalError::Json {
        path: manifest.to_path_buf(),
        source,
    })?;

    if backlog.schema_version != "1.0" {
        return invalid("rule backlog schema_version must be 1.0");
    }
    if backlog.candidates.len() < 100 {
        return invalid("rule backlog must contain at least 100 candidates");
    }
    if backlog.fields.len() != FIELD_COUNT {
        return invalid("rule backlog fields do not match the V1 tuple contract");
    }
    if backlog.rubric.formula
        != "impact*4 + prevalence*3 + detectability*2 + expected_precision*4 - overlap_penalty*5"
        || backlog.rubric.range != [0, 65]
        || backlog.rubric.tie_breaker != "candidate ID ascending"
    {
        return invalid("rule backlog scoring rubric drifted from the V1 contract");
    }

    let source_ids: HashSet<&str> = backlog
        .sources
        .iter()
        .map(|source| source.id.as_str())
        .collect();
    if source_ids.len() != backlog.sources.len()
        || backlog
            .sources
            .iter()
            .any(|source| source.kind.is_empty() || source.url.is_empty())
    {
        return invalid("rule backlog sources must have unique IDs, kinds, and links");
    }

    let mut candidate_ids = HashSet::new();
    let mut ranked = Vec::with_capacity(backlog.candidates.len());
    for row in &backlog.candidates {
        ranked.push(validate_candidate(
            row,
            &mut candidate_ids,
            &source_ids,
            backlog.rubric.range,
        )?);
    }

    ranked.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.id.cmp(&right.id))
    });
    let computed_top: Vec<&str> = ranked
        .iter()
        .take(20)
        .map(|candidate| candidate.id.as_str())
        .collect();
    let declared_top: Vec<&str> = backlog.selected_top_20.iter().map(String::as_str).collect();
    if computed_top != declared_top {
        return invalid("selected_top_20 does not match the deterministic rubric");
    }
    Ok(ranked.into_iter().take(20).collect())
}

fn validate_candidate(
    row: &[Value],
    candidate_ids: &mut HashSet<String>,
    source_ids: &HashSet<&str>,
    score_range: [i64; 2],
) -> Result<RankedCandidate> {
    if row.len() != FIELD_COUNT {
        return invalid("every rule candidate must populate all V1 fields");
    }
    let id = string_field(row, 0, "id")?;
    if !candidate_ids.insert(id.to_string()) {
        return invalid(&format!("duplicate rule candidate ID: {id}"));
    }
    let source_id = string_field(row, 3, "source_id")?;
    if !source_ids.contains(source_id) {
        return invalid(&format!(
            "candidate {id} has an unknown source: {source_id}"
        ));
    }
    for (index, name) in [
        (1, "title"),
        (2, "domain"),
        (4, "user_impact"),
        (5, "positive_example"),
        (6, "negative_example"),
        (7, "existing_tool_overlap"),
        (8, "required_analyzer"),
        (9, "confidence_ceiling"),
        (11, "version_gate"),
        (12, "false_positive_risk"),
        (13, "disposition"),
    ] {
        if string_field(row, index, name)?.is_empty() {
            return invalid(&format!("candidate {id} has an empty {name}"));
        }
    }
    let disposition = string_field(row, 13, "disposition")?;
    let rejection_reason = string_field(row, 14, "rejection_reason")?;
    if disposition == "rejected" && rejection_reason.is_empty() {
        return invalid(&format!("rejected candidate {id} needs a reason"));
    }
    if !matches!(disposition, "validate" | "experimental" | "rejected") {
        return invalid(&format!("candidate {id} has an invalid disposition"));
    }

    let impact = score_field(row, 15, "impact", id)?;
    let prevalence = score_field(row, 16, "prevalence", id)?;
    let detectability = score_field(row, 17, "detectability", id)?;
    let precision = score_field(row, 18, "expected_precision", id)?;
    let overlap = score_field(row, 19, "overlap_penalty", id)?;
    let score = impact * 4 + prevalence * 3 + detectability * 2 + precision * 4 - overlap * 5;
    if !(score_range[0]..=score_range[1]).contains(&score) {
        return invalid(&format!("candidate {id} score is outside the rubric range"));
    }
    Ok(RankedCandidate {
        id: id.to_string(),
        title: string_field(row, 1, "title")?.to_string(),
        disposition: disposition.to_string(),
        score,
    })
}

fn string_field<'a>(row: &'a [Value], index: usize, name: &str) -> Result<&'a str> {
    row.get(index)
        .and_then(Value::as_str)
        .ok_or_else(|| EvalError::InvalidManifest(format!("candidate field {name} must be text")))
}

fn score_field(row: &[Value], index: usize, name: &str, id: &str) -> Result<i64> {
    let value = row.get(index).and_then(Value::as_i64).ok_or_else(|| {
        EvalError::InvalidManifest(format!("candidate {id} {name} must be an integer"))
    })?;
    if !(0..=5).contains(&value) {
        return invalid(&format!("candidate {id} {name} must be between 0 and 5"));
    }
    Ok(value)
}

fn invalid<T>(message: &str) -> Result<T> {
    Err(EvalError::InvalidManifest(message.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_backlog_is_complete_and_deterministic() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("evaluation/rule-backlog-v1.json");
        let ranked = load_and_validate(&path).expect("checked backlog should be valid");
        assert_eq!(ranked.len(), 20);
        assert_eq!(ranked[0].id, "actix-web-data-lock");
        assert_eq!(ranked[19].id, "weak-crypto-hash");
    }
}
