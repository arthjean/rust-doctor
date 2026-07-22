# Rust Doctor for Zed

The adapter launches `rust-doctor --lsp` for Rust buffers. Zed first uses `lsp.rust-doctor.binary.path`; without an explicit path it resolves `rust-doctor` through the editor process environment. The binary must include the `lsp` feature.

Shared initialization options live under `lsp.rust-doctor.initialization_options`:

```json
{
  "debounceMs": 300,
  "onSaveProjectChecks": false,
  "projectBudgetMs": 10000,
  "configurationPath": "rust-doctor.toml"
}
```

No telemetry is sent.
