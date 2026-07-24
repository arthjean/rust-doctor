use std::collections::HashMap;
use zed_extension_api::{self as zed, Architecture, LanguageServerId, Worktree};

const PROTOCOL_MAJOR: u32 = 1;

struct RustDoctorExtension {
    failed_binary: Option<String>,
}

impl zed::Extension for RustDoctorExtension {
    fn new() -> Self {
        Self {
            failed_binary: None,
        }
    }

    fn language_server_command(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &Worktree,
    ) -> zed::Result<zed::Command> {
        let (_, architecture) = zed::current_platform();
        if matches!(architecture, Architecture::X86) {
            return Err(
                "Rust Doctor does not publish a 32-bit editor binary; configure a supported x64 or arm64 binary"
                    .to_string(),
            );
        }
        let settings = zed::settings::LspSettings::for_worktree("rust-doctor", worktree)?;
        let binary = settings.binary;
        let command = binary
            .as_ref()
            .and_then(|binary| binary.path.clone())
            .unwrap_or_else(|| "rust-doctor".to_string());
        if self.failed_binary.as_deref() == Some(&command) {
            return Err(
                "Rust Doctor diagnostics remain disabled; fix the previously reported binary path or version"
                    .to_string(),
            );
        }
        let mut environment = binary
            .as_ref()
            .and_then(|binary| binary.env.clone())
            .unwrap_or_else(HashMap::new);
        environment.insert("RUST_DOCTOR_SELECTED_BINARY".to_string(), command.clone());
        let version = zed::process::Command::new(command.clone())
            .arg("version")
            .envs(environment.clone())
            .output();
        let compatible = version.as_ref().is_ok_and(|output| {
            let text = String::from_utf8_lossy(&output.stdout);
            let parsed = text
                .lines()
                .next()
                .and_then(|line| line.strip_prefix("rust-doctor "))
                .and_then(|version| {
                    let mut components = version.split('.');
                    Some((
                        components.next()?.parse::<u32>().ok()?,
                        components.next()?.parse::<u32>().ok()?,
                    ))
                });
            let protocol = text.lines().find_map(|line| {
                line.strip_prefix("lsp-protocol ")
                    .and_then(|value| value.parse::<u32>().ok())
            });
            output.status == Some(0)
                && parsed.is_some_and(|(major, minor)| major > 0 || minor >= 2)
                && protocol == Some(PROTOCOL_MAJOR)
        });
        if !compatible {
            self.failed_binary = Some(command.clone());
            let reason = version.map_or_else(
                |error| error,
                |output| String::from_utf8_lossy(&output.stderr).trim().to_string(),
            );
            return Err(format!(
                "Rust Doctor diagnostics disabled: '{command}' is missing, non-executable, older than 0.2.0, or does not support LSP protocol {PROTOCOL_MAJOR}. {reason}"
            ));
        }
        let mut arguments = binary
            .and_then(|binary| binary.arguments)
            .unwrap_or_default();
        if !arguments.iter().any(|argument| argument == "--lsp") {
            arguments.push("--lsp".to_string());
        }
        eprintln!(
            "rust-doctor: selected language server binary '{command}' for {}",
            language_server_id.as_ref()
        );
        Ok(zed::Command {
            command,
            args: arguments,
            env: environment.into_iter().collect(),
        })
    }

    fn language_server_initialization_options(
        &mut self,
        _language_server_id: &LanguageServerId,
        worktree: &Worktree,
    ) -> zed::Result<Option<zed::serde_json::Value>> {
        let settings = zed::settings::LspSettings::for_worktree("rust-doctor", worktree)?;
        let mut options = settings.initialization_options.unwrap_or_else(|| {
            zed::serde_json::json!({
                "debounceMs": 300,
                "onSaveProjectChecks": false,
                "projectBudgetMs": 10000
            })
        });
        let object = options.as_object_mut().ok_or_else(|| {
            "Rust Doctor initialization options must be a JSON object".to_string()
        })?;
        object
            .entry("protocolMajor".to_string())
            .or_insert_with(|| zed::serde_json::json!(PROTOCOL_MAJOR));
        Ok(Some(options))
    }
}

zed::register_extension!(RustDoctorExtension);
