import { createHash } from "node:crypto";
import {
  copyFileSync,
  existsSync,
  lstatSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  readlinkSync,
  rmSync,
  symlinkSync,
} from "node:fs";
import { homedir } from "node:os";
import { basename, isAbsolute, join, relative, resolve } from "node:path";

export const REPO_ROOT = resolve(import.meta.dir, "../..");
export const FIXTURES = join(REPO_ROOT, "tests/fixtures");

export function resolveBinary(env: Record<string, string | undefined> = process.env): string {
  const configured = env.RUST_DOCTOR_E2E_BIN;
  const path = configured ? resolve(configured) : join(REPO_ROOT, "target/release/rust-doctor");
  if (!existsSync(path)) {
    throw new Error(`rust-doctor binary not found at ${path}: run cargo build --release first`);
  }
  return path;
}

export const BIN = resolveBinary();

export function parseNonInteractiveMarkers(source: string): string[] {
  const block = source.match(/NON_INTERACTIVE_MARKERS:\s*\[&str;\s*(\d+)\]\s*=\s*\[([\s\S]*?)\];/);
  if (!block) throw new Error("NON_INTERACTIVE_MARKERS not found in src/terminal_profile.rs");
  const body = block[2]!.replace(/\/\/[^\n]*/g, "");
  const markers = [...body.matchAll(/"([A-Za-z0-9_]+)"/g)].map((match) => match[1]!);
  if (markers.length === 0) throw new Error("NON_INTERACTIVE_MARKERS parsed to no marker");
  if (markers.length !== Number(block[1])) {
    throw new Error(`NON_INTERACTIVE_MARKERS declares ${block[1]} markers, parsed ${markers.length}`);
  }
  return markers;
}

export const NON_INTERACTIVE_MARKERS = parseNonInteractiveMarkers(
  readFileSync(join(REPO_ROOT, "src/terminal_profile.rs"), "utf8"),
);
export const UNATTENDED_VARIABLES = [...NON_INTERACTIVE_MARKERS, "CI"];

export const E2E_BASE = process.env.RUST_DOCTOR_E2E_ROOT
  ? resolve(process.env.RUST_DOCTOR_E2E_ROOT)
  : join(homedir(), ".cache", "rust-doctor-e2e");
mkdirSync(join(E2E_BASE, "runs"), { recursive: true, mode: 0o700 });
export const RUN_ROOT = mkdtempSync(join(E2E_BASE, "runs", `${process.pid}-`));
process.on("exit", () => {
  killLiveProcesses();
  rmSync(RUN_ROOT, { recursive: true, force: true });
});

let sequence = 0;

export function scratchDir(label: string): string {
  sequence += 1;
  const dir = join(RUN_ROOT, `${sequence}-${label.replace(/[^A-Za-z0-9_-]/g, "_")}`);
  mkdirSync(dir, { recursive: true });
  return dir;
}

export type Sandbox = { dir: string; home: string; tmp: string };

export function sandbox(label = "sandbox"): Sandbox {
  const dir = scratchDir(label);
  const home = join(dir, "home");
  const tmp = join(dir, "tmp");
  mkdirSync(home);
  mkdirSync(tmp);
  return { dir, home, tmp };
}

function testFileStem(): string {
  return basename(Bun.main).replace(/\.test\.ts$/, "").replace(/\.ts$/, "");
}

export function targetDir(key: string): string {
  const dir = join(E2E_BASE, "targets", testFileStem(), key);
  mkdirSync(dir, { recursive: true });
  return dir;
}

const REAL_HOME = homedir();
export const RUSTUP_HOME = process.env.RUSTUP_HOME ?? join(REAL_HOME, ".rustup");
export const CARGO_HOME = process.env.CARGO_HOME ?? join(REAL_HOME, ".cargo");

const TOOLCHAIN_TOOLS = ["cargo", "rustc", "rustdoc", "cargo-clippy", "clippy-driver", "rustup"];
const SYSTEM_TOOLS = [
  "git", "cc", "gcc", "c++", "ld", "as", "ar", "ranlib", "sh", "bash", "env", "cat", "cp", "mv", "rm", "mkdir",
  "ls", "ln", "chmod", "touch", "sleep", "printf", "echo", "head", "tail", "grep", "sed", "tr",
  "wc", "dirname", "basename", "mktemp", "readlink", "realpath", "uname", "id", "test", "true",
  "false", "sort", "cut", "tee", "xargs", "find", "diff", "kill", "ps", "stat", "od", "stty", "tput", "expr", "make",
];
const SYSTEM_PATH = "/usr/local/bin:/usr/bin:/bin";

let toolchainDir: string | undefined;

export function toolchainBin(): string {
  if (toolchainDir) return toolchainDir;
  const dir = join(RUN_ROOT, "bin");
  mkdirSync(dir);
  symlinkSync(BIN, join(dir, "rust-doctor"));
  for (const tool of TOOLCHAIN_TOOLS) {
    const path = join(CARGO_HOME, "bin", tool);
    if (existsSync(path)) symlinkSync(path, join(dir, tool));
  }
  for (const tool of SYSTEM_TOOLS) {
    const path = Bun.which(tool, { PATH: SYSTEM_PATH });
    if (path) symlinkSync(path, join(dir, tool));
  }
  if (!existsSync(join(dir, "cargo"))) throw new Error(`no cargo under ${CARGO_HOME}/bin`);
  if (!existsSync(join(dir, "git"))) throw new Error(`no git on ${SYSTEM_PATH}`);
  toolchainDir = dir;
  return dir;
}

export function pathWithout(name: string): string {
  const dir = join(scratchDir(`without-${name}`), "bin");
  mkdirSync(dir);
  for (const tool of readdirSync(toolchainBin())) {
    if (tool !== name) symlinkSync(join(toolchainBin(), tool), join(dir, tool));
  }
  return dir;
}

export type EnvOverrides = Record<string, string | undefined>;

export function hermeticEnv(box: Sandbox, overrides: EnvOverrides = {}): Record<string, string> {
  const env: Record<string, string | undefined> = {
    PATH: toolchainBin(),
    HOME: box.home,
    TMPDIR: box.tmp,
    RUSTUP_HOME,
    CARGO_HOME,
    LC_ALL: "C",
    TERM: "dumb",
    GIT_CONFIG_NOSYSTEM: "1",
    GIT_CONFIG_GLOBAL: "/dev/null",
    GIT_CONFIG_COUNT: "2",
    GIT_CONFIG_KEY_0: "maintenance.auto",
    GIT_CONFIG_VALUE_0: "false",
    GIT_CONFIG_KEY_1: "gc.auto",
    GIT_CONFIG_VALUE_1: "0",
    CARGO_NET_OFFLINE: "true",
    ...overrides,
  };
  return Object.fromEntries(
    Object.entries(env).filter((entry): entry is [string, string] => entry[1] !== undefined),
  );
}

type Live = { name: string; kill: () => void };
const live = new Map<number | string, Live>();

export function track(id: number | string, name: string, kill?: () => void): void {
  live.set(id, {
    name: typeof id === "number" ? `${name} (pid ${id})` : name,
    kill: kill ?? (() => {
      try {
        process.kill(id as number, "SIGKILL");
      } catch {}
    }),
  });
}

export function untrack(id: number | string): void {
  live.delete(id);
}

export function killLiveProcesses(): string[] {
  const left = [...live.values()];
  live.clear();
  for (const entry of left) entry.kill();
  return left.map((entry) => entry.name);
}

const SUBCOMMANDS = new Set(["rules", "why", "report", "setup", "hook", "skill", "ci", "help"]);

function isScan(args: string[]): boolean {
  const first = args[0];
  if (first === undefined) return true;
  if (SUBCOMMANDS.has(first)) return false;
  return !args.some((arg) => ["--version", "-V", "--help", "-h"].includes(arg));
}

export type DoctorOptions = {
  cwd?: string;
  env?: EnvOverrides;
  sandbox?: Sandbox;
  target?: string;
  advisoryDb?: false;
  stdin?: string | Uint8Array;
  timeoutMs?: number;
};

export type DoctorRun = Exit & { args: string[]; json: unknown; sandbox: Sandbox };

export const DEFAULT_WATCHDOG_MS = 45_000;

export function defaultTarget(args: string[], cwd: string): string {
  const path = args.find((arg) => !arg.startsWith("-"));
  const workspace = path && isScan(args) ? resolve(cwd, path) : cwd;
  const key = createHash("sha1").update(workspace).digest("hex").slice(0, 12);
  const dir = join(RUN_ROOT, "targets", key);
  mkdirSync(dir, { recursive: true });
  return dir;
}

export function doctorArgs(args: string[], options: DoctorOptions = {}): string[] {
  if (options.advisoryDb === false || !isScan(args) || args.includes("--advisory-db")) return args;
  return [...args, "--advisory-db", "/dev/null"];
}

export type Exit = {
  exitCode: number | null;
  signal: string | null;
  stdout: string;
  stderr: string;
  durationMs: number;
};

export type Tracked = {
  pid: number;
  proc: Bun.Subprocess<"pipe", "pipe", "pipe">;
  exited: Promise<Exit>;
};

export function spawnTracked(
  argv: string[],
  options: { cwd: string; env: Record<string, string>; stdin?: string | Uint8Array; timeoutMs?: number },
): Tracked {
  const started = performance.now();
  const proc = Bun.spawn(argv, {
    cwd: options.cwd,
    env: options.env,
    stdin: "pipe",
    stdout: "pipe",
    stderr: "pipe",
  });
  const command = argv.map((arg) => (arg === BIN ? "rust-doctor" : arg)).join(" ");
  track(proc.pid, command);
  if (options.stdin !== undefined) proc.stdin.write(options.stdin);
  proc.stdin.end();
  const watchdogMs = options.timeoutMs ?? DEFAULT_WATCHDOG_MS;
  let timedOut = false;
  const watchdog = setTimeout(() => {
    timedOut = true;
    proc.kill("SIGKILL");
  }, watchdogMs);
  const exited = (async (): Promise<Exit> => {
    const [stdout, stderr] = await Promise.all([
      new Response(proc.stdout).text(),
      new Response(proc.stderr).text(),
    ]);
    await proc.exited;
    clearTimeout(watchdog);
    untrack(proc.pid);
    if (timedOut) throw new Error(`watchdog killed ${command} (pid ${proc.pid}) after ${watchdogMs} ms`);
    return {
      exitCode: proc.exitCode,
      signal: proc.signalCode,
      stdout,
      stderr,
      durationMs: performance.now() - started,
    };
  })();
  return { pid: proc.pid, proc, exited };
}

export type Spawned = Tracked & { result: Promise<DoctorRun> };

export function spawnDoctor(args: string[], options: DoctorOptions = {}): Spawned {
  const box = options.sandbox ?? sandbox("run");
  const cwd = options.cwd ?? box.dir;
  const finalArgs = doctorArgs(args, options);
  const env = hermeticEnv(box, {
    CARGO_TARGET_DIR: options.target ?? defaultTarget(args, cwd),
    ...options.env,
  });
  const tracked = spawnTracked([BIN, ...finalArgs], {
    cwd,
    env,
    stdin: options.stdin,
    timeoutMs: options.timeoutMs,
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

export function runDoctor(args: string[], options: DoctorOptions = {}): Promise<DoctorRun> {
  return spawnDoctor(args, options).result;
}

export function runCommand(
  argv: string[],
  options: { cwd?: string; env?: EnvOverrides; sandbox?: Sandbox; stdin?: string; timeoutMs?: number } = {},
): Promise<Exit> {
  const box = options.sandbox ?? sandbox("command");
  return spawnTracked(argv, {
    cwd: options.cwd ?? box.dir,
    env: hermeticEnv(box, options.env),
    stdin: options.stdin,
    timeoutMs: options.timeoutMs,
  }).exited;
}

export function normalizeReport<T>(report: T): T {
  const copy = structuredClone(report) as Record<string, unknown>;
  delete copy.timings;
  delete copy.toolchain;
  const scan = copy.scan;
  if (scan && typeof scan === "object") delete (scan as Record<string, unknown>).advisory_db;
  return copy as T;
}

export function treeFingerprint(dir: string): string {
  const hash = createHash("sha256");
  const walk = (current: Buffer, rel: Buffer) => {
    for (const name of readdirSync(current, { encoding: "buffer" }).sort(Buffer.compare)) {
      const path = Buffer.concat([current, Buffer.from("/"), name]);
      const relPath = rel.length === 0 ? name : Buffer.concat([rel, Buffer.from("/"), name]);
      const relText = relPath.toString("latin1");
      if (relText === "target") continue;
      const stat = lstatSync(path);
      if (stat.isSymbolicLink()) {
        hash.update(Buffer.concat([Buffer.from("L "), relPath, Buffer.from(" "), readlinkSync(path, { encoding: "buffer" }), Buffer.from("\n")]));
      } else if (stat.isDirectory()) {
        hash.update(Buffer.concat([Buffer.from("D "), relPath, Buffer.from("\n")]));
        walk(path, relPath);
      } else {
        hash.update(Buffer.concat([Buffer.from("F "), relPath, Buffer.from(` ${stat.mode.toString(8)}\n`)]));
        hash.update(readFileSync(path));
        if (relText === ".git/index") hash.update(`M ${stat.mtimeMs} ${stat.size}\n`);
      }
    }
  };
  walk(Buffer.from(dir), Buffer.alloc(0));
  return hash.digest("hex");
}

export function copyWorkspace(fixture: string, into?: string): string {
  const source = isAbsolute(fixture) ? fixture : join(FIXTURES, fixture);
  if (!existsSync(join(source, "Cargo.toml"))) throw new Error(`no Cargo.toml in ${source}`);
  const destination = into ?? join(scratchDir(basename(source)), basename(source));
  const tracked = new Set(
    Bun.spawnSync(["git", "ls-files", "-z", "--", relative(REPO_ROOT, source)], {
      cwd: REPO_ROOT,
      env: { PATH: SYSTEM_PATH, GIT_CONFIG_NOSYSTEM: "1", GIT_CONFIG_GLOBAL: "/dev/null" },
    }).stdout.toString().split("\0").filter(Boolean).map((path) => join(REPO_ROOT, path)),
  );
  const copy = (from: string, to: string) => {
    mkdirSync(to, { recursive: true });
    for (const name of readdirSync(from)) {
      const path = join(from, name);
      const stat = lstatSync(path);
      if (name === "target" && stat.isDirectory()) continue;
      if (name === "Cargo.lock" && !tracked.has(path)) continue;
      if (stat.isSymbolicLink()) symlinkSync(readlinkSync(path), join(to, name));
      else if (stat.isDirectory()) copy(path, join(to, name));
      else copyFileSync(path, join(to, name));
    }
  };
  copy(source, destination);
  return destination;
}

export function isEmptyDir(dir: string): boolean {
  return readdirSync(dir).length === 0;
}

export function cargoVersion(): string {
  const manifest = readFileSync(join(REPO_ROOT, "Cargo.toml"), "utf8");
  const version = manifest.match(/^version\s*=\s*"([^"]+)"/m);
  if (!version) throw new Error("no package version in Cargo.toml");
  return version[1]!;
}

