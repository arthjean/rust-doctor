// What the baseline and integrity files of US-022 share: a workspace whose
// build script holds Cargo long enough to interrupt or cut a scan, the
// snapshots a run leaves in its TMPDIR, and whether a child it started
// still runs.
import { existsSync, mkdirSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { gitScenario, miniTemplate, NEW_FN, type Repo } from "./git-scenario";

// Records its pid and process group under `E2E_BUILD_PIDS`, then sleeps:
// every scan of the workspace stops in Cargo, on the base side first.
const SLOW_BUILD = `fn main() {
    if let Ok(dir) = std::env::var("E2E_BUILD_PIDS") {
        let stat = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
        let _ = std::fs::write(format!("{dir}/{}", std::process::id()), stat);
    }
    std::thread::sleep(std::time::Duration::from_secs(20));
}
`;

export type SlowRepo = Repo & { pids: string; env: Record<string, string> };

// `mini` with the sleeping build script on `main`, and a branch `feature`
// holding NEWFN, committed or staged.
export async function slowRepo(name: string, change: "commit" | "stage" = "commit"): Promise<SlowRepo> {
  const scenario = gitScenario(name).template("mini").write({ "build.rs": SLOW_BUILD }).commit("base").branch("feature");
  const repo = await (change === "commit"
    ? scenario.append("src/lib.rs", NEW_FN).commit("feature")
    : scenario.stage({ write: { "src/lib.rs": miniTemplate()["src/lib.rs"]! + NEW_FN } })
  ).build();
  const pids = join(repo.sandbox.dir, "build-pids");
  mkdirSync(pids);
  return Object.assign(repo, { pids, env: { RUST_DOCTOR_NO_CACHE: "1", E2E_BUILD_PIDS: pids } });
}

// Resolves with the pids of the build scripts once one runs.
export async function waitForBuildScript(repo: SlowRepo, timeoutMs = 20_000): Promise<number[]> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const found = readdirSync(repo.pids);
    if (found.length > 0) return found.map(Number);
    await Bun.sleep(25);
  }
  throw new Error(`no build script started within ${timeoutMs} ms`);
}

// The process group a build script recorded: Cargo's, which the binary
// starts in a group of its own.
export function buildGroup(repo: SlowRepo, pid: number): number {
  const stat = readFileSync(join(repo.pids, String(pid)), "utf8");
  const fields = stat.slice(stat.lastIndexOf(")") + 2).split(" ");
  return Number(fields[2]);
}

export function snapshots(tmp: string): string[] {
  return readdirSync(tmp).filter((name) => name.startsWith("rust-doctor-baseline-")).sort();
}

// A zombie counts as gone: it runs nothing and only waits to be reaped.
export function alive(pid: number): boolean {
  const stat = `/proc/${pid}/stat`;
  if (!existsSync(stat)) return false;
  try {
    const state = readFileSync(stat, "utf8");
    return state.slice(state.lastIndexOf(")") + 2, state.lastIndexOf(")") + 3) !== "Z";
  } catch {
    return false;
  }
}

// Waits for every pid to end, for a child a signal reaches a moment after
// its parent exits.
export async function gone(pids: number[], timeoutMs = 5_000): Promise<number[]> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (!pids.some(alive)) return [];
    await Bun.sleep(25);
  }
  return pids.filter(alive);
}
