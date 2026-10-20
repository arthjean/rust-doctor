// `tasks/e2e-catalog/tui.md` section 3.3: menus and every key. EP-006 fills
// it; the row here is the open key-parsing defect of the ledger.
import { describe, expect, test } from "bun:test";
import { copyWorkspace } from "./e2e-helpers";
import { MENU_HINT, openReport, requireTmux, VIEWER_HINT } from "./tmux-helpers";

describe.skipIf(!requireTmux())("interactive report keys", () => {
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
});
