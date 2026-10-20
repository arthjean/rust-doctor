// `tasks/e2e-catalog/tui.md` section 3.11: the bytes on the wire, read from
// the pipe-pane byte log, never from a dead pane's cursor flag.
import { describe, expect, test } from "bun:test";
import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { copyWorkspace, scratchDir } from "./e2e-helpers";
import { MENU_HINT, openReport, requireTmux } from "./tmux-helpers";
import { settledLog } from "./tui-actions-support";
import { count, faces, open, press, warm } from "./tui-geometry-support";


const SGR = /\x1b\[[0-9;]*m/;
const HIDE = "\x1b[?25l";
const SHOW = "\x1b[?25h";
const CLEAR = "\x1b[J";

// A crate whose `todo!()` line carries an OSC 8 link, a title change, a color
// and a bell in a comment, so the frame under the finding quotes them.
function hostileCrate(): string {
  const workspace = join(scratchDir("hostile"), "hostile");
  mkdirSync(join(workspace, "src"), { recursive: true });
  writeFileSync(join(workspace, "Cargo.toml"), '[package]\nname = "hostile"\nversion = "0.1.0"\nedition = "2024"\npublish = false\n\n[workspace]\n');
  writeFileSync(
    join(workspace, "src/lib.rs"),
    "pub fn hostile() -> u8 {\n    todo!() // \x1b]8;;https://evil.invalid\x07click\x1b]8;;\x07 \x1b[31mred \x1b]0;title\x07 bell\x07\n}\n",
  );
  return workspace;
}

describe.skipIf(!requireTmux())("interactive report terminal hygiene", () => {
  test("HYG-01 a color session writes SGR and OSC 8, hides the cursor once after the count-up, shows it once at exit, and keeps the last frame", async () => {
    const session = open(await warm("repo"), { width: 120, height: 40, color: true });
    try {
      await session.waitForPane(MENU_HINT, 30_000);
      await session.waitForStable(300);
      session.sendKeys("q");
      expect(await session.waitForExit()).toBe(0);
      const log = session.byteLog();
      expect(log).toMatch(SGR);
      expect(count(log, "\x1b]8;;https://")).toBeGreaterThan(0);
      expect(count(log, HIDE)).toBe(1);
      expect(count(log, SHOW)).toBe(1);
      expect(count(log.slice(0, log.indexOf(HIDE)), CLEAR)).toBeGreaterThan(1);
      expect(log.indexOf(SHOW)).toBeGreaterThan(log.lastIndexOf(CLEAR));
      expect(log).not.toContain("\x1b[?1049h");
      expect(session.capturePane({ history: true })).toContain(MENU_HINT);
    } finally {
      session.kill();
    }
  });

  test("HYG-02 NO_COLOR=1 writes no SGR and no link, draws no count-up, and still opens the landing", async () => {
    const session = openReport(copyWorkspace("kernel-contract/todo"));
    try {
      await session.waitForPane(MENU_HINT, 15_000);
      await session.waitForStable(300);
      expect(count(session.byteLog(), CLEAR)).toBe(1);
      session.sendKeys("q");
      expect(await session.waitForExit()).toBe(0);
      const log = session.byteLog();
      expect(log).not.toMatch(SGR);
      expect(log).not.toContain("\x1b]8;");
    } finally {
      session.kill();
    }
  });

  test("HYG-03 an empty NO_COLOR keeps color and the count-up, as no-color.org asks", async () => {
    const session = openReport(copyWorkspace("kernel-contract/todo"), { color: true, env: { NO_COLOR: "" } });
    try {
      await session.waitForPane(MENU_HINT, 15_000);
      await session.waitForStable(300);
      const log = session.byteLog();
      expect(log).toMatch(SGR);
      expect(count(log, CLEAR)).toBeGreaterThan(2);
    } finally {
      session.kill();
    }
  });

  test("HYG-04 every row ends with an erase, every frame with a clear, and each repaint rewinds the frame before it", async () => {
    const session = openReport(copyWorkspace("kernel-contract/todo"), { width: 80, height: 30 });
    try {
      await session.waitForPane(MENU_HINT, 15_000);
      await press(session, "Down");
      await press(session, "Up");
      await press(session, "Enter");
      await press(session, "j");
      session.sendKeys("q");
      expect(await session.waitForExit()).toBe(0);
      const log = session.byteLog();
      const frames = log.slice(log.indexOf(HIDE) + HIDE.length).split(CLEAR);
      expect(frames.at(-1)).toBe(SHOW);
      const drawn = frames.slice(0, -1);
      expect(drawn.length).toBeGreaterThanOrEqual(5);
      drawn.forEach((frame, index) => {
        // Every line feed closes a row erased to its end; the pane's tty adds a carriage return.
        expect(frame.match(/\x1b\[K\r+\n/g)?.length).toBe(count(frame, "\n"));
        if (index > 0) {
          expect(frame.startsWith(`\x1b[${count(drawn[index - 1]!, "\n")}A`)).toBe(true);
        }
      });
    } finally {
      session.kill();
    }
  });

  test("HYG-05 whatever the keys on the landing and in the viewer, exactly one score face is on screen", async () => {
    const session = open(await warm("repo"), { width: 120, height: 40 });
    try {
      await session.waitForPane(MENU_HINT, 30_000);
      expect(faces(await session.waitForStable(300))).toBe(1);
      for (const key of ["Enter", "j", "j", "G", "g", "g", "k", "PageDown", "Escape", "Up", "Enter", "PageUp", "Escape"]) {
        expect(faces(await press(session, key))).toBe(1);
      }
    } finally {
      session.kill();
    }
  });

  test.failing("HYG-06 tui/BUG-04 keys typed during the count-up are neither echoed nor replayed", async () => {
    const session = openReport(copyWorkspace("score-reasons/uncatalogued"), { color: true, width: 100, height: 40 });
    try {
      await session.waitForPane("/ 100", 15_000);
      session.sendLiteral("jq");
      const pane = await session.waitForPane(MENU_HINT, 15_000);
      await session.waitForStable(300);
      expect(session.status().dead).toBe(false);
      expect(pane).not.toContain("jq");
      expect(session.capturePane()).toContain("❯ Review");
    } finally {
      session.kill();
    }
  });

  test.failing("HYG-07 tui/BUG-04 Down and Enter typed during the count-up leave one clean landing", async () => {
    const session = openReport(copyWorkspace("score-reasons/uncatalogued"), { color: true, width: 100, height: 40 });
    try {
      await session.waitForPane("/ 100", 15_000);
      session.sendKeys("Down");
      session.sendKeys("Enter");
      await session.waitForPane((pane) => pane.includes(MENU_HINT) || pane.includes("Choose how to continue"), 15_000);
      const pane = await session.waitForStable(400);
      expect(pane).toContain(MENU_HINT);
      expect(pane).toContain("❯ Review");
      expect(faces(pane)).toBe(1);
      expect(count(pane, "/ 100")).toBe(1);
    } finally {
      session.kill();
    }
  });

  test("HYG-08 escape sequences in the workspace's source never reach the frame", async () => {
    const session = openReport(hostileCrate(), { width: 120, height: 40, color: true });
    try {
      await session.waitForPane(MENU_HINT, 15_000);
      session.sendKeys("Enter");
      await session.waitForPane("issue 1/1");
      const pane = await session.waitForStable(300);
      expect(pane).toMatch(/> 2 │ {5}todo!\(\) \/\//);
      const log = session.byteLog();
      for (const planted of ["\x1b]8;;https://evil.invalid", "\x1b]0;title", "\x1b[31mred", "\x07"]) {
        expect(log).not.toContain(planted);
      }
    } finally {
      session.kill();
    }
  });

  test("an external SIGTERM while the report waits leaves the cursor shown in the byte log", async () => {
    const session = openReport(copyWorkspace("kernel-contract/todo"), { color: true });
    try {
      await session.waitForPane(MENU_HINT, 15_000);
      await session.waitForStable(300);
      expect(session.byteLog()).toContain(HIDE);
      process.kill(session.panePid(), "SIGTERM");
      // 130, or the scan's code when the report's own exit wins the race
      // with the signal watcher: tui/BUG-14, held by tui-exit-codes.test.ts.
      expect([0, 130]).toContain((await session.waitForExit()) ?? -1);
      // pipe-pane writes the log asynchronously: wait for its last bytes.
      const log = await settledLog(session);
      // The cursor's final state is shown. The log need not end on the show:
      // a repaint racing the signal can land after it, without hiding again.
      expect(log.lastIndexOf(SHOW)).toBeGreaterThan(log.lastIndexOf(HIDE));
    } finally {
      session.kill();
    }
  });
});
