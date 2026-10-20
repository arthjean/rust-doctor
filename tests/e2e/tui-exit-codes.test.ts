// `tasks/e2e-catalog/tui.md` section 3.12: whatever way the interactive report
// is left, the process exits with the scan's code, 130 for a signal, and the
// terminal is given back.
import { describe, expect, test } from "bun:test";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { BIN, copyWorkspace, defaultTarget, doctorArgs, sandbox, targetDir } from "./e2e-helpers";
import { fakeCalls, fakeEnv } from "./fakes";
import { CI_SETUP, MENU_HINT, openReport, requireTmux, TmuxSession } from "./tmux-helpers";
import { quit } from "./tui-entry-support";
import { tuiWorkspace, type TuiKind } from "./tui-support";

const HINT = /(?:^|\s)q quit\s*$/m;
const TASKSET = Bun.which("taskset", { PATH: "/usr/bin:/bin" });

// The first CPU this process may run on, so a pin never names one the job's
// cgroup forbids.
function firstCpu(): string {
  return readFileSync("/proc/self/status", "utf8").match(/^Cpus_allowed_list:\s*(\d+)/m)?.[1] ?? "0";
}

async function exitOf(kind: TuiKind, key: string, args: string[] = []): Promise<number | null> {
  const workspace = await tuiWorkspace(kind);
  const env = kind === "repo" ? { CARGO_TARGET_DIR: targetDir("repo") } : undefined;
  const session = openReport(workspace.path, { sandbox: workspace.sandbox, args, env });
  try {
    await session.waitForPane(HINT, 60_000);
    return await quit(session, key);
  } finally {
    session.kill();
  }
}

async function waitForFile(path: string, timeoutMs = 30_000): Promise<string> {
  const deadline = Date.now() + timeoutMs;
  while (!existsSync(path) || readFileSync(path, "utf8") === "") {
    if (Date.now() > deadline) throw new Error(`${path} never appeared`);
    await Bun.sleep(25);
  }
  return readFileSync(path, "utf8");
}

// The pid of the one child of `pid`: the binary under the shell of a pane.
function childOf(pid: number): number {
  for (const task of readdirSync(`/proc/${pid}/task`)) {
    const children = readFileSync(`/proc/${pid}/task/${task}/children`, "utf8").trim();
    if (children) return Number(children.split(" ")[0]);
  }
  throw new Error(`process ${pid} has no child`);
}

describe.skipIf(!requireTmux())("interactive report exit codes", () => {
  test("EXIT-01 q, Escape and Ctrl-C leave a passing scan with 0", async () => {
    const paths: Array<[TuiKind, string]> = [
      ["clean", "q"], ["clean", "Escape"], ["clean", "C-c"],
      ["todo", "q"], ["todo", "Escape"], ["todo", "C-c"],
      ["repo", "q"],
    ];
    for (const [kind, key] of paths) expect({ kind, key, code: await exitOf(kind, key) }).toEqual({ kind, key, code: 0 });
  }, 120_000);

  test("EXIT-02 q leaves an incomplete scan with 1", async () => {
    for (const kind of ["denied", "partial", "broken-member", "no-rust"] as const) {
      expect({ kind, code: await exitOf(kind, "q") }).toEqual({ kind, code: 1 });
    }
  });

  test("EXIT-03 q leaves a warning --blocking warning blocks with 1", async () => {
    expect(await exitOf("todo", "q", ["--blocking", "warning"])).toBe(1);
  });

  test("EXIT-04 q leaves with the --warning-exit-code a warning earns", async () => {
    expect(await exitOf("todo", "q", ["--warning-exit-code", "7"])).toBe(7);
  });

  test("EXIT-05 q leaves a scan cut by --max-duration with 2", async () => {
    expect(await exitOf("sleeping", "q", ["--max-duration", "1"])).toBe(2);
  });

  test("EXIT-06 a failed scan exits 2 without opening the report", async () => {
    for (const path of [copyWorkspace("scan-failures/bad-manifest"), join(sandbox("exit-06").dir, "missing")]) {
      const session = openReport(path);
      try {
        expect(await session.waitForExit()).toBe(2);
        expect(session.capturePane({ history: true })).not.toMatch(HINT);
      } finally {
        session.kill();
      }
    }
  });

  test("EXIT-07 adding the workflow leaves a passing scan with 0", async () => {
    const workspace = await tuiWorkspace("todo-git");
    const session = openReport(workspace.path, { sandbox: workspace.sandbox, env: fakeEnv(workspace.sandbox, "all") });
    try {
      await session.waitForPane(MENU_HINT, 15_000);
      session.sendKeys("Down");
      await session.waitForPane("❯ Add to GitHub Actions");
      session.sendKeys("Enter");
      await session.waitForPane(CI_SETUP);
      session.sendKeys("Enter");
      expect(await session.waitForExit()).toBe(0);
      expect(existsSync(join(workspace.path, ".github/workflows/rust-doctor.yml"))).toBe(true);
    } finally {
      session.kill();
    }
  });

  test("EXIT-08 an agent that exits 9 leaves the process with the scan's 1", async () => {
    const workspace = await tuiWorkspace("denied");
    const session = openReport(workspace.path, {
      sandbox: workspace.sandbox,
      env: { ...fakeEnv(workspace.sandbox, "agents"), RD_FAKE_CLAUDE_EXIT: "9" },
    });
    try {
      await session.waitForPane(MENU_HINT, 15_000);
      session.sendKeys("Down");
      await session.waitForPane("❯ Hand off to an agent");
      session.sendKeys("Enter");
      await session.waitForPane("❯ Claude Code");
      session.sendKeys("Enter");
      expect(await session.waitForExit()).toBe(1);
      expect(fakeCalls(workspace.sandbox, "claude")).toHaveLength(1);
    } finally {
      session.kill();
    }
  });

  test("EXIT-09 Ctrl-C during the scan exits 130", async () => {
    const box = sandbox("exit-09");
    const pidFile = join(box.dir, "sleeper.pid");
    const session = openReport(copyWorkspace("scan-failures/sleeping-build"), { sandbox: box, env: { RUST_DOCTOR_SLEEPER_PID: pidFile } });
    try {
      await waitForFile(pidFile);
      session.sendKeys("C-c");
      expect(await session.waitForExit()).toBe(130);
    } finally {
      session.kill();
    }
  });

  test.failing("EXIT-10 tui/BUG-05 Ctrl-C during the count-up leaves with the scan's code", async () => {
    const workspace = await tuiWorkspace("broken-member");
    const session = openReport(workspace.path, { sandbox: workspace.sandbox, color: true });
    try {
      await session.waitForPane("/ 100", 15_000);
      session.sendKeys("C-c");
      expect(await session.waitForExit()).toBe(1);
    } finally {
      session.kill();
    }
  });

  // The binary under a shell that records its exit status and the terminal
  // modes it left, then signaled while the report waits for a key.
  // `pinned` runs it on one CPU, where the race of tui/BUG-14 is lost about
  // two runs in five whatever the machine.
  async function signalWaiting(signal: NodeJS.Signals, pinned = false): Promise<{ code: string; modes: string; log: string }> {
    const workspace = await tuiWorkspace("denied");
    const after = join(workspace.sandbox.dir, "stty.txt");
    if (pinned && !TASKSET) throw new Error("taskset is not installed: the tui/BUG-14 row needs a one-CPU pin to lose the race");
    const pin = pinned ? `${TASKSET} -c ${firstCpu()} ` : "";
    const session = TmuxSession.create(
      ["sh", "-c", `${pin}"$0" "$@"; echo $? > "$STTY.status"; stty -a > "$STTY"`, BIN, ...doctorArgs([workspace.path])],
      {
        sandbox: workspace.sandbox,
        width: 100,
        height: 30,
        env: { NO_COLOR: "1", STTY: after, CARGO_TARGET_DIR: defaultTarget([workspace.path], workspace.path) },
      },
    );
    try {
      await session.waitForPane(MENU_HINT, 15_000);
      await session.waitForStable(200);
      process.kill(childOf(session.panePid()), signal);
      await session.waitForExit();
      return {
        code: (await waitForFile(`${after}.status`)).trim(),
        modes: await waitForFile(after),
        log: session.byteLog(),
      };
    } finally {
      session.kill();
    }
  }

  // The report's own quit path can win the race with the signal's exit, and
  // the process then exits with the scan's code: twelve pinned runs make a
  // pass by chance unlikely (0.6^12).
  test.failing("EXIT-11 tui/BUG-14 SIGTERM, SIGHUP and SIGINT while the report waits always exit 130", async () => {
    const codes: string[] = [];
    for (let round = 0; round < 4; round += 1) {
      for (const signal of ["SIGTERM", "SIGHUP", "SIGINT"] as const) codes.push((await signalWaiting(signal, true)).code);
    }
    expect(codes).toEqual(Array.from({ length: 12 }, () => "130"));
  }, 60_000);

  test.each(["SIGTERM", "SIGHUP", "SIGINT"] as const)("%s while the report waits gives the terminal back", async (signal) => {
    const { modes, log } = await signalWaiting(signal);
    expect(modes).toMatch(/(?:^|\s)icanon(?:\s|$)/m);
    expect(modes).toMatch(/(?:^|\s)echo(?:\s|$)/m);
    expect(log.lastIndexOf("\x1b[?25h")).toBeGreaterThan(log.lastIndexOf("\x1b[?25l"));
  });

  test("EXIT-12 a terminal that fails under the report says so and keeps the scan's code", async () => {
    const workspace = await tuiWorkspace("denied");
    // A second pane lends its terminal as stdin; ending that pane hangs the
    // terminal up under the report's read, which fails with an I/O error.
    const ttyFile = join(workspace.sandbox.dir, "tty.txt");
    const lender = TmuxSession.create(["sh", "-c", 'readlink /proc/$$/fd/0 > "$0"; exec sleep 600', ttyFile], { sandbox: workspace.sandbox });
    try {
      const tty = (await waitForFile(ttyFile, 5_000)).trim();
      const session = TmuxSession.create(["sh", "-c", 'exec "$0" "$@" < "$LENT_TTY"', BIN, ...doctorArgs([workspace.path])], {
        sandbox: workspace.sandbox,
        width: 100,
        height: 30,
        env: { NO_COLOR: "1", LENT_TTY: tty, CARGO_TARGET_DIR: defaultTarget([workspace.path], workspace.path) },
      });
      try {
        await session.waitForPane(MENU_HINT, 15_000);
        lender.kill();
        expect(await session.waitForExit()).toBe(1);
        expect(session.capturePane({ history: true })).toContain("Interactive report unavailable: Input/output error (os error 5)");
      } finally {
        session.kill();
      }
    } finally {
      lender.kill();
    }
  });
});
