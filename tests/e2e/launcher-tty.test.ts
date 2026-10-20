// `tasks/e2e-catalog/real-codebases.md` section 3, item 5: the terminal passes
// through the launcher. Under tmux the interactive report opens as it does
// from the binary, follows a resize at the next key, leaves on `q` with the
// scan's code, and a Ctrl-C typed during the scan ends it as the terminal's
// signal should; off a terminal the linear report prints.
import { describe, expect, test } from "bun:test";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { copyWorkspace, defaultTarget, sandbox } from "./e2e-helpers";
import { installedLauncher, launcherEnv, runLauncher } from "./launcher-support";
import { alive, gone } from "./scope-snapshot-support";
import { MENU_HINT, openReport, requireTmux, TmuxSession } from "./tmux-helpers";

async function openThroughLauncher(workspace: string, size: { width: number; height: number }, env: Record<string, string> = {}) {
  const box = sandbox("launcher-tty");
  return TmuxSession.create([await installedLauncher(), workspace, "--advisory-db", "/dev/null"], {
    ...size,
    sandbox: box,
    env: { ...launcherEnv(box), NO_COLOR: "1", CARGO_TARGET_DIR: defaultTarget([workspace], box.dir), ...env },
  });
}

describe.skipIf(!requireTmux())("the interactive report through the launcher", () => {
  test("opens on a terminal, repaints at the new size after a key, and q exits 0", async () => {
    const workspace = copyWorkspace("kernel-contract/todo");
    const session = await openThroughLauncher(workspace, { width: 100, height: 30 });
    const direct = openReport(workspace, { width: 50, height: 20 });
    try {
      await session.waitForPane(MENU_HINT, 15_000);
      session.resize(50, 20);
      session.sendKeys("k");
      await direct.waitForPane(MENU_HINT, 15_000);
      const expected = await direct.waitForStable(300);
      await session.waitForPane((pane) => pane === expected);
      session.sendKeys("q");
      expect(await session.waitForExit()).toBe(0);
    } finally {
      session.kill();
      direct.kill();
    }
  });

  test("a Ctrl-C typed during the scan ends it with the interrupted report and no build left", async () => {
    const workspace = copyWorkspace("scan-failures/sleeping-build");
    const box = sandbox("launcher-tty-ctrl-c");
    const pidFile = join(box.dir, "sleeper.pid");
    const session = await openThroughLauncher(workspace, { width: 100, height: 30 }, { RUST_DOCTOR_SLEEPER_PID: pidFile });
    try {
      const deadline = Date.now() + 30_000;
      while (!existsSync(pidFile) && Date.now() < deadline) await Bun.sleep(25);
      const sleeper = Number(readFileSync(pidFile, "utf8"));
      expect(alive(sleeper)).toBe(true);
      session.sendKeys("C-c");
      await session.waitForExit();
      expect(session.capturePane({ history: true })).toContain("Scan failed: The scan was interrupted by a signal.");
      expect(await gone([sleeper])).toEqual([]);
    } finally {
      session.kill();
    }
  });
});

describe("off a terminal", () => {
  test("the launcher prints the linear report", async () => {
    const run = await runLauncher(await installedLauncher(), [copyWorkspace("kernel-contract/todo")]);
    expect(run.exitCode).toBe(0);
    expect(run.stdout).toStartWith("Scope: full codebase\n");
    expect(run.stdout).not.toContain(MENU_HINT);
  });
});
