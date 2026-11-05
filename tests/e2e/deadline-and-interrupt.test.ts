import { describe, expect, test } from "bun:test";
import { join } from "node:path";
import { BIN, doctorArgs, hermeticEnv, scratchDir, spawnTracked } from "./e2e-helpers";
import { waitForSnapshot } from "./git-scenario";
import { sharedClone } from "./self-clone";
import {
  processesNaming,
  report,
  scanArgs,
  scanSelf,
  SCAN_TIMEOUT_MS,
  selfEnv,
  snapshotsIn,
  TEST_TIMEOUT_MS,
  waitForNoProcess,
  waitForProcess,
  type Report,
} from "./self-scan-support";

const clone = await sharedClone();
const SETSID = Bun.which("setsid", { PATH: "/usr/bin:/bin" });

function coldTarget(label: string): string {
  return join(scratchDir(label), "target");
}

function spawnGroup(args: string[], target: string) {
  if (!SETSID) throw new Error("setsid is not installed");
  const env = hermeticEnv(clone.sandbox, { CARGO_TARGET_DIR: target, ...selfEnv() });
  return spawnTracked([SETSID, BIN, ...doctorArgs(scanArgs(clone, args))], { cwd: clone.sandbox.dir, env, timeoutMs: SCAN_TIMEOUT_MS });
}

describe("this repository: deadlines and interrupts", () => {
  test("INV-15 --max-duration 1 on a cold target exits 2 on its deadline and leaves nothing running", async () => {
    const target = coldTarget("deadline");
    const run = await scanSelf(clone, ["--json", "--max-duration", "1"], { target });
    expect(run.exitCode).toBe(2);
    const scan = report(run);
    expect(["incomplete", "failed"]).toContain(scan.status);
    expect(scan.audit.score?.authoritative ?? false).toBe(false);
    expect(scan.errors.map((error) => error.code)).toContain("deadline-exceeded");
    expect(await waitForNoProcess(target)).toEqual([]);
    expect(snapshotsIn(clone.sandbox.tmp)).toEqual([]);
  }, TEST_TIMEOUT_MS);

  test.skipIf(!SETSID)("INV-16 a group SIGINT while Clippy compiles exits 130 with an interrupted report and no orphan", async () => {
    const target = coldTarget("interrupt");
    const spawned = spawnGroup(["--json"], target);
    await waitForProcess(target);
    process.kill(-spawned.pid, "SIGINT");
    const exit = await spawned.exited;
    expect(exit.exitCode).toBe(130);
    const scan = JSON.parse(exit.stdout) as Report;
    expect(scan.status).toBe("failed");
    expect(scan.errors.map((error) => error.code)).toContain("interrupted");
    expect(scan.audit.score).toBeNull();
    expect(await waitForNoProcess(target)).toEqual([]);
    expect(snapshotsIn(clone.sandbox.tmp)).toEqual([]);
  }, TEST_TIMEOUT_MS);

  test.skipIf(!SETSID).each(["SIGINT", "SIGTERM", "SIGHUP"] as const)("a baseline interrupted by a group %s removes its snapshot and exits 130", async (signal) => {
    const target = coldTarget(`snapshot-${signal}`);
    const spawned = spawnGroup(["--json", "--scope", "baseline", "--base", await clone.sha()], target);
    await waitForSnapshot(clone.sandbox.tmp, 60_000);
    process.kill(-spawned.pid, signal);
    const exit = await spawned.exited;
    expect(exit.exitCode).toBe(130);
    expect((JSON.parse(exit.stdout) as Report).errors.map((error) => error.code)).toContain("interrupted");
    expect(snapshotsIn(clone.sandbox.tmp)).toEqual([]);
    expect(await waitForNoProcess(target)).toEqual([]);
    expect(await processesNaming(clone.sandbox.tmp)).toEqual([]);
  }, TEST_TIMEOUT_MS);
});
