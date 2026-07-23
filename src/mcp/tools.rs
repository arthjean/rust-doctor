use crate::cli::Cli;
use crate::config;
use crate::diagnostics::{ReportV1, ScanMode, ScanResult};
use crate::discovery::ProjectInfo;
use clap::Parser;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::{
    CallToolResult, Content, GetPromptResult, LoggingLevel, LoggingMessageNotificationParam,
    PromptMessage, PromptMessageRole,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, prompt, prompt_router, tool, tool_router};
use std::ffi::OsString;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use super::RustDoctorServer;
use super::helpers::{discover_and_resolve, format_report_scan, group_report_diagnostics};
use super::rules::{get_all_rules_listing, get_rule_explanation};
use super::types::{
    DeepAuditArgs, ExplainRuleInput, HealthCheckArgs, McpScanScope, ScanInput, ScoreInput,
    ScoreOutput,
};

/// MCP timeout for a single scan/score call. On expiry the work is cancelled
/// cooperatively, not detached.
const MCP_SCAN_TIMEOUT_SECS: u64 = 300;

/// Run a scan on a blocking thread under a 5-minute absolute timeout.
///
/// On timeout the shared cancel flag is set so the (now-detached) blocking scan
/// stops launching new passes instead of running to completion in the background
/// and exhausting the blocking pool (US-007). The client-facing timeout message is
/// unchanged.
async fn run_scan_with_timeout(
    project_info: ProjectInfo,
    resolved: config::ResolvedConfig,
    cli: Cli,
    tool: &str,
    request_context: &RequestContext<RoleServer>,
) -> Result<(ScanResult, ProjectInfo, config::ResolvedConfig), McpError> {
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_task = Arc::clone(&cancel);
    let mut scan_future = tokio::task::spawn_blocking(move || {
        let result =
            crate::run::run_scan_cancellable(&cli, &project_info, &resolved, &cancel_task)?;
        Ok::<_, crate::error::ScanError>((result, project_info, resolved))
    });
    let timeout = tokio::time::sleep(Duration::from_secs(MCP_SCAN_TIMEOUT_SECS));
    tokio::pin!(timeout);
    tokio::select! {
        join_result = &mut scan_future => join_result
            .map_err(|e| McpError::internal_error(format!("scan task failed: {e}"), None))?
            .map_err(|e| {
                eprintln!("MCP {tool} error: {e}");
                McpError::internal_error(
                    "scan failed — check project compiles with `cargo check`",
                    None,
                )
            }),
        () = request_context.ct.cancelled() => {
            cancel.store(true, Ordering::Relaxed);
            Err(McpError::internal_error(
                "scan cancelled by client",
                None,
            ))
        }
        () = &mut timeout => {
            cancel.store(true, Ordering::Relaxed);
            Err(McpError::internal_error(
                "scan timed out after 5 minutes — project may be too large or a subprocess is hanging",
                None,
            ))
        }
    }
}

fn scan_cli(input: &ScanInput) -> Result<Cli, McpError> {
    if input.diff.is_some()
        && (input.scope != McpScanScope::Full
            || !input.files.is_empty()
            || input.base.is_some()
            || input.include_untracked)
    {
        return Err(McpError::invalid_params(
            "diff cannot be combined with scope, files, base, or include_untracked",
            None,
        ));
    }

    let mut arguments =
        base_cli_arguments(&input.directory, input.offline, input.ignore_project_config);
    let (scope, base) = input
        .diff
        .as_ref()
        .map_or((input.scope, input.base.as_ref()), |diff| {
            (McpScanScope::Changed, (diff != "auto").then_some(diff))
        });
    match scope {
        McpScanScope::Full => {}
        McpScanScope::Files => {
            arguments.extend([OsString::from("--scope"), OsString::from("files")]);
            for file in &input.files {
                arguments.push(OsString::from(format!("--files={file}")));
            }
        }
        McpScanScope::Changed | McpScanScope::Lines => {
            arguments.extend([
                OsString::from("--scope"),
                OsString::from(if scope == McpScanScope::Changed {
                    "changed"
                } else {
                    "lines"
                }),
            ]);
            if input.include_untracked {
                arguments.push(OsString::from("--include-untracked"));
            }
        }
        McpScanScope::Staged => arguments.push(OsString::from("--staged")),
        McpScanScope::Baseline => arguments.push(OsString::from("--baseline")),
    }
    if let Some(base) = base {
        arguments.extend([OsString::from("--base"), OsString::from(base)]);
    }
    parse_mcp_cli(arguments)
}

fn score_cli(input: &ScoreInput) -> Result<Cli, McpError> {
    parse_mcp_cli(base_cli_arguments(
        &input.directory,
        input.offline,
        input.ignore_project_config,
    ))
}

fn base_cli_arguments(
    directory: &str,
    offline: bool,
    ignore_project_config: bool,
) -> Vec<OsString> {
    let mut arguments = vec![
        OsString::from("rust-doctor"),
        OsString::from(directory),
        OsString::from("--json-compact"),
        OsString::from("--no-telemetry"),
    ];
    if offline {
        arguments.push(OsString::from("--offline"));
    }
    if ignore_project_config {
        arguments.push(OsString::from("--no-project-config"));
    }
    arguments
}

fn parse_mcp_cli(arguments: Vec<OsString>) -> Result<Cli, McpError> {
    let cli = Cli::try_parse_from(arguments)
        .and_then(|cli| {
            cli.validate_contract()?;
            Ok(cli)
        })
        .map_err(|_| McpError::invalid_params("invalid MCP scan scope arguments", None))?;
    Ok(cli)
}

fn scan_mode(result: &ScanResult) -> ScanMode {
    match result.execution.reporting_scope.as_str() {
        "files" | "changed" => ScanMode::Files,
        "lines" => ScanMode::Lines,
        "staged" => ScanMode::Staged,
        "baseline" => ScanMode::Baseline,
        _ => ScanMode::Full,
    }
}

// ---------------------------------------------------------------------------
// Tool and prompt implementations
// ---------------------------------------------------------------------------

#[tool_router(vis = "pub(super)")]
#[prompt_router(vis = "pub(super)")]
impl RustDoctorServer {
    pub(super) fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
            prompt_router: Self::prompt_router(),
        }
    }

    #[tool(
        name = "scan",
        description = "Run a full Rust code health analysis on a project directory. \
Use this tool when you need detailed diagnostics — it returns all findings with file:line precision. \
Takes 5-30 seconds depending on project size. \
Returns a readable projection of canonical Report V1 diagnostics with stable rule metadata and locations. \
Severity levels: error (bugs/security), warning (code smells), info (suggestions). \
Runs Clippy, the canonical custom-rule catalog, and configured Cargo analyzers in parallel. \
Supports full, files, changed, lines, staged, and baseline scopes. \
After scanning, use explain_rule on any rule ID to get fix guidance.",
        annotations(
            title = "Scan Project",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false,
        )
    )]
    async fn scan(
        &self,
        meta: rmcp::model::Meta,
        client: rmcp::Peer<RoleServer>,
        params: Parameters<ScanInput>,
        request_context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let input = params.0;
        let requested_root = std::path::PathBuf::from(&input.directory);
        let cli = scan_cli(&input)?;
        let progress_token = meta.get_progress_token();

        // Send start progress if client supports it
        if let Some(ref token) = progress_token {
            let _ = client
                .notify_progress(rmcp::model::ProgressNotificationParam {
                    progress_token: token.clone(),
                    progress: 0.0,
                    total: Some(2.0),
                    message: Some("Bootstrapping project...".to_string()),
                })
                .await;
        }
        let _ = client
            .notify_logging_message(LoggingMessageNotificationParam {
                level: LoggingLevel::Info,
                logger: Some("rust-doctor".into()),
                data: serde_json::json!("Bootstrapping project..."),
            })
            .await;

        let (_dir, project_info, mut resolved) =
            discover_and_resolve(&input.directory, input.ignore_project_config)?;
        resolved.diff = None;

        // Send scanning progress
        if let Some(ref token) = progress_token {
            let _ = client
                .notify_progress(rmcp::model::ProgressNotificationParam {
                    progress_token: token.clone(),
                    progress: 1.0,
                    total: Some(2.0),
                    message: Some(
                        "Running analysis passes (clippy, rules, audit, machete)...".to_string(),
                    ),
                })
                .await;
        }
        let _ = client
            .notify_logging_message(LoggingMessageNotificationParam {
                level: LoggingLevel::Info,
                logger: Some("rust-doctor".into()),
                data: serde_json::json!(
                    "Running 4 analysis passes (clippy, AST rules, cargo-audit, cargo-machete)..."
                ),
            })
            .await;

        // Run the CPU-bound scan on a blocking thread with a 5-minute absolute timeout
        let (result, project_info, resolved) =
            run_scan_with_timeout(project_info, resolved, cli, "scan", &request_context).await?;

        // Send completion progress
        if let Some(ref token) = progress_token {
            let _ = client
                .notify_progress(rmcp::model::ProgressNotificationParam {
                    progress_token: token.clone(),
                    progress: 2.0,
                    total: Some(2.0),
                    message: Some(format!(
                        "Scan complete: score {}/100, {} findings",
                        result.score,
                        result.diagnostics.len()
                    )),
                })
                .await;
        }
        let _ = client
            .notify_logging_message(LoggingMessageNotificationParam {
                level: LoggingLevel::Info,
                logger: Some("rust-doctor".into()),
                data: serde_json::Value::String(format!(
                    "Scan complete: {}/100 ({}) — {} errors, {} warnings, {} info in {:.1}s",
                    result.score,
                    result.score_label,
                    result.error_count,
                    result.warning_count,
                    result.info_count,
                    result.elapsed.as_secs_f64()
                )),
            })
            .await;

        let report = ReportV1::from_scan_with_context(
            &result,
            &project_info,
            &resolved,
            scan_mode(&result),
            &requested_root,
            crate::diagnostics::GateResult::NotEvaluated,
        );
        let grouped = group_report_diagnostics(&report.diagnostics);
        let report = format_report_scan(&report, &grouped);

        Ok(CallToolResult::success(vec![Content::text(report)]))
    }

    #[tool(
        name = "score",
        description = "Get just the health score of a Rust project (0-100 integer). \
Use this tool for a quick pass/fail check without full diagnostics. \
IMPORTANT: runs the same full analysis as scan internally, so takes the same 5-30 seconds. \
Score thresholds: >=75 'Great', >=50 'Needs work', <50 'Critical'. \
Scoring: each unique error-severity rule violated costs 1.5 points, each warning costs 0.75 points. \
If you also need the diagnostics, use scan instead — it includes the score too.",
        annotations(
            title = "Score Project",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false,
        )
    )]
    async fn score(
        &self,
        meta: rmcp::model::Meta,
        client: rmcp::Peer<RoleServer>,
        params: Parameters<ScoreInput>,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<ScoreOutput>, McpError> {
        let input = params.0;
        let cli = score_cli(&input)?;
        let progress_token = meta.get_progress_token();

        if let Some(ref token) = progress_token {
            let _ = client
                .notify_progress(rmcp::model::ProgressNotificationParam {
                    progress_token: token.clone(),
                    progress: 0.0,
                    total: Some(1.0),
                    message: Some("Scoring project...".to_string()),
                })
                .await;
        }
        let _ = client
            .notify_logging_message(LoggingMessageNotificationParam {
                level: LoggingLevel::Info,
                logger: Some("rust-doctor".into()),
                data: serde_json::json!("Scoring project..."),
            })
            .await;

        let (_dir, project_info, mut resolved) =
            discover_and_resolve(&input.directory, input.ignore_project_config)?;
        resolved.diff = None;

        // Run the CPU-bound scan on a blocking thread with a 5-minute absolute timeout
        let (result, _project_info, _resolved) =
            run_scan_with_timeout(project_info, resolved, cli, "score", &request_context).await?;

        if let Some(ref token) = progress_token {
            let _ = client
                .notify_progress(rmcp::model::ProgressNotificationParam {
                    progress_token: token.clone(),
                    progress: 1.0,
                    total: Some(1.0),
                    message: Some(format!(
                        "Score: {}/100 ({})",
                        result.score, result.score_label
                    )),
                })
                .await;
        }
        let _ = client
            .notify_logging_message(LoggingMessageNotificationParam {
                level: LoggingLevel::Info,
                logger: Some("rust-doctor".into()),
                data: serde_json::Value::String(format!(
                    "Score: {}/100 ({})",
                    result.score, result.score_label
                )),
            })
            .await;

        Ok(Json(ScoreOutput {
            score: result.score,
            score_label: result.score_label,
        }))
    }

    #[tool(
        name = "explain_rule",
        description = "Get a detailed markdown explanation of a specific rust-doctor rule. \
Use this after scan to understand what a rule detects and how to fix violations. \
Returns: rule name, category, severity, description, and fix guidance. \
Accepts custom rule IDs (e.g. 'unwrap-in-production') and clippy lint names (e.g. 'clippy::expect_used'). \
Instant response — no project scanning required. \
For unknown rules, returns guidance to use list_rules.",
        annotations(
            title = "Explain Rule",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false,
        )
    )]
    async fn explain_rule(
        &self,
        params: Parameters<ExplainRuleInput>,
    ) -> Result<CallToolResult, McpError> {
        let explanation = get_rule_explanation(&params.0.rule);
        Ok(CallToolResult::success(vec![Content::text(explanation)]))
    }

    #[tool(
        name = "list_rules",
        description = "List all available rust-doctor rules as formatted markdown. \
Use this to discover which checks exist before scanning, or to find a rule ID for explain_rule. \
Instant response — no project scanning required. \
Returns every canonical custom, Clippy, external, and project rule directly from the shared catalog. \
Each entry shows rule ID, severity, and one-line summary.",
        annotations(
            title = "List Rules",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false,
        )
    )]
    async fn list_rules(&self) -> Result<CallToolResult, McpError> {
        let listing = get_all_rules_listing();
        Ok(CallToolResult::success(vec![Content::text(listing)]))
    }

    // -- Prompts --------------------------------------------------------------

    #[prompt(
        name = "deep-audit",
        description = "Comprehensive Rust code audit: explores codebase architecture, runs rust-doctor \
analysis, performs deep code review against production best practices, researches current Rust patterns \
on the web, cross-references findings, and generates a full remediation report. Ends with a choice: \
implement all fixes, generate a PRD, or manual prompt. Use this for thorough, expert-level code audits \
that go far beyond linting."
    )]
    pub(super) async fn deep_audit(&self, params: Parameters<DeepAuditArgs>) -> GetPromptResult {
        GetPromptResult::new(vec![PromptMessage::new_text(
            PromptMessageRole::User,
            super::prompts::deep_audit_prompt(&params.0.directory),
        )])
        .with_description(
            "Expert-level Rust audit: codebase exploration + static analysis + deep code review \
             + best practices research + synthesis report + actionable remediation choices",
        )
    }

    #[prompt(
        name = "health-check",
        description = "Run a full health check on a Rust project: scan, generate a prioritized \
remediation plan, and optionally apply fixes. Combines scan + plan + fix into one structured workflow."
    )]
    pub(super) async fn health_check(
        &self,
        params: Parameters<HealthCheckArgs>,
    ) -> GetPromptResult {
        GetPromptResult::new(vec![PromptMessage::new_text(
            PromptMessageRole::User,
            super::prompts::health_check_prompt(&params.0.directory),
        )])
        .with_description(
            "Full health audit with prioritized remediation plan and structured fix workflow",
        )
    }
}
