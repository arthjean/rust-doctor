use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::Instant;

#[derive(Deserialize)]
struct FixtureManifest {
    schema_version: u32,
    generator_revision: String,
    fixtures: Vec<Fixture>,
}

#[derive(Deserialize)]
struct Fixture {
    id: String,
    topology: Topology,
    members: usize,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Topology {
    Single,
    VirtualWorkspace,
    ProcMacro,
    BuildScript,
    Workspace,
}

#[derive(Clone, Copy)]
enum Strategy {
    Full,
    PackageSelected,
    ReportFiltered,
}

impl Strategy {
    const ALL: [Self; 3] = [Self::Full, Self::PackageSelected, Self::ReportFiltered];

    const fn name(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::PackageSelected => "package_selected",
            Self::ReportFiltered => "report_filtered",
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("benchmark failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let manifest_path = std::env::args_os().nth(1).map_or_else(
        || PathBuf::from("benchmarks/scope/fixtures.json"),
        PathBuf::from,
    );
    let content = std::fs::read_to_string(&manifest_path)
        .map_err(|error| format!("failed to read '{}': {error}", manifest_path.display()))?;
    let manifest: FixtureManifest = serde_json::from_str(&content)
        .map_err(|error| format!("invalid fixture manifest: {error}"))?;
    if manifest.schema_version != 1 || manifest.generator_revision != "scope-fixtures-v1" {
        return Err("unsupported scope fixture manifest revision".to_string());
    }

    for fixture in &manifest.fixtures {
        let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
        let selected_package = materialize_fixture(temp.path(), fixture)?;
        for strategy in Strategy::ALL {
            let measurement = measure(temp.path(), &selected_package, strategy)?;
            println!(
                "{}",
                serde_json::json!({
                    "fixture": fixture.id,
                    "topology": topology_name(fixture.topology),
                    "members": fixture.members,
                    "strategy": strategy.name(),
                    "elapsed_ms": measurement.elapsed_ms,
                    "compiler_messages": measurement.compiler_messages,
                    "success": measurement.success,
                })
            );
        }
    }
    Ok(())
}

struct Measurement {
    elapsed_ms: u128,
    compiler_messages: usize,
    success: bool,
}

fn measure(root: &Path, selected_package: &str, strategy: Strategy) -> Result<Measurement, String> {
    let target_dir = root.join("targets").join(strategy.name());
    let mut command = Command::new("cargo");
    command
        .current_dir(root)
        .env("CARGO_TARGET_DIR", target_dir)
        .args([
            "clippy",
            "--offline",
            "--message-format=json",
            "--manifest-path",
        ])
        .arg(root.join("Cargo.toml"))
        .arg("--all-targets");
    match strategy {
        Strategy::PackageSelected => {
            command.args(["-p", selected_package]);
        }
        Strategy::Full | Strategy::ReportFiltered => {
            command.arg("--workspace");
        }
    }

    let started = Instant::now();
    let output = command
        .output()
        .map_err(|error| format!("failed to launch cargo clippy: {error}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(Measurement {
        elapsed_ms: started.elapsed().as_millis(),
        compiler_messages: stdout.matches("\"reason\":\"compiler-message\"").count(),
        success: output.status.success(),
    })
}

fn materialize_fixture(root: &Path, fixture: &Fixture) -> Result<String, String> {
    match fixture.topology {
        Topology::Single => write_package(root, &fixture.id, PackageKind::Library)?,
        Topology::ProcMacro => write_package(root, &fixture.id, PackageKind::ProcMacro)?,
        Topology::BuildScript => write_package(root, &fixture.id, PackageKind::BuildScript)?,
        Topology::VirtualWorkspace | Topology::Workspace => {
            if fixture.members == 0 {
                return Err(format!("fixture '{}' has no members", fixture.id));
            }
            let members: Vec<String> = (0..fixture.members)
                .map(|index| format!("member-{index:02}"))
                .collect();
            let member_list = members
                .iter()
                .map(|member| format!("\"{member}\""))
                .collect::<Vec<_>>()
                .join(", ");
            write_file(
                &root.join("Cargo.toml"),
                &format!("[workspace]\nresolver = \"2\"\nmembers = [{member_list}]\n"),
            )?;
            for member in &members {
                write_package(&root.join(member), member, PackageKind::Library)?;
            }
            return members
                .first()
                .cloned()
                .ok_or_else(|| "workspace fixture has no selected package".to_string());
        }
    }
    Ok(fixture.id.clone())
}

#[derive(Clone, Copy)]
enum PackageKind {
    Library,
    ProcMacro,
    BuildScript,
}

fn write_package(root: &Path, name: &str, kind: PackageKind) -> Result<(), String> {
    std::fs::create_dir_all(root.join("src"))
        .map_err(|error| format!("failed to create '{}': {error}", root.display()))?;
    let mut manifest = format!(
        "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\nrust-version = \"1.97\"\n"
    );
    if matches!(kind, PackageKind::ProcMacro) {
        manifest.push_str("\n[lib]\nproc-macro = true\n");
    } else if matches!(kind, PackageKind::BuildScript) {
        manifest.push_str("build = \"build.rs\"\n");
        write_file(&root.join("build.rs"), "fn main() {}\n")?;
    }
    write_file(&root.join("Cargo.toml"), &manifest)?;
    let source = if matches!(kind, PackageKind::ProcMacro) {
        "extern crate proc_macro;\nuse proc_macro::TokenStream;\n#[proc_macro]\npub fn identity(input: TokenStream) -> TokenStream { input }\n"
    } else {
        "pub fn fixture_value() -> usize { 42 }\n"
    };
    write_file(&root.join("src/lib.rs"), source)
}

fn write_file(path: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create '{}': {error}", parent.display()))?;
    }
    std::fs::write(path, content)
        .map_err(|error| format!("failed to write '{}': {error}", path.display()))
}

const fn topology_name(topology: Topology) -> &'static str {
    match topology {
        Topology::Single => "single",
        Topology::VirtualWorkspace => "virtual_workspace",
        Topology::ProcMacro => "proc_macro",
        Topology::BuildScript => "build_script",
        Topology::Workspace => "workspace",
    }
}
