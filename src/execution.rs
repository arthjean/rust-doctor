use std::ffi::OsString;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use cargo_metadata::{Message, Metadata};
use serde::Deserialize;
use serde_json::Value;

use crate::cargo_health::{self, CargoHealthScan};
use crate::configuration::{self, WorkspaceConfiguration};
use crate::policy::{PolicyPlan, Producer};
use crate::scan_target::{self, ResolvedScanTarget};
use crate::source_kernel::{self, SourceScan};

mod baseline;
mod clippy;

pub(crate) use baseline::{BaselineExecution, execute as execute_baseline};
#[cfg(test)]
pub(crate) use clippy::arguments_for_rules as clippy_arguments_for_rules;
use clippy::{ClippyExecution, arguments_for_plan as clippy_arguments_for_plan};

#[derive(Debug)]
pub(crate) struct ExecutionResult {
    pub(crate) manifest_path: Option<PathBuf>,
    pub(crate) metadata: Option<Metadata>,
    pub(crate) toolchain: ToolchainProvenance,
    pub(crate) scan: ClippyExecution,
    pub(crate) source: Option<SourceScan>,
    pub(crate) cargo_health: Option<CargoHealthScan>,
    pub(crate) error: Option<InternalError>,
}

impl ExecutionResult {
    fn new(manifest_path: Option<PathBuf>) -> Self {
        Self {
            manifest_path,
            metadata: None,
            toolchain: ToolchainProvenance::default(),
            scan: ClippyExecution::NotRun,
            source: None,
            cargo_health: None,
            error: None,
        }
    }

    fn fail(&mut self, error: InternalError) {
        self.error = Some(error);
    }

    pub(crate) fn is_complete(&self) -> bool {
        self.error.is_none()
            && self.scan.is_complete()
            && self
                .source
                .as_ref()
                .is_none_or(|source| source.errors.is_empty())
    }
}

#[derive(Debug, Clone, Default)]
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
    /// Target kind as Cargo declares it: `lib`, `bin`, `test`, `bench`,
    /// `example`, `custom-build`. The build system answers, not a path
    /// heuristic, and it stays exact where `target.test` does not: under
    /// `--all-targets`, Cargo marks `test` true even on a binary with no test
    /// at all, while `kind` keeps separating what the project ships from what
    /// it compiles to check itself.
    #[serde(default)]
    pub(crate) kind: Vec<String>,
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
    pub(crate) fn new(stage: &'static str, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            stage,
            code,
            message: message.into(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct PreparedInspection {
    target: ResolvedScanTarget,
    pub(crate) configuration: WorkspaceConfiguration,
}

impl PreparedInspection {
    pub(crate) fn workspace_root(&self) -> &Path {
        self.target.workspace_root()
    }

    pub(crate) fn fail(self, error: InternalError) -> ExecutionResult {
        let mut result = ExecutionResult::new(Some(self.target.manifest_path));
        result.metadata = Some(self.target.metadata);
        result.fail(error);
        result
    }
}

#[derive(Debug)]
struct Programs {
    cargo: PathBuf,
    rustc: PathBuf,
}

#[derive(Debug, Clone, Default)]
struct CommandEnvironment {
    rustup_toolchain: Option<OsString>,
}

impl CommandEnvironment {
    fn apply(&self, command: &mut Command) {
        if let Some(toolchain) = self.rustup_toolchain.as_ref() {
            command.env("RUSTUP_TOOLCHAIN", toolchain);
        }
    }
}

impl Default for Programs {
    fn default() -> Self {
        Self {
            cargo: PathBuf::from("cargo"),
            rustc: PathBuf::from("rustc"),
        }
    }
}

pub(crate) fn prepare(path: &Path) -> Result<PreparedInspection, Box<ExecutionResult>> {
    prepare_with(path, &Programs::default())
}

fn prepare_with(
    path: &Path,
    programs: &Programs,
) -> Result<PreparedInspection, Box<ExecutionResult>> {
    let target = match scan_target::resolve(path, &programs.cargo) {
        Ok(target) => target,
        Err(failure) => {
            let mut result = ExecutionResult::new(failure.manifest_path);
            result.fail(failure.error);
            return Err(Box::new(result));
        }
    };
    let configuration = match configuration::load(&target) {
        Ok(configuration) => configuration,
        Err(error) => {
            let mut result = ExecutionResult::new(Some(target.manifest_path));
            result.metadata = Some(target.metadata);
            result.fail(error);
            return Err(Box::new(result));
        }
    };

    Ok(PreparedInspection {
        target,
        configuration,
    })
}

pub(crate) fn execute(prepared: PreparedInspection, plan: &PolicyPlan) -> ExecutionResult {
    execute_with_plan(prepared, &Programs::default(), plan)
}

fn execute_with_plan(
    prepared: PreparedInspection,
    programs: &Programs,
    plan: &PolicyPlan,
) -> ExecutionResult {
    let environment = CommandEnvironment::default();
    let toolchain = match resolve_toolchain(programs, prepared.workspace_root(), &environment) {
        Ok(toolchain) => toolchain,
        Err(error) => return prepared.fail(error),
    };
    execute_target(
        prepared.target,
        programs,
        plan,
        toolchain,
        None,
        &environment,
    )
}

fn resolve_toolchain(
    programs: &Programs,
    workspace_root: &Path,
    environment: &CommandEnvironment,
) -> Result<ToolchainProvenance, InternalError> {
    let cargo = tool_version(
        &programs.cargo,
        &["--version"],
        workspace_root,
        "cargo-unavailable",
        "Cargo",
        environment,
    )?;
    let rustc = tool_version(
        &programs.rustc,
        &["--version"],
        workspace_root,
        "rustc-unavailable",
        "rustc",
        environment,
    )?;
    let clippy = tool_version(
        &programs.cargo,
        &["clippy", "--version"],
        workspace_root,
        "clippy-unavailable",
        "Clippy",
        environment,
    )?;
    Ok(ToolchainProvenance {
        cargo: Some(cargo),
        rustc: Some(rustc),
        clippy: Some(clippy),
    })
}

fn execute_target(
    target: ResolvedScanTarget,
    programs: &Programs,
    plan: &PolicyPlan,
    toolchain: ToolchainProvenance,
    target_dir: Option<&Path>,
    environment: &CommandEnvironment,
) -> ExecutionResult {
    let ResolvedScanTarget {
        manifest_path,
        metadata,
    } = target;
    let workspace_root = metadata.workspace_root.clone();
    let mut result = ExecutionResult::new(Some(manifest_path));
    result.metadata = Some(metadata);
    result.toolchain = toolchain;

    // The dependency pack reads the resolved graph from disk, so it produces
    // bounded errors like the source kernel: it runs here, not during
    // normalization, so that its errors join those of the scan.
    //
    // It runs before Clippy: `cargo clippy` creates or rewrites `Cargo.lock`,
    // so reading it afterwards would measure the graph the tool just wrote
    // instead of the one the repository commits.
    if plan.active_rules(Producer::CargoHealth).next().is_some() {
        result.cargo_health = result
            .metadata
            .as_ref()
            .map(|metadata| cargo_health::inspect(metadata, plan));
    }
    if plan.active_rules(Producer::Clippy).next().is_none() {
        result.scan = ClippyExecution::Disabled;
    } else {
        match run_clippy(
            &programs.cargo,
            workspace_root.as_std_path(),
            plan,
            target_dir,
            environment,
        ) {
            Ok(scan) => result.scan = ClippyExecution::Finished(scan),
            Err(error) => result.fail(error),
        }
    }
    if result.error.is_none() && plan.active_rules(Producer::SourceKernel).next().is_some() {
        result.source = result
            .metadata
            .as_ref()
            .map(|metadata| source_kernel::inspect(metadata, plan));
    }

    result
}

fn tool_version(
    program: &Path,
    arguments: &[&str],
    working_directory: &Path,
    code: &'static str,
    label: &str,
    environment: &CommandEnvironment,
) -> Result<String, InternalError> {
    let output = version_command(program, arguments, working_directory, environment)
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

fn version_command(
    program: &Path,
    arguments: &[&str],
    working_directory: &Path,
    environment: &CommandEnvironment,
) -> Command {
    let mut command = Command::new(program);
    command
        .args(arguments)
        .current_dir(working_directory)
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    environment.apply(&mut command);
    command
}

fn run_clippy(
    cargo: &Path,
    workspace_root: &Path,
    plan: &PolicyPlan,
    target_dir: Option<&Path>,
    environment: &CommandEnvironment,
) -> Result<ScanExecution, InternalError> {
    let arguments = clippy_arguments_for_plan(plan);
    let mut child = clippy_command(cargo, workspace_root, &arguments, target_dir, environment)
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

fn clippy_command(
    cargo: &Path,
    workspace_root: &Path,
    arguments: &[&str],
    target_dir: Option<&Path>,
    environment: &CommandEnvironment,
) -> Command {
    let mut command = Command::new(cargo);
    command
        .args(arguments)
        .current_dir(workspace_root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(target_dir) = target_dir {
        command.env("CARGO_TARGET_DIR", target_dir);
    }
    environment.apply(&mut command);
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
    use std::ffi::OsStr;
    use std::io::Cursor;

    use super::*;

    fn execute(path: &Path) -> ExecutionResult {
        let prepared = match super::prepare(path) {
            Ok(prepared) => prepared,
            Err(result) => return *result,
        };
        super::execute(prepared, &PolicyPlan::default())
    }

    fn execute_with(path: &Path, programs: &Programs) -> ExecutionResult {
        let prepared = match prepare_with(path, programs) {
            Ok(prepared) => prepared,
            Err(result) => return *result,
        };
        execute_with_plan(prepared, programs, &PolicyPlan::default())
    }

    fn clippy_arguments() -> Vec<&'static str> {
        clippy_arguments_for_plan(&PolicyPlan::default())
    }

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
    fn missing_manifest_returns_structured_failure_without_scan() {
        let result = execute(Path::new("/definitely/missing/rust-doctor-fixture"));

        assert_eq!(
            result.error.as_ref().map(|error| (error.stage, error.code)),
            Some(("discovery", "no-manifest"))
        );
        assert!(result.metadata.is_none());
        assert!(result.scan.finished().is_none());
    }

    #[test]
    fn version_command_uses_the_requested_working_directory() {
        let workspace = fixture("clean").canonicalize().unwrap();
        let command = version_command(
            Path::new("cargo"),
            &["--version"],
            &workspace,
            &CommandEnvironment::default(),
        );

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
        let command = clippy_command(
            Path::new("cargo"),
            &workspace,
            &arguments,
            None,
            &CommandEnvironment::default(),
        );
        let arguments: Vec<_> = command.get_args().collect();

        assert_eq!(command.get_program(), OsStr::new("cargo"));
        // The catalog is the only source of the list: the command must carry
        // every active Clippy rule, in catalog order, under `-W`.
        let expected: Vec<String> = [
            "clippy",
            "--workspace",
            "--no-deps",
            "--message-format=json",
            "--",
            "-A",
            "clippy::all",
        ]
        .into_iter()
        .map(str::to_owned)
        .chain(
            crate::policy::CATALOG
                .iter()
                .filter(|definition| definition.producer == Producer::Clippy)
                .flat_map(|definition| ["-W".to_owned(), definition.id.to_owned()]),
        )
        .collect();
        assert_eq!(expected.len(), 7 + 2 * 36);
        assert_eq!(
            arguments,
            expected
                .iter()
                .map(|argument| OsStr::new(argument.as_str()))
                .collect::<Vec<_>>()
        );
        for forbidden in ["clippy::restriction", "--force-warn", "-D"] {
            assert!(!arguments.contains(&OsStr::new(forbidden)));
        }
        // `clippy::all` appears once, and only to be switched off: raising the
        // whole group would flood the report with rules the catalog cannot
        // explain.
        assert_eq!(
            arguments
                .windows(2)
                .filter(|pair| pair[1] == OsStr::new("clippy::all"))
                .map(|pair| pair[0])
                .collect::<Vec<_>>(),
            [OsStr::new("-A")]
        );
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
    fn cargo_spawn_failure_is_classified_before_versions_or_scan() {
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
        assert!(result.scan.finished().is_none());
    }

    #[test]
    fn nonzero_clippy_preflight_is_classified() {
        let error = tool_version(
            Path::new("/bin/false"),
            &["clippy", "--version"],
            &fixture("clean"),
            "clippy-unavailable",
            "Clippy",
            &CommandEnvironment::default(),
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
        let scan = result.scan.into_finished().unwrap();
        assert_eq!(
            scan.command,
            std::iter::once("cargo".to_owned())
                .chain(
                    clippy_arguments_for_plan(&PolicyPlan::default())
                        .into_iter()
                        .map(str::to_owned)
                )
                .collect::<Vec<_>>()
        );
        assert_eq!(scan.command.len(), 1 + 7 + 2 * 36);
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
        let scan = result.scan.into_finished().unwrap();
        assert_eq!(scan.exit_code, Some(101));
        assert_eq!(scan.exit_success, Some(false));
        assert_eq!(scan.build_finished, Some(false));
        assert!(compiler_message_count(&scan) >= 1);
    }
}
