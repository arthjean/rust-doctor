# Rust Doctor for VS Code and Cursor

The extension launches `rust-doctor --lsp` for Rust files. Set `rustDoctor.binaryPath` when the binary is not on `PATH`; the selected binary must be built with the `lsp` feature. File-local diagnostics are enabled by default. Offline project checks remain opt-in through `rustDoctor.onSaveProjectChecks`.

No telemetry is sent.
