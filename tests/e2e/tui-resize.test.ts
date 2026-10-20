// `tasks/e2e-catalog/tui.md` section 3.6: resizing. EP-006 fills it; the row
// here is the open repaint defect of the ledger.
import { describe, expect, test } from "bun:test";
import { copyWorkspace } from "./e2e-helpers";
import { MENU_HINT, openReport, requireTmux } from "./tmux-helpers";

describe.skipIf(!requireTmux())("interactive report resize", () => {
  test.failing("RSZ-01 tui/BUG-08 a resize repaints the frame without waiting for a key", async () => {
    const workspace = copyWorkspace("kernel-contract/todo");
    const fresh = openReport(workspace, { width: 80, height: 24 });
    const resized = openReport(workspace, { width: 120, height: 40 });
    try {
      await fresh.waitForPane(MENU_HINT, 15_000);
      const expected = await fresh.waitForStable(300);
      await resized.waitForPane(MENU_HINT, 15_000);
      resized.resize(80, 24);
      expect(await resized.waitForStable(400)).toBe(expected);
    } finally {
      fresh.kill();
      resized.kill();
    }
  });
});
