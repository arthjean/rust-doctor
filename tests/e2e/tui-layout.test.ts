// `tasks/e2e-catalog/tui.md` section 3.5: the geometry matrix, from 1x1 to
// 300x80. Every frame a test here captures is kept, and LAY-11, declared
// last, holds each one to the terminal it was drawn for.
import { describe, expect, test } from "bun:test";
import { chmodSync, mkdirSync } from "node:fs";
import { join } from "node:path";
import { copyWorkspace } from "./e2e-helpers";
import { gitScenario } from "./git-scenario";
import { CI_SETUP, MENU_HINT, openReport, requireTmux, type TmuxSession } from "./tmux-helpers";
import { barCells, cells, dividerColumns, faces, listEntries, open, rows, rowsUsed, warm, widest } from "./tui-geometry-support";


type Frame = { label: string; cols: number; rows: number; pane: string };
const frames: Frame[] = [];

async function snap(session: TmuxSession, label: string): Promise<string> {
  const pane = await session.waitForStable(300);
  const { cols, rows: height } = session.size();
  frames.push({ label, cols, rows: height, pane });
  return pane;
}

// Opens the issue list and returns its settled frame, at any size: the
// compact status row is cut before `issue N/M` in the narrowest windows.
async function viewer(session: TmuxSession, label: string): Promise<string> {
  await session.waitForPane("Revi", 30_000);
  await snap(session, `${label} landing`);
  session.sendKeys("Enter");
  await session.waitForPane((pane) => pane.includes("› ") && pane.includes("finding"));
  return snap(session, `${label} viewer`);
}

const index = (pane: string, predicate: (line: string) => boolean) => rows(pane).findIndex(predicate);

describe.skipIf(!requireTmux())("interactive report layout", () => {
  test("LAY-01 at 200x50 the list and the detail split at a fixed divider", async () => {
    const session = open(await warm("repo"), { width: 200, height: 50 });
    try {
      const pane = await viewer(session, "200x50");
      expect(dividerColumns(pane)).toEqual([78]);
      for (const line of rows(pane).filter((row) => row.includes("│"))) {
        expect(cells([...line].slice(0, 78).join(""))).toBeLessThanOrEqual(77);
      }
      expect(rows(pane)[0]).toMatch(/│ [⚠✖ℹ] /);
      expect(rowsUsed(pane)).toBeLessThanOrEqual(49);
      expect(widest(pane)).toBeLessThan(200);
    } finally {
      session.kill();
    }
  });

  test("LAY-02 at 120x40 the split keeps a 45-column list, cuts the header in it and wraps the detail", async () => {
    const session = open(await warm("repo"), { width: 120, height: 40 });
    try {
      const pane = await viewer(session, "120x40");
      expect(dividerColumns(pane)).toEqual([46]);
      const left = rows(pane).map((line) => [...line].slice(0, 46).join("").trimEnd());
      expect(left.slice(0, 4).some((line) => line.endsWith("…"))).toBe(true);
      const right = rows(pane).map((line) => [...line].slice(47).join(""));
      expect(Math.max(...right.map(cells))).toBeLessThanOrEqual(72);
      expect(right.filter((line) => line.startsWith("   ") && cells(line) > 40).length).toBeGreaterThan(1);
    } finally {
      session.kill();
    }
  });

  test("LAY-03 at 119x30 the landing keeps a whole header and a 50-cell bar", async () => {
    const session = open(await warm("repo"), { width: 119, height: 30 });
    try {
      await session.waitForPane(MENU_HINT, 30_000);
      const pane = await snap(session, "119x30 landing");
      expect(barCells(pane)).toBe(50);
      expect(pane).toContain("Rust Doctor (https://rust-doctor.com)");
      expect(rows(pane).slice(0, 5).join("\n")).not.toContain("…");
    } finally {
      session.kill();
    }
  });

  test("LAY-04 at 100x40 the viewer stacks: a list of at most 15 rows, a divider, a detail of at most 15 rows, two status rows", async () => {
    const repo = await warm("repo");
    const session = open(repo, { width: 100, height: 40 });
    try {
      const pane = await viewer(session, "100x40");
      const lines = rows(pane);
      const divider = index(pane, (line) => /^─{90,}$/.test(line));
      const first = index(pane, (line) => /^[A-Z]/.test(line));
      const status = index(pane, (line) => /issue \d+\//.test(line));
      expect(dividerColumns(pane)).toEqual([]);
      // The list holds every entry up to 15 rows, however many this repository has.
      expect(divider - first).toBe(Math.min(15, listEntries(repo.report)));
      expect(lines.slice(divider + 1, status).filter((line, at, all) => at < all.length - 1 || line !== "").length).toBeLessThanOrEqual(15);
      expect(lines[status + 1]).toContain("unread");
    } finally {
      session.kill();
    }
  });

  test.each([
    [80, false],
    [60, true],
  ])("LAY-05 at %ix24 the stack keeps a 3-row list, its second status row cut only at 60: %p", async (width, cut) => {
    const session = open(await warm("repo"), { width, height: 24 });
    try {
      const pane = await viewer(session, `${width}x24`);
      const divider = index(pane, (line) => /^─{50,}$/.test(line));
      const first = index(pane, (line) => /^[A-Z]/.test(line));
      expect(divider - first).toBe(3);
      const unreadRow = rows(pane).find((line) => line.includes("unread")) ?? "";
      expect(unreadRow.endsWith("…")).toBe(cut);
    } finally {
      session.kill();
    }
  });

  test("LAY-06 at 80x23 the viewer turns compact: header, 16 list rows, one status row", async () => {
    const session = open(await warm("repo"), { width: 80, height: 23 });
    try {
      const pane = await viewer(session, "80x23");
      const lines = rows(pane);
      const status = index(pane, (line) => /issue 1\//.test(line));
      expect(index(pane, (line) => /^─{50,}$/.test(line))).toBe(-1);
      expect(faces(pane)).toBe(1);
      expect(status - (index(pane, (line) => line.includes("└─────┘")) + 1)).toBe(16);
      expect(lines[status]).toMatch(/issue 1\/\d+ {2}· {2}↑\/↓ move · enter copy…$/);
      expect(lines.slice(status + 1).join("").trim()).toBe("");
    } finally {
      session.kill();
    }
  });

  test("LAY-07 at 120x21 the viewer is compact although wide, with the whole status on one row", async () => {
    const session = openReport(copyWorkspace("kernel-contract/denied"), { width: 120, height: 21 });
    try {
      const pane = await viewer(session, "denied 120x21");
      const status = rows(pane).filter((line) => line.includes("issue 1/1"));
      expect(status).toEqual(["1 finding  1 error  0 warnings  ·  issue 1/1  ·  ↑/↓ move · enter copy context · esc back · q to quit"]);
      expect(dividerColumns(pane)).toEqual([]);
      expect(index(pane, (line) => /^─{50,}$/.test(line))).toBe(-1);
    } finally {
      session.kill();
    }
  });

  test.each([
    [40, 10, 1],
    [24, 8, 0],
  ])("LAY-08 at %ix%i the compact viewer keeps the score header only where it fits: %i face", async (width, height, face) => {
    const session = open(await warm("repo"), { width, height });
    try {
      const pane = await viewer(session, `${width}x${height}`);
      expect(faces(pane)).toBe(face);
      expect(rows(pane).some((line) => line.startsWith("› "))).toBe(true);
      expect(rows(pane).at(rowsUsed(pane) - 1)).toMatch(/^\d+ findings/);
      expect(rowsUsed(pane)).toBe(height - 1);
    } finally {
      session.kill();
    }
  });

  test.each([
    [20, 2, "94 / 100 Great  · …"],
    [5, 3, "94 …"],
    [1, 1, ""],
  ])("LAY-09 at %ix%i the report survives Enter, j, Escape and q, and exits 0", async (width, height, first) => {
    const session = openReport(copyWorkspace("kernel-contract/todo"), { width, height });
    try {
      // The smallest windows show no marker to wait on: the frame settles once the scan has printed it.
      await session.waitForPane((pane) => rows(pane)[0] === first && session.byteLog().includes("\x1b[J"), 15_000);
      expect(rows(await snap(session, `todo ${width}x${height} landing`))[0]).toBe(first);
      for (const key of ["Enter", "j", "Escape"]) {
        session.sendKeys(key);
        await snap(session, `todo ${width}x${height} ${key}`);
        expect(session.status().dead).toBe(false);
      }
      session.sendKeys("q");
      expect(await session.waitForExit()).toBe(0);
    } finally {
      session.kill();
    }
  });

  test("LAY-10 at 300x80 the bar stays at 50 cells", async () => {
    const session = openReport(copyWorkspace("kernel-contract/todo"), { width: 300, height: 80 });
    try {
      await session.waitForPane(MENU_HINT, 15_000);
      expect(barCells(await snap(session, "todo 300x80 landing"))).toBe(50);
      session.sendKeys("Enter");
      await session.waitForPane("issue 1/1");
      expect(barCells(await snap(session, "todo 300x80 viewer"))).toBe(50);
    } finally {
      session.kill();
    }
  });

  test.failing("LAY-12 tui/BUG-01 the landing header is whole at 120 columns", async () => {
    const session = openReport(copyWorkspace("kernel-contract/todo"), { width: 120, height: 40 });
    try {
      const pane = await session.waitForPane(MENU_HINT, 15_000);
      expect(pane).toContain("Rust Doctor (https://rust-doctor.com)");
      expect(pane).toContain("Potential score 100 after priority fixes +6");
    } finally {
      session.kill();
    }
  });

  test.failing("LAY-13 tui/BUG-07 the selected entry stays on screen at 40x10", async () => {
    const repo = await gitScenario("lay-13").fixture("kernel-contract/todo").commit("base").build();
    const session = openReport(repo.path, { width: 40, height: 10, sandbox: repo.sandbox });
    try {
      await session.waitForPane("❯ Review", 15_000);
      session.sendKeys("Down");
      await session.waitForPane("❯ Add to GitHub Actions");
      session.sendKeys("Down");
      await session.waitForStable(300);
      expect(session.capturePane()).toContain("❯ Hand off to an agent");
    } finally {
      session.kill();
    }
  });

  test("LAY-14 at 60x30 the CI entry's justification rows are cut, never wrapped", async () => {
    const repo = await gitScenario("lay-14").fixture("kernel-contract/todo").write({ ".gitignore": "/target\n" }).commit("base").build();
    const session = openReport(repo.path, { width: 60, height: 30, sandbox: repo.sandbox });
    try {
      await session.waitForPane(MENU_HINT, 15_000);
      session.sendKeys("Down");
      await session.waitForPane("❯ Add to GitHub Actions");
      const pane = await snap(session, "todo-git 60x30 CI entry");
      const entry = index(pane, (line) => line.startsWith("❯ Add to GitHub Actions"));
      const justification = rows(pane).slice(entry + 1, entry + 4);
      expect(justification).toHaveLength(3);
      expect(justification.some((line) => line.endsWith("…"))).toBe(true);
      expect(rows(pane)[entry + 4]?.trim()).toBe("");
      expect(widest(pane)).toBeLessThan(60);
    } finally {
      session.kill();
    }
  });

  // The catalog's subject, another workflow already running rust-doctor, is
  // tui/BUG-10 (an entry offered where the install must fail); a read-only
  // workflows directory is the failure the entry is right to offer.
  test.skipIf(process.getuid?.() === 0)("LAY-15 a workflow install failure notice is cut at the frame width (root ignores the mode)", async () => {
    const repo = await gitScenario("lay-15").fixture("kernel-contract/todo").write({ ".gitignore": "/target\n" }).commit("base").build();
    const workflows = join(repo.path, ".github/workflows");
    mkdirSync(workflows, { recursive: true });
    chmodSync(workflows, 0o555);
    const session = openReport(repo.path, { width: 60, height: 30, sandbox: repo.sandbox });
    try {
      await session.waitForPane(MENU_HINT, 15_000);
      session.sendKeys("Down");
      await session.waitForPane("❯ Add to GitHub Actions");
      session.sendKeys("Enter");
      await session.waitForPane(CI_SETUP);
      session.sendKeys("Enter");
      await session.waitForPane("Could not add the workflow");
      const pane = await snap(session, "read-only workflows 60x30 notice");
      const notice = rows(pane).find((line) => line.includes("Could not add the workflow")) ?? "";
      expect(notice.endsWith("…")).toBe(true);
      expect(widest(pane)).toBeLessThan(60);
      expect(await repo.git(["status", "--porcelain", "--untracked-files=all"])).toBe("");
    } finally {
      session.kill();
      chmodSync(workflows, 0o755);
    }
  });

  test("LAY-11 no frame of this file reaches the last column or the last row", () => {
    expect(frames.length).toBeGreaterThan(0);
    const overflowing = frames
      .filter((frame) => widest(frame.pane) >= frame.cols || rowsUsed(frame.pane) > Math.max(frame.rows - 1, 1))
      .map((frame) => `${frame.label}: ${widest(frame.pane)} columns, ${rowsUsed(frame.pane)} rows in ${frame.cols}x${frame.rows}`);
    expect(overflowing).toEqual([]);
  });
});
