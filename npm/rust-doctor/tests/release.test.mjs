import { afterEach, describe, expect, test } from "bun:test";
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { PLATFORM_PACKAGES } from "../lib/launcher.js";
import { binaryName, stageNativePackages, wrapperVersion } from "../scripts/pack-release.mjs";

const roots = [];

function temporary(name) {
  const root = mkdtempSync(join(tmpdir(), `rust-doctor-${name}-`));
  roots.push(root);
  return root;
}

// Mirrors what actions/download-artifact leaves behind: one directory per
// uploaded artifact, holding the staged binary with the executable bit already
// lost to the zip round trip.
function downloadedArtifacts({ skip = [], empty = [] } = {}) {
  const root = temporary("artifacts");
  for (const key of Object.keys(PLATFORM_PACKAGES)) {
    if (skip.includes(key)) continue;
    const directory = join(root, `rust-doctor-${key}`);
    mkdirSync(directory, { recursive: true });
    const binary = join(directory, binaryName(key));
    writeFileSync(binary, empty.includes(key) ? "" : `binary for ${key}\n`);
    chmodSync(binary, 0o644);
  }
  return root;
}

afterEach(() => {
  for (const root of roots.splice(0)) {
    rmSync(root, { recursive: true, force: true });
  }
});

describe("release staging", () => {
  test("stages the five native packages the wrapper declares", () => {
    const staged = stageNativePackages({
      artifacts: downloadedArtifacts(),
      output: join(temporary("output"), "packages"),
      expectVersion: wrapperVersion(),
    });

    expect(staged.version).toBe(wrapperVersion());
    expect(staged.packages.map((package_) => package_.packageName)).toEqual(
      Object.values(PLATFORM_PACKAGES),
    );

    for (const package_ of staged.packages) {
      const [os, cpu] = package_.key.split("-");
      const manifest = JSON.parse(
        readFileSync(join(package_.directory, "package.json"), "utf8"),
      );
      expect(manifest).toMatchObject({
        name: package_.packageName,
        version: staged.version,
        os: [os],
        cpu: [cpu],
        files: ["bin/"],
      });
      expect(manifest.name.startsWith("@rustdoctor/")).toBeTrue();
      expect(Object.hasOwn(manifest, "scripts")).toBeFalse();

      // The launcher refuses a binary that is not executable, and the artifact
      // it came from was not.
      const embedded = statSync(package_.binary);
      expect(embedded.isFile()).toBeTrue();
      expect(embedded.mode & 0o111).not.toBe(0);
      expect(package_.binary.endsWith(binaryName(package_.key))).toBeTrue();
    }
  });

  test("a missing, empty or mismatched candidate fails closed", () => {
    const output = join(temporary("output-failures"), "packages");

    expect(() => stageNativePackages({
      artifacts: downloadedArtifacts({ skip: ["darwin-arm64"] }),
      output,
    })).toThrow("artifact for darwin-arm64 is missing");

    expect(() => stageNativePackages({
      artifacts: downloadedArtifacts({ empty: ["win32-x64"] }),
      output,
    })).toThrow("artifact for win32-x64 is not a regular non-empty file");

    expect(() => stageNativePackages({
      artifacts: downloadedArtifacts(),
      output,
      expectVersion: "9.9.9",
    })).toThrow("release asked for 9.9.9");
  });
});
