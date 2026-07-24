# Rust Doctor for Zed

The adapter launches `rust-doctor --lsp` for Rust buffers. Zed first uses `lsp.rust-doctor.binary.path`; without an explicit path it resolves `rust-doctor` through the editor process environment. The binary must include the `lsp` feature and support Rust Doctor editor protocol major 1.

Shared initialization options live under `lsp.rust-doctor.initialization_options`:

```json
{
  "protocolMajor": 1,
  "debounceMs": 300,
  "onSaveProjectChecks": false,
  "projectBudgetMs": 10000
}
```

Omitting `configurationPath` uses normal project discovery and defaults. Set it only to require a specific project-relative configuration file.

No telemetry is sent.
