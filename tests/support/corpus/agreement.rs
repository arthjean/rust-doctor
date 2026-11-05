use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::coefficients::Coefficient;
use super::{Adjudication, Population, SiteContext, Verdict};

pub(crate) const SCHEMA_VERSION: u64 = 8;

pub(crate) const UNDATED_PAIRS: usize = 223;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Independence {
    SeparateContext,
    SeparateModel,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Pass {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) evidence: Vec<Citation>,
    pub(crate) judge: String,
    pub(crate) justification: String,
    pub(crate) verdict: Verdict,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Citation {
    pub(crate) end_line: u64,
    pub(crate) path: String,
    pub(crate) quote: String,
    pub(crate) start_line: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AdjudicatedPair {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) adjudicated_on: Option<String>,
    pub(crate) context: SiteContext,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) family: Option<String>,
    pub(crate) independence: Independence,
    pub(crate) line: u64,
    pub(crate) passes: [Pass; 2],
    pub(crate) path: String,
    pub(crate) population: Population,
    pub(crate) repository: String,
    pub(crate) rule: String,
}

pub(crate) type SiteIdentity<'a> = (&'a str, Population, &'a str, &'a str, u64);

impl AdjudicatedPair {
    pub(crate) fn agrees(&self) -> bool {
        self.passes[0].verdict == self.passes[1].verdict
    }

    pub(crate) fn verdict(&self) -> Option<Verdict> {
        self.agrees().then_some(self.passes[0].verdict)
    }

    pub(crate) fn identity(&self) -> SiteIdentity<'_> {
        (
            self.rule.as_str(),
            self.population,
            self.repository.as_str(),
            self.path.as_str(),
            self.line,
        )
    }

    pub(crate) fn label(&self) -> String {
        format!(
            "{} on {:?} at {}/{}:{}",
            self.rule, self.population, self.repository, self.path, self.line
        )
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Agreement {
    pub(crate) coefficients: Vec<Coefficient>,
    pub(crate) escalations_open: u64,
    pub(crate) pairs: Vec<AdjudicatedPair>,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProtocolScope {
    pub(crate) population: Population,
    pub(crate) rule: String,
}

pub(crate) const ENROLLED_SCOPES: [(Population, &str); 7] = [
    (Population::Agent, "rust_doctor::structure::duplicate_function_body"),
    (Population::Agent, "rust_doctor::structure::near_duplicate_function_body"),
    (Population::Agent, "rust_doctor::structure::orphan_module_file"),
    (Population::Agent, "rust_doctor::structure::oversized_unit"),
    (Population::Healthy, "clippy::undocumented_unsafe_blocks"),
    (Population::Healthy, "rust_doctor::structure::near_duplicate_function_body"),
    (Population::Healthy, "rust_doctor::structure::oversized_unit"),
];

pub(crate) const PROTOCOL_JUDGE: &str = "claude-opus-5";

pub(crate) const PROTOCOL_JUDGES: [&str; 2] = [PROTOCOL_JUDGE, "claude-opus-5-5"];

pub(crate) fn escalations_open(agreement: &Agreement) -> u64 {
    agreement.pairs.iter().filter(|pair| !pair.agrees()).count() as u64
}

pub(crate) fn backing(agreement: &Agreement) -> BTreeMap<SiteIdentity<'_>, &AdjudicatedPair> {
    agreement
        .pairs
        .iter()
        .filter(|pair| pair.agrees())
        .map(|pair| (pair.identity(), pair))
        .collect()
}

pub(crate) fn agreement_defects(adjudication: &Adjudication) -> Vec<String> {
    agreement_defects_under(adjudication, &BTreeSet::new())
}

pub(crate) fn agreement_defects_under(
    adjudication: &Adjudication,
    demoted: &BTreeSet<SiteIdentity<'_>>,
) -> Vec<String> {
    let agreement = &adjudication.agreement;
    let verdict = |pair: &AdjudicatedPair| pair.verdict().filter(|_| !demoted.contains(&pair.identity()));
    let mut defects = Vec::new();

    let mut seen: BTreeSet<SiteIdentity<'_>> = BTreeSet::new();
    for pair in &agreement.pairs {
        if !seen.insert(pair.identity()) {
            defects.push(format!("two pairs share one identity: {}", pair.label()));
        }
        for pass in &pair.passes {
            if pass.judge.trim().is_empty() {
                defects.push(format!("pair with an empty judge: {}", pair.label()));
            }
            if pass.justification.trim().is_empty() {
                defects.push(format!("pass with an empty justification: {}", pair.label()));
            }
        }
        if pair.independence == Independence::SeparateModel
            && pair.passes[0].judge == pair.passes[1].judge
        {
            defects.push(format!(
                "pair declared separate_model with one judge, {}: {}",
                pair.passes[0].judge,
                pair.label()
            ));
        }
    }

    let published: BTreeMap<SiteIdentity<'_>, Verdict> = adjudication
        .reviewed
        .iter()
        .map(|site| {
            (
                (
                    site.rule.as_str(),
                    site.population,
                    site.repository.as_str(),
                    site.path.as_str(),
                    site.line,
                ),
                site.verdict,
            )
        })
        .collect();

    for pair in &agreement.pairs {
        match (verdict(pair), published.get(&pair.identity())) {
            (Some(verdict), Some(site)) if verdict != *site => defects.push(format!(
                "agreeing pair judged {verdict:?} while the reviewed site publishes {site:?}: {}",
                pair.label()
            )),
            (Some(_), Some(_)) => {}
            (Some(_), None) => defects.push(format!(
                "agreeing pair with no reviewed site of its identity: {}",
                pair.label()
            )),
            (None, Some(_)) => defects.push(format!(
                "escalated site carries a published verdict: {}",
                pair.label()
            )),
            (None, None) => {}
        }
    }

    let open = agreement.pairs.iter().filter(|pair| verdict(pair).is_none()).count() as u64;
    if open != agreement.escalations_open {
        defects.push(format!(
            "escalations_open published {} against {open} disagreeing pairs",
            agreement.escalations_open
        ));
    }

    defects
}

const STRUCTURAL_PREFIX: &str = "rust_doctor::structure::";

pub(crate) fn family_defects(adjudication: &Adjudication) -> Vec<String> {
    let mut defects = Vec::new();

    let mut check = |rule: &str, family: Option<&String>, label: String| {
        match (rule.starts_with(STRUCTURAL_PREFIX), family) {
            (true, None) => defects.push(format!("structural site with no family: {label}")),
            (false, Some(_)) => defects.push(format!(
                "family published on a rule whose unit is a position: {label}"
            )),
            (true, Some(digest)) if digest.trim().is_empty() => {
                defects.push(format!("site with an empty family: {label}"));
            }
            _ => {}
        }
    };
    for site in &adjudication.reviewed {
        let label = format!(
            "{} on {:?} at {}/{}:{}",
            site.rule, site.population, site.repository, site.path, site.line
        );
        check(&site.rule, site.family.as_ref(), label);
    }
    for pair in &adjudication.agreement.pairs {
        check(&pair.rule, pair.family.as_ref(), pair.label());
    }

    let published: BTreeMap<SiteIdentity<'_>, Option<&String>> = adjudication
        .reviewed
        .iter()
        .map(|site| {
            (
                (
                    site.rule.as_str(),
                    site.population,
                    site.repository.as_str(),
                    site.path.as_str(),
                    site.line,
                ),
                site.family.as_ref(),
            )
        })
        .collect();
    for pair in &adjudication.agreement.pairs {
        if let Some(site) = published.get(&pair.identity())
            && *site != pair.family.as_ref()
        {
            defects.push(format!(
                "pair and reviewed site publish different families: {}",
                pair.label()
            ));
        }
    }

    defects
}

pub(crate) fn protocol_defects(adjudication: &Adjudication) -> Vec<String> {
    let cutoff = adjudication.protocol_cutoff.as_str();
    let mut defects = Vec::new();
    if !is_iso_date(cutoff) {
        defects.push(format!("protocol_cutoff is not a date: {cutoff}"));
    }

    let mut previous: Option<&ProtocolScope> = None;
    for scope in &adjudication.adjudicated_after_cutoff {
        if previous.is_some_and(|earlier| earlier >= scope) {
            defects.push(format!(
                "adjudicated_after_cutoff is not sorted and unique at {} on {:?}",
                scope.rule, scope.population
            ));
        }
        previous = Some(scope);
    }

    let enrolled: BTreeSet<(&str, Population)> = adjudication
        .adjudicated_after_cutoff
        .iter()
        .map(|scope| (scope.rule.as_str(), scope.population))
        .collect();
    let pairs: BTreeMap<SiteIdentity<'_>, &AdjudicatedPair> = adjudication
        .agreement
        .pairs
        .iter()
        .map(|pair| (pair.identity(), pair))
        .collect();

    for site in &adjudication.reviewed {
        if !enrolled.contains(&(site.rule.as_str(), site.population)) {
            continue;
        }
        let identity = (
            site.rule.as_str(),
            site.population,
            site.repository.as_str(),
            site.path.as_str(),
            site.line,
        );
        let label = format!(
            "{} on {:?} at {}/{}:{}",
            site.rule, site.population, site.repository, site.path, site.line
        );
        match pairs.get(&identity) {
            None => defects.push(format!(
                "adjudicated after the cutoff {cutoff} with no pair behind it: {label}"
            )),
            Some(pair) if !pair.agrees() => defects.push(format!(
                "adjudicated after the cutoff {cutoff} on a pair whose passes disagree: {label}"
            )),
            Some(_) => {}
        }
    }

    defects
}

pub(crate) fn citation_defects(adjudication: &Adjudication) -> Vec<String> {
    let cutoff = adjudication.evidence_cutoff.as_str();
    let mut defects = Vec::new();
    if !is_iso_date(cutoff) {
        defects.push(format!("evidence_cutoff is not a date: {cutoff}"));
    }
    let pairs = &adjudication.agreement.pairs;
    let undated = pairs.iter().filter(|pair| pair.adjudicated_on.is_none()).count();
    if undated > UNDATED_PAIRS {
        defects.push(format!(
            "{undated} pairs carry no adjudicated_on, over the {UNDATED_PAIRS} that predate {cutoff}"
        ));
    }
    for pair in pairs {
        let label = pair.label();
        if let Some(date) = &pair.adjudicated_on
            && (!is_iso_date(date) || date.as_str() < cutoff)
        {
            defects.push(format!("adjudicated_on {date} is not a date on or after {cutoff}: {label}"));
        }
        for pass in &pair.passes {
            if pair.adjudicated_on.is_some() && pass.evidence.is_empty() {
                defects.push(format!("pass of {} cites no evidence: {label}", pass.judge));
                continue;
            }
            for citation in &pass.evidence {
                if citation.start_line == 0
                    || citation.end_line < citation.start_line
                    || citation.quote.is_empty()
                {
                    defects.push(format!(
                        "citation {}:{}-{} is empty or reversed: {label}",
                        citation.path, citation.start_line, citation.end_line
                    ));
                }
            }
            let covers = pass.evidence.iter().any(|citation| {
                citation.path == pair.path && (citation.start_line..=citation.end_line).contains(&pair.line)
            });
            if !pass.evidence.is_empty() && !covers {
                defects.push(format!(
                    "no citation of the pass of {} covers the site line: {label}",
                    pass.judge
                ));
            }
        }
    }
    defects
}

pub(crate) fn quote_failures<'a>(
    pairs: impl IntoIterator<Item = &'a AdjudicatedPair>,
    mut lines: impl FnMut(&str, &str) -> Option<String>,
) -> BTreeMap<SiteIdentity<'a>, String> {
    let mut failures = BTreeMap::new();
    for pair in pairs {
        for pass in &pair.passes {
            for citation in &pass.evidence {
                let held = lines(&pair.repository, &citation.path).and_then(|text| {
                    let start = usize::try_from(citation.start_line).ok()?.checked_sub(1)?;
                    let count = usize::try_from(citation.end_line.checked_sub(citation.start_line)? + 1).ok()?;
                    let cited: Vec<&str> = text.lines().skip(start).take(count).collect();
                    (cited.len() == count).then(|| cited.join("\n"))
                });
                if held.as_deref() != Some(citation.quote.as_str()) {
                    failures.entry(pair.identity()).or_insert_with(|| {
                        format!(
                            "the quote of {}:{}-{} by {} is not what those lines hold: {}",
                            citation.path,
                            citation.start_line,
                            citation.end_line,
                            pass.judge,
                            pair.label()
                        )
                    });
                }
            }
        }
    }
    failures
}

pub(crate) fn is_iso_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 10
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| match index {
                4 | 7 => *byte == b'-',
                _ => byte.is_ascii_digit(),
            })
}
