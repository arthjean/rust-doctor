use rmcp::handler::server::router::prompt::PromptRouter;
use rmcp::handler::server::tool::ToolRouter;
use rmcp::model::{
    AnnotateAble, GetPromptRequestParams, GetPromptResult, ListPromptsResult, ListResourcesResult,
    PaginatedRequestParams, RawResource, ReadResourceRequestParams, ReadResourceResult, Resource,
    ResourceContents, ServerCapabilities, ServerInfo,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, prompt_handler};

use super::rules::{get_rule_explanation, rule_docs};

// ---------------------------------------------------------------------------
// MCP server struct
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct RustDoctorServer {
    #[allow(
        dead_code,
        reason = "rmcp's generated tool handler consumes this router outside compiler-visible calls"
    )]
    pub(super) tool_router: ToolRouter<Self>,
    pub(super) prompt_router: PromptRouter<Self>,
}

// ---------------------------------------------------------------------------
// ServerHandler implementation
// ---------------------------------------------------------------------------

#[rmcp::tool_handler]
#[prompt_handler(router = self.prompt_router)]
impl rmcp::handler::server::ServerHandler for RustDoctorServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .enable_prompts()
                .enable_logging()
                .build(),
        )
        .with_instructions(
            "rust-doctor is a Rust code health scanner. It analyzes projects for security, \
             performance, correctness, architecture, and dependency issues.\n\n\
             ## Recommended workflow\n\
             1. `scan` a project directory → get diagnostics + score (5-30s)\n\
             2. `explain_rule` for any rule you want to understand → instant\n\
             3. `list_rules` to browse all available checks → instant\n\
             4. `score` for a quick pass/fail without diagnostics (same 5-30s as scan)\n\n\
             ## Resources\n\
             - `rule://` — read rule documentation by URI (e.g. `rule://unwrap-in-production`)\n\n\
             ## Prompts\n\
             - `deep-audit` — comprehensive expert audit: explores codebase, scans, deep code review, \
             web research for best practices, synthesis report, then offers to implement all fixes / generate PRD / manual\n\
             - `health-check` — quick health check with scan + prioritized remediation plan + fix workflow\n\n\
             ## Tips\n\
             - Prefer `scan` over `score` — it includes the score plus full diagnostics\n\
             - Use `diff` parameter in scan to focus on changed files only\n\
             - All tools are read-only and safe to call repeatedly\n\
             - `explain_rule` and `list_rules` are instant (no project scanning)",
        )
    }

    async fn list_resources(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        let docs = rule_docs();
        let resources: Vec<Resource> = docs
            .iter()
            .map(|doc| {
                RawResource::new(format!("rule://{}", doc.name), doc.name)
                    .with_description(format!("[{}] {}", doc.severity, doc.description))
                    .with_mime_type("text/markdown")
                    .no_annotation()
            })
            .collect();

        Ok(ListResourcesResult {
            resources,
            next_cursor: None,
            meta: None,
        })
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, McpError> {
        let uri = request.uri.as_str();
        let rule_name = uri.strip_prefix("rule://").ok_or_else(|| {
            McpError::invalid_params(
                format!("Unknown URI scheme: {uri}. Expected rule://{{rule-name}}"),
                None,
            )
        })?;

        let explanation = get_rule_explanation(rule_name);
        Ok(ReadResourceResult::new(vec![
            ResourceContents::text(explanation, uri).with_mime_type("text/markdown"),
        ]))
    }
}
