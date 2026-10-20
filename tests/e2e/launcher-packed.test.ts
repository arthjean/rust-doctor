// `tasks/e2e-catalog/real-codebases.md` section 3, item 2: the launcher an
// `npx rust-doctor` or `npm install -g` user runs is the binary, byte for
// byte. Each case runs the release binary and the installed launcher on the
// same workspace, target directory and environment, and compares stdout,
// stderr and the exit code; a scan's only volatile bytes, its timings, are
// removed first.
import { describe, expect, test } from "bun:test";
import { existsSync } from "node:fs";
import { join } from "node:path";
import { copyWorkspace, normalizeReport, runDoctor, sandbox, scratchDir, spawnDoctor, type DoctorRun } from "./e2e-helpers";
import { installedLauncher, runLauncher, shellStatus, spawnLauncher } from "./launcher-support";

// The linear report states its own duration, the JSON one its timings.
function stable(run: DoctorRun): { status: number | null; stdout: unknown; stderr: string } {
  const stdout = run.json !== undefined ? normalizeReport(run.json) : run.stdout.replace(/ in \d+(?:\.\d+)?s$/m, " in <duration>");
  return { status: shellStatus(run), stdout, stderr: run.stderr };
}

async function parity(args: string[], workspace?: string): Promise<DoctorRun> {
  const launcher = await installedLauncher();
  const target = scratchDir("parity-target");
  const full = workspace ? [workspace, ...args] : args;
  const direct = await runDoctor(full, { target });
  const wrapped = await runLauncher(launcher, full, { target });
  expect(stable(wrapped)).toEqual(stable(direct));
  return direct;
}

async function waitFor(path: string): Promise<void> {
  const deadline = Date.now() + 30_000;
  while (!existsSync(path) && Date.now() < deadline) await Bun.sleep(25);
  expect(existsSync(path)).toBe(true);
}

describe("the installed launcher answers as the binary does", () => {
  test("--help", async () => {
    const run = await parity(["--help"]);
    expect(run.exitCode).toBe(0);
    expect(run.stdout).toContain("Exit status:");
  });

  test("rules list --json", async () => {
    const run = await parity(["rules", "list", "--json"]);
    expect(run.exitCode).toBe(0);
    expect(Array.isArray(JSON.parse(run.stdout))).toBe(true);
  });

  test("rules explain", async () => {
    const run = await parity(["rules", "explain", "clippy::todo"]);
    expect(run.exitCode).toBe(0);
    expect(run.stdout).toContain("clippy::todo");
  });

  test("exit 0: a scan whose gate passed, linear and JSON", async () => {
    const workspace = copyWorkspace("kernel-contract/todo");
    expect((await parity(["--yes"], workspace)).exitCode).toBe(0);
    expect((await parity(["--yes", "--json"], workspace)).exitCode).toBe(0);
  });

  test("exit 1: a gate that fails under --blocking warning", async () => {
    const run = await parity(["--yes", "--json", "--blocking", "warning"], copyWorkspace("kernel-contract/todo"));
    expect(run.exitCode).toBe(1);
  });

  test("exit 2: an argument error", async () => {
    const run = await parity(["--bogus"], copyWorkspace("kernel-contract/todo"));
    expect(run.exitCode).toBe(2);
    expect(run.stdout).toBe("");
    expect(run.stderr).toContain("unexpected argument '--bogus'");
  });

  test("the warning exit code", async () => {
    const run = await parity(["--yes", "--json", "--warning-exit-code", "42"], copyWorkspace("kernel-contract/todo"));
    expect(run.exitCode).toBe(42);
  });

  test("exit 130: a SIGINT during the build, sent to the process the user started", async () => {
    const launcher = await installedLauncher();
    const workspace = copyWorkspace("scan-failures/sleeping-build");
    const target = scratchDir("interrupt-target");
    const runs: DoctorRun[] = [];
    for (const via of ["direct", "launcher"] as const) {
      const box = sandbox(`interrupt-${via}`);
      const pidFile = join(box.dir, "sleeper.pid");
      const options = { sandbox: box, target, env: { RUST_DOCTOR_SLEEPER_PID: pidFile } };
      const spawned = via === "direct"
        ? spawnDoctor([workspace, "--yes", "--json"], options)
        : spawnLauncher(launcher, [workspace, "--yes", "--json"], options);
      await waitFor(pidFile);
      spawned.proc.kill("SIGINT");
      runs.push(await spawned.result);
    }
    const [direct, wrapped] = runs as [DoctorRun, DoctorRun];
    expect(direct.exitCode).toBe(130);
    expect(stable(wrapped)).toEqual(stable(direct));
    expect((direct.json as { status: string }).status).toBe("failed");
  });
});
