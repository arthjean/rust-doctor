// `tasks/e2e-catalog/tui.md` section 3.11: the bytes on the wire. EP-006
// fills it; the row here is the open input-handling defect of the ledger.
import { describe, expect, test } from "bun:test";
import { copyWorkspace } from "./e2e-helpers";
import { MENU_HINT, openReport, requireTmux } from "./tmux-helpers";

describe.skipIf(!requireTmux())("interactive report terminal hygiene", () => {
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
});
