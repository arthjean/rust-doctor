// `tasks/e2e-catalog/tui.md` section 3.7: Enter in the viewer copies one
// rule. EP-006 fills it; the row here is the open clipboard defect of the
// ledger.
import { describe, expect, test } from "bun:test";
import { copyWorkspace, sandbox } from "./e2e-helpers";
import { fakeCalls, fakeEnv } from "./fakes";
import { MENU_HINT, openReport, requireTmux, VIEWER_HINT } from "./tmux-helpers";

describe.skipIf(!requireTmux())("interactive report clipboard", () => {
  test.failing("CLIP-03 tui/BUG-03 a copy in the stacked layout says so on screen", async () => {
    const box = sandbox("clip-03");
    const session = openReport(copyWorkspace("kernel-contract/todo"), {
      width: 100,
      height: 40,
      sandbox: box,
      env: fakeEnv(box, "clipboard"),
    });
    try {
      await session.waitForPane(MENU_HINT, 15_000);
      session.sendKeys("Enter");
      await session.waitForPane(VIEWER_HINT);
      session.sendKeys("Enter");
      await session.waitForStable(300);
      expect(fakeCalls(box, "wl-copy")).toHaveLength(1);
      expect(session.capturePane()).toContain("Copied issue context");
    } finally {
      session.kill();
    }
  });
});
