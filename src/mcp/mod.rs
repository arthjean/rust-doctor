mod handler;
mod helpers;
mod prompts;
mod rules;
mod tools;
mod types;

// Re-export the public API
pub use handler::RustDoctorServer;
pub use types::{
    DeepAuditArgs, DiagnosticExample, DiagnosticGroup, ExplainRuleInput, HealthCheckArgs,
    ScanInput, ScoreInput, ScoreOutput,
};

use rmcp::service::ServiceExt;

// ---------------------------------------------------------------------------
// Error type & public entry point
// ---------------------------------------------------------------------------

/// Typed error enum for MCP server failures — replaces `Box<dyn Error>` so
/// callers can match on specific failure modes.
#[derive(Debug, thiserror::Error)]
pub enum McpServerError {
    #[error("failed to create tokio runtime: {0}")]
    RuntimeCreation(#[from] std::io::Error),

    #[error("MCP server initialization failed: {0}")]
    Initialize(#[from] Box<rmcp::service::ServerInitializeError>),

    #[error("MCP server task failed: {0}")]
    TaskJoin(#[from] tokio::task::JoinError),
}

/// Run the MCP server over stdio. Called from main when `--mcp` is passed.
///
/// # Errors
///
/// Returns an error if the tokio runtime cannot be created, the MCP transport
/// fails to initialize, or the server encounters a fatal error.
pub fn run_mcp_server() -> Result<(), McpServerError> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let server = RustDoctorServer::new();
        let transport = rmcp::transport::io::stdio();
        let service = server.serve(transport).await.map_err(Box::new)?;
        service.waiting().await?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::helpers::{discover_and_resolve, format_scan_report, group_diagnostics};
    use super::rules::{get_all_rules_listing, get_rule_explanation, rule_docs};
    use super::types::{DeepAuditArgs, HealthCheckArgs, MAX_EXAMPLES_PER_GROUP, ScoreOutput};
    use super::*;
    use crate::diagnostics::Diagnostic;
    use crate::scan;
    use rmcp::handler::server::wrapper::Parameters;

    fn isolated_scan_fixture() -> (
        tempfile::TempDir,
        crate::discovery::ProjectInfo,
        crate::config::ResolvedConfig,
    ) {
        let fixture = tempfile::Builder::new()
            .prefix("rust-doctor-mcp-test-")
            .tempdir_in(env!("CARGO_MANIFEST_DIR"))
            .unwrap();
        std::fs::create_dir(fixture.path().join("src")).unwrap();
        std::fs::write(
            fixture.path().join("Cargo.toml"),
            "[package]\nname = \"mcp-test\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        std::fs::write(
            fixture.path().join("rust-doctor.toml"),
            "lint = true\ndependencies = false\n",
        )
        .unwrap();
        std::fs::write(
            fixture.path().join("src/lib.rs"),
            "pub fn first(value: Option<u8>) -> u8 { value.unwrap() }\n\
             pub fn second(value: Result<u8, ()>) -> u8 { value.unwrap() }\n",
        )
        .unwrap();
        let directory = fixture.path().to_string_lossy();
        let (_root, project_info, resolved) =
            discover_and_resolve(directory.as_ref(), false).unwrap();
        (fixture, project_info, resolved)
    }

    // --- RULE_DOCS completeness ---

    #[test]
    fn test_rule_docs_covers_all_custom_rules() {
        let docs = rule_docs();
        let expected: Vec<String> = crate::scan::custom_rule_names()
            .into_iter()
            .filter(|name| name != "unused-dependency") // external tool rule, not AST
            .collect();

        for rule_name in &expected {
            assert!(
                docs.iter().any(|doc| doc.name == *rule_name),
                "rule_docs() is missing entry for custom rule '{rule_name}'"
            );
        }
    }

    #[test]
    fn test_rule_docs_has_no_duplicates() {
        let docs = rule_docs();
        let mut seen = std::collections::HashSet::new();
        for doc in docs {
            assert!(
                seen.insert(&doc.name),
                "rule_docs() has duplicate entry for '{}'",
                doc.name
            );
        }
    }

    #[test]
    fn test_rule_docs_fields_not_empty() {
        let docs = rule_docs();
        for doc in docs {
            assert!(!doc.name.is_empty(), "Rule has empty name");
            assert!(
                !doc.category.is_empty(),
                "Rule '{}' has empty category",
                doc.name
            );
            assert!(
                !doc.severity.is_empty(),
                "Rule '{}' has empty severity",
                doc.name
            );
            assert!(
                !doc.description.is_empty(),
                "Rule '{}' has empty description",
                doc.name
            );
            assert!(!doc.fix.is_empty(), "Rule '{}' has empty fix", doc.name);
        }
    }

    // --- get_rule_explanation ---

    #[test]
    fn test_explain_known_custom_rule() {
        let explanation = get_rule_explanation("unwrap-in-production");
        assert!(explanation.contains("## unwrap-in-production"));
        assert!(explanation.contains("Error Handling"));
        assert!(explanation.contains("warning"));
        assert!(explanation.contains("Fix:"));
        // US-013: custom rules are flagged heuristic, with their known limitation.
        assert!(explanation.contains("Heuristic"));
        assert!(explanation.contains("Known limitation"));
    }

    #[test]
    fn test_explain_custom_rule_without_limitation_is_still_heuristic() {
        // A custom rule with no documented blind spot still carries the marker.
        let explanation = get_rule_explanation("result-unit-error");
        assert!(explanation.contains("Heuristic"));
        assert!(!explanation.contains("Known limitation"));
    }

    #[test]
    fn test_explain_clippy_lint_is_type_aware() {
        // US-013: clippy lints are distinguished as type-aware, never heuristic.
        let explanation = get_rule_explanation("clippy::expect_used");
        assert!(explanation.contains("Type-aware"));
        assert!(!explanation.contains("Heuristic"));
    }

    #[test]
    fn test_explain_known_clippy_lint() {
        let explanation = get_rule_explanation("clippy::expect_used");
        assert!(explanation.contains("clippy::expect_used"));
        assert!(explanation.contains("Clippy lint"));
        assert!(explanation.contains("rust-lang.github.io"));
    }

    #[test]
    fn test_explain_clippy_lint_without_prefix() {
        let explanation = get_rule_explanation("expect_used");
        assert!(explanation.contains("expect_used"));
        assert!(explanation.contains("Clippy lint"));
    }

    #[test]
    fn test_explain_unknown_rule() {
        let explanation = get_rule_explanation("nonexistent-rule-xyz");
        assert!(explanation.contains("Unknown rule"));
        assert!(explanation.contains("list_rules"));
    }

    // --- get_all_rules_listing ---

    #[test]
    fn test_rules_listing_has_all_sections() {
        let listing = get_all_rules_listing();
        assert!(listing.contains("# rust-doctor Rules"));
        assert!(listing.contains("## Custom Rules"));
        assert!(listing.contains("## Clippy Lints"));
        assert!(listing.contains("## External Tools"));
    }

    #[test]
    fn test_rules_listing_marks_heuristic_and_limitations() {
        // US-013: the listing frames custom rules as heuristic, distinguishes
        // type-aware clippy, and surfaces the known-FP limitations.
        let listing = get_all_rules_listing();
        assert!(listing.contains("heuristic"));
        assert!(listing.contains("type-aware"));
        assert!(listing.contains("Known heuristic limitations"));
        assert!(listing.contains("large-enum-variant"));
    }

    #[test]
    fn test_rules_listing_contains_all_categories() {
        let listing = get_all_rules_listing();
        assert!(listing.contains("### Error Handling"));
        assert!(listing.contains("### Performance"));
        assert!(listing.contains("### Security"));
        assert!(listing.contains("### Async"));
        assert!(listing.contains("### Framework"));
    }

    #[test]
    fn test_rules_listing_contains_all_custom_rules() {
        let listing = get_all_rules_listing();
        let docs = rule_docs();
        for doc in docs {
            assert!(
                listing.contains(doc.name),
                "Rules listing is missing '{}'",
                doc.name
            );
        }
    }

    // --- ServerInfo ---

    #[test]
    fn test_server_info_has_instructions() {
        let server = RustDoctorServer::new();
        let info = <RustDoctorServer as rmcp::handler::server::ServerHandler>::get_info(&server);
        let instructions = info.instructions.as_deref().unwrap_or("");
        assert!(!instructions.is_empty());
        assert!(instructions.contains("scan"));
        assert!(instructions.contains("explain_rule"));
        assert!(instructions.contains("list_rules"));
        assert!(instructions.contains("score"));
    }

    // --- Prompt registration ---

    #[test]
    fn test_prompt_router_has_all_prompts() {
        let server = RustDoctorServer::new();
        let prompts = server.prompt_router.list_all();
        let names: Vec<&str> = prompts.iter().map(|p| &*p.name).collect();
        assert!(names.contains(&"deep-audit"), "Missing deep-audit prompt");
        assert!(
            names.contains(&"health-check"),
            "Missing health-check prompt"
        );
        assert_eq!(
            names.len(),
            2,
            "Expected exactly 2 prompts, got {}",
            names.len()
        );
    }

    #[test]
    fn test_deep_audit_prompt_registered_with_description() {
        let server = RustDoctorServer::new();
        let prompts = server.prompt_router.list_all();
        let deep_audit = prompts.iter().find(|p| p.name == "deep-audit").unwrap();
        let desc = deep_audit.description.as_deref().unwrap_or("");
        assert!(
            desc.contains("audit"),
            "deep-audit description should mention audit"
        );
        assert!(
            desc.contains("best practices"),
            "deep-audit description should mention best practices"
        );
    }

    #[test]
    fn test_server_info_mentions_deep_audit() {
        let server = RustDoctorServer::new();
        let info = <RustDoctorServer as rmcp::handler::server::ServerHandler>::get_info(&server);
        let instructions = info.instructions.as_deref().unwrap_or("");
        assert!(
            instructions.contains("deep-audit"),
            "Server instructions should mention deep-audit prompt"
        );
        assert!(
            instructions.contains("health-check"),
            "Server instructions should mention health-check prompt"
        );
    }

    /// Extract text from a `PromptMessageContent::Text` variant.
    fn extract_prompt_text(content: &rmcp::model::PromptMessageContent) -> &str {
        match content {
            rmcp::model::PromptMessageContent::Text { text } => text,
            _ => panic!("expected Text content in prompt message"),
        }
    }

    #[tokio::test]
    async fn test_deep_audit_prompt_content() {
        let server = RustDoctorServer::new();
        let result = server
            .deep_audit(Parameters(DeepAuditArgs {
                directory: "/home/user/my-project".to_string(),
            }))
            .await;
        assert_eq!(result.messages.len(), 1);
        assert!(result.description.is_some());
        let text = extract_prompt_text(&result.messages[0].content);
        // Directory is interpolated
        assert!(
            text.contains("/home/user/my-project"),
            "directory should be interpolated into prompt"
        );
        // All 6 phases present
        for phase in 1..=6 {
            assert!(
                text.contains(&format!("PHASE {phase}")),
                "Missing PHASE {phase} in prompt"
            );
        }
        // Decision options present
        assert!(text.contains("Implement all fixes"));
        assert!(text.contains("Generate a PRD"));
        assert!(text.contains("Manual"));
        // Hard rules present
        assert!(text.contains("HARD RULES"));
    }

    #[tokio::test]
    async fn test_health_check_prompt_content() {
        let server = RustDoctorServer::new();
        let result = server
            .health_check(Parameters(HealthCheckArgs {
                directory: "/home/user/test".to_string(),
            }))
            .await;
        assert_eq!(result.messages.len(), 1);
        let text = extract_prompt_text(&result.messages[0].content);
        assert!(
            text.contains("/home/user/test"),
            "directory should be interpolated"
        );
        assert!(text.contains("Phase 1"));
        assert!(text.contains("Phase 4"));
    }

    // --- Tool registration ---

    #[test]
    fn test_tool_router_has_all_tools() {
        let server = RustDoctorServer::new();
        let tools = server.tool_router.list_all();
        let names: Vec<&str> = tools.iter().map(|t| &*t.name).collect();
        assert!(names.contains(&"scan"), "Missing scan tool");
        assert!(names.contains(&"score"), "Missing score tool");
        assert!(names.contains(&"explain_rule"), "Missing explain_rule tool");
        assert!(names.contains(&"list_rules"), "Missing list_rules tool");
        assert_eq!(
            names.len(),
            4,
            "Expected exactly 4 tools, got {}",
            names.len()
        );
    }

    #[test]
    fn test_scan_tool_returns_call_tool_result() {
        // scan returns CallToolResult (text summary + structuredContent),
        // not Json<T>, so it has no auto-generated outputSchema.
        let server = RustDoctorServer::new();
        let tools = server.tool_router.list_all();
        let scan = tools.iter().find(|t| t.name == "scan").unwrap();
        assert!(
            scan.output_schema.is_none(),
            "scan uses CallToolResult, not Json<T>"
        );
    }

    #[test]
    fn test_score_tool_has_output_schema() {
        let server = RustDoctorServer::new();
        let tools = server.tool_router.list_all();
        let score = tools.iter().find(|t| t.name == "score").unwrap();
        assert!(
            score.output_schema.is_some(),
            "score tool should have outputSchema from Json<ScoreOutput>"
        );
    }

    // --- Tool annotations ---

    #[test]
    fn test_all_tools_have_correct_annotations() {
        let server = RustDoctorServer::new();
        let tools = server.tool_router.list_all();
        for tool in &tools {
            let ann = tool
                .annotations
                .as_ref()
                .unwrap_or_else(|| panic!("tool '{}' missing annotations", tool.name));
            assert_eq!(
                ann.read_only_hint,
                Some(true),
                "tool '{}' should be read-only",
                tool.name
            );
            assert_eq!(
                ann.destructive_hint,
                Some(false),
                "tool '{}' should not be destructive",
                tool.name
            );
            assert_eq!(
                ann.idempotent_hint,
                Some(true),
                "tool '{}' should be idempotent",
                tool.name
            );
            assert_eq!(
                ann.open_world_hint,
                Some(false),
                "tool '{}' should be closed-world",
                tool.name
            );
            assert!(
                ann.title.is_some(),
                "tool '{}' should have a title",
                tool.name
            );
        }
    }

    // --- discover_and_resolve error mapping ---

    #[test]
    fn test_discover_and_resolve_invalid_path() {
        let result = discover_and_resolve("/nonexistent/path/to/project", false);
        assert!(result.is_err());
        let err = result.unwrap_err();
        // Should be invalid_params (not internal_error) for bad input
        assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    }

    #[test]
    fn test_discover_and_resolve_error_does_not_contain_raw_path() {
        let result = discover_and_resolve("/nonexistent/path/to/project", false);
        let err = result.unwrap_err();
        let msg = err.message.to_string();
        // Sanitized: must NOT contain the raw filesystem path
        assert!(
            !msg.contains("/nonexistent/path"),
            "MCP error should not contain raw path, got: {msg}"
        );
    }

    #[test]
    fn test_discover_and_resolve_outside_home() {
        // /tmp is typically outside $HOME — should be rejected
        if std::env::var("HOME").is_ok() {
            let result = discover_and_resolve("/etc", false);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        }
    }

    // --- MCP e2e: scan + score on a bounded project ---

    #[test]
    fn test_scan_tool_on_fixture() {
        let (_fixture, project_info, resolved) = isolated_scan_fixture();
        let scan_result = scan::scan_project(&project_info, &resolved, true, &[], true);
        assert!(scan_result.is_ok(), "scan_project failed: {scan_result:?}");
        let result = scan_result.unwrap();
        // Verify ScanResult structure
        assert!(result.score <= 100);
        assert!(result.source_file_count > 0);
    }

    // --- Diagnostic grouping unit tests ---

    fn make_diagnostic(
        rule: &str,
        severity: crate::diagnostics::Severity,
        help: Option<&str>,
    ) -> Diagnostic {
        Diagnostic {
            file_path: std::path::PathBuf::from("src/lib.rs"),
            rule: rule.to_string(),
            category: crate::diagnostics::Category::ErrorHandling,
            severity,
            message: format!("test finding for {rule}"),
            help: help.map(String::from),
            line: Some(1),
            column: None,
            fix: None,
        }
    }

    #[test]
    fn test_group_diagnostics_empty() {
        let groups = group_diagnostics(&[]);
        assert!(groups.is_empty());
    }

    #[test]
    fn test_group_diagnostics_single() {
        let diag = make_diagnostic("rule-a", crate::diagnostics::Severity::Error, None);
        let groups = group_diagnostics(&[diag]);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].count, 1);
        assert_eq!(groups[0].examples.len(), 1);
    }

    #[test]
    fn test_group_diagnostics_caps_examples() {
        let diags: Vec<_> = (0..10)
            .map(|_| make_diagnostic("rule-a", crate::diagnostics::Severity::Warning, None))
            .collect();
        let groups = group_diagnostics(&diags);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].count, 10);
        assert_eq!(groups[0].examples.len(), MAX_EXAMPLES_PER_GROUP);
    }

    #[test]
    fn test_group_diagnostics_sorts_errors_first() {
        let diags = vec![
            make_diagnostic("warn-rule", crate::diagnostics::Severity::Warning, None),
            make_diagnostic("info-rule", crate::diagnostics::Severity::Info, None),
            make_diagnostic("err-rule", crate::diagnostics::Severity::Error, None),
        ];
        let groups = group_diagnostics(&diags);
        assert_eq!(groups[0].severity, "error");
        assert_eq!(groups[1].severity, "warning");
        assert_eq!(groups[2].severity, "info");
    }

    #[test]
    fn test_group_diagnostics_help_finds_first_non_none() {
        let diags = vec![
            make_diagnostic("rule-a", crate::diagnostics::Severity::Warning, None),
            make_diagnostic(
                "rule-a",
                crate::diagnostics::Severity::Warning,
                Some("fix it"),
            ),
            make_diagnostic("rule-a", crate::diagnostics::Severity::Warning, None),
        ];
        let groups = group_diagnostics(&diags);
        assert_eq!(groups[0].help.as_deref(), Some("fix it"));
    }

    // --- Integration test: grouping on real project ---

    #[test]
    fn test_scan_output_grouping() {
        let (_fixture, project_info, resolved) = isolated_scan_fixture();
        let result = scan::scan_project(&project_info, &resolved, true, &[], true).unwrap();

        let total = result.diagnostics.len();
        let grouped = group_diagnostics(&result.diagnostics);
        let report = format_scan_report(&result, &grouped);

        // Grouping reduces count
        assert!(
            grouped.len() < total,
            "grouping should compress: {} groups from {} diagnostics",
            grouped.len(),
            total
        );
        // Each group has examples
        for g in &grouped {
            assert!(!g.examples.is_empty(), "group '{}' has no examples", g.rule);
            assert!(
                g.examples.len() <= MAX_EXAMPLES_PER_GROUP,
                "group '{}' has too many examples",
                g.rule
            );
            assert!(g.count > 0);
        }
        // Report is non-empty and contains score
        assert!(report.contains(&result.score.to_string()));
        // Sorted: errors before warnings before info
        let severities: Vec<&str> = grouped.iter().map(|g| g.severity.as_str()).collect();
        for window in severities.windows(2) {
            let ord = |s: &str| -> u8 {
                match s {
                    "error" => 0,
                    "warning" => 1,
                    _ => 2,
                }
            };
            assert!(
                ord(window[0]) <= ord(window[1]),
                "groups not sorted by severity: {} before {}",
                window[0],
                window[1]
            );
        }
    }

    #[test]
    fn test_score_output_structure() {
        let (_fixture, project_info, resolved) = isolated_scan_fixture();
        let result = scan::scan_project(&project_info, &resolved, true, &[], true).unwrap();
        let output = ScoreOutput {
            score: result.score,
            score_label: result.score_label,
        };
        assert!(output.score <= 100);
        // Verify it serializes correctly
        let json = serde_json::to_value(&output).unwrap();
        assert!(json.get("score").is_some());
        assert!(json.get("score_label").is_some());
    }

    // --- US-009: MCP timeout wrapper & spawn_blocking integration tests ---

    #[tokio::test]
    async fn test_scan_via_spawn_blocking_completes() {
        let (_fixture, project_info, resolved) = isolated_scan_fixture();

        let result = tokio::task::spawn_blocking(move || {
            scan::scan_project(&project_info, &resolved, true, &[], true)
        })
        .await;

        assert!(result.is_ok(), "spawn_blocking should not panic");
        let scan_result = result.unwrap();
        assert!(scan_result.is_ok(), "scan_project should succeed");
        let scan = scan_result.unwrap();
        assert!(
            scan.score <= 100,
            "score should be 0-100, got {}",
            scan.score
        );
    }

    #[tokio::test]
    async fn test_timeout_fires_for_slow_task() {
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(1),
            tokio::task::spawn_blocking(|| {
                std::thread::sleep(std::time::Duration::from_secs(2));
                42
            }),
        )
        .await;

        assert!(result.is_err(), "Expected timeout error");
    }

    #[tokio::test]
    async fn test_spawn_blocking_panic_is_caught() {
        let result = tokio::task::spawn_blocking(|| {
            panic!("intentional test panic");
        })
        .await;

        assert!(result.is_err(), "Expected JoinError from panic");
    }

    #[tokio::test]
    async fn test_mcp_scan_pipeline_produces_valid_result() {
        let (_fixture, project_info, resolved) = isolated_scan_fixture();

        let offline = true;
        let result = tokio::time::timeout(
            std::time::Duration::from_mins(5),
            tokio::task::spawn_blocking(move || {
                scan::scan_project(&project_info, &resolved, offline, &[], true)
            }),
        )
        .await
        .expect("scan should not time out")
        .expect("spawn_blocking should not panic")
        .expect("scan_project should succeed");

        assert!(
            result.score <= 100,
            "score should be 0-100, got {}",
            result.score
        );
        assert!(
            !result.diagnostics.is_empty(),
            "rust-doctor always has some findings on itself"
        );
        assert!(
            result.source_file_count > 0,
            "should have scanned at least one source file"
        );
    }

    // --- US-007: cooperative cancellation actually stops the work ---

    #[test]
    fn test_cancellation_stops_scan_work() {
        use std::sync::Arc;
        use std::sync::atomic::AtomicBool;

        let (_fixture, project_info, resolved) = isolated_scan_fixture();

        // Pre-cancelled: run_passes must break before launching any pass, so no
        // diagnostics and no files are scanned — proves the WORK stops, not just
        // that the caller sees an error (fixes the prior timeout-only coverage).
        let cancelled = Arc::new(AtomicBool::new(true));
        let stopped =
            scan::scan_project_cancellable(&project_info, &resolved, true, &[], true, &cancelled)
                .unwrap();
        assert_eq!(
            stopped.diagnostics.len(),
            0,
            "a cancelled scan must not run any passes"
        );
        assert_eq!(stopped.source_file_count, 0, "no files should be scanned");

        // Sanity: the same project with a live (never-set) flag does real work.
        let live = Arc::new(AtomicBool::new(false));
        let full = scan::scan_project_cancellable(&project_info, &resolved, true, &[], true, &live)
            .unwrap();
        assert!(
            full.source_file_count > 0 && !full.diagnostics.is_empty(),
            "a live scan should scan files and find diagnostics"
        );
    }

    // --- US-009: score honors ignore_project_config ---

    #[test]
    fn test_score_input_ignore_project_config_defaults_false() {
        let input: super::types::ScoreInput =
            serde_json::from_value(serde_json::json!({ "directory": "/x" })).unwrap();
        assert!(
            !input.ignore_project_config,
            "ignore_project_config must default to false (aligned with ScanInput)"
        );
    }

    #[test]
    fn test_score_input_ignore_project_config_parsed() {
        let input: super::types::ScoreInput = serde_json::from_value(
            serde_json::json!({ "directory": "/x", "ignore_project_config": true }),
        )
        .unwrap();
        assert!(input.ignore_project_config);
    }
}
