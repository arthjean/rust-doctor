// `tasks/e2e-catalog/tui.md` section 3.7: Enter in the viewer copies one
// rule. The payload is read from what the fake clipboard received on stdin,
// never from the wrapped pane, and no test reaches the machine's clipboard.
import { afterAll, describe, expect, test } from "bun:test";
import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { copyWorkspace, runDoctor, sandbox, scratchDir, type Sandbox } from "./e2e-helpers";
import { fakeCalls, fakeEnv } from "./fakes";
import { cloneThisRepository, type SelfClone } from "./self-clone";
import { infoBinary, processesMentioning } from "./tui-actions-support";
import { MENU_HINT, openReport, requireTmux, TmuxSession } from "./tmux-helpers";

const COPIED = "✓ Copied issue context";
const COPY_FAILED_SPLIT = "Couldn't copy issue context. Press Enter to try again.";
const COPY_FAILED_COMPACT = "Copy failed";

// The viewer of `workspace` under `env`, at its first issue.
async function viewer(workspace: string, box: Sandbox, options: { width: number; height: number; env?: Record<string, string>; target?: string }): Promise<TmuxSession> {
  const session = openReport(workspace, {
    width: options.width,
    height: options.height,
    sandbox: box,
    env: { ...options.env, ...(options.target ? { CARGO_TARGET_DIR: options.target } : {}) },
  });
  await session.waitForPane(MENU_HINT, 30_000);
  session.sendKeys("Enter");
  await session.waitForPane("issue 1/");
  return session;
}

async function todoViewer(label: string, width: number, height: number, env: Record<string, string> = {}, profile: Parameters<typeof fakeEnv>[1] = "clipboard") {
  const box = sandbox(label);
  const session = await viewer(copyWorkspace("kernel-contract/todo"), box, { width, height, env: { ...fakeEnv(box, profile), ...env } });
  return { box, session };
}

let clone: SelfClone | undefined;

// The clone of this repository, warmed once for the file. Its report is
// asserted by shape only: never its score, its counts or a rule it carries.
async function repo(): Promise<SelfClone> {
  if (!clone) {
    clone = await cloneThisRepository("clipboard");
    const warm = await runDoctor([clone.path, "--yes", "--json"], { sandbox: clone.sandbox, target: clone.target("tui"), timeoutMs: 300_000 });
    expect((warm.json as { status: string }).status).toBe("complete");
  }
  return clone;
}

afterAll(() => {
  clone = undefined;
});

describe.skipIf(!requireTmux())("interactive report clipboard", () => {
  test("CLIP-01 the last issue of a real report copies one rule, its sites bounded", async () => {
    const self = await repo();
    const box = self.sandbox;
    const session = await viewer(self.path, box, { width: 120, height: 40, env: fakeEnv(box, "clipboard"), target: self.target("tui") });
    try {
      session.sendKeys("G");
      await session.waitForPane(/issue (\d+)\/\1\b/);
      session.sendKeys("Enter");
      await session.waitForPane(COPIED);
      const calls = fakeCalls(box, "wl-copy");
      expect(calls).toHaveLength(1);
      expect(calls[0]!.argv).toEqual([]);
      const payload = calls[0]!.stdin;
      expect(payload).toStartWith("Fix exactly one Rust Doctor rule in rust-doctor:\n");
      const sites = payload.split("Affected sites:\n")[1]!.split("\n\n")[0]!.split("\n");
      const listed = sites.filter((line) => /^- \S+:\d+$/.test(line));
      expect(listed.length).toBeGreaterThan(0);
      expect(listed.length).toBeLessThanOrEqual(8);
      expect(sites.filter((line) => !/^- \S+:\d+$/.test(line)).every((line) => /^- \+\d+ more sites?$/.test(line))).toBe(true);
      expect(payload).toMatch(/\nLearn more: https:\/\/rust-doctor\.com\/rules\/\S+\n/);
      expect(payload).toMatch(/\nVerify with `rust-doctor \. --yes --verbose` and confirm \S+ is gone before moving on\.\n$/);
    } finally {
      session.kill();
    }
  }, 120_000);

  test("CLIP-02 a compact viewer confirms the copy in its status row", async () => {
    for (const width of [80, 100]) {
      const { box, session } = await todoViewer(`clip-02-${width}`, width, 23);
      try {
        session.sendKeys("Enter");
        const pane = await session.waitForPane(COPIED);
        const row = pane.split("\n").find((line) => line.includes("issue 1/1")) ?? "";
        // At 80 columns the hints after the feedback are cut, never the feedback.
        expect(row).toContain(width === 80 ? `·  ${COPIED}` : `${COPIED} · esc back · q to quit`);
        expect(fakeCalls(box, "wl-copy")).toHaveLength(1);
      } finally {
        session.kill();
      }
    }
  });

  test.failing("CLIP-03 tui/BUG-03 a copy in the stacked layout says so on screen", async () => {
    const { box, session } = await todoViewer("clip-03", 100, 40);
    try {
      session.sendKeys("Enter");
      await session.waitForStable(300);
      expect(fakeCalls(box, "wl-copy")).toHaveLength(1);
      expect(session.capturePane()).toContain(COPIED);
    } finally {
      session.kill();
    }
  });

  test("CLIP-04 no clipboard command in a split viewer offers a retry", async () => {
    const { session } = await todoViewer("clip-04", 120, 40, {}, "none");
    try {
      session.sendKeys("Enter");
      await session.waitForPane(COPY_FAILED_SPLIT);
    } finally {
      session.kill();
    }
  });

  test("CLIP-05 no clipboard command in a compact viewer says so in the status row", async () => {
    const { session } = await todoViewer("clip-05", 80, 23, {}, "none");
    try {
      session.sendKeys("Enter");
      const pane = await session.waitForPane("Copy failed · enter retry");
      expect(pane).not.toContain(COPIED);
    } finally {
      session.kill();
    }
  });

  test("CLIP-06 a clipboard command that fails is a failed copy", async () => {
    const { box, session } = await todoViewer("clip-06", 100, 23, { RD_FAKE_WL_COPY_EXIT: "1" }, ["wl-copy"]);
    try {
      session.sendKeys("Enter");
      await session.waitForPane(COPY_FAILED_COMPACT);
      expect(fakeCalls(box, "wl-copy")).toHaveLength(1);
    } finally {
      session.kill();
    }
  });

  test("CLIP-07 a clipboard command that hangs fails after the two-second bound, and a key typed meanwhile still counts", async () => {
    const { box, session } = await todoViewer("clip-07", 100, 23, { RD_FAKE_WL_COPY_SLEEP: "5" }, ["wl-copy"]);
    try {
      const started = Date.now();
      session.sendKeys("Enter");
      session.sendKeys("q");
      await session.waitForPane(COPY_FAILED_COMPACT, 6_000).catch(() => "");
      expect(await session.waitForExit(6_000)).toBe(0);
      const elapsed = Date.now() - started;
      expect(elapsed).toBeGreaterThanOrEqual(1_900);
      expect(elapsed).toBeLessThan(4_500);
      // The fake records its call before it hangs; its shell is gone.
      expect(fakeCalls(box, "wl-copy")).toHaveLength(1);
      expect(processesMentioning("profiles/custom-wl-copy/wl-copy")).toEqual([]);
    } finally {
      session.kill();
    }
  });

  test("CLIP-08 moving the selection clears the copy feedback", async () => {
    const self = await repo();
    const box = sandbox("clip-08");
    const session = await viewer(self.path, box, { width: 120, height: 40, env: fakeEnv(box, "clipboard"), target: self.target("tui") });
    try {
      session.sendKeys("Enter");
      await session.waitForPane(COPIED);
      session.sendKeys("j");
      await session.waitForPane("issue 2/");
      expect(await session.waitForStable(300)).not.toContain(COPIED);
    } finally {
      session.kill();
    }
  }, 120_000);

  test("CLIP-09 Enter after a failed copy runs the copy again", async () => {
    const { box, session } = await todoViewer("clip-09", 120, 40, { RD_FAKE_WL_COPY_EXIT: "1" }, ["wl-copy"]);
    try {
      session.sendKeys("Enter");
      await session.waitForPane(COPY_FAILED_SPLIT);
      session.sendKeys("Enter");
      const deadline = Date.now() + 3_000;
      while (fakeCalls(box, "wl-copy").length < 2 && Date.now() < deadline) await Bun.sleep(25);
      expect(fakeCalls(box, "wl-copy")).toHaveLength(2);
      expect(await session.waitForStable(300)).toContain(COPY_FAILED_SPLIT);
    } finally {
      session.kill();
    }
  });

  test("CLIP-10 an error-level rule is copied as ERROR", async () => {
    const box = sandbox("clip-10");
    const session = await viewer(copyWorkspace("kernel-contract/denied"), box, { width: 120, height: 40, env: fakeEnv(box, "clipboard") });
    try {
      session.sendKeys("Enter");
      await session.waitForPane(COPIED);
      expect(fakeCalls(box, "wl-copy")[0]!.stdin).toContain("\nERROR Bugs: Todo (clippy::todo, ×1)\n");
    } finally {
      session.kill();
    }
  });

  test.failing("CLIP-11 tui/BUG-02 an info rule is not copied as a warning", async () => {
    const box = sandbox("clip-11");
    const session = await viewer(infoBinary(), box, { width: 120, height: 40, env: fakeEnv(box, "clipboard") });
    try {
      session.sendKeys("G");
      await session.waitForPane("› ℹ Print stdout ×3");
      session.sendKeys("Enter");
      await session.waitForPane(COPIED);
      const payload = fakeCalls(box, "wl-copy")[0]!.stdin;
      expect(payload).not.toContain("WARN Maintainability: Print stdout");
    } finally {
      session.kill();
    }
  });

  test("CLIP-12 the clipboard commands are tried in order: wl-copy, xclip, xsel", async () => {
    const copy = async (label: string, names: string[], env: Record<string, string> = {}) => {
      const { box, session } = await todoViewer(label, 120, 40, env, names);
      try {
        session.sendKeys("Enter");
        await session.waitForPane(COPIED);
        return Object.fromEntries(names.map((name) => [name, fakeCalls(box, name).map((call) => call.argv)]));
      } finally {
        session.kill();
      }
    };
    expect(await copy("clip-12-a", ["wl-copy", "xclip"])).toEqual({ "wl-copy": [[]], xclip: [] });
    expect(await copy("clip-12-b", ["xclip", "xsel"])).toEqual({ xclip: [["-selection", "clipboard"]], xsel: [] });
    expect(await copy("clip-12-c", ["xclip", "xsel"], { RD_FAKE_XCLIP_EXIT: "1" })).toEqual({
      xclip: [["-selection", "clipboard"]],
      xsel: [["--clipboard", "--input"]],
    });
  });

  test("CLIP-13 a workspace path carrying an escape sequence reaches the clipboard without a control character", async () => {
    const workspace = join(scratchDir("hostile"), "hostile");
    mkdirSync(join(workspace, "src"), { recursive: true });
    writeFileSync(join(workspace, "Cargo.toml"), '[package]\nname = "hostile"\nversion = "0.1.0"\nedition = "2024"\npublish = false\n\n[workspace]\n');
    writeFileSync(join(workspace, "src/lib.rs"), '#[path = "ev\x1b[31mil.rs"]\npub mod evil;\n');
    writeFileSync(join(workspace, "src/ev\x1b[31mil.rs"), "pub fn f() -> u8 {\n    todo!()\n}\n");
    const box = sandbox("clip-13");
    const session = await viewer(workspace, box, { width: 120, height: 40, env: fakeEnv(box, "clipboard") });
    try {
      session.sendKeys("Enter");
      await session.waitForPane(COPIED);
      const payload = fakeCalls(box, "wl-copy")[0]!.stdin;
      expect(payload).toContain("Affected sites:\n- src/ev");
      expect(payload).not.toMatch(/[\x00-\x09\x0b-\x1f\x7f]/);
      expect(Buffer.byteLength(payload)).toBeLessThanOrEqual(12 * 1024);
    } finally {
      session.kill();
    }
  });
});
