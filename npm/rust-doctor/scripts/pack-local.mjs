import {
  chmodSync,
  copyFileSync,
  cpSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { basename, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { PLATFORM_PACKAGES, platformKey } from "../lib/launcher.js";

export const packageRoot = resolve(fileURLToPath(new URL("..", import.meta.url)));
export const repositoryRoot = resolve(packageRoot, "../..");

function fail(message) {
  throw new Error(`local npm pack: ${message}`);
}

export function validateWrapperManifest(manifest, cargoVersion) {
  if (manifest.name !== "rust-doctor" || manifest.version !== cargoVersion) {
    fail(`wrapper name and version must match Cargo.toml ${cargoVersion}`);
  }
  if (manifest.bin?.["rust-doctor"] !== "bin/rust-doctor.js") {
    fail("wrapper bin.rust-doctor must point to bin/rust-doctor.js");
  }
  if (manifest.engines?.node !== "^20.19.0 || >=22.13.0") {
    fail("wrapper Node engine must be ^20.19.0 || >=22.13.0");
  }
  const expected = Object.values(PLATFORM_PACKAGES).sort();
  const actual = Object.keys(manifest.optionalDependencies ?? {}).sort();
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    fail("wrapper must declare exactly the five supported optional dependencies");
  }
  for (const packageName of expected) {
    if (manifest.optionalDependencies[packageName] !== cargoVersion) {
      fail(`optional dependency ${packageName} must equal ${cargoVersion}`);
    }
  }
  if (Object.hasOwn(manifest.scripts ?? {}, "postinstall")) {
    fail("wrapper must not define postinstall");
  }
}

function run(command, options = {}) {
  const result = Bun.spawnSync(command, {
    cwd: options.cwd ?? repositoryRoot,
    env: options.env ?? process.env,
    stdin: options.stdin ?? "ignore",
    stdout: options.stdout ?? "pipe",
    stderr: options.stderr ?? "pipe",
  });
  if (result.exitCode !== 0) {
    const error = result.stderr?.toString().trim() ?? "see inherited stderr";
    fail(`${command[0]} ${command.slice(1).join(" ")} failed (${result.exitCode}): ${error}`);
  }
  return result.stdout?.toString() ?? "";
}

export function cargoVersion() {
  const metadata = JSON.parse(run([
    "cargo",
    "metadata",
    "--locked",
    "--offline",
    "--no-deps",
    "--format-version",
    "1",
  ]));
  const manifestPath = join(repositoryRoot, "Cargo.toml");
  const package_ = metadata.packages.find(
    (candidate) => resolve(candidate.manifest_path) === manifestPath,
  );
  if (!package_?.version) fail("cargo metadata did not return the root package version");
  return package_.version;
}

function validateBinary(binaryPath) {
  if (!existsSync(binaryPath)) fail("native binary is missing; build rust-doctor first");
  const metadata = statSync(binaryPath);
  if (!metadata.isFile() || (metadata.mode & 0o111) === 0) {
    fail("native binary must be a regular executable file");
  }
}

// The dual license is two files at the repository root, and npm auto-includes a
// license only from a package root and only under a name it recognizes, which
// `LICENSE-MIT` is not. So every published package declares the pair in `files`
// and is handed the text at staging: the six tarballs each state
// `MIT OR Apache-2.0`, and a tarball that states terms it does not carry is a
// license claim with nothing behind it.
export const LICENSE_FILES = ["LICENSE-MIT", "LICENSE-APACHE"];

export function stageLicenses(directory) {
  for (const name of LICENSE_FILES) {
    const source = join(repositoryRoot, name);
    if (!existsSync(source)) fail(`${name} is missing from the repository root`);
    copyFileSync(source, join(directory, name));
  }
}

// The wrapper is staged rather than published from the checkout, for the reason
// the five native packages are: what a release uploads is then exactly what the
// packed smoke test installs, down to the license text, instead of the two
// diverging on whichever files happen to sit next to the manifest.
export function stageWrapper(directory) {
  mkdirSync(directory, { recursive: true });
  copyFileSync(join(packageRoot, "package.json"), join(directory, "package.json"));
  cpSync(join(packageRoot, "bin"), join(directory, "bin"), { recursive: true });
  cpSync(join(packageRoot, "lib"), join(directory, "lib"), { recursive: true });
  stageLicenses(directory);
}

export function nativeManifest(packageName, version, key) {
  const [os, cpu] = key.split("-");
  return {
    name: packageName,
    version,
    description: `Rust Doctor native binary for ${os} ${cpu}`,
    license: "MIT OR Apache-2.0",
    // The canonical git+https form: npm resolves provenance against this field,
    // and a bare https URL is not what it matches the workflow's repository to.
    repository: {
      type: "git",
      url: "git+https://github.com/arthjean/rust-doctor.git",
    },
    os: [os],
    cpu: [cpu],
    files: ["bin/", ...LICENSE_FILES],
    preferUnplugged: true,
  };
}

function pack(directory, outputDirectory, filename) {
  run(
    [
      "bun",
      "pm",
      "pack",
      "--ignore-scripts",
      "--quiet",
      "--filename",
      filename,
    ],
    { cwd: directory },
  );
  const packed = join(directory, filename);
  if (!existsSync(packed)) fail(`expected archive ${filename} was not produced`);
  const archive = join(outputDirectory, filename);
  copyFileSync(packed, archive);
  return archive;
}

export function archiveInventory(archive) {
  return run(["tar", "-tzf", archive])
    .trim()
    .split("\n")
    .filter(Boolean)
    .sort();
}

export async function sha256(path) {
  const bytes = await Bun.file(path).arrayBuffer();
  return new Bun.CryptoHasher("sha256").update(bytes).digest("hex");
}

export function packLocal({ binaryPath } = {}) {
  const version = cargoVersion();
  const wrapperManifest = JSON.parse(readFileSync(join(packageRoot, "package.json"), "utf8"));
  validateWrapperManifest(wrapperManifest, version);
  if (!readFileSync(join(packageRoot, "bin/rust-doctor.js"), "utf8").startsWith("#!/usr/bin/env node\n")) {
    fail("wrapper bin must start with the Node shebang");
  }

  const key = platformKey();
  const packageName = PLATFORM_PACKAGES[key];
  if (!packageName || key !== "linux-x64") {
    fail(`local proof supports linux-x64, received ${key}`);
  }

  const temporary = mkdtempSync(join(tmpdir(), "rust-doctor-local-pack-"));
  const output = mkdtempSync(join(tmpdir(), "rust-doctor-local-artifacts-"));
  try {
    let nativeBinary = binaryPath ? resolve(binaryPath) : undefined;
    if (!nativeBinary) {
      const target = join(temporary, "cargo-target");
      run([
        "cargo",
        "build",
        "--locked",
        "--release",
        "--bin",
        "rust-doctor",
        "--target-dir",
        target,
      ], { stdout: "inherit", stderr: "inherit" });
      nativeBinary = join(target, "release", "rust-doctor");
    }
    validateBinary(nativeBinary);

    const nativeRoot = join(temporary, "native");
    const wrapperRoot = join(temporary, "wrapper");
    const nativeBin = join(nativeRoot, "bin");
    mkdirSync(nativeBin, { recursive: true });
    stageWrapper(wrapperRoot);
    stageLicenses(nativeRoot);
    writeFileSync(
      join(nativeRoot, "package.json"),
      `${JSON.stringify(nativeManifest(packageName, version, key), null, 2)}\n`,
    );
    const embedded = join(nativeBin, "rust-doctor");
    copyFileSync(nativeBinary, embedded);
    chmodSync(embedded, 0o755);

    const wrapperArchive = pack(
      wrapperRoot,
      output,
      `rust-doctor-${version}.tgz`,
    );
    const nativeArchive = pack(
      nativeRoot,
      output,
      `rust-doctor-linux-x64-${version}.tgz`,
    );
    return { version, key, packageName, output, wrapperArchive, nativeArchive };
  } catch (error) {
    rmSync(output, { recursive: true, force: true });
    throw error;
  } finally {
    rmSync(temporary, { recursive: true, force: true });
  }
}

if (import.meta.main) {
  const result = packLocal();
  process.stdout.write(
    `${JSON.stringify({
      version: result.version,
      platform: result.key,
      output: result.output,
      tarballs: [basename(result.wrapperArchive), basename(result.nativeArchive)],
    })}\n`,
  );
}
