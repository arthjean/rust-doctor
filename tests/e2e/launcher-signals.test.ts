// `tasks/e2e-catalog/real-codebases.md` section 3, item 1: signals through the
// installed launcher. A signal to the launcher's pid is what a supervisor
// sends; one to its process group is a terminal's Ctrl-C, which reaches the
// native binary twice, from the terminal and from the launcher's forward
// (real/B2, fixed by US-010). Every case runs a baseline scan held in a
// sleeping build script, so a leftover snapshot or Cargo group shows.
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

// Starts the launcher on a baseline scan of a fresh slow repository, in a
// session of its own when `group` is set, and resolves once Cargo runs the
// build script.
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
    // The first press may already have published the report; the second
    // either finds the run over or takes the immediate exit, which prints
    // nothing.
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

  // The binary documents 130 for a SIGINT, SIGTERM or SIGHUP; the launcher
  // ends itself by the signal it forwarded, so a shell reads 143 for SIGTERM.
  test.failing("real/B7 a SIGTERM through the launcher exits 130, the status the binary documents", async () => {
    const held = await heldScan("launcher-sigterm-status");
    held.spawned.proc.kill("SIGTERM");
    expect(shellStatus(await held.spawned.result)).toBe(130);
  });
});
