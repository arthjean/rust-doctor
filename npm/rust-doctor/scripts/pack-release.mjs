import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { join, resolve } from "node:path";

import { PLATFORM_PACKAGES } from "../lib/launcher.js";
import {
  nativeManifest,
  packageRoot,
  stageLicenses,
  stageWrapper,
} from "./pack-local.mjs";

function fail(message) {
  throw new Error(`release pack: ${message}`);
}

export function binaryName(key) {
  return key.startsWith("win32-") ? "rust-doctor.exe" : "rust-doctor";
}

export function wrapperVersion() {
  const manifest = JSON.parse(readFileSync(join(packageRoot, "package.json"), "utf8"));
  if (typeof manifest.version !== "string" || manifest.version.length === 0) {
    fail("wrapper package.json declares no version");
  }
  return manifest.version;
}

export function stageReleasePackages({ artifacts, output, expectVersion } = {}) {
  if (!artifacts) fail("an artifacts directory is required");
  if (!output) fail("an output directory is required");
  const version = wrapperVersion();
  if (expectVersion !== undefined && expectVersion !== version) {
    fail(`wrapper version is ${version}, release asked for ${expectVersion}`);
  }

  const artifactsRoot = resolve(artifacts);
  const outputRoot = resolve(output);
  rmSync(outputRoot, { recursive: true, force: true });
  mkdirSync(outputRoot, { recursive: true });

  const packages = [];
  for (const [key, packageName] of Object.entries(PLATFORM_PACKAGES)) {
    const name = binaryName(key);
    const source = join(artifactsRoot, `rust-doctor-${key}`, name);
    if (!existsSync(source)) fail(`artifact for ${key} is missing at ${source}`);
    const metadata = statSync(source);
    if (!metadata.isFile() || metadata.size === 0) {
      fail(`artifact for ${key} is not a regular non-empty file`);
    }

    const directory = join(outputRoot, key);
    const bin = join(directory, "bin");
    mkdirSync(bin, { recursive: true });
    writeFileSync(
      join(directory, "package.json"),
      `${JSON.stringify(nativeManifest(packageName, version, key), null, 2)}\n`,
    );
    stageLicenses(directory);
    const embedded = join(bin, name);
    copyFileSync(source, embedded);
    chmodSync(embedded, 0o755);

    packages.push({ key, packageName, directory, binary: embedded, size: metadata.size });
  }

  const wrapper = join(outputRoot, "wrapper");
  stageWrapper(wrapper);
  return { version, packages, wrapper };
}

if (import.meta.main) {
  const options = {};
  const argv = process.argv.slice(2);
  for (let index = 0; index < argv.length; index += 1) {
    const flag = argv[index];
    if (!flag.startsWith("--")) fail(`unexpected argument ${flag}`);
    const value = argv[index + 1];
    if (value === undefined) fail(`${flag} needs a value`);
    index += 1;
    if (flag === "--artifacts") options.artifacts = value;
    else if (flag === "--output") options.output = value;
    else if (flag === "--expect-version") options.expectVersion = value;
    else fail(`unknown flag ${flag}`);
  }

  const staged = stageReleasePackages(options);
  process.stdout.write(
    `${JSON.stringify({
      version: staged.version,
      wrapper: staged.wrapper,
      packages: staged.packages.map(({ key, packageName, directory, size }) => ({
        key,
        packageName,
        directory,
        size,
      })),
    }, null, 2)}\n`,
  );
}
