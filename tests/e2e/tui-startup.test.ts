// `tasks/e2e-catalog/tui.md` section 3.2: the landing per workspace kind, and
// the count-up. Every workspace's git state is set by tui-support.ts, since
// the same fixture scores differently inside and outside a repository. The
// clone of this repository is asserted by shape only: its score, label,
// counts and findings move with every commit.
import { describe, expect, test } from "bun:test";
import { join } from "node:path";
import { targetDir } from "./e2e-helpers";
import { gitScenario } from "./git-scenario";
import { MENU_HINT, openReport, requireTmux, VIEWER_HINT, type TmuxSession } from "./tmux-helpers";
import { frames, frameScores, quit } from "./tui-entry-support";
import { tuiWorkspace, type TuiKind } from "./tui-support";

type Open = { session: TmuxSession; pane: string };

// The landing of `kind` once it has stopped changing.
async function landing(kind: TuiKind, options: { args?: string[]; color?: boolean; sub?: string; env?: Record<string, string> } = {}): Promise<Open> {
  const workspace = await tuiWorkspace(kind);
  const session = openReport(options.sub ? join(workspace.path, options.sub) : workspace.path, {
    sandbox: workspace.sandbox,
    width: 100,
    height: 40,
    args: options.args,
    color: options.color,
    env: options.env,
  });
  await session.waitForPane((pane) => /(?:^|\s)q quit\s*$/m.test(pane), 30_000);
  return { session, pane: await session.waitForStable(300) };
}

function rows(pane: string): string[] {
  return pane.replace(/\n+$/, "").split("\n");
}

const JUSTIFICATION = [
  "  Scan every pull request to prevent new Rust issues while you fix the backlog.",
  "  Baseline scope on a pull request, so only what the change introduces is judged.",
  "  https://github.com/arthjean/rust-doctor#github-actions",
];

describe.skipIf(!requireTmux())("interactive report landing", () => {
  test("LAND-01 todo lands on its score, its projection and two actions", async () => {
    const { session, pane } = await landing("todo");
    try {
      expect(pane).toContain("  ┌─────┐  94 / 100 Great  ·  kernel-todo");
      expect(pane).toContain("│ ◠ ◠ │");
      expect(pane).toContain("│  ▽  │");
      expect(pane).toContain("Rust Doctor (https://rust-doctor.com)");
      expect(pane).toContain("Potential score 100 after priority fixes +6");
      expect(pane).toContain("❯ Review 1 issue");
      expect(pane).toContain("› Hand off to an agent");
      expect(pane).not.toContain("GitHub Actions");
      expect(rows(pane)).toHaveLength(11);
      expect(rows(pane).at(-1)).toBe(MENU_HINT);
    } finally {
      session.kill();
    }
  });

  test("LAND-02 a clean workspace offers no action, ignores the menu keys and leaves on Escape", async () => {
    const { session, pane } = await landing("clean");
    try {
      expect(pane).toContain("100 / 100 Great  ·  clean");
      expect(pane).toContain("✔ No issues found. Nice work.");
      expect(rows(pane).at(-1)).toBe("q quit");
      expect(pane).not.toContain("❯");
      for (const key of ["Enter", "j", "k", "Down", "G"]) {
        session.sendKeys(key);
        expect(await session.waitForStable(200)).toBe(pane);
      }
      expect(await quit(session, "Escape")).toBe(0);
    } finally {
      session.kill();
    }
  });

  test("LAND-03 a files scope with no change offers GitHub Actions as the only action", async () => {
    const { session, pane } = await landing("todo-git", { args: ["--scope", "files"] });
    try {
      expect(pane).toContain("✔ No issues found. Nice work.");
      expect(pane).toContain(["❯ Add to GitHub Actions (Recommended)", ...JUSTIFICATION].join("\n"));
      expect(pane).not.toContain("Review");
      expect(pane).not.toContain("Hand off");
      expect(rows(pane).at(-1)).toBe(MENU_HINT);
    } finally {
      session.kill();
    }
  });

  test("LAND-04 this repository lands on a score, a review, GitHub Actions and a handoff, in that order", async () => {
    const { session, pane } = await landing("repo", { env: { CARGO_TARGET_DIR: targetDir("repo") } });
    try {
      expect(pane).toMatch(/^ {2}┌─────┐ {2}\d+ \/ 100 \S[^\n]* {2}· {2}rust-doctor$/m);
      expect(pane).toMatch(/^ {2}│ \S \S │/m);
      expect(pane).toContain("Rust Doctor (https://rust-doctor.com)");
      const review = pane.search(/❯ Review \d+ issues?$/m);
      const ci = pane.indexOf("› Add to GitHub Actions (Recommended)");
      const handoff = pane.indexOf("› Hand off to an agent");
      expect(review).toBeGreaterThan(0);
      expect(ci).toBeGreaterThan(review);
      expect(handoff).toBeGreaterThan(ci);
    } finally {
      session.kill();
    }
  }, 120_000);

  test("LAND-05 a scan that linted no package says the score is unavailable and why", async () => {
    const { session, pane } = await landing("denied");
    try {
      expect(pane).toContain("Score unavailable: the scan could not score, since no package was linted.");
      expect(pane).toContain("1 finding · kernel-denied");
      expect(pane).toContain("⚠ clippy and execution checks did not complete: results are incomplete.");
      expect(pane).not.toContain("/ 100");
      expect(pane).not.toContain("Potential score");
      expect(await quit(session)).toBe(1);
    } finally {
      session.kill();
    }
  });

  test.each([
    ["partial", ["Score unavailable: the scan could not score, since no package was linted.", "5 findings · partial-failure", "❯ Review 4 issues"]],
    ["broken-member", [
      "94 / 100 Core partial  ·  broken-member",
      "Score is partial: a stage of the scan failed. Fix what its error reports, then rerun.",
      "Score is partial: some packages did not compile and were not linted. Fix their build, then rerun.",
      "❯ Review 2 issues",
    ]],
  ] as const)("LAND-06 an incomplete %s scan says what did not complete", async (kind, lines) => {
    const { session, pane } = await landing(kind);
    try {
      for (const line of lines) expect(pane).toContain(line);
      expect(pane).toContain("⚠ clippy and execution checks did not complete: results are incomplete.");
    } finally {
      session.kill();
    }
  });

  test("LAND-07 a workspace with no Rust file has no score and no action", async () => {
    const { session, pane } = await landing("no-rust");
    try {
      expect(pane).not.toContain("┌─────┐");
      expect(pane).toContain("Score unavailable: no Rust files were analyzed.");
      expect(pane).toContain("0 findings · no-rust");
      expect(pane).toContain("⚠ No issues detected, but execution checks did not complete.");
      expect(rows(pane).at(-1)).toBe("q quit");
      expect(await quit(session)).toBe(1);
    } finally {
      session.kill();
    }
  });

  test("LAND-08 a workspace of tests only has no score and no issue", async () => {
    const { session, pane } = await landing("tests-only");
    try {
      expect(pane).toContain("Score unavailable: only tests, benches, examples or build scripts were found.");
      expect(pane).toContain("0 findings · tests-only");
      expect(pane).toContain("✔ No issues found. Nice work.");
    } finally {
      session.kill();
    }
  });

  test("LAND-09 uncatalogued findings land on 65 and a projection of +35", async () => {
    const { session, pane } = await landing("uncatalogued");
    try {
      expect(pane).toContain("65 / 100 Needs work");
      expect(pane).toContain("Potential score 100 after priority fixes +35");
    } finally {
      session.kill();
    }
  });

  test("LAND-10 the discovery notice stays above an intact score block", async () => {
    const { session, pane } = await landing("parent1");
    try {
      const [first, second] = rows(pane);
      expect(first).toBe("No Cargo workspace at this path: scanning the one in inner/.");
      expect(second).toBe("  ┌─────┐  94 / 100 Great  ·  kernel-todo");
      expect(pane).toContain("  └─────┘");
    } finally {
      session.kill();
    }
  });

  test("LAND-11 a scan cut by --max-duration says so and leaves with 2", async () => {
    const { session, pane } = await landing("sleeping", { args: ["--max-duration", "1"] });
    try {
      expect(pane).toContain("Score is partial: the scan hit its --max-duration limit. Raise the limit or narrow the scope.");
      expect(pane).toContain("⚠ clippy checks did not complete: results are incomplete.");
      expect(await quit(session)).toBe(2);
    } finally {
      session.kill();
    }
  });

  test("LAND-12 the score counts up from 0, the menu waits for it, and the gain grows after it", async () => {
    const { session } = await landing("uncatalogued", { color: true });
    try {
      const drawn = frames(session);
      const scores = frameScores(session);
      expect(scores[0]).toBe(0);
      expect(scores.at(-1)).toBe(65);
      for (let index = 1; index < scores.length; index += 1) expect(scores[index]!).toBeGreaterThanOrEqual(scores[index - 1]!);
      expect(new Set(scores).size).toBeGreaterThan(10);
      expect(drawn.slice(0, -1).some((frame) => frame.includes("❯"))).toBe(false);
      const gains = drawn.filter((frame) => frame.includes("65 / 100")).map((frame) => [...frame].filter((cell) => cell === "▓").length);
      expect(gains[0]).toBe(0);
      expect(gains.at(-1)).toBeGreaterThan(0);
    } finally {
      session.kill();
    }
  });

  test("LAND-13 the count-up draws 58 frames before the first key", async () => {
    const { session } = await landing("uncatalogued", { color: true });
    try {
      expect(session.byteLog().split("\x1b[J").length - 1).toBe(58);
    } finally {
      session.kill();
    }
  });

  test("LAND-14 without color the first frame already shows the final score", async () => {
    const { session } = await landing("todo");
    try {
      expect(frames(session)).toHaveLength(1);
      expect(frameScores(session)).toEqual([94]);
    } finally {
      session.kill();
    }
  });

  test("LAND-15 a report with no score never counts up, even in color", async () => {
    const { session } = await landing("no-rust", { color: true });
    try {
      expect(frames(session)).toHaveLength(1);
      expect(frameScores(session)).toEqual([]);
    } finally {
      session.kill();
    }
  });

  test("LAND-16 a partial score with no projection counts up and draws no gain", async () => {
    const { session } = await landing("broken-member", { color: true });
    try {
      expect(frames(session).length).toBeGreaterThan(10);
      expect(frames(session).some((frame) => frame.includes("▓"))).toBe(false);
      expect(frameScores(session).at(-1)).toBe(94);
    } finally {
      session.kill();
    }
  });

  test("LAND-17 a subdirectory lands on the workspace above it, with no discovery notice", async () => {
    const { session, pane } = await landing("todo", { sub: "src" });
    try {
      expect(pane).toContain("94 / 100 Great  ·  kernel-todo");
      expect(pane).not.toContain("No Cargo workspace");
    } finally {
      session.kill();
    }
  });

  test("LAND-18 a baseline scope lands on the findings the change introduced, and only those", async () => {
    const repo = await gitScenario("land-18")
      .fixture("kernel-contract/todo")
      .write({ ".gitignore": "/target\n" })
      .commit("base")
      .append("src/lib.rs", "\npub fn later() -> u8 {\n    unimplemented!()\n}\n")
      .build();
    const session = openReport(repo.path, { sandbox: repo.sandbox, width: 100, height: 40, args: ["--scope", "baseline", "--base", "HEAD"] });
    try {
      const pane = await session.waitForPane(MENU_HINT, 30_000);
      expect(pane).toContain("❯ Review 1 issue");
      session.sendKeys("Enter");
      const viewer = await session.waitForPane(VIEWER_HINT);
      expect(viewer).toContain("Unimplemented");
      expect(viewer).not.toContain("Todo");
    } finally {
      session.kill();
    }
  });
});
