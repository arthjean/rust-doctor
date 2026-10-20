// The npm launcher as a user installs it: `packLocal({ binaryPath })` wraps the
// release binary under test in the two tarballs a release publishes, without
// rebuilding it, and npm installs them offline into a scratch prefix, the
// layout `npm install -g rust-doctor` and the Action produce. Packing and
// installing happen once per process; a test that breaks the layout takes a
// copy of it.
import { cpSync, existsSync, mkdirSync, realpathSync, rmSync, symlinkSync } from "node:fs";
import { constants } from "node:os";
import { join } from "node:path";
import {
  BIN,
  DEFAULT_WATCHDOG_MS,
  defaultTarget,
  doctorArgs,
  hermeticEnv,
  REPO_ROOT,
  RUN_ROOT,
  sandbox,
  scratchDir,
  spawnTracked,
  toolchainBin,
  type DoctorOptions,
  type DoctorRun,
  type EnvOverrides,
  type Sandbox,
  type Spawned,
} from "./e2e-helpers";

const NODE = Bun.which("node");
const NPM = Bun.which("npm");

export const NATIVE_PACKAGE = "@rustdoctor/linux-x64";

// npm's offline settings: a registry and proxies that refuse every
// connection, so an install that would reach the network fails instead.
const OFFLINE: Record<string, string> = {
  npm_config_registry: "http://127.0.0.1:9",
  npm_config_update_notifier: "false",
  npm_config_audit: "false",
  npm_config_fund: "false",
  HTTP_PROXY: "http://127.0.0.1:9",
  HTTPS_PROXY: "http://127.0.0.1:9",
  ALL_PROXY: "http://127.0.0.1:9",
  NO_PROXY: "",
};

let nodeDir: string | undefined;

// A directory holding a link to node and nothing else, put in front of the
// suite's toolchain directory: the launcher's shebang finds node there.
function nodeBin(): string {
  if (nodeDir) return nodeDir;
  if (!NODE) throw new Error("node is not installed: the launcher files need it");
  const dir = join(RUN_ROOT, "node-bin");
  mkdirSync(dir, { recursive: true });
  symlinkSync(realpathSync(NODE), join(dir, "node"));
  nodeDir = dir;
  return dir;
}

// The variables the installed launcher needs on top of the sealed
// environment: node on PATH, before the toolchain.
export function launcherEnv(box: Sandbox): EnvOverrides {
  return { PATH: `${nodeBin()}:${toolchainBin()}`, npm_config_cache: join(box.dir, "npm-cache") };
}

export type Packed = { version: string; wrapperArchive: string; nativeArchive: string };

let packed: Promise<Packed> | undefined;

// The two tarballs, written under the run's scratch root: packLocal makes its
// directories under the system temporary directory, which is pointed there
// for the call.
export function packedTarballs(): Promise<Packed> {
  packed ??= (async () => {
    const tmp = scratchDir("pack-tmp");
    const previous = process.env.TMPDIR;
    process.env.TMPDIR = tmp;
    try {
      const { packLocal } = await import(join(REPO_ROOT, "npm/rust-doctor/scripts/pack-local.mjs"));
      const result = packLocal({ binaryPath: BIN }) as Packed;
      return { version: result.version, wrapperArchive: result.wrapperArchive, nativeArchive: result.nativeArchive };
    } finally {
      if (previous === undefined) delete process.env.TMPDIR;
      else process.env.TMPDIR = previous;
    }
  })();
  return packed;
}

// `npm install -g --prefix <prefix> --offline` of `tarballs`, the native one
// first, as smoke-packed.mjs does: offline, npm cannot resolve the wrapper's
// optional dependencies, so the native tarball is installed beside it.
export async function npmInstall(prefix: string, tarballs: string[], extra: string[] = []): Promise<void> {
  if (!NPM) throw new Error("npm is not installed: the launcher files need it");
  const box = sandbox("npm-install");
  const run = await spawnTracked(
    [nodeBin() + "/node", realpathSync(NPM), "install", "-g", "--prefix", prefix, "--ignore-scripts", "--offline", ...extra, ...tarballs],
    { cwd: box.dir, env: hermeticEnv(box, { ...OFFLINE, ...launcherEnv(box) }), timeoutMs: 120_000 },
  ).exited;
  if (run.exitCode !== 0) throw new Error(`npm install failed (exit ${run.exitCode}): ${run.stderr.trim()}`);
  if (/registry\.npmjs|GET https?:|ECONNREFUSED/u.test(run.stderr)) throw new Error(`npm install reached for the network: ${run.stderr}`);
}

let installed: Promise<string> | undefined;

function prefixRoot(): Promise<string> {
  installed ??= (async () => {
    const tarballs = await packedTarballs();
    const prefix = join(scratchDir("npm-prefix"), "prefix");
    await npmInstall(prefix, [tarballs.nativeArchive, tarballs.wrapperArchive]);
    if (!existsSync(join(prefix, "bin/rust-doctor"))) throw new Error("npm installed no rust-doctor bin");
    return prefix;
  })();
  return installed;
}

// The installed `rust-doctor` launcher, absolute.
export async function installedLauncher(): Promise<string> {
  return join(await prefixRoot(), "bin/rust-doctor");
}

// A private copy of the installed prefix, links kept as they are, for a test
// that breaks the layout. Returns the copy's launcher and its native package.
export async function installedCopy(label: string): Promise<{ launcher: string; native: string; prefix: string }> {
  const prefix = join(scratchDir(label), "prefix");
  cpSync(await prefixRoot(), prefix, { recursive: true, verbatimSymlinks: true });
  return { prefix, launcher: join(prefix, "bin/rust-doctor"), native: join(prefix, "lib/node_modules", NATIVE_PACKAGE) };
}

export function removeTree(path: string): void {
  rmSync(path, { recursive: true, force: true });
}

// `spawnDoctor` with `launcher` in place of the binary: the same sealed
// environment, target directory and advisory default, node on PATH.
export function spawnLauncher(launcher: string, args: string[], options: DoctorOptions & { prefix?: string[] } = {}): Spawned {
  const box = options.sandbox ?? sandbox("launcher");
  const cwd = options.cwd ?? box.dir;
  const finalArgs = doctorArgs(args, options);
  const env = hermeticEnv(box, {
    ...launcherEnv(box),
    CARGO_TARGET_DIR: options.target ?? defaultTarget(args, cwd),
    ...options.env,
  });
  const tracked = spawnTracked([...(options.prefix ?? []), launcher, ...finalArgs], {
    cwd,
    env,
    stdin: options.stdin,
    timeoutMs: options.timeoutMs ?? DEFAULT_WATCHDOG_MS,
  });
  const result = tracked.exited.then((exit): DoctorRun => {
    let json: unknown = undefined;
    if (finalArgs.includes("--json") && exit.stdout.trim().length > 0) {
      try {
        json = JSON.parse(exit.stdout);
      } catch {}
    }
    return { ...exit, args: finalArgs, json, sandbox: box };
  });
  return { ...tracked, result };
}

export function runLauncher(launcher: string, args: string[], options: DoctorOptions = {}): Promise<DoctorRun> {
  return spawnLauncher(launcher, args, options).result;
}

// What a shell reports: the exit code, or 128 plus the signal that ended the
// process. The launcher ends itself by the signal it forwarded, as its own
// tests require, so the shell reads 130 from both.
export function shellStatus(run: { exitCode: number | null; signal: string | null }): number | null {
  if (run.exitCode !== null) return run.exitCode;
  const number = run.signal ? constants.signals[run.signal as keyof typeof constants.signals] : undefined;
  return typeof number === "number" ? 128 + number : null;
}
