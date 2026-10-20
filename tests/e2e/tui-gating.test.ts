// `tasks/e2e-catalog/tui.md` section 3.1: when the interactive report opens.
// The report opens only when stdin and stdout are both terminals and nothing
// says the run is unattended; every other run prints the linear report. The
// rows that only ask that question run on a pseudo-terminal, the ones that
// look at a screen or a signal under tmux.
import { beforeAll, describe, expect, test } from "bun:test";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { copyWorkspace, NON_INTERACTIVE_MARKERS, sandbox } from "./e2e-helpers";
import { MENU_HINT, openReport, requireTmux, TmuxSession } from "./tmux-helpers";
import { ptyReport, SGR } from "./tui-entry-support";
import { tuiWorkspace } from "./tui-support";

const LINEAR = "Scope: full codebase";

let todo = "";

beforeAll(async () => {
  todo = copyWorkspace("kernel-contract/todo");
  // One cold build, so every later run of the same copy is warm.
  expect((await ptyReport(todo, { args: ["--yes"] })).exitCode).toBe(0);
});

function widest(text: string): number {
  return Math.max(...text.split("\n").filter((line) => !line.includes("https://")).map((line) => [...line].length));
}

function alive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

describe.skipIf(!requireTmux())("interactive report gating", () => {
  test("GATE-01 a terminal opens the landing instead of the linear report", async () => {
    const session = openReport(todo, { width: 100, height: 40 });
    try {
      const pane = await session.waitForPane(MENU_HINT, 15_000);
      expect(pane).not.toContain("All 3 occurrences");
      expect(pane).not.toContain(LINEAR);
      session.sendKeys("q");
      expect(await session.waitForExit()).toBe(0);
    } finally {
      session.kill();
    }
  });

  test("GATE-02 --json prints one document and no menu", async () => {
    const run = await ptyReport(todo, { args: ["--json"] });
    expect(run.landing).toBe(false);
    expect(run.exitCode).toBe(0);
    expect(run.output).not.toMatch(SGR);
    expect((JSON.parse(run.text) as { status: string }).status).toBe("complete");
  });

  test("GATE-03 --yes prints the linear report without rewinding", async () => {
    const run = await ptyReport(todo, { args: ["--yes"] });
    expect(run.landing).toBe(false);
    expect(run.exitCode).toBe(0);
    expect(run.text).toContain(LINEAR);
    expect(run.text).toContain("94 / 100 Great");
    expect(run.output).not.toMatch(/\x1b\[\d+A/);
  });

  test("GATE-04 --verbose prints the linear report with every occurrence", async () => {
    const run = await ptyReport(todo, { args: ["--verbose"] });
    expect(run.landing).toBe(false);
    expect(run.exitCode).toBe(0);
    for (const site of ["src/lib.rs:4:5", "src/lib.rs:8:5", "src/lib.rs:12:5"]) expect(run.text).toContain(site);
  });

  test.each(NON_INTERACTIVE_MARKERS)("GATE-05 %s=1 keeps the linear report on a terminal", async (marker) => {
    const run = await ptyReport(todo, { env: { [marker]: "1" } });
    expect(run.landing).toBe(false);
    expect(run.text).toContain(LINEAR);
    expect(run.exitCode).toBe(0);
  });

  test.each(NON_INTERACTIVE_MARKERS)("GATE-06 %s set to the empty string counts as unset", async (marker) => {
    const run = await ptyReport(todo, { env: { [marker]: "" } });
    expect(run.landing).toBe(true);
    expect(run.exitCode).toBe(0);
  });

  test.each(["false", "0", "FALSE", ""])("GATE-07 CI=%s says the shell is not CI, so the landing opens", async (value) => {
    const run = await ptyReport(todo, { env: { CI: value } });
    expect(run.landing).toBe(true);
    expect(run.exitCode).toBe(0);
  });

  test.each(["1", "true", "yes", "off", "no"])("GATE-08 CI=%s keeps the linear report", async (value) => {
    const run = await ptyReport(todo, { env: { CI: value } });
    expect(run.landing).toBe(false);
    expect(run.text).toContain(LINEAR);
  });

  test("GATE-09 TERM=dumb prints the linear report uncolored at 80 columns", async () => {
    const run = await ptyReport(todo, { env: { TERM: "dumb" }, cols: 100 });
    expect(run.landing).toBe(false);
    expect(run.output).not.toMatch(SGR);
    expect(widest(run.text)).toBeLessThanOrEqual(80);
    expect(widest(run.text)).toBeGreaterThan(70);
  });

  test("GATE-10 an unset TERM still opens the landing", async () => {
    const run = await ptyReport(todo, { env: { TERM: undefined } });
    expect(run.landing).toBe(true);
    expect(run.exitCode).toBe(0);
  });

  test("GATE-11 stdin from /dev/null prints the linear report, still colored", async () => {
    const run = await ptyReport(todo, { shell: 'exec "$0" "$@" </dev/null', env: { NO_COLOR: undefined } });
    expect(run.landing).toBe(false);
    expect(run.text).toContain(LINEAR);
    expect(run.output).toMatch(SGR);
    expect(run.exitCode).toBe(0);
  });

  test("GATE-12 stdout into a pipe prints the linear report uncolored at 80 columns", async () => {
    const run = await ptyReport(todo, { shell: '"$0" "$@" 2>/dev/null | cat', env: { NO_COLOR: undefined }, cols: 100 });
    expect(run.landing).toBe(false);
    expect(run.text).toContain(LINEAR);
    expect(run.output).not.toMatch(SGR);
    expect(widest(run.text)).toBeLessThanOrEqual(80);
    expect(run.exitCode).toBe(0);
  });

  test("GATE-13 stderr redirected keeps the landing, and the progress lands in the file once per phase", async () => {
    const box = sandbox("gate-13");
    const run = await ptyReport(todo, { shell: 'exec "$0" "$@" 2>stderr.txt', sandbox: box });
    expect(run.landing).toBe(true);
    expect(run.exitCode).toBe(0);
    expect(readFileSync(join(box.dir, "stderr.txt"), "utf8")).toBe(
      "Running native passes...\nCompiling dependencies...\nLinting kernel-todo (1/1)\n",
    );
  });

  test.each([
    ["bad-manifest", "(metadata/cargo-metadata)"],
    ["a path that does not exist", "(discovery/no-manifest)"],
  ])("GATE-14 a failed scan of %s prints the linear failure and no menu", async (subject, code) => {
    const workspace = subject === "bad-manifest" ? copyWorkspace("scan-failures/bad-manifest") : join(sandbox("gate-14").dir, "missing");
    const run = await ptyReport(workspace);
    expect(run.landing).toBe(false);
    expect(run.text).toContain("Scan failed: ");
    expect(run.text).toContain(code);
    expect(run.exitCode).toBe(2);
  });

  test.each([
    ["denied", [], 1],
    ["partial", [], 1],
    ["broken-member", [], 1],
    ["sleeping", ["--max-duration", "1"], 2],
  ] as const)("GATE-15 an incomplete scan of %s still opens the landing", async (kind, args, code) => {
    const workspace = await tuiWorkspace(kind);
    const run = await ptyReport(workspace.path, { args: [...args], sandbox: workspace.sandbox });
    expect(run.landing).toBe(true);
    expect(run.exitCode).toBe(code);
  });

  test("GATE-16 Ctrl-C during the scan prints the interrupted failure and no menu", async () => {
    const box = sandbox("gate-16");
    const pidFile = join(box.dir, "sleeper.pid");
    const session = openReport(copyWorkspace("scan-failures/sleeping-build"), { sandbox: box, env: { RUST_DOCTOR_SLEEPER_PID: pidFile } });
    try {
      const deadline = Date.now() + 30_000;
      while (!existsSync(pidFile) || readFileSync(pidFile, "utf8") === "") {
        if (Date.now() > deadline) throw new Error("the sleeping build script never started");
        await Bun.sleep(25);
      }
      const sleeper = Number(readFileSync(pidFile, "utf8"));
      session.sendKeys("C-c");
      expect(await session.waitForExit()).toBe(130);
      const pane = session.capturePane({ history: true });
      expect(pane).toContain("Scan failed: The scan was interrupted by a signal. (clippy/interrupted)");
      expect(pane).not.toContain(MENU_HINT);
      expect(alive(sleeper)).toBe(false);
    } finally {
      session.kill();
    }
  });

  test("GATE-17 an uncommitted change scans the full codebase without asking", async () => {
    const repo = await tuiWorkspace("todo-git");
    const edited = join(repo.path, "src/lib.rs");
    writeFileSync(edited, `${readFileSync(edited, "utf8")}\npub fn more() -> u8 {\n    1\n}\n`);
    const body = readFileSync(edited, "utf8");
    const status = await repo.git!(["status", "--porcelain"]);
    const session = openReport(repo.path, { sandbox: repo.sandbox });
    try {
      const pane = await session.waitForPane(MENU_HINT, 15_000);
      expect(pane).not.toContain("Choose what to scan");
      expect(pane).not.toContain("Uncommitted changes");
      session.sendKeys("q");
      expect(await session.waitForExit()).toBe(0);
      expect(readFileSync(edited, "utf8")).toBe(body);
      expect(await repo.git!(["status", "--porcelain"])).toBe(status);
    } finally {
      session.kill();
    }
  });

  test("GATE-18 the inspect alias opens the landing too", async () => {
    const session = TmuxSession.doctor(["inspect", todo], { width: 100, height: 30, env: { NO_COLOR: "1" } });
    try {
      await session.waitForPane(MENU_HINT, 15_000);
      session.sendKeys("q");
      expect(await session.waitForExit()).toBe(0);
    } finally {
      session.kill();
    }
  });
});
