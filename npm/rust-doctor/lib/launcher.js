import { statSync, readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { constants as osConstants } from "node:os";
import { spawn } from "node:child_process";

export const PLATFORM_PACKAGES = Object.freeze({
  "darwin-x64": "@rust-doctor/darwin-x64",
  "darwin-arm64": "@rust-doctor/darwin-arm64",
  "linux-x64": "@rust-doctor/linux-x64",
  "linux-arm64": "@rust-doctor/linux-arm64",
  "win32-x64": "@rust-doctor/win32-x64",
});

const wrapperManifest = JSON.parse(
  readFileSync(new URL("../package.json", import.meta.url), "utf8"),
);

export class LauncherError extends Error {}

export function platformKey(platform = process.platform, arch = process.arch) {
  return `${platform}-${arch}`;
}

export function resolveNativeBinary({
  platform = process.platform,
  arch = process.arch,
  resolve = createRequire(import.meta.url).resolve,
} = {}) {
  const key = platformKey(platform, arch);
  const packageName = PLATFORM_PACKAGES[key];
  if (!packageName) {
    throw new LauncherError(
      `Rust Doctor does not yet ship a binary for ${key}. Install Rust Doctor from source with cargo install rust-doctor.`,
    );
  }

  let manifestPath;
  let binaryPath;
  const binaryName = platform === "win32" ? "rust-doctor.exe" : "rust-doctor";
  try {
    manifestPath = resolve(`${packageName}/package.json`);
    binaryPath = resolve(`${packageName}/bin/${binaryName}`);
  } catch {
    throw new LauncherError(
      `Native package ${packageName} is not installed. Reinstall rust-doctor with its optional dependencies.`,
    );
  }

  let nativeManifest;
  let metadata;
  try {
    nativeManifest = JSON.parse(readFileSync(manifestPath, "utf8"));
    metadata = statSync(binaryPath);
  } catch {
    throw new LauncherError(
      `Native package ${packageName} is incomplete. Reinstall rust-doctor with its optional dependencies.`,
    );
  }
  if (nativeManifest.name !== packageName || nativeManifest.version !== wrapperManifest.version) {
    throw new LauncherError(
      `Native package ${packageName} must match rust-doctor ${wrapperManifest.version}. Reinstall both packages together.`,
    );
  }
  if (
    nativeManifest.os?.length !== 1
    || nativeManifest.os[0] !== platform
    || nativeManifest.cpu?.length !== 1
    || nativeManifest.cpu[0] !== arch
  ) {
    throw new LauncherError(
      `Native package ${packageName} does not match ${key}. Reinstall the package for this platform.`,
    );
  }
  const executable = platform === "win32" || (metadata.mode & 0o111) !== 0;
  if (!metadata.isFile() || !executable) {
    throw new LauncherError(
      `Native package ${packageName} does not contain an executable ${binaryName}. Reinstall the package.`,
    );
  }
  return binaryPath;
}

export function signalExitCode(signal) {
  const number = osConstants.signals[signal];
  return typeof number === "number" ? 128 + number : 1;
}

function boundedMessage(message) {
  const clean = String(message)
    .replace(/[\u0000-\u001f\u007f]/gu, " ")
    .replace(/\s+/gu, " ")
    .trim();
  const prefix = "rust-doctor: ";
  const limit = 1023 - Buffer.byteLength(prefix + "\n");
  let body = clean;
  while (Buffer.byteLength(body) > limit) {
    body = body.slice(0, -1);
  }
  return `${prefix}${body}\n`;
}

export function launchNative(binaryPath, arguments_, {
  platform = process.platform,
  cwd = process.cwd(),
  env = process.env,
  spawnChild = spawn,
  signals = process,
} = {}) {
  return new Promise((resolve) => {
    let spawnFailure;
    let forwardedSignal;
    const child = spawnChild(binaryPath, arguments_, {
      cwd,
      env,
      stdio: "inherit",
      shell: false,
    });

    const forward = (signal) => {
      if (forwardedSignal) return;
      forwardedSignal = signal;
      child.kill(platform === "win32" ? undefined : signal);
    };
    const onSigint = () => forward("SIGINT");
    const onSigterm = () => forward("SIGTERM");
    signals.on("SIGINT", onSigint);
    signals.on("SIGTERM", onSigterm);

    child.once("error", (error) => {
      const code = typeof error?.code === "string" ? error.code : "unknown error";
      spawnFailure = `Native binary could not start (${code}). Reinstall rust-doctor and its native package.`;
    });
    child.once("close", (code, signal) => {
      signals.off("SIGINT", onSigint);
      signals.off("SIGTERM", onSigterm);
      const windowsSignalCode = platform === "win32" && forwardedSignal
        ? signalExitCode(forwardedSignal)
        : undefined;
      resolve({
        code: spawnFailure
          ? 1
          : (windowsSignalCode ?? code ?? signalExitCode(signal ?? forwardedSignal)),
        error: spawnFailure,
        signal: spawnFailure ? undefined : (signal ?? forwardedSignal),
      });
    });
  });
}

export async function runCli({
  arguments_ = process.argv.slice(2),
  platform = process.platform,
  arch = process.arch,
} = {}) {
  let binaryPath;
  try {
    binaryPath = resolveNativeBinary({ platform, arch });
  } catch (error) {
    process.stderr.write(boundedMessage(error instanceof Error ? error.message : error));
    process.exitCode = 1;
    return;
  }

  const result = await launchNative(binaryPath, arguments_, { platform });
  if (result.error) {
    process.stderr.write(boundedMessage(result.error));
    process.exitCode = 1;
    return;
  }
  if (result.signal && platform !== "win32") {
    try {
      process.kill(process.pid, result.signal);
      return;
    } catch {
      process.exitCode = signalExitCode(result.signal);
      return;
    }
  }
  process.exitCode = result.code;
}
