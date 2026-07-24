# Rust Doctor for VS Code and Cursor

The extension launches `rust-doctor --lsp` for Rust files. Set `rustDoctor.binaryPath` when the binary is not on `PATH`; the selected binary must be built with the `lsp` feature and support Rust Doctor editor protocol major 1. File-local diagnostics are enabled by default. Offline project checks remain opt-in through `rustDoctor.onSaveProjectChecks`.

`rustDoctor.configurationPath` is empty by default, which uses normal project discovery and works without `rust-doctor.toml`. Set it only when a specific project-relative configuration file must exist; a missing explicit path disables diagnostics with one actionable startup error.

No telemetry is sent.
