// `tasks/e2e-catalog/tui.md` section 3.8: the GitHub Actions screens. EP-006
// fills it; the rows here are the open CI-screen defects of the ledger.
import { describe, expect, test } from "bun:test";
import { join } from "node:path";
import { BIN, defaultTarget } from "./e2e-helpers";
import { gitScenario } from "./git-scenario";
import { CI_SETUP, MENU_HINT, openReport, requireTmux, TmuxSession } from "./tmux-helpers";

const OTHER_WORKFLOW = "on: push\njobs:\n  scan:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: arthjean/rust-doctor@v0\n";

describe.skipIf(!requireTmux())("interactive report CI screens", () => {
  test.failing("CI-10 tui/BUG-10 no install is recommended where another workflow already runs rust-doctor", async () => {
    const repo = await gitScenario("ci-10")
      .fixture("kernel-contract/todo")
      .write({ ".gitignore": "/target\n", ".github/workflows/scan.yml": OTHER_WORKFLOW })
      .commit("base")
      .build();
    const session = openReport(repo.path, { sandbox: repo.sandbox });
    try {
      const pane = await session.waitForPane(MENU_HINT, 15_000);
      expect(pane).not.toContain("Add to GitHub Actions (Recommended)");
    } finally {
      session.kill();
    }
  });

  test.failing("CI-08 tui/BUG-13 adding the workflow confirms it in the frame, stderr redirected", async () => {
    const repo = await gitScenario("ci-08").fixture("kernel-contract/todo").write({ ".gitignore": "/target\n" }).commit("base").build();
    const stderr = join(repo.sandbox.dir, "stderr.txt");
    const session = TmuxSession.create(
      ["sh", "-c", 'exec "$0" "$1" --advisory-db /dev/null 2>"$2"', BIN, repo.path, stderr],
      {
        width: 100,
        height: 30,
        sandbox: repo.sandbox,
        env: { NO_COLOR: "1", CARGO_TARGET_DIR: defaultTarget([repo.path], repo.path) },
      },
    );
    try {
      await session.waitForPane(MENU_HINT, 15_000);
      session.sendKeys("Down");
      await session.waitForPane("❯ Add to GitHub Actions");
      session.sendKeys("Enter");
      await session.waitForPane(CI_SETUP);
      session.sendKeys("Enter");
      expect(await session.waitForExit()).toBe(0);
      expect(await repo.git(["status", "--porcelain", "--untracked-files=all"])).toBe("?? .github/workflows/rust-doctor.yml\n");
      expect(session.capturePane({ history: true })).toContain(".github/workflows/rust-doctor.yml");
    } finally {
      session.kill();
    }
  });
});
