import { describe, expect, test } from "bun:test";

const root = new URL("..", import.meta.url);
const manifest = await Bun.file(new URL("extension.toml", root)).text();
const source = await Bun.file(new URL("src/lib.rs", root)).text();

describe("Zed adapter package", () => {
  test("registers the shared server for Rust only", () => {
    expect(manifest).toContain("[language_servers.rust-doctor]");
    expect(manifest).toContain('languages = ["Rust"]');
    expect(source).toContain('"--lsp"');
    expect(source).toContain('"debounceMs": 300');
    expect(source).toContain('"onSaveProjectChecks": false');
  });

  test("contains no developer paths, downloads, or telemetry", () => {
    expect(source).not.toContain("/home/");
    expect(source).not.toContain("download_file");
    expect(source.toLowerCase()).not.toContain("telemetry");
    expect(source).toContain("RUST_DOCTOR_SELECTED_BINARY");
  });
});
