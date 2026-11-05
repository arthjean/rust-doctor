use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use super::agreement::is_iso_date;
use super::{Adjudication, CorpusArtifact, Population, SiteContext, write_atomically};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PositionProof {
    pub(crate) date: String,
    pub(crate) digest: String,
    pub(crate) sites: u64,
    pub(crate) toolchain: String,
}

pub(crate) struct PublishedSite<'a> {
    pub(crate) context: SiteContext,
    pub(crate) family: Option<&'a str>,
    pub(crate) line: u64,
    pub(crate) path: &'a str,
    pub(crate) repository: &'a str,
    pub(crate) rule: &'a str,
}

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

pub(crate) fn recompute(adjudication: &Adjudication) -> (String, u64) {
    let lines = identity_lines(adjudication);
    let mut hasher = blake3::Hasher::new();
    for line in &lines {
        hasher.update(line.as_bytes());
        hasher.update(b"\n");
    }
    (hasher.finalize().to_hex().to_string(), lines.len() as u64)
}

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

const ATTESTED: [Population; 2] = [Population::Agent, Population::Healthy];

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

fn today() -> String {
    let output = Command::new("date").args(["-u", "+%F"]).output().unwrap();
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}
