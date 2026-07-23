/// Prompt template for the deep-audit workflow.
/// Interpolated with `directory` at runtime.
pub fn deep_audit_prompt(directory: &str) -> String {
    format!(include_str!("prompts/deep_audit.md"), directory = directory)
}

/// Prompt template for the health-check workflow.
/// Interpolated with `directory` at runtime.
pub fn health_check_prompt(directory: &str) -> String {
    format!(
        include_str!("prompts/health_check.md"),
        directory = directory
    )
}
