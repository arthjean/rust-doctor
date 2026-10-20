// `tasks/e2e-catalog/tui.md` section 3.5: the geometry matrix. EP-006 fills
// it; the rows here are the open layout defects of the ledger.
import { describe, expect, test } from "bun:test";
import { copyWorkspace } from "./e2e-helpers";
import { gitScenario } from "./git-scenario";
import { MENU_HINT, openReport, requireTmux } from "./tmux-helpers";

describe.skipIf(!requireTmux())("interactive report layout", () => {
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
});
