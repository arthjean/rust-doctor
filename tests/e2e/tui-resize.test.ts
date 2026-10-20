// `tasks/e2e-catalog/tui.md` section 3.6: resizing. The report repaints on
// its next key (tui/BUG-08 is that it should not wait for one), so every row
// but RSZ-01 resizes, sends a key and holds the frame to the one a session
// opened at the new size draws in the same state.
import { describe, expect, test } from "bun:test";
import { copyWorkspace } from "./e2e-helpers";
import { gitScenario } from "./git-scenario";
import { CI_SETUP, MENU_HINT, openReport, requireTmux, type TmuxSession } from "./tmux-helpers";
import { faces, open, openViewer, position, press, rows, warm, widest } from "./tui-geometry-support";


async function resized(session: TmuxSession, cols: number, height: number, key: string): Promise<string> {
  session.resize(cols, height);
  await session.waitForPane(() => session.size().cols === cols);
  return press(session, key);
}

// The visible rows with the blank tail dropped, so two frames of one state
// compare whatever was left below them.
function frame(pane: string): string {
  return rows(pane).join("\n").trimEnd();
}

describe.skipIf(!requireTmux())("interactive report resize", () => {
  test.failing("RSZ-01 tui/BUG-08 a resize repaints the frame without waiting for a key", async () => {
    const workspace = copyWorkspace("kernel-contract/todo");
    const fresh = openReport(workspace, { width: 80, height: 24 });
    const resizedSession = openReport(workspace, { width: 120, height: 40 });
    try {
      await fresh.waitForPane(MENU_HINT, 15_000);
      const expected = await fresh.waitForStable(300);
      await resizedSession.waitForPane(MENU_HINT, 15_000);
      resizedSession.resize(80, 24);
      expect(await resizedSession.waitForStable(400)).toBe(expected);
    } finally {
      fresh.kill();
      resizedSession.kill();
    }
  });

  test("RSZ-02 the viewer shrunk to 60x20 redraws compact, with one face, on the next key", async () => {
    const repo = await warm("repo");
    const session = open(repo, { width: 120, height: 40 });
    const fresh = open(repo, { width: 60, height: 20 });
    try {
      await openViewer(session);
      const pane = await resized(session, 60, 20, "j");
      expect(faces(pane)).toBe(1);
      expect(widest(pane)).toBeLessThan(60);
      expect(pane).not.toMatch(/^─{50,}$/m);
      await fresh.waitForPane(MENU_HINT, 30_000);
      await press(fresh, "Enter");
      expect(frame(pane)).toBe(frame(await press(fresh, "j")));
    } finally {
      session.kill();
      fresh.kill();
    }
  });

  test("RSZ-03 grown back to 200x50, the next key leaves exactly one face of the reflowed history", async () => {
    const session = open(await warm("many"), { width: 120, height: 40 });
    try {
      await openViewer(session);
      await resized(session, 60, 20, "j");
      const pane = await resized(session, 200, 50, "j");
      expect(faces(pane)).toBe(1);
      expect(position(pane).index).toBe(3);
      expect(widest(pane)).toBeLessThan(200);
    } finally {
      session.kill();
    }
  });

  test("RSZ-04 120x40 then 100x30 redraw split then stacked, one face each", async () => {
    const session = open(await warm("many"), { width: 80, height: 24 });
    try {
      await openViewer(session);
      const split = await resized(session, 120, 40, "j");
      expect(faces(split)).toBe(1);
      expect(rows(split).filter((line) => line.includes(" │ ")).length).toBeGreaterThan(10);
      const stacked = await resized(session, 100, 30, "j");
      expect(faces(stacked)).toBe(1);
      expect(stacked).toMatch(/^─{90,}$/m);
      expect(position(stacked).index).toBe(3);
    } finally {
      session.kill();
    }
  });

  test("RSZ-05 a landing widened to 130x30 is laid out as one opened at 130x30", async () => {
    const repo = await warm("repo");
    const session = open(repo, { width: 100, height: 30 });
    const fresh = open(repo, { width: 130, height: 30 });
    try {
      await session.waitForPane(MENU_HINT, 30_000);
      await session.waitForStable(300);
      const pane = await resized(session, 130, 30, "k");
      await fresh.waitForPane(MENU_HINT, 30_000);
      expect(faces(pane)).toBe(1);
      expect(frame(pane)).toBe(frame(await press(fresh, "k")));
    } finally {
      session.kill();
      fresh.kill();
    }
  });

  test("RSZ-06 a resize during the count-up draws the rest of it, and the landing, at the new size", async () => {
    const workspace = copyWorkspace("score-reasons/uncatalogued");
    const session = openReport(workspace, { color: true, width: 120, height: 40 });
    const fresh = openReport(workspace, { color: true, width: 70, height: 30 });
    try {
      await session.waitForPane("/ 100", 15_000);
      session.resize(70, 30);
      await session.waitForPane(MENU_HINT, 15_000);
      const pane = await session.waitForStable(400);
      expect(faces(pane)).toBe(1);
      expect(widest(pane)).toBeLessThan(70);
      await fresh.waitForPane(MENU_HINT, 15_000);
      expect(frame(pane)).toBe(frame(await fresh.waitForStable(400)));
    } finally {
      session.kill();
      fresh.kill();
    }
  });

  test("RSZ-07 the CI and handoff screens cut their rows again at the new width", async () => {
    const repo = await gitScenario("rsz-07").fixture("kernel-contract/todo").write({ ".gitignore": "/target\n" }).commit("base").build();
    const session = openReport(repo.path, { width: 100, height: 30, sandbox: repo.sandbox });
    try {
      await session.waitForPane(MENU_HINT, 15_000);
      session.sendKeys("Down");
      await session.waitForPane("❯ Add to GitHub Actions");
      session.sendKeys("Enter");
      await session.waitForPane(CI_SETUP);
      const ci = await resized(session, 50, 30, "Down");
      expect(ci).toContain("❯ Open the GitHub Actions guide");
      expect(widest(ci)).toBeLessThan(50);
      expect(rows(ci).some((line) => line.endsWith("…"))).toBe(true);
      session.sendKeys("Escape");
      await session.waitForPane(MENU_HINT);
      session.sendKeys("Down");
      await session.waitForPane("❯ Hand off to an agent");
      session.sendKeys("Enter");
      await session.waitForPane("Add Rust Doctor to GitHub Actions first");
      const handoff = await resized(session, 40, 30, "Down");
      expect(handoff).toContain("❯ Continue without");
      expect(widest(handoff)).toBeLessThan(40);
      expect(rows(handoff).some((line) => line.endsWith("…"))).toBe(true);
      expect(await repo.git(["status", "--porcelain", "--untracked-files=all"])).toBe("");
    } finally {
      session.kill();
    }
  });

  test("RSZ-08 shrunk at the last issue, the viewport follows the selection on the next move", async () => {
    const repo = await warm("many");
    const session = open(repo, { width: 120, height: 40 });
    try {
      await openViewer(session);
      const total = position(await press(session, "G")).total;
      const pane = await resized(session, 60, 20, "k");
      expect(position(pane).index).toBe(total - 1);
      expect(rows(pane).filter((line) => line.startsWith("› "))).toHaveLength(1);
    } finally {
      session.kill();
    }
  });
});
