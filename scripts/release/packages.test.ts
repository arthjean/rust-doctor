import { describe, expect, test } from "bun:test";
import { cargoVersionFromManifest } from "./packages";

describe("Cargo manifest version parsing", () => {
  test.each([
    ["LF", "\n"],
    ["CRLF", "\r\n"],
  ])("accepts %s line endings", (_, newline) => {
    const manifest = [
      "[package]",
      'name = "rust-doctor"',
      'version = "0.2.0"',
      "",
      "[dependencies]",
      'serde = "1"',
      "",
    ].join(newline);

    expect(cargoVersionFromManifest(manifest)).toBe("0.2.0");
  });

  test("rejects a missing package version", () => {
    expect(() => cargoVersionFromManifest("[package]\nname = \"rust-doctor\"\n")).toThrow(
      "Cargo package version is missing",
    );
  });
});
