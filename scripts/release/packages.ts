import {
  chmodSync,
  cpSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { basename, join, resolve } from "node:path";

const repositoryRoot = resolve(import.meta.dir, "../..");
const npmRoot = join(repositoryRoot, "npm");
const platformPackages = [
  "darwin-arm64",
  "darwin-x64",
  "linux-arm64",
  "linux-x64",
  "win32-x64",
] as const;
type PlatformPackage = (typeof platformPackages)[number];

function cargoVersion(): string {
  const manifest = readFileSync(join(repositoryRoot, "Cargo.toml"), "utf8");
  const packageSection = manifest.match(/^\[package\]\n([\s\S]*?)(?=^\[)/m)?.[1];
  const version = packageSection?.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
  if (!version) throw new Error("Cargo package version is missing");
  return version;
}

function packageJson(directory: string): Record<string, unknown> {
  return JSON.parse(readFileSync(join(directory, "package.json"), "utf8")) as Record<string, unknown>;
}

function validate(expectedTag?: string): string {
  const version = cargoVersion();
  if (expectedTag && expectedTag.replace(/^v/, "") !== version) {
    throw new Error(`tag ${expectedTag} does not match Cargo version ${version}`);
  }
  const wrapper = packageJson(join(npmRoot, "rust-doctor"));
  if (wrapper.version !== version) {
    throw new Error(`npm/rust-doctor version must match Cargo version ${version}`);
  }
  const optional = wrapper.optionalDependencies as Record<string, string> | undefined;
  for (const platform of platformPackages) {
    const manifest = packageJson(join(npmRoot, platform));
    const name = `@rust-doctor/${platform}`;
    if (manifest.name !== name || manifest.version !== version) {
      throw new Error(`${name} metadata must match Cargo version ${version}`);
    }
    if (optional?.[name] !== version) {
      throw new Error(`wrapper dependency ${name} must equal ${version}`);
    }
  }
  return version;
}

function run(command: string[], cwd: string): string {
  const result = Bun.spawnSync(command, { cwd, stdout: "pipe", stderr: "inherit" });
  if (result.exitCode !== 0) {
    throw new Error(`${command.join(" ")} failed with exit code ${result.exitCode}`);
  }
  return result.stdout.toString().trim();
}

function packDirectory(directory: string, filename: string): string {
  run(["bun", "pm", "pack", "--ignore-scripts", "--filename", filename], directory);
  return join(directory, filename);
}

function pack(platform: PlatformPackage, binaryInput: string, outputInput: string): void {
  if (!platformPackages.includes(platform)) throw new Error(`unsupported platform package ${platform}`);
  const version = validate();
  const binary = resolve(binaryInput);
  if (!existsSync(binary)) throw new Error(`binary does not exist: ${binary}`);
  const output = resolve(outputInput);
  mkdirSync(output, { recursive: true });
  const temporary = mkdtempSync(join(tmpdir(), "rust-doctor-release-"));
  try {
    const platformDirectory = join(temporary, "platform");
    const wrapperDirectory = join(temporary, "wrapper");
    cpSync(join(npmRoot, platform), platformDirectory, { recursive: true });
    cpSync(join(npmRoot, "rust-doctor"), wrapperDirectory, { recursive: true });
    const binaryName = platform === "win32-x64" ? "rust-doctor.exe" : "rust-doctor";
    const embedded = join(platformDirectory, "bin", binaryName);
    mkdirSync(join(platformDirectory, "bin"), { recursive: true });
    cpSync(binary, embedded);
    if (platform !== "win32-x64") chmodSync(embedded, 0o755);

    const platformArchiveName = `rust-doctor-npm-${platform}-${version}.tgz`;
    const wrapperArchiveName = `rust-doctor-npm-${version}.tgz`;
    const platformArchive = packDirectory(platformDirectory, platformArchiveName);
    const wrapperArchive = packDirectory(wrapperDirectory, wrapperArchiveName);

    const installDirectory = join(temporary, "install");
    mkdirSync(installDirectory);
    writeFileSync(
      join(installDirectory, "package.json"),
      JSON.stringify({
        name: "rust-doctor-release-smoke",
        private: true,
        dependencies: {
          "rust-doctor": `file:${wrapperArchive}`,
          [`@rust-doctor/${platform}`]: `file:${platformArchive}`,
        },
      }),
    );
    run(["bun", "install"], installDirectory);
    const outputVersion = run(
      ["bun", join(installDirectory, "node_modules/rust-doctor/bin/rust-doctor.js"), "--version"],
      installDirectory,
    );
    if (outputVersion !== `rust-doctor ${version}`) {
      throw new Error(`packed wrapper reported ${JSON.stringify(outputVersion)}, expected rust-doctor ${version}`);
    }

    cpSync(platformArchive, join(output, platformArchiveName));
    if (platform === "linux-x64") cpSync(wrapperArchive, join(output, wrapperArchiveName));
  } finally {
    rmSync(temporary, { recursive: true, force: true });
  }
}

async function registryIntegrity(name: string, version: string): Promise<string | null> {
  const encoded = name.replace("/", "%2f");
  const response = await fetch(`https://registry.npmjs.org/${encoded}/${version}`);
  if (response.status === 404) return null;
  if (!response.ok) throw new Error(`registry lookup for ${name}@${version} failed with ${response.status}`);
  const manifest = (await response.json()) as { dist?: { integrity?: string } };
  if (!manifest.dist?.integrity) throw new Error(`registry response for ${name}@${version} has no integrity`);
  return manifest.dist.integrity;
}

async function publish(artifactsInput: string): Promise<void> {
  const version = validate();
  if (!process.env.NPM_CONFIG_TOKEN) throw new Error("NPM_CONFIG_TOKEN is required for publication");
  const artifacts = resolve(artifactsInput);
  const expected = [
    ...platformPackages.map((platform) => ({
      name: `@rust-doctor/${platform}`,
      file: join(artifacts, `rust-doctor-npm-${platform}-${version}.tgz`),
    })),
    { name: "rust-doctor", file: join(artifacts, `rust-doctor-npm-${version}.tgz`) },
  ];
  for (const artifact of expected) {
    if (!existsSync(artifact.file)) throw new Error(`missing package archive ${artifact.file}`);
    const bytes = await Bun.file(artifact.file).arrayBuffer();
    const digest = new Bun.CryptoHasher("sha512").update(bytes).digest("base64");
    const localIntegrity = `sha512-${digest}`;
    const remoteIntegrity = await registryIntegrity(artifact.name, version);
    if (remoteIntegrity === localIntegrity) {
      console.log(`Verified existing immutable package ${artifact.name}@${version}`);
      continue;
    }
    if (remoteIntegrity) {
      throw new Error(`${artifact.name}@${version} exists with different immutable content`);
    }
    run(["bun", "publish", artifact.file, "--access", "public"], repositoryRoot);
  }
}

async function checksums(artifactsInput: string): Promise<void> {
  const version = validate();
  const artifacts = resolve(artifactsInput);
  const files = readdirSync(artifacts)
    .filter((file) => file.endsWith(".tar.gz") || file.endsWith(".zip") || file.endsWith(".tgz"))
    .sort();
  const entries: Record<string, string> = {};
  for (const file of files) {
    const bytes = await Bun.file(join(artifacts, file)).arrayBuffer();
    entries[file] = new Bun.CryptoHasher("sha256").update(bytes).digest("hex");
  }
  writeFileSync(
    join(artifacts, "checksums.json"),
    `${JSON.stringify({ schema_version: "1", version, artifacts: entries }, null, 2)}\n`,
  );
}

function updateAction(version: string): void {
  if (version !== cargoVersion()) throw new Error("action version must match Cargo version");
  const path = join(repositoryRoot, "action.yml");
  const source = readFileSync(path, "utf8");
  const updated = source.replace(
    /(  version:\n(?:    .*\n){2}    default: )[0-9]+\.[0-9]+\.[0-9]+/,
    `$1${version}`,
  );
  if (updated === source) throw new Error("action.yml version default was not updated");
  writeFileSync(path, updated);
}

const [command, ...arguments_] = process.argv.slice(2);
switch (command) {
  case "validate":
    console.log(validate(arguments_[0]));
    break;
  case "pack":
    pack(arguments_[0] as PlatformPackage, arguments_[1] ?? "", arguments_[2] ?? "");
    break;
  case "publish":
    await publish(arguments_[0] ?? "");
    break;
  case "checksums":
    await checksums(arguments_[0] ?? "");
    break;
  case "update-action":
    updateAction(arguments_[0] ?? "");
    break;
  default:
    throw new Error(`unknown command ${JSON.stringify(command)}`);
}
