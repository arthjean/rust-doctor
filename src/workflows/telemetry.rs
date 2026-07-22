use crate::cli::TelemetryCommand;
use crate::telemetry::{self, TelemetryError};
use dialoguer::Confirm;
use dialoguer::theme::ColorfulTheme;

pub(super) fn handle(command: &TelemetryCommand) -> Result<String, TelemetryError> {
    match command {
        TelemetryCommand::Enable(arguments) => {
            telemetry::validate_endpoint(&arguments.endpoint)
                .map_err(TelemetryError::InvalidEndpoint)?;
            if !arguments.yes {
                eprintln!(
                    "Events contain tool version, platform, invocation surface, duration bucket, completeness, aggregate counts, pass states, and suppression count."
                );
                eprintln!(
                    "They never contain source, paths, repository names, diagnostics, remotes, environment values, or command arguments."
                );
                let confirmed = Confirm::with_theme(&ColorfulTheme::default())
                    .with_prompt("Enable aggregate Rust Doctor observability for this endpoint?")
                    .default(false)
                    .interact()
                    .map_err(TelemetryError::Prompt)?;
                if !confirmed {
                    return Ok("Telemetry remains disabled.".to_string());
                }
            }
            telemetry::enable(&arguments.endpoint)?;
            Ok("Telemetry enabled. Events are attempted once, never queued locally, and contain no cross-project identifier.".to_string())
        }
        TelemetryCommand::Disable => {
            telemetry::disable()?;
            Ok("Telemetry disabled and local consent removed.".to_string())
        }
        TelemetryCommand::Status => Ok(telemetry::status()),
    }
}
