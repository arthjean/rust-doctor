// `tasks/e2e-catalog/tui.md` section 3.4: the issue viewer's content,
// navigation and counters. EP-006 fills it; the rows here are the open viewer
// defects of the ledger.
import { describe, expect, test } from "bun:test";
import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { copyWorkspace, runDoctor, scratchDir } from "./e2e-helpers";
import { MENU_HINT, openReport, requireTmux, VIEWER_HINT } from "./tmux-helpers";

// A binary whose only findings are three `println!` sites, which the report
// grades as info, beside the two warnings a lockless binary gets.
function infoBinary(): string {
  const workspace = join(scratchDir("info-binary"), "info-binary");
  mkdirSync(join(workspace, "src"), { recursive: true });
  writeFileSync(join(workspace, "Cargo.toml"), '[package]\nname = "info-binary"\nversion = "0.1.0"\nedition = "2024"\npublish = false\n\n[workspace]\n');
  writeFileSync(join(workspace, "src/main.rs"), 'fn main() {\n    println!("a");\n    println!("b");\n    println!("c");\n}\n');
  return workspace;
}

describe.skipIf(!requireTmux())("interactive report issue viewer", () => {
  test.failing("VIEW-12 tui/BUG-02 the status bar counts what the report counts, info apart from warnings", async () => {
    const workspace = infoBinary();
    const summary = ((await runDoctor([workspace, "--yes", "--json"])).json as { summary: { warnings: number; info: number } }).summary;
    expect(summary.info).toBe(3);
    const session = openReport(workspace, { width: 120, height: 30 });
    try {
      await session.waitForPane(MENU_HINT, 15_000);
      session.sendKeys("Enter");
      await session.waitForPane(VIEWER_HINT);
      const pane = await session.waitForStable(300);
      const bar = pane.split("\n").find((line) => line.includes("issue 1/")) ?? "";
      expect(bar).toContain(` ${summary.warnings} warnings`);
    } finally {
      session.kill();
    }
  });

  test.failing("VIEW-11 tui/BUG-09 gg brings the first category header back", async () => {
    const session = openReport(copyWorkspace("kernel-contract/partial-failure"), { width: 100, height: 26 });
    try {
      await session.waitForPane(MENU_HINT, 15_000);
      session.sendKeys("Enter");
      await session.waitForPane(VIEWER_HINT);
      expect(await session.waitForStable(300)).toMatch(/^Bugs$/m);
      session.sendKeys("G");
      await session.waitForPane("issue 4/4");
      expect(await session.waitForStable(300)).not.toMatch(/^Bugs$/m);
      session.sendKeys("g", "g");
      await session.waitForPane("issue 1/4");
      expect(await session.waitForStable(300)).toMatch(/^Bugs$/m);
    } finally {
      session.kill();
    }
  });
});
