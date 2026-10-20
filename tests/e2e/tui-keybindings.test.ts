// `tasks/e2e-catalog/tui.md` section 3.3: menus and every key. Escape is
// always sent alone and the next frame awaited before another key, since an
// Escape and the byte after it in one read are swallowed whole (KEY-10).
import { describe, expect, test } from "bun:test";
import { copyWorkspace, targetDir } from "./e2e-helpers";
import { CI_SETUP, MENU_HINT, openReport, requireTmux, VIEWER_HINT, type TmuxSession } from "./tmux-helpers";
import { quit, selected } from "./tui-entry-support";
import { tuiWorkspace, type TuiWorkspace } from "./tui-support";

const HANDOFF = "Choose how to continue";
const RECOMMENDATION = "Add Rust Doctor to GitHub Actions first";
const JUSTIFICATION = "  Scan every pull request to prevent new Rust issues while you fix the backlog.";

function open(workspace: TuiWorkspace | string, args: string[] = [], size = { width: 100, height: 40 }): TmuxSession {
  const path = typeof workspace === "string" ? workspace : workspace.path;
  return openReport(path, { ...size, args, ...(typeof workspace === "string" ? {} : { sandbox: workspace.sandbox }) });
}

// Sends `key` and returns the frame it produced.
async function press(session: TmuxSession, ...keys: string[]): Promise<string> {
  session.sendKeys(...keys);
  return session.waitForStable(200);
}

describe.skipIf(!requireTmux())("interactive report keys", () => {
  test("KEY-01 the cursor clamps at both ends, and j and k are the arrows", async () => {
    const workspace = await tuiWorkspace("todo-git");
    const session = open(workspace);
    try {
      await session.waitForPane(MENU_HINT, 15_000);
      for (let n = 0; n < 3; n += 1) await press(session, "Down");
      expect(selected(session.capturePane())).toBe("Hand off to an agent");
      for (let n = 0; n < 4; n += 1) await press(session, "Up");
      expect(selected(session.capturePane())).toBe("Review 1 issue");
      const byArrow = await press(session, "Down");
      await press(session, "Up");
      expect(await press(session, "j")).toBe(byArrow);
      await press(session, "j");
      expect(selected(await press(session, "k"))).toBe("Add to GitHub Actions (Recommended)");
    } finally {
      session.kill();
    }
  });

  test("KEY-02 the GitHub Actions entry shows its justification only while selected", async () => {
    const session = open(await tuiWorkspace("todo-git"));
    try {
      expect(await session.waitForPane(MENU_HINT, 15_000)).not.toContain(JUSTIFICATION);
      expect(await press(session, "Down")).toContain(`❯ Add to GitHub Actions (Recommended)\n${JUSTIFICATION}\n`);
      expect(await press(session, "Down")).not.toContain(JUSTIFICATION);
    } finally {
      session.kill();
    }
  });

  test("KEY-03 Enter on each entry opens its screen", async () => {
    const session = open(copyWorkspace("kernel-contract/todo"));
    try {
      await session.waitForPane(MENU_HINT, 15_000);
      session.sendKeys("Enter");
      await session.waitForPane(VIEWER_HINT);
      session.sendKeys("Escape");
      await session.waitForPane(MENU_HINT);
      expect(selected(session.capturePane())).toBe("Hand off to an agent");
      session.sendKeys("Enter");
      await session.waitForPane(HANDOFF);
    } finally {
      session.kill();
    }
  });

  // How each screen is reached from the landing of a repository that can
  // take the workflow.
  const SCREENS: Array<[string, string[], string]> = [
    ["landing", [], MENU_HINT],
    ["viewer", ["Enter"], VIEWER_HINT],
    ["CI", ["Down", "Enter"], CI_SETUP],
    ["recommendation", ["Down", "Down", "Enter"], RECOMMENDATION],
    ["handoff", ["Down", "Down", "Enter", "Down", "Enter"], HANDOFF],
  ];

  test.each([
    ["KEY-04", "q"],
    ["KEY-05", "C-c"],
  ])("%s %s leaves every screen with the scan's exit code", async (_id, key) => {
    const workspace = await tuiWorkspace("todo-git");
    for (const [name, keys, marker] of SCREENS) {
      const session = open(workspace, ["--blocking", "warning"]);
      try {
        await session.waitForPane(MENU_HINT, 15_000);
        for (const step of keys) await press(session, step);
        await session.waitForPane(marker);
        session.sendKeys(key);
        expect({ name, code: await session.waitForExit() }).toEqual({ name, code: 1 });
      } finally {
        session.kill();
      }
    }
  });

  test("KEY-06 Escape on the landing quits with the scan's exit code", async () => {
    const session = open(copyWorkspace("kernel-contract/todo"), ["--blocking", "warning"]);
    try {
      await session.waitForPane(MENU_HINT, 15_000);
      expect(await quit(session, "Escape")).toBe(1);
    } finally {
      session.kill();
    }
  });

  test("KEY-07 keys the landing does not bind change nothing, Ctrl-Z included", async () => {
    const session = open(copyWorkspace("kernel-contract/todo"));
    try {
      await session.waitForPane(MENU_HINT, 15_000);
      const pane = await session.waitForStable(200);
      for (const key of ["C-z", "C-d", "Q", "Space", "BSpace", "F1", "C-l"]) {
        expect({ key, pane: await press(session, key) }).toEqual({ key, pane });
        expect(session.status().dead).toBe(false);
      }
    } finally {
      session.kill();
    }
  });

  test("KEY-08 Escape from the viewer lands on the next action, the handoff without CI", async () => {
    const session = open(copyWorkspace("kernel-contract/todo"));
    try {
      await session.waitForPane(MENU_HINT, 15_000);
      session.sendKeys("Enter");
      await session.waitForPane(VIEWER_HINT);
      session.sendKeys("Escape");
      expect(selected(await session.waitForPane(MENU_HINT))).toBe("Hand off to an agent");
    } finally {
      session.kill();
    }
  });

  test("KEY-09 Escape from the viewer lands on GitHub Actions, justification shown, when it is offered", async () => {
    const session = open(await tuiWorkspace("todo-git"));
    try {
      await session.waitForPane(MENU_HINT, 15_000);
      session.sendKeys("Enter");
      await session.waitForPane(VIEWER_HINT);
      session.sendKeys("Escape");
      const pane = await session.waitForPane(MENU_HINT);
      expect(selected(pane)).toBe("Add to GitHub Actions (Recommended)");
      expect(pane).toContain(JUSTIFICATION);
    } finally {
      session.kill();
    }
  });

  test.failing("KEY-10 tui/BUG-12 Escape and the byte after it in one write leave the viewer", async () => {
    const session = openReport(copyWorkspace("kernel-contract/todo"));
    try {
      await session.waitForPane(MENU_HINT, 15_000);
      session.sendKeys("Enter");
      await session.waitForPane(VIEWER_HINT);
      session.sendKeys("-H", "1b", "6a");
      expect(await session.waitForPane(MENU_HINT, 1_000)).not.toContain(VIEWER_HINT);
    } finally {
      session.kill();
    }
  });

  test("KEY-11 SS3 Down and Home are ignored; only CSI arrows move", async () => {
    const session = open(copyWorkspace("kernel-contract/todo"));
    try {
      await session.waitForPane(MENU_HINT, 15_000);
      const pane = await session.waitForStable(200);
      expect(await press(session, "-H", "1b", "4f", "42")).toBe(pane);
      expect(await press(session, "-H", "1b", "5b", "48")).toBe(pane);
      expect(selected(await press(session, "Down"))).toBe("Hand off to an agent");
    } finally {
      session.kill();
    }
  });

  test("KEY-12 a bracketed paste of jq is read as its keys: the markers are ignored and q quits", async () => {
    const session = open(copyWorkspace("kernel-contract/todo"));
    try {
      await session.waitForPane(MENU_HINT, 15_000);
      session.sendKeys("Enter");
      await session.waitForPane(VIEWER_HINT);
      session.sendKeys("-H", "1b", "5b", "32", "30", "30", "7e", "6a", "71", "1b", "5b", "32", "30", "31", "7e");
      expect(await session.waitForExit()).toBe(0);
    } finally {
      session.kill();
    }
  });

  test("KEY-13 a burst of j in one write is processed key by key, to the last issue", async () => {
    const workspace = await tuiWorkspace("repo");
    const session = openReport(workspace.path, { sandbox: workspace.sandbox, width: 120, height: 40, env: { CARGO_TARGET_DIR: targetDir("repo") } });
    try {
      await session.waitForPane(MENU_HINT, 60_000);
      session.sendKeys("Enter");
      const viewer = await session.waitForPane(/issue 1\/\d+/);
      const total = Number(viewer.match(/issue 1\/(\d+)/)![1]);
      expect(total).toBeGreaterThan(1);
      session.sendKeys("-H", ...Array.from({ length: total - 1 }, () => "6a"));
      await session.waitForPane(`issue ${total}/${total}`);
    } finally {
      session.kill();
    }
  }, 120_000);
});
