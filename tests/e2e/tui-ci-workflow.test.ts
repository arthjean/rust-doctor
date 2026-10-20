// `tasks/e2e-catalog/tui.md` section 3.8: the GitHub Actions screens. Every
// scenario that can write reads `git status --porcelain --untracked-files=all`
// of its copy, and a browser is only ever the recording `xdg-open` fake.
import { describe, expect, test } from "bun:test";
import { mkdirSync, readFileSync, symlinkSync } from "node:fs";
import { join } from "node:path";
import { BIN, copyWorkspace, defaultTarget } from "./e2e-helpers";
import { fakeCalls, fakeEnv } from "./fakes";
import { gitScenario } from "./git-scenario";
import { IS_ROOT, porcelain, readOnlyWorkflows } from "./tui-actions-support";
import { CI_SETUP, MENU_HINT, openReport, requireTmux, TmuxSession } from "./tmux-helpers";
import { tuiWorkspace, type TuiWorkspace } from "./tui-support";

const GUIDE = "https://github.com/arthjean/rust-doctor#github-actions";
const CI_ENTRY = "Add to GitHub Actions (Recommended)";
const NO_BROWSER = `Couldn't open a browser. Visit ${GUIDE}`;

function open(ws: TuiWorkspace, env: Record<string, string> = {}): TmuxSession {
  return openReport(ws.path, { sandbox: ws.sandbox, env: { ...fakeEnv(ws.sandbox, "all"), ...env } });
}

// From the landing to the CI setup screen.
async function toSetup(session: TmuxSession): Promise<string> {
  await session.waitForPane(MENU_HINT, 15_000);
  session.sendKeys("Down");
  await session.waitForPane(`❯ ${CI_ENTRY}`);
  session.sendKeys("Enter");
  return session.waitForPane(CI_SETUP);
}

async function installFails(ws: TuiWorkspace, notice: string): Promise<void> {
  const before = await porcelain(ws);
  const session = open(ws);
  try {
    await toSetup(session);
    session.sendKeys("Enter");
    await session.waitForPane(`Could not add the workflow: ${notice}`);
    expect(session.status().dead).toBe(false);
    expect(await porcelain(ws)).toBe(before);
  } finally {
    session.kill();
  }
}

describe.skipIf(!requireTmux())("interactive report CI screens", () => {
  test("CI-01 the entry is offered only in a repository root without the workflow", async () => {
    const landing = async (ws: TuiWorkspace): Promise<string> => {
      const session = open(ws);
      try {
        return await session.waitForPane(MENU_HINT, 15_000);
      } finally {
        session.kill();
      }
    };
    expect(await landing(await tuiWorkspace("todo"))).not.toContain("Add to GitHub Actions");
    const repo = await tuiWorkspace("todo-git");
    expect(await landing(repo)).toContain(CI_ENTRY);
    const session = open(repo);
    try {
      await toSetup(session);
      session.sendKeys("Enter");
      expect(await session.waitForExit()).toBe(0);
    } finally {
      session.kill();
    }
    expect(await landing(repo)).not.toContain("Add to GitHub Actions");
  });

  test("CI-02 the setup screen asks, justifies and offers two answers", async () => {
    const ws = await tuiWorkspace("todo-git");
    const session = open(ws);
    try {
      const pane = await toSetup(session);
      expect(pane).toContain(`? ${CI_SETUP}`);
      expect(pane).toContain("Scan every pull request to prevent new Rust issues while you fix the backlog.");
      expect(pane).toContain("Baseline scope on a pull request, so only what the change introduces is judged.");
      expect(pane).toContain(GUIDE);
      expect(pane).toContain("❯ Yes, add the workflow");
      expect(pane).toContain("› Open the GitHub Actions guide");
      expect(pane).toContain("↑/↓ move · enter select · esc back · q quit");
    } finally {
      session.kill();
    }
  });

  test("CI-03 the guide opens through the desktop launcher and writes nothing", async () => {
    const ws = await tuiWorkspace("todo-git");
    const session = open(ws);
    try {
      await toSetup(session);
      session.sendKeys("Down");
      await session.waitForPane("❯ Open the GitHub Actions guide");
      session.sendKeys("Enter");
      await session.waitForPane("✓ Opened the GitHub Actions guide in your browser");
      expect(fakeCalls(ws.sandbox, "xdg-open").map((call) => call.argv)).toEqual([[GUIDE]]);
      expect(await porcelain(ws)).toBe("");
    } finally {
      session.kill();
    }
  });

  test("CI-04 a launcher that fails gives the guide's address instead", async () => {
    const ws = await tuiWorkspace("todo-git");
    const session = open(ws, { RD_FAKE_XDG_OPEN_EXIT: "3" });
    try {
      await toSetup(session);
      session.sendKeys("Down");
      await session.waitForPane("❯ Open the GitHub Actions guide");
      session.sendKeys("Enter");
      await session.waitForPane(NO_BROWSER);
      expect(fakeCalls(ws.sandbox, "xdg-open")).toHaveLength(1);
    } finally {
      session.kill();
    }
  });

  test("CI-05 no launcher on PATH gives the guide's address instead", async () => {
    const ws = await tuiWorkspace("todo-git");
    const session = open(ws, fakeEnv(ws.sandbox, "none"));
    try {
      await toSetup(session);
      session.sendKeys("Down");
      await session.waitForPane("❯ Open the GitHub Actions guide");
      session.sendKeys("Enter");
      await session.waitForPane(NO_BROWSER);
    } finally {
      session.kill();
    }
  });

  test("CI-06 Escape goes back to the landing on the CI entry, justification shown", async () => {
    const ws = await tuiWorkspace("todo-git");
    const session = open(ws);
    try {
      await toSetup(session);
      session.sendKeys("Escape");
      const pane = await session.waitForPane(MENU_HINT);
      expect(pane).toContain(`❯ ${CI_ENTRY}`);
      expect(pane).toContain("  Scan every pull request to prevent new Rust issues while you fix the backlog.");
      expect(await porcelain(ws)).toBe("");
    } finally {
      session.kill();
    }
  });

  test("CI-07 Yes writes the workflow, leaves the report with 0 and says so on the terminal", async () => {
    const ws = await tuiWorkspace("todo-git");
    const session = open(ws);
    try {
      await toSetup(session);
      session.sendKeys("Enter");
      expect(await session.waitForExit()).toBe(0);
      expect(await porcelain(ws)).toBe("?? .github/workflows/rust-doctor.yml\n");
      const pane = session.capturePane({ history: true });
      expect(pane.lastIndexOf("Added .github/workflows/rust-doctor.yml.")).toBeGreaterThan(pane.lastIndexOf(CI_SETUP));
      const workflow = readFileSync(join(ws.path, ".github/workflows/rust-doctor.yml"), "utf8");
      expect(workflow).toStartWith("# Written by rust-doctor ci install");
      expect(workflow).toContain("branches: ['main']");
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

  // tui/BUG-10 is that the entry is offered at all here; once it is offered,
  // the install writes nothing and says why without leaving the screen.
  test("CI-09 a repository without a commit writes nothing and says there is no default branch", async () => {
    await installFails(await tuiWorkspace("todo-nocommit"), "no default branch found");
  });

  test.failing("CI-10 tui/BUG-10 no install is recommended where another workflow already runs rust-doctor", async () => {
    const ws = await tuiWorkspace("todo-otherwf");
    const session = open(ws);
    try {
      const pane = await session.waitForPane(MENU_HINT, 15_000);
      expect(pane).not.toContain(CI_ENTRY);
    } finally {
      session.kill();
    }
  });

  test.skipIf(IS_ROOT)("CI-11 a workflows directory that cannot be written is named, and nothing is written", async () => {
    const ws = await readOnlyWorkflows();
    try {
      await installFails(ws, "could not write .github/workflows/rust-doctor.yml (permission denied)");
    } finally {
      ws.restore();
    }
  });

  test("CI-12 nothing is written through a symlinked .github", async () => {
    const ws = await tuiWorkspace("todo-git");
    mkdirSync(join(ws.path, "elsewhere"));
    symlinkSync("elsewhere", join(ws.path, ".github"));
    await installFails(ws, ".github is a symlink, and nothing is written through one");
  });

  test("CI-13 a workspace nested below the repository root offers no CI entry", async () => {
    const repo = await gitScenario("ci-13").write({ "README.md": "# outer\n", ".gitignore": "target/\n" }).commit("base").build();
    const workspace = copyWorkspace("kernel-contract/todo", join(repo.path, "crate"));
    const session = openReport(workspace, { sandbox: repo.sandbox, env: fakeEnv(repo.sandbox, "all") });
    try {
      const pane = await session.waitForPane(MENU_HINT, 15_000);
      expect(pane).toContain("❯ Review 1 issue");
      expect(pane).not.toContain("Add to GitHub Actions");
    } finally {
      session.kill();
    }
  });

  test("CI-14 opening the guide after a failed install replaces the failure notice", async () => {
    const ws = await tuiWorkspace("todo-otherwf");
    const session = open(ws);
    try {
      await toSetup(session);
      session.sendKeys("Enter");
      await session.waitForPane("Could not add the workflow:");
      session.sendKeys("Down");
      await session.waitForPane("❯ Open the GitHub Actions guide");
      session.sendKeys("Enter");
      const pane = await session.waitForPane("✓ Opened the GitHub Actions guide in your browser");
      expect(pane).not.toContain("Could not add the workflow");
    } finally {
      session.kill();
    }
  });
});
