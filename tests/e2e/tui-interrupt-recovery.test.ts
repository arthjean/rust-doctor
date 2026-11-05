import { describe, expect, test } from "bun:test";
import { join } from "node:path";
import { copyWorkspace } from "./e2e-helpers";
import { agentLog, agentPath, alive, probeAgent, settledLog, waitForFile } from "./tui-actions-support";
import { MENU_HINT, openReport, requireTmux, TmuxSession } from "./tmux-helpers";
import { tuiWorkspace } from "./tui-support";

const INTERRUPTED = "Scan failed: The scan was interrupted by a signal. (clippy/interrupted)";
const HIDE = "\x1b[?25l";
const SHOW = "\x1b[?25h";

async function scanning(): Promise<{ session: TmuxSession; sleeper: number }> {
  const ws = await tuiWorkspace("sleeping");
  const pidFile = join(ws.sandbox.dir, "sleeper.pid");
  const session = openReport(ws.path, { sandbox: ws.sandbox, env: { RUST_DOCTOR_SLEEPER_PID: pidFile } });
  return { session, sleeper: Number(await waitForFile(pidFile, 60_000)) };
}

async function gone(pid: number, timeoutMs = 3_000): Promise<boolean> {
  const deadline = Date.now() + timeoutMs;
  while (alive(pid) && Date.now() < deadline) await Bun.sleep(25);
  return !alive(pid);
}

async function endsInterrupted(interrupt: (session: TmuxSession) => void): Promise<void> {
  const { session, sleeper } = await scanning();
  try {
    const started = Date.now();
    interrupt(session);
    expect(await session.waitForExit()).toBe(130);
    expect(Date.now() - started).toBeLessThan(1_500);
    const pane = session.capturePane({ history: true });
    expect(pane).toContain(INTERRUPTED);
    expect(pane.split(INTERRUPTED)).toHaveLength(2);
    expect(pane).not.toContain(MENU_HINT);
    expect(await gone(sleeper)).toBe(true);
  } finally {
    session.kill();
  }
}

async function restoresCursor(signal: NodeJS.Signals): Promise<void> {
  const ws = await tuiWorkspace("denied");
  const session = openReport(ws.path, { sandbox: ws.sandbox });
  try {
    await session.waitForPane(MENU_HINT, 15_000);
    process.kill(session.panePid(), signal);
    expect(await session.waitForExit()).toBe(130);
    const log = await settledLog(session);
    expect(log.lastIndexOf(HIDE)).toBeGreaterThanOrEqual(0);
    expect(log.lastIndexOf(SHOW)).toBeGreaterThan(log.lastIndexOf(HIDE));
  } finally {
    session.kill();
  }
}

describe.skipIf(!requireTmux())("interactive report interrupts", () => {
  test("SIG-01 Ctrl-C during the scan ends it with 130 and the failure line, no menu, no build left", async () => {
    await endsInterrupted((session) => session.sendKeys("C-c"));
  });

  test("SIG-02 a double Ctrl-C in one breath is one interruption, reported once", async () => {
    await endsInterrupted((session) => session.sendKeys("C-c", "C-c"));
  });

  test("SIG-03 SIGTERM during the scan ends it as Ctrl-C does", async () => {
    await endsInterrupted((session) => process.kill(session.panePid(), "SIGTERM"));
  });

  test.failing("SIG-04 tui/BUG-05 Ctrl-C during the count-up exits with the scan's code, as q does", async () => {
    const session = openReport(copyWorkspace("kernel-contract/todo"), { color: true, width: 100, height: 40 });
    try {
      await session.waitForPane("/ 100", 15_000);
      session.sendKeys("C-c");
      expect(await session.waitForExit()).toBe(0);
    } finally {
      session.kill();
    }
  });

  test("SIG-05 Ctrl-C once the menu is up exits with the scan's code and shows the cursor last", async () => {
    const ws = await tuiWorkspace("denied");
    const session = openReport(ws.path, { sandbox: ws.sandbox, color: true });
    try {
      await session.waitForPane(MENU_HINT, 15_000);
      session.sendKeys("C-c");
      expect(await session.waitForExit()).toBe(1);
      const log = await settledLog(session);
      expect(log.split(HIDE)).toHaveLength(2);
      expect(log.split(SHOW)).toHaveLength(2);
      expect(log).toEndWith(SHOW);
    } finally {
      session.kill();
    }
  });

  test("SIG-06 SIGTERM while the report waits exits 130 and gives the cursor back", async () => {
    await restoresCursor("SIGTERM");
  });

  test("SIG-07 SIGINT while the report waits exits 130 and gives the cursor back", async () => {
    await restoresCursor("SIGINT");
  });

  test("SIG-08 SIGHUP while the report waits exits 130 and gives the cursor back", async () => {
    await restoresCursor("SIGHUP");
  });

  test("SIG-09 Ctrl-C while a launched agent runs ends the agent and the report", async () => {
    const ws = await tuiWorkspace("todo");
    const env = { PATH: agentPath({ codex: probeAgent("codex", { sleep: 30 }) }), RD_AGENT_LOG: agentLog(ws.sandbox) };
    const session = openReport(ws.path, { sandbox: ws.sandbox, env });
    let agent = 0;
    try {
      await session.waitForPane(MENU_HINT, 15_000);
      session.sendKeys("Down");
      await session.waitForPane("❯ Hand off to an agent");
      session.sendKeys("Enter");
      await session.waitForPane("❯ Codex");
      session.sendKeys("Enter");
      agent = Number(await waitForFile(join(agentLog(ws.sandbox), "codex.pid")));
      await session.waitForPane("FAKE-AGENT codex RAN");
      session.sendKeys("C-c");
      expect([0, 130]).toContain((await session.waitForExit()) ?? -1);
      expect(await gone(agent)).toBe(true);
    } finally {
      if (agent > 0 && alive(agent)) process.kill(agent, "SIGKILL");
      session.kill();
    }
  });
});
