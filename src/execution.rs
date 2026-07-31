use std::ffi::OsStr;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use cargo_metadata::{Message, Metadata, MetadataCommand};
use serde::Deserialize;
use serde_json::Value;

use crate::rules::RULES;

const CLIPPY_BASE_ARGS: [&str; 5] = [
    "clippy",
    "--workspace",
    "--all-targets",
    "--no-deps",
    "--message-format=json",
];

#[derive(Debug)]
pub(crate) struct ExecutionResult {
    pub(crate) manifest_path: Option<PathBuf>,
    pub(crate) metadata: Option<Metadata>,
    pub(crate) toolchain: ToolchainProvenance,
    pub(crate) scan: Option<ScanExecution>,
    pub(crate) error: Option<InternalError>,
}

impl ExecutionResult {
    fn new(manifest_path: Option<PathBuf>) -> Self {
        Self {
            manifest_path,
            metadata: None,
            toolchain: ToolchainProvenance::default(),
            scan: None,
            error: None,
        }
    }

    fn fail(&mut self, error: InternalError) {
        self.error = Some(error);
    }
}

#[derive(Debug, Default)]
pub(crate) struct ToolchainProvenance {
    pub(crate) cargo: Option<String>,
    pub(crate) rustc: Option<String>,
    pub(crate) clippy: Option<String>,
}

#[derive(Debug)]
pub(crate) struct ScanExecution {
    pub(crate) command: Vec<String>,
    pub(crate) exit_code: Option<i32>,
    pub(crate) exit_success: Option<bool>,
    pub(crate) build_finished: Option<bool>,
    pub(crate) noise_lines: usize,
    pub(crate) malformed_messages: usize,
    pub(crate) messages: Vec<CapturedMessage>,
    pub(crate) errors: Vec<InternalError>,
}

#[derive(Debug)]
pub(crate) enum CapturedMessage {
    Compiler(CompilerMessageData),
    Known(Box<Message>),
    Unknown(Value),
}

#[derive(Debug, Deserialize)]
pub(crate) struct CompilerMessageData {
    pub(crate) package_id: String,
    pub(crate) target: CapturedTarget,
    pub(crate) message: CapturedDiagnostic,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CapturedTarget {
    pub(crate) name: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CapturedDiagnostic {
    pub(crate) message: String,
    pub(crate) code: Option<CapturedDiagnosticCode>,
    pub(crate) level: String,
    #[serde(default)]
    pub(crate) spans: Vec<CapturedSpan>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CapturedDiagnosticCode {
    pub(crate) code: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CapturedSpan {
    pub(crate) file_name: String,
    pub(crate) line_start: usize,
    pub(crate) line_end: usize,
    pub(crate) column_start: usize,
    pub(crate) column_end: usize,
    pub(crate) is_primary: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct InternalError {
    pub(crate) stage: &'static str,
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

impl InternalError {
    fn new(stage: &'static str, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            stage,
            code,
            message: message.into(),
        }
    }
}

#[derive(Debug)]
struct Programs {
    cargo: PathBuf,
    rustc: PathBuf,
}

impl Default for Programs {
    fn default() -> Self {
        Self {
            cargo: PathBuf::from("cargo"),
            rustc: PathBuf::from("rustc"),
        }
    }
}

pub(crate) fn execute(path: &Path) -> ExecutionResult {
    execute_with(path, &Programs::default())
}

fn execute_with(path: &Path, programs: &Programs) -> ExecutionResult {
    let manifest_path = match discover_manifest(path) {
        Ok(manifest_path) => manifest_path,
        Err(error) => {
            let mut result = ExecutionResult::new(None);
            result.fail(error);
            return result;
        }
    };
    let mut result = ExecutionResult::new(Some(manifest_path.clone()));
    let Some(manifest_directory) = manifest_path.parent() else {
        result.fail(no_manifest_error(
            &manifest_path,
            "manifest has no parent directory",
        ));
        return result;
    };

    let cargo_version = match tool_version(
        &programs.cargo,
        &["--version"],
        manifest_directory,
        "cargo-unavailable",
        "Cargo",
    ) {
        Ok(version) => version,
        Err(error) => {
            result.fail(error);
            return result;
        }
    };
    result.toolchain.cargo = Some(cargo_version);

    let metadata = match load_metadata(&manifest_path, manifest_directory, &programs.cargo) {
        Ok(metadata) => metadata,
        Err(error) => {
            result.fail(error);
            return result;
        }
    };
    let workspace_root = metadata.workspace_root.clone();
    result.metadata = Some(metadata);

    let rustc_version = match tool_version(
        &programs.rustc,
        &["--version"],
        workspace_root.as_std_path(),
        "rustc-unavailable",
        "rustc",
    ) {
        Ok(version) => version,
        Err(error) => {
            result.fail(error);
            return result;
        }
    };
    result.toolchain.rustc = Some(rustc_version);

    let clippy_version = match tool_version(
        &programs.cargo,
        &["clippy", "--version"],
        workspace_root.as_std_path(),
        "clippy-unavailable",
        "Clippy",
    ) {
        Ok(version) => version,
        Err(error) => {
            result.fail(error);
            return result;
        }
    };
    result.toolchain.clippy = Some(clippy_version);

    match run_clippy(&programs.cargo, workspace_root.as_std_path()) {
        Ok(scan) => result.scan = Some(scan),
        Err(error) => result.fail(error),
    }

    result
}

fn discover_manifest(path: &Path) -> Result<PathBuf, InternalError> {
    if path.file_name() == Some(OsStr::new("Cargo.toml")) {
        return resolve_manifest_boundary(path);
    }

    if !path.is_dir() {
        return Err(no_manifest_error(path, "path is not a directory"));
    }

    let directory = path.canonicalize().map_err(|error| {
        no_manifest_error(path, format!("could not resolve directory: {error}"))
    })?;
    for ancestor in directory.ancestors() {
        let candidate = ancestor.join("Cargo.toml");
        match candidate.symlink_metadata() {
            Ok(_) => return resolve_manifest_boundary(&candidate),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(invalid_manifest_error(
                    &candidate,
                    format!("could not inspect manifest path: {error}"),
                ));
            }
        }
    }

    Err(no_manifest_error(
        path,
        "no Cargo.toml found in this path or its ancestors",
    ))
}

fn resolve_manifest_boundary(path: &Path) -> Result<PathBuf, InternalError> {
    let resolved = path.canonicalize().map_err(|error| {
        invalid_manifest_error(path, format!("could not resolve manifest path: {error}"))
    })?;
    let metadata = resolved.metadata().map_err(|error| {
        invalid_manifest_error(
            path,
            format!("could not inspect resolved manifest: {error}"),
        )
    })?;
    if !metadata.is_file() {
        return Err(invalid_manifest_error(
            path,
            "manifest path does not resolve to a regular file",
        ));
    }

    Ok(resolved)
}

fn no_manifest_error(path: &Path, detail: impl AsRef<str>) -> InternalError {
    InternalError::new(
        "discovery",
        "no-manifest",
        format!("{}: {}", detail.as_ref(), path.display()),
    )
}

fn invalid_manifest_error(path: &Path, detail: impl AsRef<str>) -> InternalError {
    InternalError::new(
        "discovery",
        "invalid-manifest",
        format!("{}: {}", detail.as_ref(), path.display()),
    )
}

fn load_metadata(
    manifest_path: &Path,
    manifest_directory: &Path,
    cargo: &Path,
) -> Result<Metadata, InternalError> {
    metadata_command(cargo, manifest_path, manifest_directory)
        .exec()
        .map_err(|error| {
            let detail = if matches!(&error, cargo_metadata::Error::CargoMetadata { .. }) {
                "cargo metadata exited with an error".to_owned()
            } else {
                error.to_string()
            };
            InternalError::new(
                "metadata",
                "cargo-metadata",
                format!("cargo metadata failed: {detail}"),
            )
        })
}

fn metadata_command(
    cargo: &Path,
    manifest_path: &Path,
    manifest_directory: &Path,
) -> MetadataCommand {
    let mut command = MetadataCommand::new();
    command
        .cargo_path(cargo)
        .manifest_path(manifest_path)
        .current_dir(manifest_directory)
        .no_deps();
    command
}

fn tool_version(
    program: &Path,
    arguments: &[&str],
    working_directory: &Path,
    code: &'static str,
    label: &str,
) -> Result<String, InternalError> {
    let output = version_command(program, arguments, working_directory)
        .output()
        .map_err(|error| {
            InternalError::new(
                "execution",
                code,
                format!("{label} could not be started: {error}"),
            )
        })?;

    if !output.status.success() {
        return Err(InternalError::new(
            "execution",
            code,
            format!("{label} exited with status {}", output.status),
        ));
    }

    let version = String::from_utf8(output.stdout).map_err(|error| {
        InternalError::new(
            "execution",
            code,
            format!("{label} returned non-UTF-8 version output: {error}"),
        )
    })?;
    let version = version.trim();
    if version.is_empty() {
        return Err(InternalError::new(
            "execution",
            code,
            format!("{label} returned an empty version"),
        ));
    }

    Ok(version.to_owned())
}

fn version_command(program: &Path, arguments: &[&str], working_directory: &Path) -> Command {
    let mut command = Command::new(program);
    command
        .args(arguments)
        .current_dir(working_directory)
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    command
}

fn run_clippy(cargo: &Path, workspace_root: &Path) -> Result<ScanExecution, InternalError> {
    let arguments = clippy_arguments();
    let mut child = clippy_command(cargo, workspace_root, &arguments)
        .spawn()
        .map_err(|error| {
            InternalError::new(
                "execution",
                "clippy-start-failed",
                format!("Clippy could not be started: {error}"),
            )
        })?;

    let Some(stdout) = child.stdout.take() else {
        let _ = child.wait();
        return Err(InternalError::new(
            "execution",
            "clippy-stdout-unavailable",
            "Clippy started without a readable stdout pipe",
        ));
    };

    let mut stream = collect_messages(BufReader::new(stdout));
    let (exit_code, exit_success) = match child.wait() {
        Ok(status) => (status.code(), Some(status.success())),
        Err(error) => {
            stream.errors.push(InternalError::new(
                "execution",
                "clippy-wait-failed",
                format!("could not collect Clippy exit status: {error}"),
            ));
            (None, None)
        }
    };

    Ok(ScanExecution {
        command: std::iter::once("cargo")
            .chain(arguments)
            .map(str::to_owned)
            .collect(),
        exit_code,
        exit_success,
        build_finished: stream.build_finished,
        noise_lines: stream.noise_lines,
        malformed_messages: stream.malformed_messages,
        messages: stream.messages,
        errors: stream.errors,
    })
}

fn clippy_arguments() -> Vec<&'static str> {
    let mut arguments = Vec::with_capacity(CLIPPY_BASE_ARGS.len() + 1 + RULES.len() * 2);
    arguments.extend(CLIPPY_BASE_ARGS);
    arguments.push("--");
    for rule in RULES {
        arguments.extend([rule.activation.flag(), rule.code]);
    }
    arguments
}

fn clippy_command(cargo: &Path, workspace_root: &Path, arguments: &[&str]) -> Command {
    let mut command = Command::new(cargo);
    command
        .args(arguments)
        .current_dir(workspace_root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    command
}

#[derive(Debug, Default)]
struct CollectedMessages {
    build_finished: Option<bool>,
    noise_lines: usize,
    malformed_messages: usize,
    messages: Vec<CapturedMessage>,
    errors: Vec<InternalError>,
}

fn collect_messages<R: BufRead>(mut reader: R) -> CollectedMessages {
    let mut collected = CollectedMessages::default();
    let mut line = String::new();

    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                let record = line.strip_suffix('\n').unwrap_or(&line);
                let record = record.strip_suffix('\r').unwrap_or(record);
                let normalized =
                    record.trim_start_matches(|character: char| character.is_ascii_whitespace());
                if normalized.is_empty() {
                    if !record.is_empty() {
                        collected.noise_lines += 1;
                    }
                    continue;
                } else if normalized.starts_with('{') {
                    capture_json_line(normalized, &mut collected);
                } else if has_contaminated_cargo_suffix(normalized) {
                    collected.malformed_messages += 1;
                } else {
                    collected.noise_lines += 1;
                }
            }
            Err(error) => {
                collected.errors.push(InternalError::new(
                    "parsing",
                    "stdout-read",
                    format!("could not read Clippy stdout: {error}"),
                ));
                break;
            }
        }
    }

    collected
}

fn has_contaminated_cargo_suffix(line: &str) -> bool {
    line.char_indices()
        .filter(|(_, character)| *character == '{')
        .any(|(index, _)| {
            serde_json::from_str::<Value>(&line[index..])
                .ok()
                .is_some_and(|value| value.get("reason").and_then(Value::as_str).is_some())
        })
}

fn capture_json_line(line: &str, collected: &mut CollectedMessages) {
    let value = match serde_json::from_str::<Value>(line) {
        Ok(value) => value,
        Err(_) => {
            collected.malformed_messages += 1;
            return;
        }
    };

    let reason = value.get("reason").and_then(Value::as_str);
    if reason == Some("compiler-message") {
        match serde_json::from_value::<CompilerMessageData>(value) {
            Ok(message) => collected.messages.push(CapturedMessage::Compiler(message)),
            Err(_) => collected.malformed_messages += 1,
        }
        return;
    }

    let known_reason = matches!(
        reason,
        Some("compiler-artifact" | "build-script-executed" | "build-finished")
    );
    if !known_reason {
        if reason.is_some() {
            collected.messages.push(CapturedMessage::Unknown(value));
        } else {
            collected.malformed_messages += 1;
        }
        return;
    }

    match serde_json::from_value::<Message>(value) {
        Ok(message) => {
            if let Message::BuildFinished(finished) = &message {
                collected.build_finished = Some(finished.success);
            }
            collected
                .messages
                .push(CapturedMessage::Known(Box::new(message)));
        }
        Err(_) => collected.malformed_messages += 1,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Cursor;
    #[cfg(unix)]
    use std::os::unix::fs::symlink;

    use super::*;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/projects")
            .join(name)
    }

    fn compiler_message_count(scan: &ScanExecution) -> usize {
        scan.messages
            .iter()
            .filter(|message| matches!(message, CapturedMessage::Compiler(_)))
            .count()
    }

    #[test]
    fn discovers_directory_and_direct_manifest() {
        let project = fixture("clean");
        let expected = project.join("Cargo.toml").canonicalize().unwrap();

        assert_eq!(discover_manifest(&project).unwrap(), expected);
        assert_eq!(
            discover_manifest(&project.join("Cargo.toml")).unwrap(),
            expected
        );
        assert_eq!(discover_manifest(&project.join("src")).unwrap(), expected);
    }

    #[cfg(unix)]
    #[test]
    fn invalid_manifest_boundary_stops_discovery_before_processes_start() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!("discovery-boundary-{}", std::process::id()));
        if root.exists() {
            fs::remove_dir_all(&root).unwrap();
        }
        let nested = root.join("nested");
        fs::create_dir_all(nested.join("Cargo.toml")).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"ancestor\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();

        let directory_error = discover_manifest(&nested).unwrap_err();
        assert_eq!(
            (directory_error.stage, directory_error.code),
            ("discovery", "invalid-manifest")
        );

        let programs = Programs {
            cargo: PathBuf::from("/definitely/missing/rust-doctor-cargo"),
            rustc: PathBuf::from("/definitely/missing/rust-doctor-rustc"),
        };
        let result = execute_with(&nested, &programs);
        assert_eq!(
            result.error.as_ref().map(|error| (error.stage, error.code)),
            Some(("discovery", "invalid-manifest"))
        );
        assert!(result.metadata.is_none());
        assert!(result.scan.is_none());
        assert!(result.toolchain.cargo.is_none());

        fs::remove_dir_all(nested.join("Cargo.toml")).unwrap();
        symlink("missing-target", nested.join("Cargo.toml")).unwrap();
        let symlink_error = discover_manifest(&nested).unwrap_err();
        assert_eq!(
            (symlink_error.stage, symlink_error.code),
            ("discovery", "invalid-manifest")
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_manifest_returns_structured_failure_without_scan() {
        let result = execute(Path::new("/definitely/missing/rust-doctor-fixture"));

        assert_eq!(
            result.error.as_ref().map(|error| (error.stage, error.code)),
            Some(("discovery", "no-manifest"))
        );
        assert!(result.metadata.is_none());
        assert!(result.scan.is_none());
    }

    #[test]
    fn metadata_command_uses_the_versioned_no_deps_contract() {
        let manifest = fixture("clean").join("Cargo.toml");
        let directory = manifest.parent().unwrap();
        let command = metadata_command(Path::new("cargo"), &manifest, directory).cargo_command();
        let arguments: Vec<_> = command.get_args().collect();

        assert_eq!(command.get_program(), OsStr::new("cargo"));
        assert_eq!(
            arguments,
            [
                OsStr::new("metadata"),
                OsStr::new("--format-version"),
                OsStr::new("1"),
                OsStr::new("--no-deps"),
                OsStr::new("--manifest-path"),
                manifest.as_os_str(),
            ]
        );
        assert_eq!(command.get_current_dir(), Some(directory));
    }

    #[test]
    fn version_command_uses_the_requested_working_directory() {
        let workspace = fixture("clean").canonicalize().unwrap();
        let command = version_command(Path::new("cargo"), &["--version"], &workspace);

        assert_eq!(command.get_program(), OsStr::new("cargo"));
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [OsStr::new("--version")]
        );
        assert_eq!(command.get_current_dir(), Some(workspace.as_path()));
    }

    #[test]
    fn clippy_command_has_exact_arguments_and_workspace() {
        let workspace = fixture("clean").canonicalize().unwrap();
        let arguments = clippy_arguments();
        let command = clippy_command(Path::new("cargo"), &workspace, &arguments);
        let arguments: Vec<_> = command.get_args().collect();

        assert_eq!(command.get_program(), OsStr::new("cargo"));
        assert_eq!(
            arguments,
            [
                "clippy",
                "--workspace",
                "--all-targets",
                "--no-deps",
                "--message-format=json",
                "--",
                "-W",
                "clippy::dbg_macro",
                "-W",
                "clippy::todo",
                "-W",
                "clippy::unimplemented",
            ]
            .map(OsStr::new)
        );
        for forbidden in ["clippy::restriction", "clippy::all", "--force-warn", "-D"] {
            assert!(!arguments.contains(&OsStr::new(forbidden)));
        }
        assert_eq!(
            arguments
                .iter()
                .filter(|argument| **argument == OsStr::new("--"))
                .count(),
            1
        );
        assert_eq!(command.get_current_dir(), Some(workspace.as_path()));
    }

    #[test]
    fn cargo_spawn_failure_is_classified_before_metadata_or_scan() {
        let programs = Programs {
            cargo: PathBuf::from("/definitely/missing/rust-doctor-cargo"),
            rustc: PathBuf::from("rustc"),
        };
        let result = execute_with(&fixture("clean"), &programs);

        assert_eq!(
            result.error.as_ref().map(|error| (error.stage, error.code)),
            Some(("execution", "cargo-unavailable"))
        );
        assert!(result.metadata.is_none());
        assert!(result.scan.is_none());
    }

    #[test]
    fn nonzero_clippy_preflight_is_classified() {
        let error = tool_version(
            Path::new("/bin/false"),
            &["clippy", "--version"],
            &fixture("clean"),
            "clippy-unavailable",
            "Clippy",
        )
        .unwrap_err();

        assert_eq!(
            (error.stage, error.code),
            ("execution", "clippy-unavailable")
        );
    }

    #[test]
    fn parser_distinguishes_noise_malformed_and_future_messages() {
        let input = concat!(
            "third-party output\n",
            "{\"reason\":\"build-finished\",\"success\":true}\n",
            "{\"reason\":\"future-message\",\"value\":1}\n",
            "{\"reason\":\n",
        );
        let collected = collect_messages(Cursor::new(input));

        assert_eq!(collected.noise_lines, 1);
        assert_eq!(collected.malformed_messages, 1);
        assert_eq!(collected.build_finished, Some(true));
        assert_eq!(collected.messages.len(), 2);
        assert!(matches!(
            &collected.messages[0],
            CapturedMessage::Known(message)
                if matches!(message.as_ref(), Message::BuildFinished(_))
        ));
        assert!(matches!(collected.messages[1], CapturedMessage::Unknown(_)));
    }

    #[test]
    fn parser_accepts_ascii_whitespace_before_json() {
        let message = "{\"reason\":\"build-finished\",\"success\":true}\n";
        let prefixes = [' ', '\t', '\n', '\r', '\u{000c}'];
        assert!(prefixes.iter().all(char::is_ascii_whitespace));
        let input: String = prefixes
            .into_iter()
            .map(|prefix| format!("{prefix}{message}"))
            .collect();
        let collected = collect_messages(Cursor::new(input));

        assert_eq!(collected.noise_lines, 0);
        assert_eq!(collected.malformed_messages, 0);
        assert_eq!(collected.build_finished, Some(true));
        assert_eq!(collected.messages.len(), prefixes.len());
    }

    #[test]
    fn parser_counts_nonempty_whitespace_records_as_noise() {
        let input = "\n \t\u{000c}\r\n{\"reason\":\"build-finished\",\"success\":true}\n";
        let collected = collect_messages(Cursor::new(input));

        assert_eq!(collected.noise_lines, 1);
        assert_eq!(collected.malformed_messages, 0);
        assert_eq!(collected.build_finished, Some(true));
    }

    #[test]
    fn parser_rejects_contaminated_json_boundary_without_losing_neighbors() {
        let compiler_message = concat!(
            "{\"reason\":\"compiler-message\",",
            "\"package_id\":\"path+file:///project#example@0.1.0\",",
            "\"target\":{\"name\":\"example\"},",
            "\"message\":{\"message\":\"diagnostic\",",
            "\"code\":null,\"level\":\"warning\",\"spans\":[]}}\n",
        );
        let input = format!(
            "{compiler_message}third-party prefix{compiler_message}\
             {{\"reason\":\"build-finished\",\"success\":true}}\n"
        );
        let collected = collect_messages(Cursor::new(input));

        assert_eq!(collected.noise_lines, 0);
        assert_eq!(collected.malformed_messages, 1);
        assert_eq!(collected.build_finished, Some(true));
        assert_eq!(
            collected
                .messages
                .iter()
                .filter(|message| matches!(message, CapturedMessage::Compiler(_)))
                .count(),
            1
        );
    }

    #[test]
    fn parser_preserves_future_diagnostic_severity() {
        let input = concat!(
            "{\"reason\":\"compiler-message\",",
            "\"package_id\":\"path+file:///project#example@0.1.0\",",
            "\"target\":{\"name\":\"example\"},",
            "\"message\":{\"message\":\"future diagnostic\",",
            "\"code\":null,\"level\":\"future-level\",\"spans\":[]}}\n",
        );
        let collected = collect_messages(Cursor::new(input));

        assert_eq!(collected.malformed_messages, 0);
        assert!(matches!(
            &collected.messages[0],
            CapturedMessage::Compiler(message) if message.message.level == "future-level"
        ));
    }

    #[test]
    fn virtual_workspace_metadata_is_preserved() {
        let result = execute(&fixture("virtual-workspace"));

        assert!(result.error.is_none(), "{:?}", result.error);
        let metadata = result.metadata.unwrap();
        assert!(metadata.resolve.is_none());
        assert_eq!(metadata.workspace_members.len(), 2);
        assert_eq!(metadata.packages.len(), 2);
        let member = metadata
            .packages
            .iter()
            .find(|package| package.name == "workspace-member")
            .unwrap();
        assert_eq!(member.targets[0].name, "workspace_member");
        let external = metadata
            .packages
            .iter()
            .find(|package| package.name == "shared")
            .unwrap();
        assert_eq!(external.targets[0].name, "shared");
        assert!(!external.manifest_path.starts_with(&metadata.workspace_root));
        assert!(
            metadata
                .workspace_root
                .as_std_path()
                .ends_with("virtual-workspace")
        );
    }

    #[test]
    fn valid_scan_collects_provenance_and_clippy_diagnostics() {
        let result = execute(&fixture("clippy-warning"));

        assert!(result.error.is_none(), "{:?}", result.error);
        assert!(
            result
                .toolchain
                .cargo
                .as_deref()
                .unwrap()
                .starts_with("cargo ")
        );
        assert!(
            result
                .toolchain
                .rustc
                .as_deref()
                .unwrap()
                .starts_with("rustc ")
        );
        assert!(
            result
                .toolchain
                .clippy
                .as_deref()
                .unwrap()
                .starts_with("clippy ")
        );
        let scan = result.scan.unwrap();
        assert_eq!(
            scan.command,
            [
                "cargo",
                "clippy",
                "--workspace",
                "--all-targets",
                "--no-deps",
                "--message-format=json",
                "--",
                "-W",
                "clippy::dbg_macro",
                "-W",
                "clippy::todo",
                "-W",
                "clippy::unimplemented",
            ]
        );
        assert_eq!(scan.exit_code, Some(0));
        assert_eq!(scan.exit_success, Some(true));
        assert_eq!(scan.build_finished, Some(true));
        assert_eq!(scan.noise_lines, 0);
        assert_eq!(scan.malformed_messages, 0);
        assert!(scan.errors.is_empty());
        assert!(compiler_message_count(&scan) >= 1);
    }

    #[test]
    fn nonzero_scan_preserves_prior_diagnostics() {
        let result = execute(&fixture("compile-error"));

        assert!(result.error.is_none(), "{:?}", result.error);
        let scan = result.scan.unwrap();
        assert_eq!(scan.exit_code, Some(101));
        assert_eq!(scan.exit_success, Some(false));
        assert_eq!(scan.build_finished, Some(false));
        assert!(compiler_message_count(&scan) >= 1);
    }
}
