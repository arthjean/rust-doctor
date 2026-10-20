// `tasks/e2e-catalog/tui.md` section 3.10: signals by phase. EP-006 fills it;
// the row here is the open count-up interrupt defect of the ledger.
import { describe, expect, test } from "bun:test";
import { copyWorkspace } from "./e2e-helpers";
import { openReport, requireTmux } from "./tmux-helpers";

describe.skipIf(!requireTmux())("interactive report interrupts", () => {
  test.failing("SIG-04 tui/BUG-05 Ctrl-C during the count-up exits with the scan's code, as q does", async () => {
    const session = openReport(copyWorkspace("kernel-contract/todo"), { color: true, width: 100, height: 40 });
    try {
      await session.waitForPane("/ 100", 15_000);
      session.sendKeys("C-c");
      expect(await session.waitForExit()).toBe(0);
    } finally {
      session.kill();
    }
  });
});
