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
pub(crate) const SCHEMA_VERSION: u64 = 4;

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

/// A site judged twice, with both verdicts and both justifications.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AdjudicatedPair {
    pub(crate) context: SiteContext,
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

/// The rules adjudicated under the protocol, in the order the record publishes
/// them, all on the agent population.
///
/// Named once here rather than restated by each test that reads the enrolment,
/// because two lists of the same three rules is how a scope drops out of one
/// of them and keeps passing the other.
pub(crate) const ENROLLED_RULES: [&str; 3] = [
    "rust_doctor::structure::duplicate_function_body",
    "rust_doctor::structure::near_duplicate_function_body",
    "rust_doctor::structure::oversized_unit",
];

/// The judge behind every pass produced under the protocol.
///
/// One name, because the two passes of a pair are separated by their context
/// and not by their model: `Independence::SeparateContext` is what each pair
/// declares, and a second model would be a different claim recorded under the
/// same field.
pub(crate) const PROTOCOL_JUDGE: &str = "claude-opus-5";

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
    let agreement = &adjudication.agreement;
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
        match (pair.verdict(), published.get(&pair.identity())) {
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

    let open = escalations_open(agreement);
    if open != agreement.escalations_open {
        defects.push(format!(
            "escalations_open published {} against {open} disagreeing pairs",
            agreement.escalations_open
        ));
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

fn is_iso_date(value: &str) -> bool {
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
