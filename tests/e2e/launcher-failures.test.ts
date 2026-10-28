// `tasks/e2e-catalog/real-codebases.md` section 3, item 3: every way the
// installed layout can fail to yield a native binary, on the layout npm
// really produces rather than an injected resolver. Each case breaks its own
// copy of the installed prefix, and the launcher must answer with one line
// under 1,024 bytes on stderr, nothing on stdout, and exit 1.
import { describe, expect, test } from "bun:test";
import { chmodSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { scratchDir } from "./e2e-helpers";
import { installedCopy, NATIVE_PACKAGE, npmInstall, packedTarballs, runLauncher } from "./launcher-support";

function editManifest(native: string, edit: (manifest: Record<string, unknown>) => void): void {
  const path = join(native, "package.json");
  const manifest = JSON.parse(readFileSync(path, "utf8")) as Record<string, unknown>;
  edit(manifest);
  writeFileSync(path, `${JSON.stringify(manifest, null, 2)}\n`);
}

async function expectOneLine(launcher: string, message: string): Promise<void> {
  const run = await runLauncher(launcher, ["--version"]);
  expect(run.exitCode).toBe(1);
  expect(run.stdout).toBe("");
  expect(run.stderr).toMatch(/^rust-doctor: [^\n]+\n$/);
  expect(Buffer.byteLength(run.stderr)).toBeLessThan(1024);
  expect(run.stderr).toBe(`rust-doctor: ${message}\n`);
}

describe("the launcher refuses a broken install in one line", () => {
  test("the native package's manifest is gone", async () => {
    const { launcher, native } = await installedCopy("no-manifest");
    rmSync(join(native, "package.json"));
    await expectOneLine(launcher, `Native package ${NATIVE_PACKAGE} is not installed. Reinstall rust-doctor with its optional dependencies.`);
  });

  test("the native binary is gone", async () => {
    const { launcher, native } = await installedCopy("no-binary");
    rmSync(join(native, "bin/rust-doctor"));
    // The manifest chose this root, so its binary is never looked for elsewhere.
    await expectOneLine(launcher, `Native package ${NATIVE_PACKAGE} is incomplete. Reinstall rust-doctor with its optional dependencies.`);
  });

  test("the native package is another version", async () => {
    const { launcher, native } = await installedCopy("version-mismatch");
    const { version } = await packedTarballs();
    editManifest(native, (manifest) => void (manifest.version = "0.0.1"));
    await expectOneLine(launcher, `Native package ${NATIVE_PACKAGE} must match rust-doctor ${version}. Reinstall both packages together.`);
  });

  test("the native package declares another os", async () => {
    const { launcher, native } = await installedCopy("wrong-os");
    editManifest(native, (manifest) => void (manifest.os = ["darwin"]));
    await expectOneLine(launcher, `Native package ${NATIVE_PACKAGE} does not match linux-x64. Reinstall the package for this platform.`);
  });

  test("the native package declares another cpu", async () => {
    const { launcher, native } = await installedCopy("wrong-cpu");
    editManifest(native, (manifest) => void (manifest.cpu = ["arm64"]));
    await expectOneLine(launcher, `Native package ${NATIVE_PACKAGE} does not match linux-x64. Reinstall the package for this platform.`);
  });

  test("the native binary lost its executable bit", async () => {
    const { launcher, native } = await installedCopy("not-executable");
    chmodSync(join(native, "bin/rust-doctor"), 0o644);
    await expectOneLine(launcher, `Native package ${NATIVE_PACKAGE} does not contain an executable rust-doctor. Reinstall the package.`);
  });

  test("an install with --omit=optional carries no native package", async () => {
    const { wrapperArchive } = await packedTarballs();
    const prefix = join(scratchDir("omit-optional"), "prefix");
    await npmInstall(prefix, [wrapperArchive], ["--omit=optional"]);
    await expectOneLine(join(prefix, "bin/rust-doctor"), `Native package ${NATIVE_PACKAGE} is not installed. Reinstall rust-doctor with its optional dependencies.`);
  });
});
