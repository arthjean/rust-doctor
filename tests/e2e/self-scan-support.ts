// What the files of `tasks/e2e-catalog/real-codebases.md` section 2 share:
// one way to scan the clone of this repository, under the toolchain CI pins
// when it is installed, and the readings every invariant compares. None of it
// knows this repository's score, counts or findings: the self-scan is a gate.
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import {
  REPO_ROOT,
  RUSTUP_HOME,
  runCommand,
  runDoctor,
  spawnDoctor,
  treeFingerprint,
  type DoctorRun,
  type EnvOverrides,
  type Spawned,
} from "./e2e-helpers";
import type { SelfClone } from "./self-clone";

// The toolchain every CI job pins, read from the workflow rather than copied.
export const TOOLCHAIN_PIN = (() => {
  const found = readFileSync(join(REPO_ROOT, ".github/workflows/ci.yml"), "utf8").match(/toolchain: "(\d+\.\d+\.\d+)"/);
  if (!found) throw new Error("no pinned toolchain in .github/workflows/ci.yml");
  return found[1]!;
})();

// Whether the pin is installed beside the machine's default: CI installs it
// as the default, a development machine may hold it as one of several.
export const PIN_INSTALLED = existsSync(join(RUSTUP_HOME, "toolchains"))
  && readdirSync(join(RUSTUP_HOME, "toolchains")).some((name) => name.startsWith(`${TOOLCHAIN_PIN}-`));

// Every self-scan runs under the pin when it is installed, so the report is
// the one CI judges whatever the machine's default is.
export function selfEnv(extra: EnvOverrides = {}): EnvOverrides {
  return { ...(PIN_INSTALLED ? { RUSTUP_TOOLCHAIN: TOOLCHAIN_PIN } : {}), ...extra };
}

// A cold scan of this repository compiles its dependencies, which takes about
// six seconds here and four times that on a hosted runner.
export const SCAN_TIMEOUT_MS = 240_000;
export const TEST_TIMEOUT_MS = 600_000;

export type ScanOptions = { target?: string; env?: EnvOverrides; cwd?: string };

export function scanArgs(clone: SelfClone, args: string[]): string[] {
  return [clone.path, "--yes", ...args];
}

export function scanSelf(clone: SelfClone, args: string[] = [], options: ScanOptions = {}): Promise<DoctorRun> {
  return runDoctor(scanArgs(clone, args), {
    sandbox: clone.sandbox,
    cwd: options.cwd,
    target: options.target ?? clone.target("scan"),
    env: selfEnv(options.env),
    timeoutMs: SCAN_TIMEOUT_MS,
  });
}

export function spawnSelf(clone: SelfClone, args: string[] = [], options: ScanOptions = {}): Spawned {
  return spawnDoctor(scanArgs(clone, args), {
    sandbox: clone.sandbox,
    target: options.target ?? clone.target("scan"),
    env: selfEnv(options.env),
    timeoutMs: SCAN_TIMEOUT_MS,
  });
}

export type Diagnostic = {
  id: string;
  code: string;
  severity: string;
  category: string | null;
  path: string | null;
  span: { line_start: number; column_start: number } | null;
  message: string;
  occurrences: number;
};

export type Score = {
  model: string;
  value: number;
  label: string;
  authoritative: boolean;
  worst_tier: string | null;
  applied_ceiling: number | null;
};

export type Counts = { errors: number; warnings: number; info: number; unknown: number; total: number };

export type Report = {
  schema_version: number;
  status: string;
  complete: boolean;
  errors: Array<{ stage: string; code: string; message: string }>;
  audit: {
    source_files: number;
    production_lines: number;
    score: Score | null;
    categories: Array<{ name: string; errors: number; warnings: number; info: number; unknown: number }>;
  };
  policy: { rules: Array<{ id: string; tier: string }> };
  scope: Record<string, unknown> | null;
  toolchain: { rustc: string | null; cargo: string | null; clippy: string | null; rust_doctor: string; removed_lint_flags: string[] };
  scan: Record<string, unknown>;
  diagnostics: Diagnostic[];
  delta: { summary: { introduced: number; pre_existing: number; fixed: number }; introduced: string[] } | null;
  summary: Counts & { distinct: Counts; occurrences: Counts };
  gate: { blocking: string; status: string; blocking_diagnostics: number };
  timings?: Record<string, number>;
};

export function report(run: DoctorRun): Report {
  if (!run.json) throw new Error(`no JSON report (exit ${run.exitCode}): ${run.stderr.slice(0, 2000)}`);
  return run.json as Report;
}

// The report with the one field that differs between two identical scans.
export function withoutTimings(value: Report): Report {
  const copy = structuredClone(value);
  delete copy.timings;
  return copy;
}

// The diagnostics a baseline comparison called introduced.
export function introduced(value: Report): Diagnostic[] {
  const ids = new Set(value.delta?.introduced ?? []);
  return value.diagnostics.filter((diagnostic) => ids.has(diagnostic.id));
}

async function gitText(clone: SelfClone, args: string[]): Promise<string> {
  return clone.git(["--no-optional-locks", ...args]);
}

// The repository as `smoke-packed.mjs` `repositoryState()` reads it, with git
// told to take no optional lock, beside a fingerprint of every byte under the
// clone, `.git/index` time included. Any write a scan makes changes it.
export async function repositoryState(clone: SelfClone): Promise<string> {
  const fingerprint = treeFingerprint(clone.path);
  const parts = await Promise.all([
    gitText(clone, ["rev-parse", "HEAD", "HEAD^{tree}"]),
    gitText(clone, ["show-ref", "--head"]),
    gitText(clone, ["ls-files", "--stage", "-z"]),
    gitText(clone, ["status", "--porcelain=v2", "-z", "--untracked-files=all"]),
    gitText(clone, ["config", "--local", "--null", "--list"]),
    gitText(clone, ["count-objects", "-v"]),
  ]);
  return [fingerprint, ...parts].join("\n--\n");
}

// The pid and arguments of every process whose command line names `needle`,
// for a test that proves a scan left nothing of its own running.
export async function processesNaming(needle: string): Promise<string[]> {
  const ps = await runCommand(["ps", "-eo", "pid=,args="]);
  return ps.stdout.split("\n").filter((line) => line.includes(needle) && !line.includes(" ps -eo"));
}

// Polls until a process naming `needle` runs, the moment a scan is inside
// Cargo and a signal test can interrupt it.
export async function waitForProcess(needle: string, timeoutMs = 60_000): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if ((await processesNaming(needle)).length > 0) return;
    await Bun.sleep(50);
  }
  throw new Error(`no process naming ${needle} appeared within ${timeoutMs} ms`);
}

// Polls until no process names `needle`: a killed group takes a moment to go.
export async function waitForNoProcess(needle: string, timeoutMs = 5_000): Promise<string[]> {
  const deadline = Date.now() + timeoutMs;
  let left = await processesNaming(needle);
  while (left.length > 0 && Date.now() < deadline) {
    await Bun.sleep(50);
    left = await processesNaming(needle);
  }
  return left;
}

export function snapshotsIn(tmp: string): string[] {
  return readdirSync(tmp).filter((name) => name.startsWith("rust-doctor-baseline-"));
}

// A seeded `unwrap()` in a public module, committed on a scratch branch: one
// catalogued finding no other rule shares, and no dead code.
export const SEEDED_PATH = "src/progress.rs";
export const SEEDED_FUNCTION = "\npub fn e2e_seeded(value: Option<u8>) -> u8 {\n    value.unwrap()\n}\n";

export async function seedUnwrap(clone: SelfClone, branch = "e2e-seeded"): Promise<number> {
  const file = join(clone.path, SEEDED_PATH);
  const before = readFileSync(file, "utf8");
  await Bun.write(file, before + SEEDED_FUNCTION);
  await clone.git(["switch", "-q", "-c", branch]);
  await clone.git(["commit", "-q", "-am", "seed an unwrap"]);
  // The line of `value.unwrap()`: the appended text opens with a newline.
  return before.split("\n").length + 2;
}
