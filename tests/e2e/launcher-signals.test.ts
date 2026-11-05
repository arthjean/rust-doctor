import { describe, expect, test } from "bun:test";
import { installedLauncher, shellStatus, spawnLauncher } from "./launcher-support";
import { alive, buildGroup, gone, slowRepo, snapshots, waitForBuildScript, type SlowRepo } from "./scope-snapshot-support";

const SETSID = Bun.which("setsid", { PATH: "/usr/bin:/bin" });

type Report = { status: string; errors: Array<{ stage: string; code: string }> };

function groupAlive(pgid: number): boolean {
  try {
    process.kill(-pgid, 0);
    return true;
  } catch {
    return false;
  }
}

async function groupGone(pgid: number, timeoutMs = 5_000): Promise<boolean> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (!groupAlive(pgid)) return true;
    await Bun.sleep(25);
  }
  return !groupAlive(pgid);
}

async function heldScan(name: string, group = false) {
  const repo: SlowRepo = await slowRepo(name);
  const launcher = await installedLauncher();
  const spawned = spawnLauncher(launcher, [repo.path, "--yes", "--json", "--scope", "baseline", "--base", "main"], {
    sandbox: repo.sandbox,
    env: repo.env,
    prefix: group ? [SETSID!] : [],
    timeoutMs: 60_000,
  });
  const pids = await waitForBuildScript(repo);
  const pgid = buildGroup(repo, pids[0]!);
  return { repo, spawned, pids, pgid };
}

async function expectCleanInterrupt(held: Awaited<ReturnType<typeof heldScan>>): Promise<void> {
  const run = await held.spawned.result;
  expect(shellStatus(run)).toBe(130);
  const report = JSON.parse(run.stdout) as Report;
  expect(report.status).toBe("failed");
  expect(report.errors.map((error) => error.code)).toEqual(["interrupted"]);
  expect(await gone(held.pids)).toEqual([]);
  expect(await groupGone(held.pgid)).toBe(true);
  expect(snapshots(held.repo.sandbox.tmp)).toEqual([]);
}

describe("signals through the launcher", () => {
  test("a SIGINT to the launcher's pid ends in one interrupted report", async () => {
    const held = await heldScan("launcher-sigint");
    held.spawned.proc.kill("SIGINT");
    await expectCleanInterrupt(held);
  });

  test.skipIf(!SETSID)("a terminal Ctrl-C, SIGINT to the whole group, ends in one interrupted report (real/B2)", async () => {
    const held = await heldScan("launcher-group", true);
    process.kill(-held.spawned.pid, "SIGINT");
    await expectCleanInterrupt(held);
  });

  test.skipIf(!SETSID)("a second Ctrl-C after the duplicate window still ends the run at once, leaving no process", async () => {
    const held = await heldScan("launcher-double", true);
    process.kill(-held.spawned.pid, "SIGINT");
    await Bun.sleep(600);
    try {
      process.kill(-held.spawned.pid, "SIGINT");
    } catch {}
    const run = await held.spawned.result;
    expect(shellStatus(run)).toBe(130);
    if (run.stdout.length > 0) expect((JSON.parse(run.stdout) as Report).status).toBe("failed");
    expect(await gone(held.pids)).toEqual([]);
    expect(await groupGone(held.pgid)).toBe(true);
    expect(snapshots(held.repo.sandbox.tmp)).toEqual([]);
  });

  test("a SIGTERM to the launcher's pid ends in one interrupted report, no process left", async () => {
    const held = await heldScan("launcher-sigterm");
    held.spawned.proc.kill("SIGTERM");
    const run = await held.spawned.result;
    expect((JSON.parse(run.stdout) as Report).errors.map((error) => error.code)).toEqual(["interrupted"]);
    expect(await gone(held.pids)).toEqual([]);
    expect(await groupGone(held.pgid)).toBe(true);
    expect(snapshots(held.repo.sandbox.tmp)).toEqual([]);
    expect(alive(held.spawned.pid)).toBe(false);
  });

  test.failing("real/B7 a SIGTERM through the launcher exits 130, the status the binary documents", async () => {
    const held = await heldScan("launcher-sigterm-status");
    held.spawned.proc.kill("SIGTERM");
    expect(shellStatus(await held.spawned.result)).toBe(130);
  });
});
