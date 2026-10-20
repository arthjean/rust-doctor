// `tasks/e2e-catalog/tui.md` section 3.4: the issue viewer's content,
// navigation and counters, on fixtures and on a clone of this repository. On
// the clone every expectation is read from the report's own JSON or from the
// list itself, never from this repository's score, counts or rules.
import { describe, expect, test } from "bun:test";
import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { copyWorkspace, runDoctor, scratchDir } from "./e2e-helpers";
import { credential, gitScenario } from "./git-scenario";
import { MENU_HINT, openReport, requireTmux, VIEWER_HINT } from "./tmux-helpers";
import { issueCount, open, openViewer, position, press, rows, unread, utf8, warm } from "./tui-geometry-support";

// A binary whose only findings are three `println!` sites, which the report
// grades as info, beside the two warnings a lockless binary gets.
function infoBinary(): string {
  const workspace = join(scratchDir("info-binary"), "info-binary");
  mkdirSync(join(workspace, "src"), { recursive: true });
  writeFileSync(join(workspace, "Cargo.toml"), '[package]\nname = "info-binary"\nversion = "0.1.0"\nedition = "2024"\npublish = false\n\n[workspace]\n');
  writeFileSync(join(workspace, "src/main.rs"), 'fn main() {\n    println!("a");\n    println!("b");\n    println!("c");\n}\n');
  return workspace;
}

// A scored `todo!()` in the library and an interpolated shell command in an
// integration test, the one finding whose representative site is test code.
function testSiteCrate(): string {
  const workspace = join(scratchDir("test-site"), "test-site");
  mkdirSync(join(workspace, "src"), { recursive: true });
  mkdirSync(join(workspace, "tests"));
  writeFileSync(join(workspace, "Cargo.toml"), '[package]\nname = "test-site"\nversion = "0.1.0"\nedition = "2024"\npublish = false\n\n[workspace]\n');
  writeFileSync(join(workspace, "src/lib.rs"), "pub fn debt() -> u8 {\n    todo!()\n}\n");
  writeFileSync(
    join(workspace, "tests/it.rs"),
    'use std::process::Command;\n\nfn signal(name: &str) {\n    let _ = Command::new("sh")\n        .args(["-c", &format!("kill {name}")])\n        .status();\n}\n\n#[test]\nfn shell() {\n    signal("x");\n}\n',
  );
  return workspace;
}

// The selected row of the list: the one the `›` marker opens.
function selected(pane: string): string {
  return rows(pane).find((line) => line.startsWith("› ")) ?? "";
}


describe.skipIf(!requireTmux())("interactive report issue viewer", () => {
  test("VIEW-01 the first issue shows its category, meta, impact, why, frame, fix and counters", async () => {
    const session = openReport(copyWorkspace("kernel-contract/todo"), { width: 100, height: 40 });
    try {
      const pane = await openViewer(session);
      expect(rows(pane)).toContain("Bugs");
      expect(pane).toContain("› ⚠ Todo ×3");
      expect(rows(pane).some((line) => /^─{90,}$/.test(line))).toBe(true);
      expect(rows(pane)).toContain("⚠ Todo ×3");
      expect(pane).toContain("Bugs · warning · src/lib.rs:4");
      expect(pane).toContain("Impact Real users hit panics");
      expect(pane).toContain("Why `todo` should not be present in production code");
      expect(pane).toMatch(/> 4 │ {5}todo!\(\)/);
      expect(pane).toContain("^^^^^^^");
      expect(pane).toContain("Fix Replace todo! with");
      expect(pane).toContain("3 findings  0 errors  3 warnings  ·  issue 1/1");
      expect(pane).toContain("0 issues unread");
    } finally {
      session.kill();
    }
  });

  test("VIEW-02 arrows and j/k step through issues, skip headers and clamp at the first", async () => {
    const repo = await warm("many");
    const total = issueCount(repo.report);
    expect(total).toBe(7);
    const session = open(repo, { width: 120, height: 40 });
    try {
      await openViewer(session);
      const seen: number[] = [];
      for (const key of ["j", "j", "Down", "k", "Up", "Up", "Up"]) {
        const pane = await press(session, key);
        seen.push(position(pane).index);
        expect(selected(pane)).toMatch(/^› [⚠✖ℹ] /);
      }
      expect(seen).toEqual([2, 3, 4, 3, 2, 1, 1]);
    } finally {
      session.kill();
    }
  });

  test("VIEW-03 G jumps to the last issue and stays there, gg returns to the first, g then j cancels the chord", async () => {
    const repo = await warm("many");
    const total = issueCount(repo.report);
    const session = open(repo, { width: 120, height: 40 });
    try {
      await openViewer(session);
      expect(position(await press(session, "G"))).toEqual({ index: total, total });
      expect(position(await press(session, "j")).index).toBe(total);
      session.sendKeys("g");
      expect(position(await press(session, "g")).index).toBe(1);
      session.sendKeys("g");
      expect(position(await press(session, "j")).index).toBe(2);
      expect(position(await press(session, "j")).index).toBe(3);
    } finally {
      session.kill();
    }
  });

  test("VIEW-04 PageDown and PageUp move a page of the list, clamped to its ends", async () => {
    const repo = await warm("many");
    const total = issueCount(repo.report);
    const session = open(repo, { width: 120, height: 40 });
    try {
      const first = await openViewer(session);
      // The split list is taller than the whole list, so one page reaches either end.
      expect(rows(first).filter((line) => /^[›• ] [⚠✖ℹ] |^[A-Z][a-z]/.test(line)).length).toBeLessThan(34);
      expect(position(await press(session, "PageDown")).index).toBe(total);
      expect(position(await press(session, "PageDown")).index).toBe(total);
      expect(position(await press(session, "PageUp")).index).toBe(1);
    } finally {
      session.kill();
    }
  });

  test("VIEW-05 every issue selected once is read: the counter falls and the marker changes", async () => {
    const repo = await warm("many");
    const total = issueCount(repo.report);
    expect(total).toBe(7);
    const session = open(repo, { width: 120, height: 40 });
    try {
      const counts = [unread(await openViewer(session))];
      for (const key of ["j", "j", "Down", "G"]) counts.push(unread(await press(session, key)));
      expect(counts).toEqual([total - 1, total - 2, total - 3, total - 4, total - 5]);
      const pane = session.capturePane();
      const list = rows(pane).map((line) => line.slice(0, 3));
      expect(list.filter((marker) => marker === "• ⚠" || marker === "• ✖" || marker === "• ℹ")).toHaveLength(total - 5);
      expect(list.filter((marker) => /^ {2}[⚠✖ℹ]$/.test(marker))).toHaveLength(4);
      expect(list.filter((marker) => marker.startsWith("› "))).toHaveLength(1);
    } finally {
      session.kill();
    }
  });

  test("VIEW-06 End, Home, Left, Tab and an unbound letter change nothing", async () => {
    const repo = await warm("repo");
    const session = open(repo, { width: 120, height: 40 });
    try {
      await openViewer(session);
      const before = await press(session, "j");
      for (const key of ["End", "Home", "Left", "Tab", "x"]) {
        expect(await press(session, key)).toBe(before);
      }
    } finally {
      session.kill();
    }
  });

  test("VIEW-07 categories follow the audit's order, compiler notes apart, one row per rule", async () => {
    const repo = await warm("repo");
    type Category = { name: string; distinct: { total: number } };
    const categories = (repo.report.audit.categories as Category[])
      .filter((category) => category.name !== "Other" && category.distinct.total > 0)
      .map((category) => category.name);
    // A rule is listed as info when every one of its sites is.
    const codes = new Set(repo.report.diagnostics.map((d) => d.code));
    const info = [...codes].filter((code) => repo.report.diagnostics.every((d) => d.code !== code || d.severity === "info"));
    const session = open(repo, { width: 120, height: 40 });
    try {
      const pane = await openViewer(session);
      const list = rows(pane).map((line) => line.split("│")[0]!.trimEnd());
      expect(list.filter((line) => categories.includes(line))).toEqual(categories);
      expect(list.filter((line) => /^[›• ] [⚠✖ℹ] /.test(line))).toHaveLength(issueCount(repo.report));
      expect(list.filter((line) => /^[›• ] ℹ /.test(line))).toHaveLength(info.length);
    } finally {
      session.kill();
    }
  });

  test("VIEW-08 compiler notes come last under their own heading", async () => {
    const session = openReport(copyWorkspace("score-reasons/uncatalogued"), { width: 120, height: 40 });
    try {
      const list = rows(await openViewer(session)).map((line) => line.split("│")[0]!.trimEnd());
      const notes = list.indexOf("Compiler notes (not scored)");
      expect(notes).toBeGreaterThan(list.indexOf("Bugs"));
      expect(list.slice(notes + 1).filter((line) => /^[›• ] [⚠✖ℹ] /.test(line))).toEqual(["• ⚠ Must use candidate", "• ⚠ Unused imports"]);
    } finally {
      session.kill();
    }
  });

  test("VIEW-09 a hardcoded credential's frame prints the mask, and its body reaches no byte", async () => {
    const repo = await gitScenario("view-09").template("mini").commit("base").build();
    const session = openReport(repo.path, { width: 120, height: 40, sandbox: repo.sandbox });
    try {
      const pane = await openViewer(session);
      expect(selected(pane)).toContain("Hardcoded credential");
      expect(pane).toContain("Security · warning · src/creds.rs:2");
      const marked = rows(pane).find((line) => line.includes("> 2 │")) ?? "";
      expect(marked).toContain("<redacted>");
      const body = credential("access-key").slice(4);
      expect(pane).not.toContain(body);
      expect(utf8(session.byteLog())).not.toContain(body);
    } finally {
      session.kill();
    }
  });

  test("VIEW-10 an error-level issue shows the error icon and singular counters", async () => {
    const session = openReport(copyWorkspace("kernel-contract/denied"), { width: 120, height: 21 });
    try {
      const pane = await openViewer(session);
      expect(pane).toContain("› ✖ Todo");
      expect(pane).toContain("1 finding  1 error  0 warnings  ·  issue 1/1");
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

  test("VIEW-13 an issue whose site is test code names the context on its meta row", async () => {
    const session = openReport(testSiteCrate(), { width: 120, height: 40 });
    try {
      const pane = await openViewer(session);
      expect(selected(pane)).toContain("Dynamic shell command");
      const meta = rows(pane).map((line) => line.split("│ ").at(-1)!.trim()).find((line) => line.startsWith("Security · "));
      expect(meta).toMatch(/^Security · warning · tests\/it\.rs:\d+ · tests$/);
      expect(position(await press(session, "j")).index).toBe(2);
      expect(session.capturePane()).toMatch(/Bugs · warning · src\/lib\.rs:2$/m);
    } finally {
      session.kill();
    }
  });

  test("VIEW-14 the rule guide is an OSC 8 link to the rule's page", async () => {
    const session = openReport(copyWorkspace("kernel-contract/todo"), { width: 120, height: 40, color: true });
    try {
      const pane = await openViewer(session);
      expect(pane).toContain("Rule guide: https://rust-doctor.com/rules/clippy%3A%3Atodo");
      expect(session.byteLog()).toContain("\x1b]8;;https://rust-doctor.com/rules/clippy%3A%3Atodo\x1b\\");
    } finally {
      session.kill();
    }
  });

  test("VIEW-15 a source file gone after the scan drops the frame and keeps every other section", async () => {
    const workspace = copyWorkspace("kernel-contract/todo");
    const session = openReport(workspace, { width: 120, height: 40 });
    try {
      await session.waitForPane(MENU_HINT, 15_000);
      rmSync(join(workspace, "src/lib.rs"));
      const pane = await openViewer(session);
      for (const section of ["Bugs · warning · src/lib.rs:4", "Impact ", "Why `todo`", "Fix Replace todo!", "Rule guide: "]) {
        expect(pane).toContain(section);
      }
      expect(pane).not.toMatch(/\d │ /);
      expect(pane.toLowerCase()).not.toMatch(/error:|could not|no such file/);
    } finally {
      session.kill();
    }
  });
});
