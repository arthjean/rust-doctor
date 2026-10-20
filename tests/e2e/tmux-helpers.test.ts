// The tmux driver, proven on a shell before any interactive-report test uses
// it: size, keys, the three captures, the waits and their errors, resize, the
// byte log, the exit status, the sealed environment and the kill.
import { afterEach, describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { runCommand, sandbox, toolchainBin } from "./e2e-helpers";
import { requireTmux, TmuxSession, zombieExit } from "./tmux-helpers";

let session: TmuxSession | undefined;

afterEach(() => {
  session?.kill();
  session = undefined;
});

function shell(width = 60, height = 12): TmuxSession {
  session = TmuxSession.create(["sh"], { width, height, env: { PS1: "$ " } });
  return session;
}

describe.skipIf(!requireTmux())("tmux driver", () => {
  test("the pane has the size it was created with, and resizes", async () => {
    const pane = shell(50, 12);
    expect(pane.size()).toEqual({ cols: 50, rows: 12 });
    pane.sendLiteral("stty size");
    pane.sendKeys("Enter");
    await pane.waitForPane("12 50");
    pane.resize(90, 20);
    expect(pane.size()).toEqual({ cols: 90, rows: 20 });
    pane.sendLiteral("stty size");
    pane.sendKeys("Enter");
    await pane.waitForPane("20 90");
  });

  test("captures plain, with escapes and with history", async () => {
    const pane = shell(60, 8);
    pane.sendLiteral("printf '\\033[31mred\\033[0m\\n'; i=1; while [ $i -le 40 ]; do echo line-$i; i=$((i+1)); done");
    pane.sendKeys("Enter");
    await pane.waitForPane("line-40");
    expect(pane.capturePane()).not.toContain("\x1b[");
    expect(pane.capturePane({ escapes: true, history: true })).toContain("\x1b[31m");
    expect(pane.capturePane()).not.toContain("line-1\n");
    expect(pane.capturePane({ history: true })).toContain("line-1\n");
  });

  test("a wait that times out names the last pane", async () => {
    const pane = shell();
    pane.sendLiteral("echo visible-text");
    pane.sendKeys("Enter");
    await pane.waitForPane("visible-text");
    await expect(pane.waitForPane("never-shown", 200)).rejects.toThrow(/timed out after 200 ms waiting for never-shown[\s\S]*visible-text/);
  });

  test("waitForStable returns once the pane stops changing", async () => {
    const pane = shell();
    pane.sendLiteral("for i in 1 2 3 4 5; do echo tick-$i; sleep 0.1; done");
    pane.sendKeys("Enter");
    const stable = await pane.waitForStable(300);
    expect(stable).toContain("tick-5");
  });

  test("the byte log holds every byte the program wrote, escapes included", async () => {
    const pane = shell();
    pane.sendLiteral("printf '\\033[?25l\\033[?25h'; echo logged");
    pane.sendKeys("Enter");
    await pane.waitForPane(/^logged$/m);
    expect(pane.byteLog()).toContain("\x1b[?25l\x1b[?25h");
  });

  test("the pane stays after its program exits and reports the status", async () => {
    session = TmuxSession.create(["sh", "-c", "echo bye; exit 7"], { width: 40, height: 6 });
    expect(await session.waitForExit()).toBe(7);
    // tmux writes its "Pane is dead" line at the bottom and scrolls the
    // program's output up: read a dead pane with its history.
    expect(session.capturePane({ history: true })).toContain("bye");
  });

  test("the program runs under the sealed environment", async () => {
    const saved = process.env.CLAUDECODE;
    process.env.CLAUDECODE = "1";
    try {
      const box = sandbox("tmux-env");
      // Written to a file rather than read off the screen: the pane wraps a
      // line as long as the scratch root, which RUST_DOCTOR_E2E_ROOT can make
      // any length.
      session = TmuxSession.create(["sh", "-c", "{ env | sort; pwd; } > seen.txt"], { width: 120, height: 30, sandbox: box });
      expect(await session.waitForExit()).toBe(0);
      const seen = readFileSync(join(box.dir, "seen.txt"), "utf8");
      expect(seen).toContain("TERM=xterm-256color\n");
      expect(seen).toContain(`HOME=${box.home}\n`);
      expect(seen).not.toContain("CLAUDECODE");
      expect(seen).not.toContain("TMUX=");
      expect(seen).toEndWith(`${box.dir}\n`);
    } finally {
      if (saved === undefined) delete process.env.CLAUDECODE;
      else process.env.CLAUDECODE = saved;
    }
  });

  test("kill is idempotent and ends the server", () => {
    const pane = shell();
    pane.kill();
    pane.kill();
    expect(() => pane.size()).toThrow();
    session = undefined;
  });
});

describe("TUI file gate", () => {
  async function gate(env: Record<string, string>) {
    const box = sandbox("gate");
    return runCommand([process.execPath, "test", "./probes/tui-gate.probe.ts"], {
      sandbox: box,
      cwd: import.meta.dir,
      env: { PATH: toolchainBin(), BUN_INSTALL_CACHE_DIR: join(box.tmp, "bun-cache"), ...env },
    });
  }

  test("without tmux a TUI file skips with one visible reason", async () => {
    const run = await gate({});
    expect(run.exitCode).toBe(0);
    expect(run.stdout + run.stderr).toContain("tmux is not installed: TUI files skipped");
    expect(run.stdout + run.stderr).toMatch(/1 skip/);
  });

  test("without tmux and with E2E_REQUIRE_TMUX=1 a TUI file fails", async () => {
    const run = await gate({ E2E_REQUIRE_TMUX: "1" });
    expect(run.exitCode).not.toBe(0);
    expect(run.stdout + run.stderr).toContain("tmux is not installed and E2E_REQUIRE_TMUX=1 requires it");
  });
});

// A child that ended under a parent that never waits stays a zombie: the
// subshell exits, and the `sh` that started it has become a `sleep`.
async function zombie(script: string): Promise<{ pid: number; parent: ReturnType<typeof Bun.spawn> }> {
  const parent = Bun.spawn(["sh", "-c", `${script} & echo $!; exec sleep 5`], { stdout: "pipe" });
  const reader = parent.stdout.getReader();
  const { value } = await reader.read();
  reader.releaseLock();
  const pid = Number(new TextDecoder().decode(value).trim());
  const deadline = Date.now() + 3_000;
  while (Date.now() < deadline && !readFileSync(`/proc/${pid}/stat`, "utf8").includes(") Z ")) await Bun.sleep(10);
  return { pid, parent };
}

describe.skipIf(process.platform !== "linux")("zombie exit status", () => {
  test("an exit status and a signal are read back from a zombie, a live or gone process gives nothing", async () => {
    const exited = await zombie("(exit 7)");
    const signaled = await zombie(`sh -c 'kill -TERM $$'`);
    try {
      expect(zombieExit(exited.pid)).toEqual({ status: 7 });
      expect(zombieExit(signaled.pid)).toEqual({ status: null });
      expect(zombieExit(process.pid)).toBeNull();
      expect(zombieExit(2 ** 22 + 1)).toBeNull();
    } finally {
      exited.parent.kill("SIGKILL");
      signaled.parent.kill("SIGKILL");
    }
  });
});
