//! Pure, ownership-aware MCP configuration edits.

use super::McpLaunch;
use serde_json::{Map, Value, json};
use std::fs;
use std::path::Path;
use toml_edit::{Array, DocumentMut, Item, Table, value};

/// Configuration shape used by an agent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum McpFormat {
    /// Claude, Cursor and Windsurf use `mcpServers` JSON.
    StandardJson,
    /// OpenCode uses a local command under the `mcp` JSON object.
    OpenCodeJson,
    /// Codex uses `mcp_servers` TOML tables.
    CodexToml,
}

/// Render an MCP install. `Ok(None)` means the existing file is already the
/// exact managed state and must not be rewritten.
pub(super) fn install(
    content: Option<&str>,
    format: McpFormat,
    launch: &McpLaunch,
) -> Result<Option<String>, String> {
    match format {
        McpFormat::StandardJson => install_json(content, "mcpServers", launch, false),
        McpFormat::OpenCodeJson => install_json(content, "mcp", launch, true),
        McpFormat::CodexToml => install_codex(content, launch),
    }
}

/// Render an MCP uninstall. Only an entry exactly matching `launch` is
/// considered owned. A same-name custom entry is a conflict.
pub(super) fn uninstall(
    content: Option<&str>,
    format: McpFormat,
    launch: &McpLaunch,
) -> Result<Option<String>, String> {
    match format {
        McpFormat::StandardJson => uninstall_json(content, "mcpServers", launch, false),
        McpFormat::OpenCodeJson => uninstall_json(content, "mcp", launch, true),
        McpFormat::CodexToml => uninstall_codex(content, launch),
    }
}

pub(super) fn contains_namespace(path: &Path, format: McpFormat) -> bool {
    let Ok(content) = fs::read_to_string(path) else {
        return false;
    };

    match format {
        McpFormat::StandardJson => json_namespace_exists(&content, "mcpServers"),
        McpFormat::OpenCodeJson => json_namespace_exists(&content, "mcp"),
        McpFormat::CodexToml => content
            .parse::<DocumentMut>()
            .ok()
            .and_then(|document| {
                document
                    .get("mcp_servers")
                    .and_then(Item::as_table)
                    .map(|servers| servers.contains_key("rust-doctor"))
            })
            .unwrap_or(false),
    }
}

fn install_json(
    content: Option<&str>,
    namespace: &str,
    launch: &McpLaunch,
    opencode: bool,
) -> Result<Option<String>, String> {
    let mut config = parse_json_object(content)?;
    let desired = json_entry(launch, opencode);

    if !config.contains_key(namespace) {
        config.insert(namespace.to_owned(), Value::Object(Map::new()));
    }
    let servers = config
        .get_mut(namespace)
        .and_then(Value::as_object_mut)
        .ok_or_else(|| format!("`{namespace}` exists but is not a JSON object"))?;

    if let Some(existing) = servers.get("rust-doctor") {
        if existing == &desired {
            return Ok(None);
        }
        if json_entry_is_managed(existing, opencode) {
            servers.insert("rust-doctor".to_owned(), desired);
            return serialize_json(config).map(Some);
        }
        return Err(format!(
            "the `rust-doctor` MCP namespace already contains an unmanaged entry: {existing}"
        ));
    }

    servers.insert("rust-doctor".to_owned(), desired);
    serialize_json(config).map(Some)
}

fn uninstall_json(
    content: Option<&str>,
    namespace: &str,
    launch: &McpLaunch,
    opencode: bool,
) -> Result<Option<String>, String> {
    let Some(content) = content else {
        return Ok(None);
    };
    let mut config = parse_json_object(Some(content))?;
    let Some(servers) = config.get_mut(namespace) else {
        return Ok(None);
    };
    let servers = servers
        .as_object_mut()
        .ok_or_else(|| format!("`{namespace}` exists but is not a JSON object"))?;
    let Some(existing) = servers.get("rust-doctor") else {
        return Ok(None);
    };
    let desired = json_entry(launch, opencode);
    if existing != &desired && !json_entry_is_managed(existing, opencode) {
        return Err(format!(
            "refusing to remove unmanaged `rust-doctor` MCP entry: {existing}"
        ));
    }

    servers.remove("rust-doctor");
    serialize_json(config).map(Some)
}

fn parse_json_object(content: Option<&str>) -> Result<Map<String, Value>, String> {
    let Some(content) = content else {
        return Ok(Map::new());
    };
    let parsed = serde_json::from_str::<Value>(content)
        .map_err(|error| format!("malformed JSON configuration: {error}"))?;
    parsed
        .as_object()
        .cloned()
        .ok_or_else(|| "configuration root is not a JSON object".to_owned())
}

fn serialize_json(config: Map<String, Value>) -> Result<String, String> {
    let mut output = serde_json::to_string_pretty(&Value::Object(config))
        .map_err(|error| format!("failed to serialize JSON configuration: {error}"))?;
    output.push('\n');
    Ok(output)
}

fn json_entry(launch: &McpLaunch, opencode: bool) -> Value {
    if opencode {
        let mut command = Vec::with_capacity(launch.args.len() + 1);
        command.push(launch.command.clone());
        command.extend(launch.args.iter().cloned());
        json!({
            "type": "local",
            "command": command,
            "enabled": true,
        })
    } else {
        json!({
            "command": launch.command,
            "args": launch.args,
        })
    }
}

fn json_entry_is_managed(entry: &Value, opencode: bool) -> bool {
    if opencode {
        return entry
            == &json!({
                "type": "local",
                "command": ["rust-doctor", "--mcp"],
                "enabled": true,
            });
    }
    entry
        == &json!({
            "command": "rust-doctor",
            "args": ["--mcp"],
        })
        || entry
            == &json!({
                "command": "npx",
                "args": ["-y", "rust-doctor@latest", "--mcp"],
            })
}

fn json_namespace_exists(content: &str, namespace: &str) -> bool {
    serde_json::from_str::<Value>(content)
        .ok()
        .and_then(|config| config.get(namespace).cloned())
        .and_then(|servers| servers.get("rust-doctor").cloned())
        .is_some()
}

fn install_codex(content: Option<&str>, launch: &McpLaunch) -> Result<Option<String>, String> {
    let mut document = parse_toml(content)?;
    ensure_server_table(&mut document)?;
    let servers = document
        .get_mut("mcp_servers")
        .and_then(Item::as_table_mut)
        .ok_or_else(|| "`mcp_servers` exists but is not a TOML table".to_owned())?;

    if let Some(existing) = servers.get("rust-doctor") {
        if codex_entry_matches(existing, launch) {
            return Ok(None);
        }
        return Err(
            "the `mcp_servers.rust-doctor` namespace already contains an unmanaged entry"
                .to_owned(),
        );
    }

    servers.insert("rust-doctor", Item::Table(codex_server_table(launch)));
    Ok(Some(document.to_string()))
}

fn uninstall_codex(content: Option<&str>, launch: &McpLaunch) -> Result<Option<String>, String> {
    let Some(content) = content else {
        return Ok(None);
    };
    let mut document = parse_toml(Some(content))?;
    let Some(servers_item) = document.get_mut("mcp_servers") else {
        return Ok(None);
    };
    let servers = servers_item
        .as_table_mut()
        .ok_or_else(|| "`mcp_servers` exists but is not a TOML table".to_owned())?;
    let Some(existing) = servers.get("rust-doctor") else {
        return Ok(None);
    };
    if !codex_entry_matches(existing, launch) {
        return Err("refusing to remove unmanaged `mcp_servers.rust-doctor` entry".to_owned());
    }

    servers.remove("rust-doctor");
    Ok(Some(document.to_string()))
}

fn parse_toml(content: Option<&str>) -> Result<DocumentMut, String> {
    content.map_or_else(
        || Ok(DocumentMut::new()),
        |content| {
            content
                .parse::<DocumentMut>()
                .map_err(|error| format!("malformed TOML configuration: {error}"))
        },
    )
}

fn ensure_server_table(document: &mut DocumentMut) -> Result<(), String> {
    if document.get("mcp_servers").is_none() {
        document.insert("mcp_servers", Item::Table(Table::new()));
    }
    if document
        .get("mcp_servers")
        .and_then(Item::as_table)
        .is_none()
    {
        return Err("`mcp_servers` exists but is not a TOML table".to_owned());
    }
    Ok(())
}

fn codex_server_table(launch: &McpLaunch) -> Table {
    let mut server = Table::new();
    server.insert("command", value(&launch.command));
    let mut args = Array::new();
    for argument in &launch.args {
        args.push(argument.as_str());
    }
    server.insert("args", value(args));
    server
}

fn codex_entry_matches(entry: &Item, launch: &McpLaunch) -> bool {
    let Some(table) = entry.as_table() else {
        return false;
    };
    if table.len() != 2 || !table.contains_key("command") || !table.contains_key("args") {
        return false;
    }
    if table.get("command").and_then(Item::as_str) != Some(launch.command.as_str()) {
        return false;
    }
    let Some(args) = table.get("args").and_then(Item::as_array) else {
        return false;
    };
    args.len() == launch.args.len()
        && args
            .iter()
            .zip(&launch.args)
            .all(|(actual, expected)| actual.as_str() == Some(expected.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn launch() -> McpLaunch {
        McpLaunch::default()
    }

    #[test]
    fn standard_json_preserves_unrelated_servers_and_is_idempotent() {
        let input = r#"{"theme":"dark","mcpServers":{"other":{"command":"other"}}}"#;
        let first = install(Some(input), McpFormat::StandardJson, &launch())
            .expect("valid install")
            .expect("changed config");
        let parsed: Value = serde_json::from_str(&first).expect("generated JSON");
        assert_eq!(parsed["theme"], "dark");
        assert_eq!(parsed["mcpServers"]["other"]["command"], "other");
        assert_eq!(
            parsed["mcpServers"]["rust-doctor"]["command"],
            "rust-doctor"
        );
        assert_eq!(
            install(Some(&first), McpFormat::StandardJson, &launch()).expect("second install"),
            None
        );
    }

    #[test]
    fn malformed_json_is_refused_without_fallback() {
        let error = install(
            Some("{ definitely not json"),
            McpFormat::StandardJson,
            &launch(),
        )
        .expect_err("malformed config must fail");
        assert!(error.contains("malformed JSON"));
    }

    #[test]
    fn conflicting_json_namespace_is_refused() {
        let input = r#"{"mcpServers":{"rust-doctor":{"command":"custom"}}}"#;
        let error = install(Some(input), McpFormat::StandardJson, &launch())
            .expect_err("custom namespace must fail");
        assert!(error.contains("unmanaged"));
    }

    #[test]
    fn legacy_npx_entry_is_upgraded_as_managed_content() {
        let input = r#"{"mcpServers":{"rust-doctor":{"command":"npx","args":["-y","rust-doctor@latest","--mcp"]}}}"#;
        let output = install(Some(input), McpFormat::StandardJson, &launch())
            .expect("legacy upgrade")
            .expect("changed config");
        let parsed: Value = serde_json::from_str(&output).expect("generated JSON");
        assert_eq!(
            parsed["mcpServers"]["rust-doctor"]["command"],
            "rust-doctor"
        );
    }

    #[test]
    fn opencode_uses_local_command_array() {
        let output = install(None, McpFormat::OpenCodeJson, &launch())
            .expect("valid install")
            .expect("new config");
        let parsed: Value = serde_json::from_str(&output).expect("generated JSON");
        assert_eq!(
            parsed["mcp"]["rust-doctor"]["command"],
            json!(["rust-doctor", "--mcp"])
        );
    }

    #[test]
    fn codex_preserves_comments_and_unrelated_tables() {
        let input = "# user comment\nmodel = \"gpt-5\"\n\n[other]\nenabled = true\n";
        let output = install(Some(input), McpFormat::CodexToml, &launch())
            .expect("valid TOML")
            .expect("changed TOML");
        assert!(output.contains("# user comment"));
        assert!(output.contains("[other]"));
        assert!(output.contains("[mcp_servers.rust-doctor]"));
        assert_eq!(
            install(Some(&output), McpFormat::CodexToml, &launch()).expect("second install"),
            None
        );
    }

    #[test]
    fn uninstall_removes_only_managed_entry() {
        let installed = install(
            Some(r#"{"mcpServers":{"other":{"command":"other"}}}"#),
            McpFormat::StandardJson,
            &launch(),
        )
        .expect("install")
        .expect("changed config");
        let removed = uninstall(Some(&installed), McpFormat::StandardJson, &launch())
            .expect("uninstall")
            .expect("changed config");
        let parsed: Value = serde_json::from_str(&removed).expect("generated JSON");
        assert!(parsed["mcpServers"]["other"].is_object());
        assert!(parsed["mcpServers"].get("rust-doctor").is_none());
    }
}
