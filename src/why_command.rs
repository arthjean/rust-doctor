//! `rust-doctor why PATH:LINE[:COLUMN]`: the findings at one line, how each
//! is set, and what would silence it.
//!
//! Without `--report` it scans the one member owning the file, the way
//! `--package` does; with `--report` it reads that saved JSON and spawns
//! nothing. Both answer from the same JSON document.

use std::collections::BTreeSet;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Args;
use rust_doctor::explain::{WhyLocation, declared_generated, explain_site};
use rust_doctor::render::markdown::{check_schema, read_saved_report};
use rust_doctor::{InspectRequest, inspect};
use serde_json::Value;

#[derive(Debug, Clone, Args)]
pub struct WhyArgs {
    /// The site, as a report prints it: `src/lib.rs:12` or `src/lib.rs:12:5`.
    #[arg(value_name = "PATH:LINE[:COLUMN]")]
    location: String,
    /// A report `rust-doctor --json` saved, read instead of scanning.
    #[arg(long, value_name = "FILE")]
    report: Option<PathBuf>,
    #[arg(long)]
    json: bool,
}

pub fn run(arguments: &WhyArgs) -> ExitCode {
    let mut location: WhyLocation = match arguments.location.parse() {
        Ok(location) => location,
        Err(usage) => {
            eprintln!("rust-doctor: why {}: {usage}.", arguments.location);
            return ExitCode::from(2);
        }
    };
    let file = PathBuf::from(&location.path);
    let (report, root, declared) = match &arguments.report {
        Some(saved) => match read_saved_report(saved).and_then(|report| check_schema(&report).map(|()| report)) {
            Ok(report) => (report, saved_root(&file), BTreeSet::new()),
            Err(error) => {
                eprintln!("rust-doctor: {}: {error}.", saved.display());
                return ExitCode::from(2);
            }
        },
        None => match scan_owner(&file) {
            Ok((report, root)) => {
                let declared = declared_generated(&root);
                (report, root, declared)
            }
            Err(message) => {
                eprintln!("rust-doctor: why {}: {message}", location.path);
                return ExitCode::from(2);
            }
        },
    };
    if let Some(relative) = relative_to(&file, &root) {
        location.path = relative;
    }
    let site = explain_site(&report, location, &root, &declared);
    let written = if arguments.json {
        crate::rules_command::json_line(&site)
    } else {
        io::stdout().lock().write_all(site.text(&root).as_bytes())
    };
    match written {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}

/// Scans the member whose manifest is the nearest above `file`, and answers
/// the report as JSON with the workspace root its paths are relative to.
fn scan_owner(file: &Path) -> Result<(Value, PathBuf), String> {
    let absolute = file
        .canonicalize()
        .map_err(|_| "the file cannot be read.".to_owned())?;
    let (manifest_dir, name) = absolute
        .ancestors()
        .skip(1)
        .find_map(|directory| package_name(directory).map(|name| (directory.to_path_buf(), name)))
        .ok_or_else(|| "no Cargo package holds this file.".to_owned())?;
    let root = rust_doctor::cargo_workspace_root(&manifest_dir).unwrap_or(manifest_dir);
    rust_doctor::handle_interrupts();
    let report = inspect(InspectRequest::new(&root).with_package(name));
    if report.status == rust_doctor::Status::Failed {
        let reasons: Vec<&str> = report.errors.iter().map(|error| error.message.as_str()).collect();
        return Err(format!("the scan failed: {}", reasons.join(" ")));
    }
    let report = serde_json::to_value(&report).map_err(|error| error.to_string())?;
    Ok((report, root))
}

fn manifest(directory: &Path) -> Option<toml::Table> {
    std::fs::read_to_string(directory.join("Cargo.toml")).ok()?.parse().ok()
}

/// The `[package] name` of the manifest in `directory`, when it declares one.
fn package_name(directory: &Path) -> Option<String> {
    manifest(directory)?
        .get("package")?
        .get("name")?
        .as_str()
        .map(str::to_owned)
}

/// The root a saved report's paths are relative to, found without a
/// process: the nearest directory above the file whose manifest declares a
/// `[workspace]`, else the nearest holding a manifest, else the current
/// directory. It used to look for `Cargo.lock` first, which a library
/// commits nowhere and a scan no longer leaves behind, and found the
/// lockfile of whatever repository held the workspace.
fn saved_root(file: &Path) -> PathBuf {
    let Ok(absolute) = file.canonicalize() else {
        return PathBuf::from(".");
    };
    let manifests = || {
        absolute
            .ancestors()
            .skip(1)
            .filter(|directory| directory.join("Cargo.toml").is_file())
    };
    manifests()
        .find(|directory| manifest(directory).is_some_and(|manifest| manifest.contains_key("workspace")))
        .or_else(|| manifests().next())
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

/// `file` relative to `root` with `/` separators, as a report spells it.
fn relative_to(file: &Path, root: &Path) -> Option<String> {
    let relative = file
        .canonicalize()
        .ok()?
        .strip_prefix(root.canonicalize().ok()?)
        .ok()?
        .to_path_buf();
    let parts: Vec<String> = relative
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect();
    Some(parts.join("/"))
}
