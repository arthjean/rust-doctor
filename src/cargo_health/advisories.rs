//! Known-vulnerable and unmaintained crates, read from a RustSec database the
//! machine already holds.
//!
//! The tool never reaches the network, so it never fetches the database: it
//! reads the copy `cargo audit` or `cargo deny` fetched last under
//! `$CARGO_HOME`, or the directory `--advisory-db` names. Discovery accepts only a clone of the
//! RustSec remote: `cargo deny` clones whatever database a `deny.toml` names
//! into the same directory, and one of those deciding the verdicts in silence
//! would be another authority than the one the rules cite. Without one, both
//! rules are published as not evaluated and the score keeps its authority,
//! since a missing database is a machine that never asked, not a scan that
//! failed.
//!
//! Every read is bounded: an advisory file over 1 MiB, one that does not
//! parse, and every file past the 20,000th are skipped and counted, and the
//! scan completes with what remained.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use cargo_metadata::semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};

use super::resolution::LockedPackage;
use super::{Candidate, CargoHealthScan};
use crate::policy::{ActiveRules, CARGO_UNMAINTAINED_DEPENDENCY, CARGO_VULNERABLE_DEPENDENCY};
use crate::source_text::SourceSpan;

/// Largest advisory file read.
const MAX_ADVISORY_BYTES: u64 = 1024 * 1024;

/// Most advisory files read from one database.
const MAX_ADVISORY_FILES: usize = 20_000;

/// The one remote a discovered database may be a clone of, compared without
/// case and without a trailing `.git` or `/`.
const RUSTSEC_REMOTE: &str = "https://github.com/rustsec/advisory-db";

/// Largest `.git/config` read to tell a clone's remote.
const MAX_GIT_CONFIG_BYTES: u64 = 64 * 1024;

/// Age past which the report says the database is stale.
pub(crate) const STALE_AFTER_DAYS: i64 = 30;

/// The two registries crates.io is reached through, as `Cargo.lock` spells them.
const CRATES_IO: [&str; 2] = [
    "registry+https://github.com/rust-lang/crates.io-index",
    "sparse+https://index.crates.io/",
];

/// Which tool left the database, published without its path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AdvisorySource {
    /// `$CARGO_HOME/advisory-db`, where `cargo audit` fetches.
    CargoAudit,
    /// The newest directory under `$CARGO_HOME/advisory-dbs`, where `cargo
    /// deny` fetches, when it was fetched after `cargo audit`'s.
    CargoDeny,
    /// The directory `--advisory-db` named.
    Explicit,
}

/// A database to read, and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvisoryDatabase {
    pub(crate) source: AdvisorySource,
    pub(crate) path: PathBuf,
}

impl AdvisoryDatabase {
    /// The directory the caller named.
    pub fn explicit(path: impl Into<PathBuf>) -> Self {
        Self {
            source: AdvisorySource::Explicit,
            path: path.into(),
        }
    }

    /// The database the command line reads: the one `--advisory-db` named,
    /// or the one `discover` finds under `$CARGO_HOME`, which defaults to
    /// `~/.cargo` as it does for Cargo. `inspect` never calls this: a library
    /// caller names its database, and a scan without one reads none.
    pub fn locate(explicit: Option<PathBuf>) -> Option<Self> {
        if let Some(path) = explicit {
            return Some(Self::explicit(path));
        }
        let cargo_home = std::env::var_os("CARGO_HOME")
            .filter(|home| !home.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cargo")))?;
        Self::discover(&cargo_home)
    }

    /// The RustSec clone `cargo audit` or `cargo deny` left under
    /// `cargo_home` that was fetched last, or none. A directory counts when it
    /// holds `crates/`, which is where every advisory of the database lives,
    /// and its `.git/config` names the RustSec remote and no other.
    ///
    /// Freshness is the modification time of `.git/FETCH_HEAD`, which every
    /// fetch rewrites. `cargo audit`'s clone used to win whenever it existed,
    /// so a copy left 88 days ago hid the advisories a `cargo deny` clone
    /// fetched the same day knew. It still wins a tie, and when neither clone
    /// records a fetch, since it is the one the missing-database notice tells
    /// the reader to create. Of the `cargo deny` clones, the one fetched last
    /// is the candidate.
    pub fn discover(cargo_home: &Path) -> Option<Self> {
        let audit = Some(cargo_home.join("advisory-db")).filter(|path| rustsec_clone(path));
        let deny = fs::read_dir(cargo_home.join("advisory-dbs"))
            .ok()
            .and_then(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|entry| entry.path())
                    .filter(|path| rustsec_clone(path))
                    .filter_map(|path| Some((last_fetch(&path)?, path)))
                    .max()
            })
            .map(|(_, path)| path);
        let (source, path) = match (audit, deny) {
            (Some(audit), Some(deny)) if fetched(&deny) > fetched(&audit) => {
                (AdvisorySource::CargoDeny, deny)
            }
            (Some(audit), _) => (AdvisorySource::CargoAudit, audit),
            (None, Some(deny)) => (AdvisorySource::CargoDeny, deny),
            (None, None) => return None,
        };
        Some(Self { source, path })
    }
}

/// When a clone was last fetched, absent when it records no fetch.
fn fetched(path: &Path) -> Option<SystemTime> {
    fs::metadata(path.join(".git").join("FETCH_HEAD"))
        .and_then(|metadata| metadata.modified())
        .ok()
}

/// Whether `path` holds advisories and is a git clone whose every remote is
/// RustSec's. The config is read as text: asking git would start a program
/// for what one file says.
fn rustsec_clone(path: &Path) -> bool {
    if !path.join("crates").is_dir() {
        return false;
    }
    let mut config = String::new();
    let read = fs::File::open(path.join(".git").join("config"))
        .and_then(|file| file.take(MAX_GIT_CONFIG_BYTES).read_to_string(&mut config));
    if read.is_err() {
        return false;
    }
    let urls = remote_urls(&config);
    !urls.is_empty() && urls.iter().all(|url| is_rustsec_remote(url))
}

/// The `url` of every `[remote "..."]` section of a git config.
fn remote_urls(config: &str) -> Vec<&str> {
    let mut in_remote = false;
    let mut urls = Vec::new();
    for line in config.lines().map(str::trim) {
        if let Some(header) = line.strip_prefix('[') {
            in_remote = header
                .split([' ', ']'])
                .next()
                .is_some_and(|name| name.eq_ignore_ascii_case("remote"));
        } else if in_remote
            && let Some((key, value)) = line.split_once('=')
            && key.trim().eq_ignore_ascii_case("url")
        {
            urls.push(value.trim().trim_matches('"'));
        }
    }
    urls
}

fn is_rustsec_remote(url: &str) -> bool {
    let url = url.to_ascii_lowercase();
    let url = url.trim_end_matches('/');
    url.strip_suffix(".git").unwrap_or(url).trim_end_matches('/') == RUSTSEC_REMOTE
}

/// When a clone was last fetched, or cloned: the modification time of
/// `.git/FETCH_HEAD`, or of `.git/HEAD` for a clone never fetched since.
fn last_fetch(path: &Path) -> Option<SystemTime> {
    fetched(path).or_else(|| {
        fs::metadata(path.join(".git").join("HEAD"))
            .and_then(|metadata| metadata.modified())
            .ok()
    })
}

/// What the report publishes about the database it read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AdvisoryDbReport {
    pub source: AdvisorySource,
    /// The date of the newest advisory the database holds, `YYYY-MM-DD`.
    pub date: Option<String>,
    /// Advisories read.
    pub advisories: usize,
    /// Files skipped: over the size limit, unparsable, or past the file limit.
    pub skipped: usize,
}

impl AdvisoryDbReport {
    /// Whole days from the newest advisory to `now`, when the database dates
    /// one.
    pub(crate) fn age_days(&self, now: SystemTime) -> Option<i64> {
        let days = days_from_civil(self.date.as_deref()?)?;
        let today = i64::try_from(now.duration_since(UNIX_EPOCH).ok()?.as_secs() / 86_400).ok()?;
        Some(today - days)
    }
}

/// What the advisory pass came to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum AdvisoryOutcome {
    /// Both rules are off: nothing was looked for.
    #[default]
    NotRequested,
    /// No database: both rules are published as not evaluated.
    Missing,
    Consulted(AdvisoryDbReport),
}

#[derive(Debug, Deserialize)]
struct AdvisoryFile {
    advisory: Header,
    #[serde(default)]
    versions: Versions,
}

#[derive(Debug, Deserialize)]
struct Header {
    id: String,
    package: String,
    date: String,
    informational: Option<String>,
    withdrawn: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct Versions {
    #[serde(default)]
    patched: Vec<String>,
    #[serde(default)]
    unaffected: Vec<String>,
}

/// One advisory, its requirements parsed.
#[derive(Debug)]
struct Advisory {
    id: String,
    kind: Kind,
    patched: Vec<VersionReq>,
    written_patched: Vec<String>,
    unaffected: Vec<VersionReq>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Vulnerable,
    Unmaintained,
}

impl Advisory {
    fn affects(&self, version: &Version) -> bool {
        !self
            .patched
            .iter()
            .chain(&self.unaffected)
            .any(|requirement| requirement.matches(version))
    }
}

/// Every advisory of the database, by package, with what the report says of it.
fn read(database: &AdvisoryDatabase) -> Option<(BTreeMap<String, Vec<Advisory>>, AdvisoryDbReport)> {
    let crates = database.path.join("crates");
    let mut files = Vec::new();
    for directory in fs::read_dir(&crates).ok()?.filter_map(Result::ok) {
        let Ok(entries) = fs::read_dir(directory.path()) else {
            continue;
        };
        files.extend(entries.filter_map(Result::ok).map(|entry| entry.path()).filter(|path| {
            matches!(path.extension().and_then(|extension| extension.to_str()), Some("md" | "toml"))
        }));
    }
    files.sort();

    let mut report = AdvisoryDbReport {
        source: database.source,
        date: None,
        advisories: 0,
        skipped: files.len().saturating_sub(MAX_ADVISORY_FILES),
    };
    let mut advisories: BTreeMap<String, Vec<Advisory>> = BTreeMap::new();
    for path in files.iter().take(MAX_ADVISORY_FILES) {
        let Some(file) = parse(path) else {
            report.skipped += 1;
            continue;
        };
        if days_from_civil(&file.advisory.date).is_none() {
            report.skipped += 1;
            continue;
        }
        report.advisories += 1;
        if report.date.as_deref().is_none_or(|newest| file.advisory.date.as_str() > newest) {
            report.date = Some(file.advisory.date.clone());
        }
        let kind = match file.advisory.informational.as_deref() {
            None | Some("unsound") => Kind::Vulnerable,
            Some("unmaintained") => Kind::Unmaintained,
            Some(_) => continue,
        };
        if file.advisory.withdrawn.is_some() {
            continue;
        }
        let requirements = |written: &[String]| -> Option<Vec<VersionReq>> {
            written.iter().map(|requirement| VersionReq::parse(requirement).ok()).collect()
        };
        let (Some(patched), Some(unaffected)) = (
            requirements(&file.versions.patched),
            requirements(&file.versions.unaffected),
        ) else {
            report.advisories -= 1;
            report.skipped += 1;
            continue;
        };
        advisories.entry(file.advisory.package).or_default().push(Advisory {
            id: file.advisory.id,
            kind,
            patched,
            written_patched: file.versions.patched,
            unaffected,
        });
    }
    Some((advisories, report))
}

/// The TOML an advisory carries: the whole file for the legacy `.toml` form,
/// the fenced block the `.md` form opens on otherwise.
fn parse(path: &Path) -> Option<AdvisoryFile> {
    let metadata = fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_ADVISORY_BYTES {
        return None;
    }
    let text = fs::read_to_string(path).ok()?;
    let front = if path.extension().is_some_and(|extension| extension == "toml") {
        text.as_str()
    } else {
        let body = text.trim_start().strip_prefix("```toml")?;
        body.get(..body.find("\n```")?)?
    };
    toml::from_str(front).ok()
}

/// Judges every crates.io package `Cargo.lock` resolves against the advisories
/// filed under its name.
pub(super) fn inspect(
    database: Option<&AdvisoryDatabase>,
    packages: &[LockedPackage],
    lockfile: &str,
    owner: Option<&str>,
    active: &ActiveRules,
    scan: &mut CargoHealthScan,
) {
    let Some((advisories, report)) = database.and_then(read) else {
        scan.advisory = AdvisoryOutcome::Missing;
        return;
    };
    for package in packages {
        if !package
            .source
            .as_deref()
            .is_some_and(|source| CRATES_IO.iter().any(|registry| source.starts_with(registry)))
        {
            continue;
        }
        let (Some(filed), Ok(version)) = (advisories.get(&package.name), Version::parse(&package.version))
        else {
            continue;
        };
        for advisory in filed.iter().filter(|advisory| advisory.affects(&version)) {
            let (definition, message) = match advisory.kind {
                Kind::Vulnerable => (
                    &CARGO_VULNERABLE_DEPENDENCY,
                    format!(
                        "Cargo.lock resolves {} {}, which {} affects; {}.",
                        package.name,
                        package.version,
                        advisory.id,
                        patched(&advisory.written_patched)
                    ),
                ),
                Kind::Unmaintained => (
                    &CARGO_UNMAINTAINED_DEPENDENCY,
                    format!(
                        "Cargo.lock resolves {} {}, which {} reports unmaintained.",
                        package.name, package.version, advisory.id
                    ),
                ),
            };
            if !active.on(definition) {
                continue;
            }
            scan.candidates.push(Candidate {
                definition,
                message,
                package: owner.map(str::to_owned),
                manifest_path: Some("Cargo.lock".to_owned()),
                span: entry_span(lockfile, &package.name, &package.version),
                key: None,
                informational: false,
            });
        }
    }
    scan.advisory = AdvisoryOutcome::Consulted(report);
}

fn patched(written: &[String]) -> String {
    if written.is_empty() {
        "no patched release exists".to_owned()
    } else {
        format!("patched in {}", written.join(" or "))
    }
}

/// The `name = "..."` line of the `[[package]]` entry resolving `name` at
/// `version`.
fn entry_span(lockfile: &str, name: &str, version: &str) -> Option<SourceSpan> {
    let name_line = format!("name = \"{name}\"");
    let version_line = format!("version = \"{version}\"");
    let lines: Vec<&str> = lockfile.lines().collect();
    let index = lines.windows(2).position(|pair| {
        pair[0].trim() == name_line && pair[1].trim() == version_line
    })?;
    let line = index + 1;
    Some(SourceSpan {
        line_start: line,
        column_start: 1,
        line_end: line,
        column_end: lines[index].chars().count() + 1,
    })
}

/// Days from 1970-01-01 to a `YYYY-MM-DD` date, in the proleptic Gregorian
/// calendar (Howard Hinnant's `days_from_civil`).
fn days_from_civil(date: &str) -> Option<i64> {
    let mut parts = date.splitn(3, '-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: i64 = parts.next()?.parse().ok()?;
    let day: i64 = parts.next()?.parse().ok()?;
    if date.len() != 10 || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    Some(era * 146_097 + day_of_era - 719_468)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// EP-003 US-013: the RustSec remote matches in any spelling git keeps
    /// it, and a config naming another remote beside it is not a RustSec
    /// clone.
    #[test]
    fn only_the_rustsec_remote_is_a_rustsec_clone() {
        for url in [
            "https://github.com/rustsec/advisory-db",
            "https://github.com/RustSec/advisory-db.git/",
            "https://github.com/rustsec/advisory-db/",
            "\"https://github.com/RustSec/advisory-db.git\"",
        ] {
            let config = format!("[core]\n\tbare = false\n[remote \"origin\"]\n\tURL = {url}\n");
            let urls = remote_urls(&config);
            assert!(urls.len() == 1 && is_rustsec_remote(urls[0]), "{url}");
        }
        for url in [
            "https://example.invalid/advisory-db",
            "https://github.com/rustsec/advisory-db-mirror",
            "git@github.com:rustsec/advisory-db.git",
        ] {
            assert!(!is_rustsec_remote(url), "{url}");
        }
        let config = "[remote \"origin\"]\n\turl = https://github.com/rustsec/advisory-db\n\
                      [remote \"mirror\"]\n\turl = https://example.invalid/advisory-db\n\
                      [branch \"main\"]\n\turl = ignored\n";
        assert_eq!(remote_urls(config).len(), 2);

        let root = scratch("remote-check");
        fs::create_dir_all(root.join("db/crates")).unwrap();
        assert!(!rustsec_clone(&root.join("db")), "a database with no git config is not discovered");
        fs::create_dir_all(root.join("db/.git")).unwrap();
        fs::write(root.join("db/.git/config"), config).unwrap();
        assert!(!rustsec_clone(&root.join("db")));
        fs::write(root.join("db/.git/config"), "[remote \"origin\"]\n\turl = https://github.com/rustsec/advisory-db\n").unwrap();
        assert!(rustsec_clone(&root.join("db")));
    }

    #[test]
    fn civil_days_count_from_the_epoch() {
        assert_eq!(days_from_civil("1970-01-01"), Some(0));
        assert_eq!(days_from_civil("2000-03-01"), Some(11_017));
        assert_eq!(days_from_civil("2026-09-26"), Some(20_722));
        assert_eq!(days_from_civil("2026-13-01"), None);
        assert_eq!(days_from_civil("26-09-26"), None);
    }

    #[test]
    fn a_version_is_affected_when_no_requirement_covers_it() {
        let advisory = Advisory {
            id: "RUSTSEC-0000-0000".to_owned(),
            kind: Kind::Vulnerable,
            patched: vec![VersionReq::parse(">= 1.2.3").unwrap()],
            written_patched: vec![">= 1.2.3".to_owned()],
            unaffected: vec![VersionReq::parse("< 1.0.0").unwrap()],
        };
        let version = |text: &str| Version::parse(text).unwrap();
        assert!(advisory.affects(&version("1.2.2")));
        assert!(!advisory.affects(&version("1.2.3")));
        assert!(!advisory.affects(&version("0.9.0")));
    }

    fn advisory(id: &str, date: &str) -> String {
        format!(
            "```toml\n[advisory]\nid = \"{id}\"\npackage = \"bulk\"\ndate = \"{date}\"\n\n[versions]\npatched = []\n```\n"
        )
    }

    fn scratch(label: &str) -> PathBuf {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/advisory-limits")
            .join(format!("{}-{label}", std::process::id()));
        if root.exists() {
            fs::remove_dir_all(&root).unwrap();
        }
        fs::create_dir_all(root.join("crates/bulk")).unwrap();
        root
    }

    /// An advisory over 1 MiB and every file past the 20,000th are skipped and
    /// counted, and what remains is read.
    #[test]
    fn oversized_and_surplus_advisory_files_are_skipped_and_counted() {
        let root = scratch("limits");
        let crates = root.join("crates/bulk");
        for index in 0..MAX_ADVISORY_FILES + 2 {
            fs::write(
                crates.join(format!("RUSTSEC-{index:05}.md")),
                advisory(&format!("RUSTSEC-{index:05}"), "2020-01-01"),
            )
            .unwrap();
        }
        // Sorted first, so it is one of the 20,000 read.
        let mut oversized = advisory("RUSTSEC-00000", "2020-01-02");
        oversized.push_str(&"x".repeat(usize::try_from(MAX_ADVISORY_BYTES).unwrap()));
        fs::write(crates.join("RUSTSEC-00000.md"), oversized).unwrap();

        let (advisories, report) = read(&AdvisoryDatabase::explicit(&root)).unwrap();
        assert_eq!(report.skipped, 3);
        assert_eq!(report.advisories, MAX_ADVISORY_FILES - 1);
        assert_eq!(advisories["bulk"].len(), MAX_ADVISORY_FILES - 1);
        assert_eq!(report.date.as_deref(), Some("2020-01-01"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_database_dated_in_the_future_is_not_stale() {
        let report = AdvisoryDbReport {
            source: AdvisorySource::Explicit,
            date: Some("2999-01-01".to_owned()),
            advisories: 1,
            skipped: 0,
        };
        assert!(report.age_days(SystemTime::now()).is_some_and(|days| days < STALE_AFTER_DAYS));
    }

    #[test]
    fn the_entry_span_names_the_package_line_of_its_version() {
        let lockfile = "version = 4\n\n[[package]]\nname = \"a\"\nversion = \"0.1.0\"\n\n[[package]]\nname = \"a\"\nversion = \"0.2.0\"\n";
        let span = entry_span(lockfile, "a", "0.2.0").unwrap();
        assert_eq!((span.line_start, span.column_start, span.column_end), (8, 1, 11));
    }
}
