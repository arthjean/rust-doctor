use std::ffi::OsString;
use std::fmt;
use std::io::{self, Read};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;

use serde::{Serialize, Serializer};

use crate::execution::InternalError;
use crate::workspace_path;

const OID_OUTPUT_LIMIT: usize = 4_096;
const STDERR_OUTPUT_LIMIT: usize = 65_536;
const DIFF_OUTPUT_LIMIT: usize = 1_048_576;
const SCOPE_OUTPUT_LIMIT: usize = 2_097_152;
const PATH_LIMIT: usize = 4_096;
const FILE_LIMIT: usize = 10_000;

const GIT_ENVIRONMENT_OVERRIDES: [&str; 10] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_CONFIG",
    "GIT_CONFIG_COUNT",
    "GIT_EXTERNAL_DIFF",
    "GIT_PAGER",
];

#[derive(Clone, PartialEq, Eq)]
pub(crate) enum ScopeRequest {
    Full,
    Files { base: String },
}

impl fmt::Debug for ScopeRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Full => formatter.write_str("Full"),
            Self::Files { .. } => formatter.write_str("Files { base: <redacted> }"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ScopeMode {
    Full,
    Files,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionScope {
    Workspace,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeReport {
    kind: ResolvedScope,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ResolvedScope {
    Full,
    Files {
        comparison_base: String,
        files: Vec<String>,
    },
}

#[derive(Serialize)]
struct SerializedScope<'a> {
    mode: ScopeMode,
    execution_scope: ExecutionScope,
    comparison_base: Option<&'a str>,
    files: Option<&'a [String]>,
}

impl ScopeReport {
    pub const fn mode(&self) -> ScopeMode {
        match self.kind {
            ResolvedScope::Full => ScopeMode::Full,
            ResolvedScope::Files { .. } => ScopeMode::Files,
        }
    }

    pub const fn execution_scope(&self) -> ExecutionScope {
        ExecutionScope::Workspace
    }

    pub fn comparison_base(&self) -> Option<&str> {
        match &self.kind {
            ResolvedScope::Full => None,
            ResolvedScope::Files {
                comparison_base, ..
            } => Some(comparison_base),
        }
    }

    pub fn files(&self) -> Option<&[String]> {
        match &self.kind {
            ResolvedScope::Full => None,
            ResolvedScope::Files { files, .. } => Some(files),
        }
    }

    pub(crate) fn files_details(&self) -> Option<(&str, &[String])> {
        match &self.kind {
            ResolvedScope::Full => None,
            ResolvedScope::Files {
                comparison_base,
                files,
            } => Some((comparison_base, files)),
        }
    }

    pub(crate) const fn full() -> Self {
        Self {
            kind: ResolvedScope::Full,
        }
    }

    #[cfg(test)]
    pub(crate) fn files_scope(comparison_base: String, mut files: Vec<String>) -> Self {
        files.sort();
        files.dedup();
        Self {
            kind: ResolvedScope::Files {
                comparison_base,
                files,
            },
        }
    }

    pub(crate) fn includes(&self, path: Option<&str>) -> bool {
        match (&self.kind, path) {
            (ResolvedScope::Full, _) => true,
            (ResolvedScope::Files { files, .. }, Some(path)) => files
                .binary_search_by(|candidate| candidate.as_str().cmp(path))
                .is_ok(),
            (ResolvedScope::Files { .. }, None) => false,
        }
    }
}

impl Serialize for ScopeReport {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        SerializedScope {
            mode: self.mode(),
            execution_scope: self.execution_scope(),
            comparison_base: self.comparison_base(),
            files: self.files(),
        }
        .serialize(serializer)
    }
}

#[derive(Debug)]
struct GitCall {
    arguments: Vec<OsString>,
    stdout_limit: usize,
    failure_code: &'static str,
    failure_message: &'static str,
}

#[derive(Debug)]
struct GitOutput {
    stdout: Vec<u8>,
}

#[derive(Debug)]
struct BoundedOutput {
    bytes: Vec<u8>,
    exceeded: bool,
}

pub(crate) fn validate(request: &ScopeRequest) -> Result<(), InternalError> {
    match request {
        ScopeRequest::Full => Ok(()),
        ScopeRequest::Files { base } if valid_base(base) => Ok(()),
        ScopeRequest::Files { .. } => Err(invalid_base()),
    }
}

pub(crate) fn resolve(
    request: &ScopeRequest,
    workspace_root: &Path,
) -> Result<ScopeReport, InternalError> {
    resolve_with(request, workspace_root, |call| {
        run_git(Path::new("git"), workspace_root, call)
    })
}

fn resolve_with(
    request: &ScopeRequest,
    workspace_root: &Path,
    mut run: impl FnMut(&GitCall) -> Result<GitOutput, InternalError>,
) -> Result<ScopeReport, InternalError> {
    let base = match request {
        ScopeRequest::Full => return Ok(ScopeReport::full()),
        ScopeRequest::Files { base } => {
            validate(request)?;
            base
        }
    };

    let revision = format!("{base}^{{commit}}");
    let base_output = run(&GitCall {
        arguments: git_arguments(
            workspace_root,
            [
                OsString::from("rev-parse"),
                OsString::from("--verify"),
                OsString::from("--quiet"),
                OsString::from("--end-of-options"),
                OsString::from(revision),
            ],
        ),
        stdout_limit: OID_OUTPUT_LIMIT,
        failure_code: "base-unavailable",
        failure_message: "Git base commit is unavailable.",
    })?;
    let base_commit = parse_single_oid(&base_output.stdout).ok_or_else(base_unavailable)?;

    let merge_output = run(&GitCall {
        arguments: git_arguments(
            workspace_root,
            [
                OsString::from("merge-base"),
                OsString::from("--all"),
                OsString::from(&base_commit),
                OsString::from("HEAD"),
            ],
        ),
        stdout_limit: OID_OUTPUT_LIMIT,
        failure_code: "merge-base-unavailable",
        failure_message: "Git merge base is unavailable.",
    })?;
    let merge_bases =
        parse_oids(&merge_output.stdout, base_commit.len()).ok_or_else(merge_base_unavailable)?;
    let comparison_base = match merge_bases.as_slice() {
        [] => return Err(merge_base_unavailable()),
        [comparison_base] => comparison_base.clone(),
        _ => {
            return Err(InternalError::new(
                "scope",
                "merge-base-ambiguous",
                "Git merge base is ambiguous.",
            ));
        }
    };

    let diff_output = run(&GitCall {
        arguments: git_arguments(
            workspace_root,
            [
                OsString::from("diff"),
                OsString::from("--no-ext-diff"),
                OsString::from("--no-renames"),
                OsString::from("--relative"),
                OsString::from("--name-only"),
                OsString::from("-z"),
                OsString::from("--diff-filter=ACMR"),
                OsString::from(&comparison_base),
                OsString::from("--"),
                OsString::from("."),
            ],
        ),
        stdout_limit: DIFF_OUTPUT_LIMIT,
        failure_code: "git-diff-failed",
        failure_message: "Git changed files could not be read.",
    })?;

    let scope = ScopeReport {
        kind: ResolvedScope::Files {
            comparison_base,
            files: parse_paths(&diff_output.stdout)?,
        },
    };
    ensure_scope_output_bound(&scope)?;
    Ok(scope)
}

fn git_arguments<const N: usize>(workspace_root: &Path, operation: [OsString; N]) -> Vec<OsString> {
    [
        OsString::from("-c"),
        OsString::from("color.ui=false"),
        OsString::from("-c"),
        OsString::from("core.fsmonitor=false"),
        OsString::from("--no-pager"),
        OsString::from("-C"),
        workspace_root.as_os_str().to_owned(),
    ]
    .into_iter()
    .chain(operation)
    .collect()
}

fn run_git(git: &Path, workspace_root: &Path, call: &GitCall) -> Result<GitOutput, InternalError> {
    let mut child = git_command(git, workspace_root, &call.arguments)
        .spawn()
        .map_err(|_| git_unavailable())?;
    let stdout = child.stdout.take().ok_or_else(git_unavailable)?;
    let stderr = child.stderr.take().ok_or_else(git_unavailable)?;
    let stdout_limit = call.stdout_limit;
    let stdout_reader = thread::spawn(move || collect_bounded(stdout, stdout_limit));
    let stderr_reader = thread::spawn(move || collect_bounded(stderr, STDERR_OUTPUT_LIMIT));
    let status = child
        .wait()
        .map_err(|_| phase_failure(call.failure_code, call.failure_message))?;
    let stdout = stdout_reader
        .join()
        .map_err(|_| phase_failure(call.failure_code, call.failure_message))?
        .map_err(|_| phase_failure(call.failure_code, call.failure_message))?;
    let stderr = stderr_reader
        .join()
        .map_err(|_| phase_failure(call.failure_code, call.failure_message))?
        .map_err(|_| phase_failure(call.failure_code, call.failure_message))?;

    if stdout.exceeded || stderr.exceeded {
        return Err(output_too_large());
    }
    if !status.success() {
        return Err(phase_failure(call.failure_code, call.failure_message));
    }

    Ok(GitOutput {
        stdout: stdout.bytes,
    })
}

fn git_command(git: &Path, workspace_root: &Path, arguments: &[OsString]) -> Command {
    let mut command = Command::new(git);
    command
        .args(arguments)
        .current_dir(workspace_root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("LC_ALL", "C");
    for variable in GIT_ENVIRONMENT_OVERRIDES {
        command.env_remove(variable);
    }
    command
}

fn collect_bounded(mut reader: impl Read, limit: usize) -> io::Result<BoundedOutput> {
    let mut bytes = Vec::new();
    let mut exceeded = false;
    let mut buffer = [0_u8; 8_192];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        if !exceeded {
            let remaining = limit.saturating_sub(bytes.len());
            let kept = remaining.min(read);
            bytes.extend_from_slice(&buffer[..kept]);
            exceeded = kept < read;
        }
    }
    Ok(BoundedOutput { bytes, exceeded })
}

fn ensure_scope_output_bound(scope: &ScopeReport) -> Result<(), InternalError> {
    match serde_json::to_vec(scope) {
        Ok(serialized) if serialized.len() < SCOPE_OUTPUT_LIMIT => Ok(()),
        Ok(_) | Err(_) => Err(output_too_large()),
    }
}

fn valid_base(base: &str) -> bool {
    let bytes = base.as_bytes();
    if matches!(bytes.len(), 40 | 64) && bytes.iter().all(u8::is_ascii_hexdigit) {
        return true;
    }
    if bytes.is_empty()
        || bytes.len() > 255
        || !base.is_ascii()
        || base.starts_with('-')
        || base.contains("..")
        || base.contains("//")
    {
        return false;
    }
    base.split('/').all(|component| {
        !component.is_empty()
            && !component.starts_with('.')
            && !component.ends_with('.')
            && !component.ends_with(".lock")
            && component
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
    })
}

fn parse_single_oid(output: &[u8]) -> Option<String> {
    let mut oids = parse_oids(output, 0)?;
    (oids.len() == 1).then(|| oids.remove(0))
}

fn parse_oids(output: &[u8], expected_length: usize) -> Option<Vec<String>> {
    let output = std::str::from_utf8(output).ok()?;
    output
        .split_ascii_whitespace()
        .map(|oid| {
            let valid_length = if expected_length == 0 {
                matches!(oid.len(), 40 | 64)
            } else {
                oid.len() == expected_length
            };
            (valid_length && oid.bytes().all(|byte| byte.is_ascii_hexdigit()))
                .then(|| oid.to_ascii_lowercase())
        })
        .collect()
}

fn parse_paths(output: &[u8]) -> Result<Vec<String>, InternalError> {
    if output.len() > DIFF_OUTPUT_LIMIT {
        return Err(output_too_large());
    }
    if output.is_empty() {
        return Ok(Vec::new());
    }
    if !output.ends_with(&[0]) {
        return Err(invalid_path());
    }

    let mut files = Vec::new();
    for entry in output[..output.len() - 1].split(|byte| *byte == 0) {
        if files.len() == FILE_LIMIT {
            return Err(InternalError::new(
                "scope",
                "too-many-files",
                "Git returned too many changed paths.",
            ));
        }
        if entry.is_empty() || entry.len() > PATH_LIMIT {
            return Err(invalid_path());
        }
        let path = std::str::from_utf8(entry).map_err(|_| invalid_path())?;
        files.push(workspace_path::normalize_changed(path).ok_or_else(invalid_path)?);
    }
    files.sort();
    files.dedup();
    Ok(files)
}

fn invalid_base() -> InternalError {
    InternalError::new("scope", "invalid-base", "Invalid Git base selector.")
}

fn git_unavailable() -> InternalError {
    InternalError::new("scope", "git-unavailable", "Git could not be started.")
}

fn base_unavailable() -> InternalError {
    phase_failure("base-unavailable", "Git base commit is unavailable.")
}

fn merge_base_unavailable() -> InternalError {
    phase_failure("merge-base-unavailable", "Git merge base is unavailable.")
}

fn output_too_large() -> InternalError {
    phase_failure(
        "git-output-too-large",
        "Git output exceeds the supported limit.",
    )
}

fn invalid_path() -> InternalError {
    phase_failure("git-path-invalid", "Git returned an invalid changed path.")
}

fn phase_failure(code: &'static str, message: &'static str) -> InternalError {
    InternalError::new("scope", code, message)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::{BTreeMap, VecDeque};
    use std::ffi::OsStr;
    #[cfg(unix)]
    use std::fs;
    use std::io::Cursor;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    #[cfg(unix)]
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    const BASE: &str = "1111111111111111111111111111111111111111";
    const MERGE_BASE: &str = "2222222222222222222222222222222222222222";
    #[cfg(unix)]
    static NEXT_SCRIPT: AtomicUsize = AtomicUsize::new(0);

    fn files(base: &str) -> ScopeRequest {
        ScopeRequest::Files {
            base: base.to_owned(),
        }
    }

    fn output(stdout: impl Into<Vec<u8>>) -> Result<GitOutput, InternalError> {
        Ok(GitOutput {
            stdout: stdout.into(),
        })
    }

    #[cfg(unix)]
    fn git_script(contents: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/git-scope-runner")
            .join(format!(
                "{}-{}",
                std::process::id(),
                NEXT_SCRIPT.fetch_add(1, Ordering::Relaxed)
            ));
        if directory.exists() {
            fs::remove_dir_all(&directory).unwrap();
        }
        fs::create_dir_all(&directory).unwrap();
        let script = directory.join("git");
        fs::write(&script, contents).unwrap();
        let mut permissions = fs::metadata(&script).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script, permissions).unwrap();
        (directory, script)
    }

    #[test]
    fn closed_base_grammar_accepts_only_named_selectors_and_full_oids() {
        for accepted in [
            "main",
            "release/1.2.3",
            "refs/remotes/origin/main",
            BASE,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ] {
            assert!(validate(&files(accepted)).is_ok(), "{accepted}");
        }
        for rejected in [
            "",
            "-main",
            ".hidden",
            "feature/.hidden",
            "feature/",
            "feature//child",
            "feature/../main",
            "main.",
            "main.lock",
            "HEAD~1",
            "main^{commit}",
            "révision",
        ] {
            let error = validate(&files(rejected)).unwrap_err();
            assert_eq!((error.stage, error.code), ("scope", "invalid-base"));
            if !rejected.is_empty() {
                assert!(!error.message.contains(rejected));
            }
        }
        assert!(validate(&files(&"a".repeat(256))).is_err());
    }

    #[test]
    fn full_returns_without_observing_git() {
        let calls = RefCell::new(0);
        let scope = resolve_with(&ScopeRequest::Full, Path::new("/workspace"), |_| {
            *calls.borrow_mut() += 1;
            output(Vec::new())
        })
        .unwrap();

        assert_eq!(*calls.borrow(), 0);
        assert_eq!(scope, ScopeReport::full());
    }

    #[test]
    fn files_runs_three_exact_calls_and_normalizes_the_result() {
        let responses = RefCell::new(VecDeque::from([
            format!("{BASE}\n").into_bytes(),
            format!("{MERGE_BASE}\n").into_bytes(),
            b"src/z.rs\0src/a.rs\0src/z.rs\0".to_vec(),
        ]));
        let calls = RefCell::new(Vec::new());
        let scope = resolve_with(&files("main"), Path::new("/workspace"), |call| {
            calls.borrow_mut().push(call.arguments.clone());
            output(responses.borrow_mut().pop_front().unwrap())
        })
        .unwrap();

        assert_eq!(calls.borrow().len(), 3);
        assert_eq!(
            calls.borrow()[0],
            [
                "-c",
                "color.ui=false",
                "-c",
                "core.fsmonitor=false",
                "--no-pager",
                "-C",
                "/workspace",
                "rev-parse",
                "--verify",
                "--quiet",
                "--end-of-options",
                "main^{commit}",
            ]
            .map(OsString::from)
        );
        assert_eq!(calls.borrow()[1][7], OsStr::new("merge-base"));
        assert_eq!(calls.borrow()[2][7], OsStr::new("diff"));
        assert_eq!(calls.borrow()[2][10], OsStr::new("--relative"));
        assert_eq!(scope.mode(), ScopeMode::Files);
        assert_eq!(scope.comparison_base(), Some(MERGE_BASE));
        assert_eq!(
            scope.files(),
            Some(&["src/a.rs".to_owned(), "src/z.rs".to_owned()][..])
        );
    }

    #[test]
    fn empty_diff_and_sha256_oids_are_closed_successes() {
        let oid64 = "a".repeat(64);
        let responses = RefCell::new(VecDeque::from([
            format!("{oid64}\n").into_bytes(),
            format!("{oid64}\n").into_bytes(),
            Vec::new(),
        ]));
        let scope = resolve_with(&files(&oid64), Path::new("/workspace"), |_| {
            output(responses.borrow_mut().pop_front().unwrap())
        })
        .unwrap();
        assert_eq!(scope.comparison_base(), Some(oid64.as_str()));
        assert_eq!(scope.files(), Some(&[][..]));
    }

    #[test]
    fn failures_stop_before_later_calls_and_never_transport_hostile_output() {
        for (failing_call, expected) in [
            (0, "base-unavailable"),
            (1, "merge-base-unavailable"),
            (2, "git-diff-failed"),
        ] {
            let calls = RefCell::new(0);
            let error = resolve_with(&files("main"), Path::new("/workspace"), |call| {
                let index = *calls.borrow();
                *calls.borrow_mut() += 1;
                if index == failing_call {
                    return Err(phase_failure(call.failure_code, call.failure_message));
                }
                match index {
                    0 => output(format!("{BASE}\n")),
                    1 => output(format!("{MERGE_BASE}\n")),
                    _ => output(Vec::new()),
                }
            })
            .unwrap_err();
            assert_eq!(error.code, expected);
            assert_eq!(*calls.borrow(), failing_call + 1);
            assert!(!error.message.contains("credential=secret"));
        }
    }

    #[test]
    fn missing_and_ambiguous_merge_bases_fail_before_diff() {
        for (merge_output, expected) in [
            (Vec::new(), "merge-base-unavailable"),
            (
                format!("{BASE}\n{MERGE_BASE}\n").into_bytes(),
                "merge-base-ambiguous",
            ),
            (b"not-an-oid\n".to_vec(), "merge-base-unavailable"),
        ] {
            let responses = RefCell::new(VecDeque::from([
                format!("{BASE}\n").into_bytes(),
                merge_output,
            ]));
            let calls = RefCell::new(0);
            let error = resolve_with(&files("main"), Path::new("/workspace"), |_| {
                *calls.borrow_mut() += 1;
                output(responses.borrow_mut().pop_front().unwrap())
            })
            .unwrap_err();
            assert_eq!(error.code, expected);
            assert_eq!(*calls.borrow(), 2);
        }
    }

    #[test]
    fn all_output_and_path_boundaries_fail_atomically() {
        assert!(parse_single_oid(format!("{BASE}\n{MERGE_BASE}\n").as_bytes()).is_none());
        assert!(parse_single_oid(b"not-an-oid\n").is_none());
        assert_eq!(parse_paths(&[]).unwrap(), Vec::<String>::new());
        assert_eq!(
            parse_paths(b"space name\0tab\tname\0line\nname\0percent%name\0").unwrap(),
            ["line%0Aname", "percent%25name", "space name", "tab%09name"]
        );

        let too_large = vec![b'a'; DIFF_OUTPUT_LIMIT + 1];
        assert_eq!(
            parse_paths(&too_large).unwrap_err().code,
            "git-output-too-large"
        );
        let too_many = b"a\0".repeat(FILE_LIMIT + 1);
        assert_eq!(parse_paths(&too_many).unwrap_err().code, "too-many-files");
        let long_path = [vec![b'a'; PATH_LIMIT + 1], vec![0]].concat();
        assert_eq!(
            parse_paths(&long_path).unwrap_err().code,
            "git-path-invalid"
        );
        for invalid in [
            vec![0],
            b"/absolute\0".to_vec(),
            b"./relative\0".to_vec(),
            b"parent/../escape\0".to_vec(),
            b"double//component\0".to_vec(),
            vec![0xff, 0],
            b"unterminated".to_vec(),
        ] {
            assert_eq!(parse_paths(&invalid).unwrap_err().code, "git-path-invalid");
        }

        let bounded = collect_bounded(
            Cursor::new(vec![b'x'; OID_OUTPUT_LIMIT + 1]),
            OID_OUTPUT_LIMIT,
        )
        .unwrap();
        assert!(bounded.exceeded);
        assert_eq!(bounded.bytes.len(), OID_OUTPUT_LIMIT);

        let exact_stderr = collect_bounded(
            Cursor::new(vec![b'x'; STDERR_OUTPUT_LIMIT]),
            STDERR_OUTPUT_LIMIT,
        )
        .unwrap();
        assert!(!exact_stderr.exceeded);
        assert_eq!(exact_stderr.bytes.len(), STDERR_OUTPUT_LIMIT);
        let oversized_stderr = collect_bounded(
            Cursor::new(vec![b'x'; STDERR_OUTPUT_LIMIT + 1]),
            STDERR_OUTPUT_LIMIT,
        )
        .unwrap();
        assert!(oversized_stderr.exceeded);
        assert_eq!(oversized_stderr.bytes.len(), STDERR_OUTPUT_LIMIT);
    }

    #[test]
    fn git_command_fixes_cwd_stdio_and_hostile_environment() {
        let arguments = git_arguments(Path::new("/workspace"), [OsString::from("status")]);
        let command = git_command(Path::new("git"), Path::new("/workspace"), &arguments);
        assert_eq!(command.get_current_dir(), Some(Path::new("/workspace")));
        assert_eq!(command.get_args().collect::<Vec<_>>(), arguments);
        let environment: BTreeMap<_, _> = command.get_envs().collect();
        assert_eq!(
            environment.get(OsStr::new("GIT_OPTIONAL_LOCKS")),
            Some(&Some(OsStr::new("0")))
        );
        assert_eq!(
            environment.get(OsStr::new("LC_ALL")),
            Some(&Some(OsStr::new("C")))
        );
        for variable in GIT_ENVIRONMENT_OVERRIDES {
            assert_eq!(environment.get(OsStr::new(variable)), Some(&None));
        }
    }

    #[test]
    fn missing_git_has_the_single_closed_error() {
        let call = GitCall {
            arguments: git_arguments(Path::new("/workspace"), [OsString::from("status")]),
            stdout_limit: OID_OUTPUT_LIMIT,
            failure_code: "base-unavailable",
            failure_message: "Git base commit is unavailable.",
        };
        let error = run_git(
            Path::new("/definitely/missing/rust-doctor-git"),
            Path::new("/workspace"),
            &call,
        )
        .unwrap_err();
        assert_eq!((error.stage, error.code), ("scope", "git-unavailable"));
    }

    #[cfg(unix)]
    #[test]
    fn real_process_output_limits_and_stderr_are_closed() {
        let (workspace, oversized) = git_script(concat!(
            "#!/bin/sh\n",
            "i=0\n",
            "while [ \"$i\" -lt 4097 ]; do printf x; i=$((i + 1)); done\n",
            "printf 'credential=secret\\n' >&2\n",
        ));
        let call = GitCall {
            arguments: git_arguments(&workspace, [OsString::from("status")]),
            stdout_limit: OID_OUTPUT_LIMIT,
            failure_code: "base-unavailable",
            failure_message: "Git base commit is unavailable.",
        };
        let error = run_git(&oversized, &workspace, &call).unwrap_err();
        assert_eq!(error.code, "git-output-too-large");
        assert!(!error.message.contains("credential=secret"));

        let (workspace, failing) = git_script(concat!(
            "#!/bin/sh\n",
            "printf 'https://secret credential=secret\\n' >&2\n",
            "exit 1\n",
        ));
        let call = GitCall {
            arguments: git_arguments(&workspace, [OsString::from("status")]),
            stdout_limit: OID_OUTPUT_LIMIT,
            failure_code: "git-diff-failed",
            failure_message: "Git changed files could not be read.",
        };
        let error = run_git(&failing, &workspace, &call).unwrap_err();
        assert_eq!(error.code, "git-diff-failed");
        assert!(!error.message.contains("https://secret"));
        assert!(!error.message.contains("credential=secret"));

        let (workspace, exact_stderr) = git_script(concat!(
            "#!/bin/sh\n",
            "i=0\n",
            "while [ \"$i\" -lt 8192 ]; do printf 12345678 >&2; i=$((i + 1)); done\n",
        ));
        let call = GitCall {
            arguments: git_arguments(&workspace, [OsString::from("status")]),
            stdout_limit: OID_OUTPUT_LIMIT,
            failure_code: "base-unavailable",
            failure_message: "Git base commit is unavailable.",
        };
        let output = run_git(&exact_stderr, &workspace, &call).unwrap();
        assert!(output.stdout.is_empty());

        let (workspace, oversized_stderr) = git_script(concat!(
            "#!/bin/sh\n",
            "i=0\n",
            "while [ \"$i\" -lt 8192 ]; do printf 12345678 >&2; i=$((i + 1)); done\n",
            "printf x >&2\n",
        ));
        let call = GitCall {
            arguments: git_arguments(&workspace, [OsString::from("status")]),
            stdout_limit: OID_OUTPUT_LIMIT,
            failure_code: "base-unavailable",
            failure_message: "Git base commit is unavailable.",
        };
        let error = run_git(&oversized_stderr, &workspace, &call).unwrap_err();
        assert_eq!(error.code, "git-output-too-large");
    }

    #[test]
    fn serialized_scope_limit_accepts_the_last_byte_and_rejects_the_boundary() {
        fn scope_with_serialized_size(size: usize) -> ScopeReport {
            let empty = ScopeReport::files_scope(MERGE_BASE.to_owned(), vec![String::new()]);
            let overhead = serde_json::to_vec(&empty).unwrap().len();
            let scope =
                ScopeReport::files_scope(MERGE_BASE.to_owned(), vec!["x".repeat(size - overhead)]);
            assert_eq!(serde_json::to_vec(&scope).unwrap().len(), size);
            scope
        }

        let last_valid = scope_with_serialized_size(SCOPE_OUTPUT_LIMIT - 1);
        assert!(ensure_scope_output_bound(&last_valid).is_ok());
        let first_invalid = scope_with_serialized_size(SCOPE_OUTPUT_LIMIT);
        assert_eq!(
            ensure_scope_output_bound(&first_invalid).unwrap_err().code,
            "git-output-too-large"
        );
    }

    #[test]
    fn normalized_scope_cannot_expand_beyond_the_report_limit() {
        let mut diff = Vec::new();
        for index in 0..FILE_LIMIT {
            let prefix = format!("{index:04}-");
            let path = format!("{prefix}{}", "%".repeat(PATH_LIMIT - prefix.len()));
            if diff.len() + path.len() + 1 > DIFF_OUTPUT_LIMIT {
                break;
            }
            diff.extend_from_slice(path.as_bytes());
            diff.push(0);
        }
        assert!(diff.len() <= DIFF_OUTPUT_LIMIT);
        let expanded_size = {
            let scope = ScopeReport {
                kind: ResolvedScope::Files {
                    comparison_base: MERGE_BASE.to_owned(),
                    files: parse_paths(&diff).unwrap(),
                },
            };
            serde_json::to_vec(&scope).unwrap().len()
        };
        assert!(expanded_size >= SCOPE_OUTPUT_LIMIT);

        let responses = RefCell::new(VecDeque::from([
            format!("{BASE}\n").into_bytes(),
            format!("{MERGE_BASE}\n").into_bytes(),
            diff,
        ]));
        let error = resolve_with(&files("main"), Path::new("/workspace"), |_| {
            output(responses.borrow_mut().pop_front().unwrap())
        })
        .unwrap_err();

        assert_eq!(error.code, "git-output-too-large");
    }
}
