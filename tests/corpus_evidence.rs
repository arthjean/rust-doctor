#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! US-021: every adjudication pass after `evidence_cutoff` cites the lines
//! that decided it.
//!
//! Offline the suite holds what the record says about its citations: a dated
//! pair cites on both passes, each pass covers the site's line on the site's
//! path, and no new pair slips in undated. The quotes themselves are checked
//! against the pinned revision by the cache-gated replays of
//! `corpus_precision.rs`, through `quote_failures`, which these tests drive
//! with an in-memory checkout.

mod support;

use std::collections::BTreeSet;

use support::corpus::agreement::{
    AdjudicatedPair, Citation, UNDATED_PAIRS, agreement_defects, agreement_defects_under,
    citation_defects, quote_failures,
};
use support::corpus::{Adjudication, artifact};

const DATE: &str = "2026-10-14";

/// The shipped record, with its first agreeing pair dated and cited as the
/// protocol now requires.
fn cited() -> (Adjudication, usize) {
    let mut adjudication = artifact().adjudication;
    let index = adjudication
        .agreement
        .pairs
        .iter()
        .position(AdjudicatedPair::agrees)
        .unwrap();
    let pair = &mut adjudication.agreement.pairs[index];
    pair.adjudicated_on = Some(DATE.to_owned());
    let citation = Citation {
        end_line: pair.line + 1,
        path: pair.path.clone(),
        quote: "    flagged();\n}".to_owned(),
        start_line: pair.line,
    };
    for pass in &mut pair.passes {
        pass.evidence = vec![citation.clone()];
    }
    (adjudication, index)
}

/// The cited file as the in-memory checkout holds it: the flagged call on the
/// site's line, closed on the next one.
fn checkout(line: u64) -> String {
    let before = "// filler\n".repeat(usize::try_from(line - 1).unwrap());
    format!("{before}    flagged();\n}}\n")
}

fn naming<'a>(defects: &'a [String], fragment: &str) -> Vec<&'a String> {
    defects.iter().filter(|defect| defect.contains(fragment)).collect()
}

#[test]
fn the_shipped_record_cites_every_dated_pass_and_grandfathers_the_rest() {
    let adjudication = artifact().adjudication;
    assert_eq!(citation_defects(&adjudication), Vec::<String>::new());
    assert_eq!(adjudication.evidence_cutoff, DATE);
    let undated = adjudication
        .agreement
        .pairs
        .iter()
        .filter(|pair| pair.adjudicated_on.is_none())
        .count();
    assert!(undated <= UNDATED_PAIRS, "{undated} undated pairs");
}

#[test]
fn a_dated_pair_whose_passes_cite_the_site_line_is_accepted() {
    let (adjudication, _) = cited();
    assert_eq!(citation_defects(&adjudication), Vec::<String>::new());
}

#[test]
fn a_dated_pass_without_evidence_is_refused_naming_its_site() {
    let (mut adjudication, index) = cited();
    adjudication.agreement.pairs[index].passes[1].evidence.clear();
    let label = adjudication.agreement.pairs[index].label();
    let defects = citation_defects(&adjudication);
    let named = naming(&defects, "cites no evidence");
    assert_eq!(named.len(), 1, "{defects:?}");
    assert!(named[0].ends_with(&label), "{}", named[0]);
}

#[test]
fn a_citation_off_the_site_line_or_path_is_refused() {
    let (mut adjudication, index) = cited();
    let pair = &mut adjudication.agreement.pairs[index];
    pair.passes[0].evidence[0].start_line = pair.line + 1;
    pair.passes[1].evidence[0].path = "src/elsewhere.rs".to_owned();
    let defects = citation_defects(&adjudication);
    assert_eq!(naming(&defects, "covers the site line").len(), 2, "{defects:?}");
}

#[test]
fn a_reversed_or_empty_citation_and_a_date_before_the_cutoff_are_refused() {
    let (mut adjudication, index) = cited();
    let pair = &mut adjudication.agreement.pairs[index];
    pair.adjudicated_on = Some("2026-10-13".to_owned());
    pair.passes[0].evidence[0].quote.clear();
    let defects = citation_defects(&adjudication);
    assert_eq!(naming(&defects, "is not a date on or after 2026-10-14").len(), 1, "{defects:?}");
    assert_eq!(naming(&defects, "is empty or reversed").len(), 1, "{defects:?}");
}

#[test]
fn a_new_undated_pair_is_refused() {
    let mut adjudication = artifact().adjudication;
    let mut extra = adjudication.agreement.pairs[0].clone();
    extra.line += 100_000;
    // Counted on the undated pairs, not on every pair: the dated ones are
    // the record's, and a pair past the grandfathered count is what is
    // refused.
    extra.adjudicated_on = None;
    let undated = |pairs: &[AdjudicatedPair]| {
        pairs.iter().filter(|pair| pair.adjudicated_on.is_none()).count()
    };
    while undated(&adjudication.agreement.pairs) <= UNDATED_PAIRS {
        adjudication.agreement.pairs.push(extra.clone());
        extra.line += 1;
    }
    let defects = citation_defects(&adjudication);
    assert_eq!(naming(&defects, "carry no adjudicated_on").len(), 1, "{defects:?}");
}

#[test]
fn a_quote_equal_to_the_cited_lines_passes_the_replay() {
    let (adjudication, index) = cited();
    let pair = &adjudication.agreement.pairs[index];
    let failures = quote_failures([pair], |_, _| Some(checkout(pair.line)));
    assert!(failures.is_empty(), "{failures:?}");
    assert_eq!(
        agreement_defects_under(&adjudication, &BTreeSet::new()),
        agreement_defects(&adjudication)
    );
}

#[test]
fn a_quote_the_code_does_not_hold_demotes_its_pair_to_an_escalation() {
    let (mut adjudication, index) = cited();
    adjudication.agreement.pairs[index].passes[0].evidence[0].quote = "    other();\n}".to_owned();
    let pair = &adjudication.agreement.pairs[index];
    let failures = quote_failures([pair], |_, _| Some(checkout(pair.line)));
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert!(failures.values().all(|failure| failure.contains("is not what those lines hold")));

    // An unreadable file fails the citation as surely as a wrong quote.
    let unread = quote_failures([pair], |_, _| None);
    assert_eq!(unread.len(), 1);

    let demoted = failures.keys().copied().collect();
    let defects = agreement_defects_under(&adjudication, &demoted);
    assert_eq!(naming(&defects, "escalated site carries a published verdict").len(), 1, "{defects:?}");
    assert_eq!(naming(&defects, "escalations_open published").len(), 1, "{defects:?}");
}
