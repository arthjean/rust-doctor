//! What anchors a published site to a run that actually saw it.
//!
//! Every other check on a site's `path:line` is gated. The two reproduction
//! tests locate each reviewed site inside the report the scan produced, and
//! they run only under `RUST_DOCTOR_CORPUS_DIR`, from a workflow that fires by
//! hand. So the position of a site, which is the one thing that makes a verdict
//! recomputable rather than declarative, was verified by nothing a contributor
//! runs: a site typed into the artifact by hand, at a line no finding was ever
//! reported on, passed `cargo test` and every gate behind it.
//!
//! The proof closes that offline. It is a digest over the identities of every
//! site the record publishes, reviewed and paired alike, written only by a run
//! that confirmed each of them against a live scan and checked by a test that
//! needs no clone cache at all. Hand-adding a site moves the digest, and the
//! only way to move the stored one is to reproduce.
//!
//! It is a digest rather than a second copy of the site list for the reason the
//! catalog is not copied to be tested: a frozen list doubles the sites in the
//! artifact and drifts against the list it mirrors, and nothing here needs to
//! read a position back, only to refuse one nobody has seen.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use super::agreement::is_iso_date;
use super::{Adjudication, CorpusArtifact, Population, SiteContext, write_atomically};

/// Digest over every published site, and the run that confirmed them.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PositionProof {
    /// Date of the reproduction that located every site in a live scan.
    ///
    /// Not the date of the measurement: `generated_at` already carries that.
    /// A record can be regenerated with its sites intact, and a site can be
    /// added to a record whose observations did not move, so what this field
    /// dates is the confirmation and nothing else.
    pub(crate) date: String,
    pub(crate) digest: String,
    /// Identities hashed: one per reviewed site, one per pair.
    ///
    /// Published beside the digest because a digest that disagrees says only
    /// that something moved, and the count is what tells a reader whether a
    /// site was added, removed or edited in place.
    pub(crate) sites: u64,
    /// The rustc the confirming run scanned under, spelled as `toolchain.rustc`
    /// spells it.
    ///
    /// Held here rather than read from that block, and asserted equal to it, so
    /// that a proof carried over from an older toolchain is a failure rather
    /// than a silent inheritance: which findings exist at all is the toolchain's
    /// answer, and a position confirmed under another one was confirmed against
    /// a different scan.
    pub(crate) toolchain: String,
}

/// A site as a reproduction has to find it: rule, position and context.
///
/// Reviewed sites and pairs are the same claim about the same corpus, and only
/// one of the two was ever located. A disagreeing pair is the case that makes
/// the difference visible: it has no reviewed site by construction, so the
/// escalation queue was the one part of the record no run has ever confirmed.
pub(crate) struct PublishedSite<'a> {
    pub(crate) context: SiteContext,
    /// The family digest a structural site claims, `None` on every other rule.
    /// What a reproduction checks with it is that the position it located is
    /// the anchor of the family that was judged, and not another family whose
    /// anchor drifted onto the same line.
    pub(crate) family: Option<&'a str>,
    pub(crate) line: u64,
    pub(crate) path: &'a str,
    pub(crate) repository: &'a str,
    pub(crate) rule: &'a str,
}

/// Every site one population publishes, reviewed and paired alike, deduplicated
/// on the identity a scan locates them by.
pub(crate) fn published_sites(
    adjudication: &Adjudication,
    population: Population,
) -> Vec<PublishedSite<'_>> {
    let mut sites: Vec<PublishedSite<'_>> = adjudication
        .reviewed
        .iter()
        .filter(|site| site.population == population)
        .map(|site| PublishedSite {
            context: site.context,
            family: site.family.as_deref(),
            line: site.line,
            path: site.path.as_str(),
            repository: site.repository.as_str(),
            rule: site.rule.as_str(),
        })
        .chain(
            adjudication
                .agreement
                .pairs
                .iter()
                .filter(|pair| pair.population == population)
                .map(|pair| PublishedSite {
                    context: pair.context,
                    family: pair.family.as_deref(),
                    line: pair.line,
                    path: pair.path.as_str(),
                    repository: pair.repository.as_str(),
                    rule: pair.rule.as_str(),
                }),
        )
        .collect();
    sites.sort_unstable_by_key(|site| (site.rule, site.repository, site.path, site.line, site.family));
    sites.dedup_by_key(|site| (site.rule, site.repository, site.path, site.line, site.family));
    sites
}

/// One hashed line. The kind is part of it, so that moving a site between
/// `reviewed` and `pairs`, which is exactly what resolving an escalation does,
/// moves the digest. The family is part of it too: a structural verdict is a
/// verdict about a family, so a site republished at the same position under a
/// family nobody judged is a site nobody has confirmed.
fn identity_line(
    kind: &str,
    population: Population,
    rule: &str,
    repository: &str,
    path: &str,
    line: u64,
    family: Option<&str>,
) -> String {
    format!(
        "{kind}\t{population:?}\t{rule}\t{repository}\t{path}\t{line}\t{}",
        family.unwrap_or_default()
    )
}

/// The sorted identities the digest is taken over.
fn identity_lines(adjudication: &Adjudication) -> Vec<String> {
    let mut lines: Vec<String> = adjudication
        .reviewed
        .iter()
        .map(|site| {
            identity_line(
                "reviewed",
                site.population,
                &site.rule,
                &site.repository,
                &site.path,
                site.line,
                site.family.as_deref(),
            )
        })
        .chain(adjudication.agreement.pairs.iter().map(|pair| {
            identity_line(
                "pair",
                pair.population,
                &pair.rule,
                &pair.repository,
                &pair.path,
                pair.line,
                pair.family.as_deref(),
            )
        }))
        .collect();
    lines.sort();
    lines
}

/// The digest of what the record publishes today, and how many identities went
/// into it.
pub(crate) fn recompute(adjudication: &Adjudication) -> (String, u64) {
    let lines = identity_lines(adjudication);
    let mut hasher = blake3::Hasher::new();
    for line in &lines {
        hasher.update(line.as_bytes());
        hasher.update(b"\n");
    }
    (hasher.finalize().to_hex().to_string(), lines.len() as u64)
}

/// Closed defects of the position proof, each naming what a reader has to do
/// about it.
///
/// The remedy is in the message rather than in a comment, because the reader of
/// this failure is whoever just added a site. It names the count and withholds
/// the recomputed digest: a message that prints the value it expects turns the
/// gate into one paste, and the count is what a reader actually needs, since it
/// says whether a site was added, removed or edited in place.
pub(crate) fn position_defects(artifact: &CorpusArtifact) -> Vec<String> {
    let proof = &artifact.adjudication.position_proof;
    let (digest, sites) = recompute(&artifact.adjudication);
    let mut defects = Vec::new();

    if proof.digest != digest {
        defects.push(format!(
            "the {sites} sites this record publishes do not hash to the stored {stored}: \
             a corpus reproduction is required before a site can be published, and it is \
             what rewrites this digest. The recomputed value is deliberately not printed, \
             since pasting it would publish a position no run has ever seen",
            stored = proof.digest
        ));
    }
    if proof.sites != sites {
        defects.push(format!(
            "position_proof claims {claimed} confirmed sites where the record publishes \
             {sites}: a corpus reproduction is required",
            claimed = proof.sites
        ));
    }
    if !is_iso_date(&proof.date) {
        defects.push(format!("position_proof.date is not a date: {}", proof.date));
    }
    if proof.toolchain != artifact.toolchain.rustc {
        defects.push(format!(
            "position_proof was written under {} while the record was measured under {}: \
             a corpus reproduction is required",
            proof.toolchain, artifact.toolchain.rustc
        ));
    }

    defects
}

/// Populations a proof covers. Both, always: the digest spans the whole record,
/// so a proof written from one confirmed population would anchor the other to
/// nothing.
const ATTESTED: [Population; 2] = [Population::Agent, Population::Healthy];

/// What one gated run confirmed, written where the other one can see it.
///
/// It carries the digest of the record it was confirming, so an attestation
/// left behind by an earlier run cannot stand in for a population this run
/// never replayed: a site added between the two runs moves the digest, and the
/// stale half stops counting.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct Attestation {
    confirmed: u64,
    digest: String,
    population: Population,
}

fn attestation_path(root: &Path, population: Population) -> PathBuf {
    root.join("position").join(format!("{population:?}.json"))
}

/// Record that a reproduction located every site of one population, and write
/// the proof once both populations have been.
///
/// The two populations replay into artifact directories of their own and are
/// confirmed by two tests, so neither of them can write the proof alone.
/// Whichever finishes second is the run that has seen every site, which is the
/// only run entitled to say so.
pub(crate) fn attest(
    root: &Path,
    artifact: &CorpusArtifact,
    population: Population,
    confirmed: u64,
) {
    let (digest, sites) = recompute(&artifact.adjudication);
    let attestation = Attestation {
        confirmed,
        digest: digest.clone(),
        population,
    };
    write_atomically(
        &attestation_path(root, population),
        &serde_json::to_vec_pretty(&attestation).unwrap(),
    );

    let covered = ATTESTED.iter().all(|population| {
        fs::read(attestation_path(root, *population))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Attestation>(&bytes).ok())
            .is_some_and(|attestation| attestation.digest == digest)
    });
    if !covered {
        return;
    }

    let proof = PositionProof {
        date: today(),
        digest,
        sites,
        toolchain: artifact.toolchain.rustc.clone(),
    };
    write_atomically(
        &root.join("position-proof.json"),
        &serde_json::to_vec_pretty(&proof).unwrap(),
    );
}

/// The day the confirmation happened, in the spelling every date of the record
/// uses.
fn today() -> String {
    let output = Command::new("date").args(["-u", "+%F"]).output().unwrap();
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}
