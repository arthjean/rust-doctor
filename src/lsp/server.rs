use super::analysis::{self, EditorFinding};
use crate::config::{self, ResolvedConfig};
use crate::diagnostics::Diagnostic as RustDiagnostic;
use crate::discovery::ProjectInfo;
use serde::Deserialize;
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::RwLock;
use tower_lsp_server::jsonrpc::Result as LspResult;
use tower_lsp_server::ls_types::{
    CodeAction, CodeActionKind, CodeActionOrCommand, CodeActionParams,
    CodeActionProviderCapability, CodeActionResponse, DidChangeTextDocumentParams,
    DidCloseTextDocumentParams, DidOpenTextDocumentParams, DidSaveTextDocumentParams, Hover,
    HoverContents, HoverParams, HoverProviderCapability, InitializeParams, InitializeResult,
    InitializedParams, MarkupContent, MarkupKind, MessageType, Position, PositionEncodingKind,
    Range, ServerCapabilities, ServerInfo, TextDocumentSyncCapability, TextDocumentSyncKind,
    TextDocumentSyncOptions, TextDocumentSyncSaveOptions, TextEdit, Uri, WorkspaceEdit,
};
use tower_lsp_server::{Client, LanguageServer, LspService, Server};

#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Settings {
    debounce_ms: u64,
    on_save_project_checks: bool,
    project_budget_ms: u64,
    configuration_path: Option<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            debounce_ms: 300,
            on_save_project_checks: false,
            project_budget_ms: 10_000,
            configuration_path: None,
        }
    }
}

impl Settings {
    fn bounded(mut self) -> Self {
        self.debounce_ms = self.debounce_ms.clamp(50, 2_000);
        self.project_budget_ms = self.project_budget_ms.clamp(1_000, 60_000);
        self
    }
}

struct ProjectContext {
    info: Arc<ProjectInfo>,
    config: ResolvedConfig,
    frameworks: Vec<String>,
}

#[derive(Clone)]
struct Document {
    text: String,
    version: i32,
    findings: Vec<EditorFinding>,
    cancellation: Arc<AtomicBool>,
}

#[derive(Default)]
struct State {
    settings: Settings,
    project: Option<Arc<ProjectContext>>,
    documents: HashMap<Uri, Document>,
}

struct Backend {
    client: Client,
    state: Arc<RwLock<State>>,
}

pub(super) async fn serve() {
    let (service, socket) = LspService::new(|client| Backend {
        client,
        state: Arc::new(RwLock::new(State::default())),
    });
    Server::new(tokio::io::stdin(), tokio::io::stdout(), socket)
        .concurrency_level(8)
        .serve(service)
        .await;
}

impl Backend {
    async fn schedule_analysis(&self, uri: Uri, text: String, version: i32) {
        let cancellation = Arc::new(AtomicBool::new(false));
        let (settings, project) = {
            let mut state = self.state.write().await;
            if let Some(previous) = state.documents.get(&uri) {
                previous.cancellation.store(true, Ordering::Release);
            }
            state.documents.insert(
                uri.clone(),
                Document {
                    text: text.clone(),
                    version,
                    findings: Vec::new(),
                    cancellation: Arc::clone(&cancellation),
                },
            );
            (state.settings.clone(), state.project.clone())
        };
        let state = Arc::clone(&self.state);
        let client = self.client.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(settings.debounce_ms)).await;
            if cancellation.load(Ordering::Acquire) {
                return;
            }
            let Some(project) = project else {
                client
                    .publish_diagnostics(uri, Vec::new(), Some(version))
                    .await;
                return;
            };
            let path = match file_path(&uri) {
                Some(path) if path.starts_with(&project.info.root_dir) => path,
                _ => {
                    client
                        .log_message(
                            MessageType::WARNING,
                            "ignored document outside the project root",
                        )
                        .await;
                    client
                        .publish_diagnostics(uri, Vec::new(), Some(version))
                        .await;
                    return;
                }
            };
            let relative = path
                .strip_prefix(&project.info.root_dir)
                .unwrap_or(&path)
                .to_path_buf();
            let source = text.clone();
            let task_cancel = Arc::clone(&cancellation);
            let analysis_project = Arc::clone(&project);
            let result = tokio::task::spawn_blocking(move || {
                analysis::analyze(
                    &source,
                    &relative,
                    &analysis_project.config,
                    &analysis_project.frameworks,
                    &task_cancel,
                )
            })
            .await;
            if cancellation.load(Ordering::Acquire) {
                return;
            }
            let findings = match result {
                Ok(Ok(findings)) => findings,
                Ok(Err(_)) => Vec::new(),
                Err(error) => {
                    client
                        .log_message(
                            MessageType::ERROR,
                            format!("document analysis failed: {error}"),
                        )
                        .await;
                    Vec::new()
                }
            };
            let diagnostics = findings.iter().map(EditorFinding::to_diagnostic).collect();
            let publish = replace_findings(&state, &uri, version, &cancellation, findings).await;
            if publish {
                client
                    .publish_diagnostics(uri, diagnostics, Some(version))
                    .await;
            }
        });
    }

    async fn run_project_checks(&self, uri: Uri) {
        let (settings, project, text, version) = {
            let state = self.state.read().await;
            let Some(project) = state.project.clone() else {
                return;
            };
            let Some(document) = state.documents.get(&uri) else {
                return;
            };
            (
                state.settings.clone(),
                project,
                document.text.clone(),
                document.version,
            )
        };
        if !settings.on_save_project_checks {
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let scan_cancel = Arc::clone(&cancel);
        let scan_project = Arc::clone(&project);
        let mut task = tokio::task::spawn_blocking(move || {
            crate::scan::scan_project_cancellable(
                &scan_project.info,
                &scan_project.config,
                true,
                &[],
                true,
                &scan_cancel,
            )
        });
        let result = tokio::select! {
            result = &mut task => Some(result),
            () = tokio::time::sleep(Duration::from_millis(settings.project_budget_ms)) => {
                cancel.store(true, Ordering::Release);
                let _ = task.await;
                None
            }
        };
        let Some(Ok(Ok(result))) = result else {
            self.client
                .log_message(
                    MessageType::WARNING,
                    "on-save project analysis did not complete within its budget",
                )
                .await;
            return;
        };
        let Some(path) = file_path(&uri) else {
            return;
        };
        let relative = path.strip_prefix(&project.info.root_dir).unwrap_or(&path);
        let diagnostics: Vec<RustDiagnostic> = result
            .diagnostics
            .into_iter()
            .filter(|diagnostic| diagnostic.file_path == path || diagnostic.file_path == relative)
            .collect();
        let findings = analysis::convert(&text, relative, diagnostics);
        let diagnostics = findings.iter().map(EditorFinding::to_diagnostic).collect();
        {
            let mut state = self.state.write().await;
            if let Some(document) = state.documents.get_mut(&uri)
                && document.version == version
            {
                document.findings = findings;
            } else {
                return;
            }
        }
        self.client
            .publish_diagnostics(uri, diagnostics, Some(version))
            .await;
    }
}

impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> LspResult<InitializeResult> {
        let settings = params
            .initialization_options
            .clone()
            .and_then(|value| serde_json::from_value::<Settings>(value).ok())
            .unwrap_or_default()
            .bounded();
        let root_uri = initialization_root(&params);
        let project = root_uri
            .as_ref()
            .and_then(file_path)
            .and_then(|root| load_project(&root, &settings).ok())
            .map(Arc::new);
        {
            let mut state = self.state.write().await;
            state.settings = settings;
            state.project = project;
        }
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                position_encoding: Some(PositionEncodingKind::UTF16),
                text_document_sync: Some(TextDocumentSyncCapability::Options(
                    TextDocumentSyncOptions {
                        open_close: Some(true),
                        change: Some(TextDocumentSyncKind::FULL),
                        save: Some(TextDocumentSyncSaveOptions::Supported(true)),
                        ..TextDocumentSyncOptions::default()
                    },
                )),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
                experimental: Some(serde_json::json!({
                    "rustDoctorProtocolVersion": 1,
                    "lspCompatibility": "3.17+"
                })),
                ..ServerCapabilities::default()
            },
            server_info: Some(ServerInfo {
                name: "rust-doctor".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
            offset_encoding: None,
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        let has_project = self.state.read().await.project.is_some();
        let message = if has_project {
            "Rust Doctor language server initialized"
        } else {
            "Rust Doctor language server initialized without a Cargo project"
        };
        self.client.log_message(MessageType::INFO, message).await;
    }

    async fn shutdown(&self) -> LspResult<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        self.schedule_analysis(
            params.text_document.uri,
            params.text_document.text,
            params.text_document.version,
        )
        .await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        if let Some(change) = params.content_changes.into_iter().last() {
            self.schedule_analysis(
                params.text_document.uri,
                change.text,
                params.text_document.version,
            )
            .await;
        }
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        self.run_project_checks(params.text_document.uri).await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        let document = {
            let mut state = self.state.write().await;
            state.documents.remove(&uri)
        };
        if let Some(document) = document {
            document.cancellation.store(true, Ordering::Release);
        }
        self.client.publish_diagnostics(uri, Vec::new(), None).await;
    }

    async fn hover(&self, params: HoverParams) -> LspResult<Option<Hover>> {
        let position = params.text_document_position_params.position;
        let uri = params.text_document_position_params.text_document.uri;
        let finding = {
            let state = self.state.read().await;
            state.documents.get(&uri).and_then(|document| {
                document
                    .findings
                    .iter()
                    .find(|finding| contains(finding.range, position))
                    .cloned()
            })
        };
        Ok(finding.map(|finding| Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: format!(
                    "**{}**\n\n{}\n\n{}\n\n[Rule documentation]({})",
                    finding.rule,
                    finding.message,
                    finding.help.as_deref().unwrap_or("No additional guidance."),
                    finding.documentation_url
                ),
            }),
            range: Some(finding.range),
        }))
    }

    async fn code_action(&self, params: CodeActionParams) -> LspResult<Option<CodeActionResponse>> {
        let uri = params.text_document.uri;
        let document = {
            let state = self.state.read().await;
            state.documents.get(&uri).cloned()
        };
        let Some(document) = document else {
            return Ok(None);
        };
        let mut actions = Vec::new();
        for diagnostic in params.context.diagnostics {
            let Some(data) = diagnostic.data.as_ref() else {
                continue;
            };
            let Some(identity) = data.get("identity").and_then(serde_json::Value::as_str) else {
                continue;
            };
            let Some(finding) = document
                .findings
                .iter()
                .find(|finding| finding.identity == identity)
            else {
                continue;
            };
            let line = finding.range.start.line;
            let indentation =
                document
                    .text
                    .lines()
                    .nth(line as usize)
                    .map_or_else(String::new, |line| {
                        line.chars()
                            .take_while(|character| character.is_whitespace())
                            .collect()
                    });
            let edit = TextEdit {
                range: Range::new(Position::new(line, 0), Position::new(line, 0)),
                new_text: format!(
                    "{indentation}// rust-doctor-disable-next-line {}\n",
                    finding.rule
                ),
            };
            actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                title: format!("Suppress {} on the next line", finding.rule),
                kind: Some(CodeActionKind::QUICKFIX),
                diagnostics: Some(vec![diagnostic]),
                edit: Some(WorkspaceEdit {
                    changes: Some(HashMap::from([(uri.clone(), vec![edit])])),
                    ..WorkspaceEdit::default()
                }),
                is_preferred: Some(false),
                data: Some(serde_json::json!({"identity": finding.identity})),
                ..CodeAction::default()
            }));
        }
        Ok((!actions.is_empty()).then_some(actions))
    }
}

fn load_project(root: &Path, settings: &Settings) -> Result<ProjectContext, String> {
    let (_, info, file_config) =
        crate::discovery::bootstrap_project(root, true).map_err(|error| error.to_string())?;
    let resolved = if let Some(config_path) = &settings.configuration_path {
        let path = if config_path.is_absolute() {
            config_path.clone()
        } else {
            info.root_dir.join(config_path)
        };
        let canonical = path
            .canonicalize()
            .map_err(|error| format!("failed to resolve LSP configuration path: {error}"))?;
        if !canonical.starts_with(&info.root_dir) {
            return Err("LSP configuration path escapes the project root".to_string());
        }
        let content = std::fs::read_to_string(&canonical)
            .map_err(|error| format!("failed to read LSP configuration: {error}"))?;
        let parsed: config::FileConfig = toml::from_str(&content)
            .map_err(|error| format!("failed to parse LSP configuration: {error}"))?;
        config::validate_file_config(&parsed, &canonical)
            .map_err(|error| format!("invalid LSP configuration: {error}"))?;
        config::resolve_config_defaults(Some(&parsed))
    } else {
        config::resolve_config_defaults(file_config.as_ref())
    };
    let frameworks = info.frameworks.iter().map(ToString::to_string).collect();
    Ok(ProjectContext {
        info: Arc::new(info),
        config: resolved,
        frameworks,
    })
}

#[allow(
    deprecated,
    reason = "root_uri remains the fallback for clients without workspace folders"
)]
fn initialization_root(params: &InitializeParams) -> Option<Uri> {
    params
        .workspace_folders
        .as_ref()
        .and_then(|folders| folders.first().map(|folder| folder.uri.clone()))
        .or_else(|| params.root_uri.clone())
}

fn file_path(uri: &Uri) -> Option<PathBuf> {
    if !uri.scheme().as_str().eq_ignore_ascii_case("file") {
        return None;
    }
    uri.to_file_path().map(Cow::into_owned)
}

async fn replace_findings(
    state: &RwLock<State>,
    uri: &Uri,
    version: i32,
    cancellation: &Arc<AtomicBool>,
    findings: Vec<EditorFinding>,
) -> bool {
    let mut state = state.write().await;
    let accepted = state.documents.get_mut(uri).is_some_and(|document| {
        if document.version != version || !Arc::ptr_eq(&document.cancellation, cancellation) {
            return false;
        }
        document.findings = findings;
        true
    });
    drop(state);
    accepted
}

const fn contains(range: Range, position: Position) -> bool {
    before_or_equal(range.start, position) && before_or_equal(position, range.end)
}

const fn before_or_equal(left: Position, right: Position) -> bool {
    left.line < right.line || (left.line == right.line && left.character <= right.character)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_hold_latency_and_network_contract() {
        let settings = Settings::default();
        assert_eq!(settings.debounce_ms, 300);
        assert!(!settings.on_save_project_checks);
        assert_eq!(settings.project_budget_ms, 10_000);
    }

    #[test]
    fn range_intersection_is_inclusive() {
        let range = Range::new(Position::new(2, 4), Position::new(2, 8));
        assert!(contains(range, Position::new(2, 4)));
        assert!(contains(range, Position::new(2, 8)));
        assert!(!contains(range, Position::new(2, 9)));
    }
}
