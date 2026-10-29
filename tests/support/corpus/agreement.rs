//! The record of a double pass.
//!
//! The protocol judges every sampled site twice, blind, and escalates a
//! disagreement to a human rather than letting an agent break its own tie.
//! Until this module existed the artifact could hold only one of the two
//! verdicts: `ReviewedSite` carries a single `verdict`, and its identity is
//! unique by construction, so writing the second pass beside the first marked
//! the rule `duplicated` and withheld its rate. The evidence the protocol
//! produced was destroyed at write time.
//!
//! A pair is that evidence, held beside `reviewed` rather than inside it, and
//! coupled to it by an invariant rather than by a copy. A pair whose passes
//! agree has exactly one `ReviewedSite` of the same identity carrying that
//! verdict. A pair whose passes disagree has none, and that absence is what
//! escalation means: no field says so, and none can be forged.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::coefficients::Coefficient;
use super::{Adjudication, Population, SiteContext, Verdict};

/// Schema version of the corpus artifact.
///
/// Asserted against the artifact rather than merely deserialized. Versions 1
/// through 3 were bumped by hand and checked by nothing, so the field cost
/// nothing to move and proved nothing once moved: a reader could not tell a
/// coordinated schema change from a typo.
///
/// 5 adds `adjudication.position_proof`, and it is a required member of a shape
/// that denies unknown fields: a reader of 4 refuses this file and this harness
/// refuses a 4. The two are different shapes, so they are different numbers.
///
/// 6 adds the difference interval, its method, its coverage flag and its
/// verdict to every `rate_comparison` row, all five required for the same
/// reason: a row carrying a gap and no interval is a row that states a distance
/// without saying whether the samples can tell the two populations apart. It
/// also adds `family` to a reviewed site and to a pair, present exactly on the
/// structural rules, whose unit is a family and not a position.
///
/// 7 adds `smoothing`, the pseudo-counts the ranking reads every published rate
/// through. Required for the same reason `lambdas` is: `precision` publishes
/// the raw counts and the raw share, and the number the report actually ranks
/// by is neither of the two until the smoothing is named.
///
/// 8 adds `evidence_cutoff`, required, the date from which a pair is dated by
/// `adjudicated_on` and every one of its passes cites the lines that decided
/// it in `evidence`. The website reads this file's snapshot, so the bump is a
/// step of the release checklist.
pub(crate) const SCHEMA_VERSION: u64 = 8;

/// The pairs adjudicated before `evidence_cutoff`, which carry neither a date
/// nor a citation. They are grandfathered by count rather than re-judged:
/// inventing the citation a pass never made is worse evidence than none, and a
/// count that can only fall is what keeps a new undated pair out. 239 at the
/// cutoff, less the orphan pair whose site US-020 fixed out of the population,
/// less the fifteen the core-v6 replay of 2026-10-29 dropped or re-judged
/// after the detector changes of the self-scan remediation.
pub(crate) const UNDATED_PAIRS: usize = 223;

/// How the two passes of a pair were kept apart.
///
/// Recorded as a fact per pair rather than assumed, because the two answer
/// different questions about the same number. Two passes of one model in two
/// contexts reduce variance; they cannot reduce self-preference bias, since
/// both passes share the generation distribution of the tool they are judging.
/// Two passes from different families reduce both. Mandating the second before
/// there are enough pairs of each kind to compare them would be a rule with no
/// measurement behind it, which is the failure this record exists to end.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Independence {
    SeparateContext,
    SeparateModel,
}

/// One judgment of one site by one judge.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Pass {
    /// The lines this verdict was decided on, each quoted as the file holds
    /// them at the pinned revision. Required on every pass of a dated pair.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) evidence: Vec<Citation>,
    /// Which judge produced this verdict.
    ///
    /// `unrecorded` is the truthful value for the passes of the 2026-08-11 run,
    /// whose model identity was never captured and whose verdicts survive only
    /// through `docs/top-rules-precision-2026-08.md`. It is what `Provenance`
    /// already does for the same reason, not a default to reuse: a pass
    /// produced under this record names its model.
    pub(crate) judge: String,
    pub(crate) justification: String,
    pub(crate) verdict: Verdict,
}

/// Lines of a file of the judged repository, workspace-relative, and the text
/// they hold, byte for byte, without the final line break.
///
/// The quote is what makes a citation checkable at all: a line range alone
/// cannot tell a pass that read the code from one that named a plausible span.
/// Offline the suite checks that the citations cover the site; the cache-gated
/// replay checks each quote against the pinned revision.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Citation {
    pub(crate) end_line: u64,
    pub(crate) path: String,
    pub(crate) quote: String,
    pub(crate) start_line: u64,
}

/// A site judged twice, with both verdicts and both justifications.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AdjudicatedPair {
    /// The day the second pass was recorded. Absent only on the pairs that
    /// predate `evidence_cutoff`, which `UNDATED_PAIRS` counts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) adjudicated_on: Option<String>,
    pub(crate) context: SiteContext,
    /// The structural family this pair judged, spelled as `ReviewedSite` spells
    /// it and asserted equal to it wherever both exist.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) family: Option<String>,
    pub(crate) independence: Independence,
    pub(crate) line: u64,
    /// Exactly two passes, enforced by the shape rather than by a check: a
    /// fixed-size array refuses one pass and refuses three at deserialization,
    /// where a `Vec` would accept both and leave a length assertion to be
    /// written, forgotten, or written twice.
    pub(crate) passes: [Pass; 2],
    pub(crate) path: String,
    pub(crate) population: Population,
    pub(crate) repository: String,
    pub(crate) rule: String,
}

/// Identity of a judged site: the identity `reviewed` is unique on, plus the
/// population, since the two populations observe the same rules on different
/// code and a site of one is not a site of the other.
pub(crate) type SiteIdentity<'a> = (&'a str, Population, &'a str, &'a str, u64);

impl AdjudicatedPair {
    pub(crate) fn agrees(&self) -> bool {
        self.passes[0].verdict == self.passes[1].verdict
    }

    /// The verdict of an agreeing pair. `None` when the passes disagree, which
    /// is the whole point: an escalated site has no verdict to publish.
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

    /// How a defect names this site. Carries the declared repository, the
    /// workspace-relative path and the line, and nothing else: no absolute
    /// path, no environment variable, no user data.
    pub(crate) fn label(&self) -> String {
        format!(
            "{} on {:?} at {}/{}:{}",
            self.rule, self.population, self.repository, self.path, self.line
        )
    }
}

/// The double-pass record.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Agreement {
    /// Agreement of the two passes over each `(rule, population)` carrying at
    /// least one pair, recomputed from those pairs by the suite rather than
    /// written by whoever ran the adjudication.
    pub(crate) coefficients: Vec<Coefficient>,
    /// Sites the two passes disagreed on and a human has not settled.
    ///
    /// Published because a queue nobody can count is a queue nobody works, and
    /// derived rather than stored per site: no pair carries an `escalated`
    /// flag, so this number cannot disagree with the pairs unless it is wrong,
    /// which is what `escalations_open` recomputes.
    pub(crate) escalations_open: u64,
    pub(crate) pairs: Vec<AdjudicatedPair>,
}

/// A `(rule, population)` sample adjudicated after the protocol cutoff.
///
/// The cutoff is a date, and a reviewed site carries none: what says whether a
/// verdict falls under the protocol is the sample it belongs to. Naming those
/// samples is what makes the rule enforceable without stamping a date on the
/// sites that predate the question.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProtocolScope {
    pub(crate) population: Population,
    pub(crate) rule: String,
}

/// The scopes adjudicated under the protocol, in the order the record publishes
/// them, which is `ProtocolScope`'s own: population first, rule second.
///
/// Named once here rather than restated by each test that reads the enrolment,
/// because two lists of the same scopes is how one drops out of one of them and
/// keeps passing the other. A scope and not a rule, since the two healthy
/// scopes deepened to their whole production subpopulation carry the same two
/// rule ids the agent population already had: a list of rules could no longer
/// say which side of the record a double pass was run on.
pub(crate) const ENROLLED_SCOPES: [(Population, &str); 7] = [
    (Population::Agent, "rust_doctor::structure::duplicate_function_body"),
    (Population::Agent, "rust_doctor::structure::near_duplicate_function_body"),
    (Population::Agent, "rust_doctor::structure::orphan_module_file"),
    (Population::Agent, "rust_doctor::structure::oversized_unit"),
    (Population::Healthy, "clippy::undocumented_unsafe_blocks"),
    (Population::Healthy, "rust_doctor::structure::near_duplicate_function_body"),
    (Population::Healthy, "rust_doctor::structure::oversized_unit"),
];

/// The judge behind every pass produced under the protocol before 2026-09-26.
///
/// One name per scope, because the two passes of a pair are separated by their
/// context and not by their model: `Independence::SeparateContext` is what each
/// pair declares, and a second model would be a different claim recorded under
/// the same field.
pub(crate) const PROTOCOL_JUDGE: &str = "claude-opus-5";

/// Every model that has judged under the protocol, oldest first. A pass names
/// the model that produced it, so a scope judged after a model change names the
/// newer one rather than borrowing the older name; what stays fixed is that
/// both passes of a pair, and every pair of one scope, name the same judge.
pub(crate) const PROTOCOL_JUDGES: [&str; 2] = [PROTOCOL_JUDGE, "claude-opus-5-5"];

/// Number of pairs whose two passes disagreed.
pub(crate) fn escalations_open(agreement: &Agreement) -> u64 {
    agreement.pairs.iter().filter(|pair| !pair.agrees()).count() as u64
}

/// Agreeing pairs by identity: the sites a published verdict is allowed to
/// rest on under the protocol.
pub(crate) fn backing(agreement: &Agreement) -> BTreeMap<SiteIdentity<'_>, &AdjudicatedPair> {
    agreement
        .pairs
        .iter()
        .filter(|pair| pair.agrees())
        .map(|pair| (pair.identity(), pair))
        .collect()
}

/// Closed defects of the double-pass record, each naming the site concerned.
///
/// Both directions of the coupling are checked, because only one of the two is
/// intuitive: an agreeing pair with nothing in `reviewed` is a verdict that was
/// produced and dropped, and a disagreeing pair with something in `reviewed` is
/// an escalation an agent settled.
pub(crate) fn agreement_defects(adjudication: &Adjudication) -> Vec<String> {
    agreement_defects_under(adjudication, &BTreeSet::new())
}

/// `agreement_defects`, with the pairs a failed citation demoted counted as
/// disagreeing: a verdict whose quoted evidence is not in the code it judged
/// is no verdict, and the site it backs is held as an escalation.
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

/// Prefix of every rule whose unit is a family rather than a position.
///
/// One spelling, read by the two directions of `family_defects`: a predicate
/// written twice is how the presence check and the absence check end up
/// disagreeing about which rules the record is talking about.
const STRUCTURAL_PREFIX: &str = "rust_doctor::structure::";

/// Closed defects of the family anchoring, each naming the site.
///
/// Both directions, for the reason the pair coupling is checked both ways: a
/// structural site with no family is a verdict about a family nobody can name,
/// and a family on a rule whose unit is a position is a second identity beside
/// one that already settles the question. The third check is the one the two
/// shapes make possible at all: a pair and the reviewed site it backs are one
/// judgment of one family, so a disagreement between them is a verdict
/// republished under a family it was never produced against.
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

/// Closed defects of the protocol cutoff, each naming the site and the cutoff.
///
/// The two conditions are reported apart, because a contributor who reads
/// "no pair" looks for the second pass they never ran, and one who reads
/// "the pair disagrees" looks for the escalation they published anyway.
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

/// Closed defects of the cited evidence, offline: what the record states
/// about its citations without the code they cite.
///
/// A dated pair cites on every pass, and some citation of each pass sits on the
/// site's path and covers its line, since a verdict that never quotes the
/// flagged line judged something else. An undated pair predates the cutoff,
/// and there can be no more of them than `UNDATED_PAIRS`.
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

/// The citations whose quote is not what the cited lines hold, each naming
/// its pair, read through `lines`, which answers a repository and a path with
/// the file at the pinned revision. A file `lines` cannot read fails the
/// citation as surely as a wrong quote.
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

/// Read by the position proof as well, which dates the run that confirmed the
/// sites rather than the record. One spelling of a date, checked in one place.
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
