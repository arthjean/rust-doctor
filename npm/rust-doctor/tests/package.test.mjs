import { afterEach, describe, expect, test } from "bun:test";
import {
  chmodSync,
  mkdtempSync,
  mkdirSync,
  readFileSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import {
  archiveInventory,
  cargoVersion,
  packLocal,
  packageRoot,
  validateWrapperManifest,
} from "../scripts/pack-local.mjs";

const temporaryRoots = [];

function temporary(name) {
  const root = mkdtempSync(join(tmpdir(), `rust-doctor-${name}-`));
  temporaryRoots.push(root);
  return root;
}

function executable(path, source = "#!/bin/sh\nexit 0\n") {
  mkdirSync(join(path, ".."), { recursive: true });
  writeFileSync(path, source);
  chmodSync(path, 0o755);
}

afterEach(() => {
  for (const root of temporaryRoots.splice(0)) {
    rmSync(root, { recursive: true, force: true });
  }
});

describe("local package contract", () => {
  test("wrapper metadata matches Cargo and exposes the five native packages", () => {
    const version = cargoVersion();
    const manifest = JSON.parse(readFileSync(join(packageRoot, "package.json"), "utf8"));
    expect(() => validateWrapperManifest(manifest, version)).not.toThrow();
    expect(manifest).toMatchObject({
      name: "rust-doctor",
      version,
      bin: { "rust-doctor": "bin/rust-doctor.js" },
      engines: { node: "^20.19.0 || >=22.13.0" },
    });
    expect(readFileSync(join(packageRoot, manifest.bin["rust-doctor"]), "utf8"))
      .toStartWith("#!/usr/bin/env node\n");
    expect(Object.keys(manifest.optionalDependencies)).toEqual([
      "@rust-doctor/darwin-x64",
      "@rust-doctor/darwin-arm64",
      "@rust-doctor/linux-x64",
      "@rust-doctor/linux-arm64",
      "@rust-doctor/win32-x64",
    ]);
    expect(manifest.scripts.postinstall).toBeUndefined();
  });

  test("manifest drift and absent optional dependencies fail closed", () => {
    const manifest = JSON.parse(readFileSync(join(packageRoot, "package.json"), "utf8"));
    expect(() => validateWrapperManifest({ ...manifest, version: "9.9.9" }, "0.1.0"))
      .toThrow("must match Cargo.toml");
    const missingOptional = structuredClone(manifest);
    delete missingOptional.optionalDependencies["@rust-doctor/linux-x64"];
    expect(() => validateWrapperManifest(missingOptional, "0.1.0"))
      .toThrow("exactly the five");
  });

  test("linux x64 pack emits only wrapper and constrained native payloads", () => {
    const root = temporary("pack");
    const binary = join(root, "input", "rust-doctor");
    executable(binary);

    const packed = packLocal({ binaryPath: binary });
    temporaryRoots.push(packed.output);
    expect(packed.key).toBe("linux-x64");
    expect(packed.packageName).toBe("@rust-doctor/linux-x64");
    expect(archiveInventory(packed.wrapperArchive)).toEqual([
      "package/bin/rust-doctor.js",
      "package/lib/launcher.js",
      "package/package.json",
    ]);
    expect(archiveInventory(packed.nativeArchive)).toEqual([
      "package/bin/rust-doctor",
      "package/package.json",
    ]);

    const extracted = join(root, "extracted");
    mkdirSync(extracted);
    const result = Bun.spawnSync(["tar", "-xzf", packed.nativeArchive, "-C", extracted]);
    expect(result.exitCode).toBe(0);
    const nativeManifest = JSON.parse(
      readFileSync(join(extracted, "package/package.json"), "utf8"),
    );
    expect(nativeManifest).toMatchObject({
      name: "@rust-doctor/linux-x64",
      version: packed.version,
      os: ["linux"],
      cpu: ["x64"],
      files: ["bin/"],
    });
    const embedded = statSync(join(extracted, "package/bin/rust-doctor"));
    expect(embedded.isFile()).toBeTrue();
    expect(embedded.mode & 0o111).not.toBe(0);
  });

  test("missing or non-executable binaries are rejected before packing", () => {
    const root = temporary("invalid-binary");
    expect(() => packLocal({
      binaryPath: join(root, "missing"),
    })).toThrow("native binary is missing");

    const binary = join(root, "not-executable");
    writeFileSync(binary, "not executable\n");
    chmodSync(binary, 0o644);
    expect(() => packLocal({
      binaryPath: binary,
    })).toThrow("regular executable file");
  });
});
