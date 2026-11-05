#![cfg(unix)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod support;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;


use support::corpus::reproduction::{Reproduction, decide};
use support::corpus::position::{PositionProof, position_defects, published_sites, recompute};
use support::corpus::agreement::family_defects;
use support::corpus::{Population, Provenance, ReviewedSite, SiteContext, Verdict, artifact};

const ESCALATED: [(&str, &str, &str, u64); 3] = [
    (
        "rust_doctor::structure::complex_function",
        "ripgrep",
        "crates/ignore/src/dir.rs",
        340,
    ),
    (
        "rust_doctor::structure::complex_function",
        "thiserror",
        "impl/src/expand.rs",
        31,
    ),
    (
        "rust_doctor::structure::complex_function",
        "thiserror",
        "impl/src/expand.rs",
        221,
    ),
];

fn probe_site() -> ReviewedSite {
    ReviewedSite {
        context: SiteContext::Production,
        family: None,
        justification: "a site nobody scanned".to_owned(),
        line: 1,
        path: "src/lib.rs".to_owned(),
        population: Population::Healthy,
        provenance: Provenance::Agent,
        repository: "ripgrep".to_owned(),
        rule: "clippy::probe".to_owned(),
        verdict: Verdict::TruePositive,
    }
}

fn defect_naming(defects: &[String], needle: &str) -> String {
    let named = defects.iter().find(|defect| defect.contains(needle));
    assert!(named.is_some(), "no defect naming {needle}: {defects:?}");
    named.cloned().unwrap_or_default()
}

#[test]
fn every_published_site_is_anchored_to_the_run_that_located_it() {
    let artifact = artifact();
    assert_eq!(position_defects(&artifact), Vec::<String>::new());

    let adjudication = &artifact.adjudication;
    let (digest, sites) = recompute(adjudication);
    assert_eq!(adjudication.position_proof.digest, digest);
    assert_eq!(
        sites,
        (adjudication.reviewed.len() + adjudication.agreement.pairs.len()) as u64,
        "one identity per reviewed site and one per pair"
    );
    assert_eq!(adjudication.position_proof.sites, sites);
}

#[test]
fn a_site_added_by_hand_is_refused_and_asks_for_a_reproduction() {
    let mut forged = artifact();
    forged.adjudication.reviewed.push(probe_site());

    let defects = position_defects(&forged);
    let named = defect_naming(&defects, "a corpus reproduction is required");
    let (_, sites) = recompute(&forged.adjudication);
    assert!(
        named.contains(&sites.to_string()),
        "the failure names the count of sites it hashed: {named}"
    );
    assert!(
        defects.iter().any(|defect| defect.contains("position_proof claims")),
        "the count moves with the digest: {defects:?}"
    );
}

#[test]
fn a_site_removed_by_hand_is_refused_the_same_way() {
    let mut forged = artifact();
    forged.adjudication.reviewed.pop();
    defect_naming(&position_defects(&forged), "a corpus reproduction is required");
}

#[test]
fn an_escalated_site_is_anchored_through_its_pair() {
    let artifact = artifact();
    let sites = published_sites(&artifact.adjudication, Population::Healthy);
    for (rule, repository, path, line) in ESCALATED {
        assert!(
            sites.iter().any(|site| {
                site.rule == rule
                    && site.repository == repository
                    && site.path == path
                    && site.line == line
            }),
            "the escalated site {rule} at {repository}/{path}:{line} is what a reproduction has to locate"
        );
        assert!(
            !artifact.adjudication.reviewed.iter().any(|site| {
                site.rule == rule
                    && site.repository == repository
                    && site.path == path
                    && site.line == line
            }),
            "an escalated site carries no published verdict: {rule} at {repository}/{path}:{line}"
        );
    }

    let mut forged = artifact.clone();
    forged.adjudication.agreement.pairs.pop();
    defect_naming(&position_defects(&forged), "a corpus reproduction is required");
}

#[test]
fn resolving_an_escalation_moves_the_digest() {
    let artifact = artifact();
    let escalated = artifact
        .adjudication
        .agreement
        .pairs
        .iter()
        .find(|pair| !pair.agrees())
        .expect("the record publishes an escalation queue")
        .clone();

    let mut forged = artifact.clone();
    forged.adjudication.reviewed.push(ReviewedSite {
        context: escalated.context,
        family: escalated.family.clone(),
        justification: "settled by a human".to_owned(),
        line: escalated.line,
        path: escalated.path.clone(),
        population: escalated.population,
        provenance: Provenance::Human,
        repository: escalated.repository.clone(),
        rule: escalated.rule.clone(),
        verdict: Verdict::TruePositive,
    });
    assert_ne!(
        recompute(&forged.adjudication).0,
        recompute(&artifact.adjudication).0
    );
}

#[test]
fn a_proof_carried_over_from_another_toolchain_is_refused() {
    let artifact = artifact();
    assert_eq!(artifact.adjudication.position_proof.toolchain, artifact.toolchain.rustc);

    let mut forged = artifact;
    forged.adjudication.position_proof.toolchain = "rustc 1.96.0 (0000000000 2026-01-01)".to_owned();
    defect_naming(&position_defects(&forged), "a corpus reproduction is required");
}

#[test]
fn the_proof_dates_the_run_that_confirmed_the_sites() {
    let artifact = artifact();
    let proof = &artifact.adjudication.position_proof;
    assert!(
        proof.date >= artifact.generated_at,
        "a record cannot be confirmed before it was measured: {} against {}",
        proof.date,
        artifact.generated_at
    );

    let mut forged = artifact.clone();
    forged.adjudication.position_proof.date = "2026-8-1".to_owned();
    defect_naming(&position_defects(&forged), "position_proof.date is not a date");
}

#[test]
fn the_proof_carries_no_path_and_no_user_data() {
    let artifact = artifact();
    let PositionProof {
        date,
        digest,
        sites: _,
        toolchain,
    } = &artifact.adjudication.position_proof;
    for text in [date, digest, toolchain] {
        assert!(
            !text.contains('/') && !text.contains("HOME") && !text.contains("RUST_DOCTOR"),
            "the proof carries no path and no environment variable: {text}"
        );
    }
    assert_eq!(digest.len(), 64, "a blake3 digest, hexadecimal");
    assert!(digest.chars().all(|character| character.is_ascii_hexdigit()));
}

#[test]
fn a_run_that_attempts_no_reproduction_says_so() {
    let reason = match decide(None, None) {
        Reproduction::Skipped(reason) => reason,
        decided => format!("neither variable set is not a skip: {decided:?}"),
    };
    assert!(reason.contains("RUST_DOCTOR_CORPUS_DIR"), "{reason}");
    assert!(reason.contains("RUST_DOCTOR_CORPUS_ARTIFACTS"), "{reason}");
    assert!(reason.starts_with("skipped:"), "{reason}");
}

#[test]
fn a_half_configured_reproduction_is_refused_rather_than_skipped() {
    for (cache, artifacts, missing) in [
        (Some(OsString::from("/cache")), None, "RUST_DOCTOR_CORPUS_ARTIFACTS"),
        (None, Some(OsString::from("/scratch")), "RUST_DOCTOR_CORPUS_DIR"),
    ] {
        let reason = match decide(cache, artifacts) {
            Reproduction::Misconfigured(reason) => reason,
            decided => format!("a half-configured run is not a skip: {decided:?}"),
        };
        assert!(reason.contains(missing), "{reason}");
    }
}

#[test]
#[ignore = "run as a child by the_skip_notice_survives_a_capturing_run"]
fn skip_notice_probe() {
    assert!(decide(None, None).directories(Path::new("/")).is_none());
}

#[test]
fn the_skip_notice_survives_a_capturing_run() {
    let binary = std::env::current_exe().expect("a test binary knows its own path");
    let run = Command::new(binary)
        .args(["--exact", "--ignored", "skip_notice_probe"])
        .output()
        .expect("the test binary should re-run itself");
    assert!(run.status.success(), "the probe should pass: {run:?}");

    let emitted = String::from_utf8_lossy(&run.stderr);
    assert!(
        emitted.contains("skipped: no corpus reproduction was attempted"),
        "a capturing run swallowed the skip notice, which is the silence it replaced: {emitted}"
    );
    assert!(
        emitted.contains("RUST_DOCTOR_CORPUS_DIR") && emitted.contains("RUST_DOCTOR_CORPUS_ARTIFACTS"),
        "the notice names both variables where a reader can see them: {emitted}"
    );
}

#[test]
fn both_variables_set_is_a_reproduction() {
    let decided = decide(
        Some(OsString::from("/cache")),
        Some(OsString::from("/scratch")),
    );
    assert_eq!(
        decided,
        Reproduction::Run {
            artifacts: PathBuf::from("/scratch"),
            cache: PathBuf::from("/cache"),
        }
    );
}

#[test]
fn every_structural_site_names_the_family_it_judges() {
    let artifact = artifact();
    assert_eq!(family_defects(&artifact.adjudication), Vec::<String>::new());

    let structural = artifact
        .adjudication
        .reviewed
        .iter()
        .filter(|site| site.rule.starts_with("rust_doctor::structure::"))
        .count();
    let named = artifact
        .adjudication
        .reviewed
        .iter()
        .filter(|site| site.family.is_some())
        .count();
    assert_eq!(structural, named);
    assert!(structural > 0);
}

#[test]
fn a_structural_site_with_no_family_is_named() {
    let mut forged = artifact();
    let site = forged
        .adjudication
        .reviewed
        .iter_mut()
        .find(|site| site.family.is_some())
        .expect("the record publishes a structural site");
    site.family = None;
    defect_naming(&family_defects(&forged.adjudication), "structural site with no family");
}

#[test]
fn a_family_on_a_positional_rule_is_named() {
    let mut forged = artifact();
    let site = forged
        .adjudication
        .reviewed
        .iter_mut()
        .find(|site| !site.rule.starts_with("rust_doctor::structure::"))
        .expect("the record publishes a Clippy site");
    site.family = Some("0".repeat(64));
    defect_naming(
        &family_defects(&forged.adjudication),
        "family published on a rule whose unit is a position",
    );
}

#[test]
fn a_pair_and_its_site_disagreeing_on_the_family_are_named() {
    let mut forged = artifact();
    let label = {
        let pair = forged
            .adjudication
            .agreement
            .pairs
            .iter_mut()
            .find(|pair| pair.agrees() && pair.family.is_some())
            .expect("the record publishes an agreeing structural pair");
        pair.family = Some("0".repeat(64));
        pair.label()
    };
    let named = defect_naming(
        &family_defects(&forged.adjudication),
        "pair and reviewed site publish different families",
    );
    assert!(named.contains(&label), "{named}");
}

#[test]
fn a_family_edited_by_hand_moves_the_digest() {
    let artifact = artifact();
    let mut forged = artifact.clone();
    let site = forged
        .adjudication
        .reviewed
        .iter_mut()
        .find(|site| site.family.is_some())
        .expect("the record publishes a structural site");
    site.family = Some("0".repeat(64));
    assert_ne!(
        recompute(&forged.adjudication).0,
        recompute(&artifact.adjudication).0
    );
}
