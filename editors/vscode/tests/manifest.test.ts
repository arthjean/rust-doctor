import { describe, expect, test } from "bun:test";
import manifest from "../package.json";
import {
  isCompatibleProtocol,
  isCompatibleVersion,
  initializationOptions,
  resolveBinaryPath,
  RUST_DOCTOR_PROTOCOL_MAJOR,
} from "../src/binary";

describe("VS Code and Cursor adapter contract", () => {
  test("uses the shared Rust language server contract", () => {
    expect(manifest.engines.vscode).toBeDefined();
    expect(manifest.activationEvents).toContain("onLanguage:rust");
    expect(manifest.contributes.configuration.properties["rustDoctor.debounceMs"].default).toBe(300);
    expect(manifest.contributes.configuration.properties["rustDoctor.onSaveProjectChecks"].default).toBe(false);
    expect(manifest.contributes.configuration.properties["rustDoctor.configurationPath"].default).toBe("");
    expect(manifest.contributes.commands.map((command) => command.command)).toEqual([
      "rustDoctor.scanWorkspace",
      "rustDoctor.explainDiagnostic",
      "rustDoctor.openRuleDocumentation",
    ]);
  });

  test("contains no telemetry contribution or dependency", () => {
    expect(JSON.stringify(manifest).toLowerCase()).not.toContain("telemetry");
    expect(Object.keys(manifest.dependencies)).toEqual(["vscode-languageclient"]);
  });

  test("resolves supported platform binaries and explicit paths", () => {
    expect(resolveBinaryPath("rust-doctor", "linux")).toBe("rust-doctor");
    expect(resolveBinaryPath("rust-doctor", "darwin")).toBe("rust-doctor");
    expect(resolveBinaryPath("rust-doctor", "win32")).toBe("rust-doctor.exe");
    expect(resolveBinaryPath("/opt/rust-doctor", "linux")).toBe("/opt/rust-doctor");
    const compatible = "rust-doctor 0.2.0\nrustc 1.97.0\nlsp-protocol 1\n";
    expect(isCompatibleVersion(compatible)).toBe(true);
    expect(isCompatibleVersion("rust-doctor 1.0.0\n")).toBe(true);
    expect(isCompatibleVersion("rust-doctor 0.1.9\n")).toBe(false);
    expect(isCompatibleProtocol(compatible)).toBe(true);
    expect(isCompatibleProtocol("rust-doctor 0.2.0\nlsp-protocol 2\n")).toBe(false);
  });

  test("negotiates the shared protocol and omits an absent config path", () => {
    const defaults = initializationOptions({
      debounceMs: 300,
      onSaveProjectChecks: false,
      projectBudgetMs: 10_000,
      configurationPath: "",
    });
    expect(defaults.protocolMajor).toBe(RUST_DOCTOR_PROTOCOL_MAJOR);
    expect(defaults).not.toHaveProperty("configurationPath");
    expect(
      initializationOptions({
        debounceMs: 300,
        onSaveProjectChecks: false,
        projectBudgetMs: 10_000,
        configurationPath: "config/rust-doctor.toml",
      }).configurationPath,
    ).toBe("config/rust-doctor.toml");
  });
});
