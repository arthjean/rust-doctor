#![cfg(unix)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! What a published site is anchored to.
//!
//! A verdict is worth what its site is worth, and a site is worth what a run
//! that saw it says. Every check on `path:line` in this repository was gated:
//! the two reproduction tests locate each reviewed site in the report the scan
//! wrote, and they run only under `RUST_DOCTOR_CORPUS_DIR`, from a workflow
//! that fires by hand. So the position of a site was verified by nothing a
//! contributor runs, and a site typed in by hand at a line no finding was ever
//! reported on passed `cargo test` and every gate behind it.
//!
//! Every proof here runs from the artifact alone: it recomputes the digest of
//! what the record publishes and compares it against the one a reproduction
//! wrote. No clone cache, no network, no environment variable.

mod support;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;


use support::corpus::reproduction::{Reproduction, decide};
use support::corpus::position::{PositionProof, position_defects, published_sites, recompute};
use support::corpus::{Population, Provenance, ReviewedSite, SiteContext, Verdict, artifact};

/// The three sites the two passes of 2026-08-11 disagreed on, which live in
/// `pairs` alone: an escalation has no reviewed site, so hashing `reviewed`
/// would leave the whole escalation queue anchored to nothing.
const ESCALATED: [(&str, &str, u64); 3] = [
    ("ripgrep", "crates/ignore/src/dir.rs", 340),
    ("thiserror", "impl/src/expand.rs", 31),
    ("thiserror", "impl/src/expand.rs", 221),
];

fn probe_site() -> ReviewedSite {
    ReviewedSite {
        context: SiteContext::Production,
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

/// The record ships anchored: every site it publishes was located by the run
/// that wrote the proof.
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

/// The regression this closes: a site added by hand, with no run behind it,
/// used to pass the whole suite.
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

/// Removing a site is as much a change to what the record publishes as adding
/// one, and the digest is what makes the two the same failure.
#[test]
fn a_site_removed_by_hand_is_refused_the_same_way() {
    let mut forged = artifact();
    forged.adjudication.reviewed.pop();
    defect_naming(&position_defects(&forged), "a corpus reproduction is required");
}

/// Pairs are hashed too, which is the only anchor an escalated site can have:
/// it has no reviewed entry by construction.
#[test]
fn an_escalated_site_is_anchored_through_its_pair() {
    let artifact = artifact();
    let sites = published_sites(&artifact.adjudication, Population::Healthy);
    for (repository, path, line) in ESCALATED {
        assert!(
            sites.iter().any(|site| {
                site.repository == repository && site.path == path && site.line == line
            }),
            "the escalated site {repository}/{path}:{line} is what a reproduction has to locate"
        );
        assert!(
            !artifact.adjudication.reviewed.iter().any(|site| {
                site.repository == repository && site.path == path && site.line == line
            }),
            "an escalated site carries no published verdict: {repository}/{path}:{line}"
        );
    }

    let mut forged = artifact.clone();
    forged.adjudication.agreement.pairs.pop();
    defect_naming(&position_defects(&forged), "a corpus reproduction is required");
}

/// Moving a site between `reviewed` and `pairs` is what resolving an escalation
/// does, and it changes what the record claims about that site, so it moves the
/// digest rather than cancelling out inside it.
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

/// The proof names the toolchain that confirmed the positions, and it has to be
/// the one the record was measured under: which findings exist at all is the
/// toolchain's answer, so a position confirmed under another one was confirmed
/// against a different scan.
#[test]
fn a_proof_carried_over_from_another_toolchain_is_refused() {
    let artifact = artifact();
    assert_eq!(artifact.adjudication.position_proof.toolchain, artifact.toolchain.rustc);

    let mut forged = artifact;
    forged.adjudication.position_proof.toolchain = "rustc 1.96.0 (0000000000 2026-01-01)".to_owned();
    defect_naming(&position_defects(&forged), "a corpus reproduction is required");
}

/// The proof dates the confirmation, not the measurement, and a date is a date.
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

/// The proof publishes no path, no environment variable and no user data: three
/// short fields and a digest, the way every other member of the record is
/// workspace-relative or nothing at all.
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

// ---------------------------------------------------------------------------
// The gate a reproduction opens on
// ---------------------------------------------------------------------------

/// A machine with no clone cache runs `cargo test` and is told, by the run
/// itself, that the check able to locate a published site did not run.
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

/// The failure the silent return hid: a reproduction that was attempted, got
/// one of its two variables wrong, and passed. Half a configuration is not a
/// reason to skip, it is a reproduction that cannot be trusted to mean
/// anything, so it fails and names the variable that is missing.
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

/// Emits the skip notice and nothing else, so that the test below can read what
/// a passing run actually shows a reader. Ignored, because its whole job is to
/// be re-run as a child process.
#[test]
#[ignore = "run as a child by the_skip_notice_survives_a_capturing_run"]
fn skip_notice_probe() {
    assert!(decide(None, None).directories(Path::new("/")).is_none());
}

/// The gap the skip left open after the silent `return` was replaced: libtest
/// captures the `print!` family for a test that passes, and a skipped
/// reproduction passes by construction, so a plain `cargo test` printed nothing
/// and the reader learned no more than before. Only `--nocapture` showed it,
/// which is not the command anyone runs.
///
/// Proven by running this same binary the way `cargo test` runs it, capture and
/// all, and reading what came out.
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

/// Both variables set is a replay, and the paths reach the caller as given.
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
