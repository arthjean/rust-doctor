// `tasks/e2e-catalog/tui.md` section 3.9: the GitHub Actions recommendation,
// the agent menu and the prompt it hands over. Agents are the recording fakes
// of the `agents` profile, or a probe agent written for the row; the prompt
// is read from what the agent or the fake clipboard received, or from the
// byte log when the report prints it.
import { describe, expect, test } from "bun:test";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { runDoctor, sandbox, scratchDir, toolchainBin } from "./e2e-helpers";
import { fakeCalls, fakeEnv, pathProfile } from "./fakes";
import {
  agentLog,
  agentPath,
  agentRecord,
  IS_ROOT,
  logText,
  porcelain,
  probeAgent,
  readOnlyWorkflows,
  reportWithStderrFile,
} from "./tui-actions-support";
import { MENU_HINT, openReport, requireTmux, TmuxSession } from "./tmux-helpers";
import { tuiWorkspace, type TuiWorkspace } from "./tui-support";

const RECOMMENDATION = "Add Rust Doctor to GitHub Actions first";
const HANDOFF = "Choose how to continue";
const RULER = "---- Agent prompt ----";
const SKILL_LINE = "The rust-doctor skill is not installed for this agent: run `rust-doctor skill install --agent claude` first.";
const PROMPT_START = "Fix the highest-priority Rust Doctor findings. Preserve unrelated changes and rescan after the fixes.";

function open(ws: TuiWorkspace, env: Record<string, string> = fakeEnv(ws.sandbox, "agents")): TmuxSession {
  return openReport(ws.path, { sandbox: ws.sandbox, env });
}

// From a `todo-git` landing to the recommendation the handoff opens on.
async function toRecommendation(session: TmuxSession): Promise<string> {
  await session.waitForPane(MENU_HINT, 15_000);
  session.sendKeys("Down", "Down");
  await session.waitForPane("❯ Hand off to an agent");
  session.sendKeys("Enter");
  return session.waitForPane(RECOMMENDATION);
}

// From a landing without the CI entry straight to the agent menu.
async function toHandoff(session: TmuxSession): Promise<string> {
  await session.waitForPane(MENU_HINT, 15_000);
  session.sendKeys("Down");
  await session.waitForPane("❯ Hand off to an agent");
  session.sendKeys("Enter");
  return session.waitForPane(HANDOFF);
}

// What stderr said once the scan's one-line-per-phase progress is set aside.
function said(stderr: string): string {
  return stderr.split("\n").filter((line) => !/^(Running native passes|Compiling dependencies)\.\.\.$|^Linting \S+ \(\d+\/\d+\)$/.test(line)).join("\n");
}

// The prompt the report printed between its two rulers.
function printedPrompt(session: TmuxSession): string {
  const parts = logText(session).split(`${RULER}\n`);
  expect(parts.length).toBeGreaterThanOrEqual(3);
  return parts[parts.length - 2]!.trimEnd();
}

describe.skipIf(!requireTmux())("interactive report handoff", () => {
  test("HO-01 a handoff from a repository without CI recommends GitHub Actions first", async () => {
    const ws = await tuiWorkspace("todo-git");
    const session = open(ws);
    try {
      const pane = await toRecommendation(session);
      expect(pane).toContain("❯ Add to GitHub Actions first (Recommended)");
      expect(pane).toContain("› Continue without GitHub Actions");
      expect(pane).toContain("↑/↓ move · enter select · esc skip · q quit");
    } finally {
      session.kill();
    }
  });

  test("HO-02 Escape skips the recommendation and writes nothing", async () => {
    const ws = await tuiWorkspace("todo-git");
    const session = open(ws);
    try {
      await toRecommendation(session);
      session.sendKeys("Escape");
      await session.waitForPane(HANDOFF);
      expect(await porcelain(ws)).toBe("");
    } finally {
      session.kill();
    }
  });

  test("HO-03 continuing without GitHub Actions writes nothing", async () => {
    const ws = await tuiWorkspace("todo-git");
    const session = open(ws);
    try {
      await toRecommendation(session);
      session.sendKeys("Down");
      await session.waitForPane("❯ Continue without GitHub Actions");
      session.sendKeys("Enter");
      await session.waitForPane(HANDOFF);
      expect(await porcelain(ws)).toBe("");
    } finally {
      session.kill();
    }
  });

  test("HO-04 accepting writes the workflow, and the landing no longer offers it", async () => {
    const ws = await tuiWorkspace("todo-git");
    const session = open(ws);
    try {
      await toRecommendation(session);
      session.sendKeys("Enter");
      await session.waitForPane(HANDOFF);
      expect(await porcelain(ws)).toBe("?? .github/workflows/rust-doctor.yml\n");
      session.sendKeys("Escape");
      const landing = await session.waitForPane(MENU_HINT);
      expect(landing).not.toContain("Add to GitHub Actions");
      expect(landing).toContain("❯ Hand off to an agent");
      session.sendKeys("Enter");
      const pane = await session.waitForPane(HANDOFF);
      expect(pane).not.toContain(RECOMMENDATION);
    } finally {
      session.kill();
    }
  });

  const failing: Array<[string, () => Promise<TuiWorkspace & { restore?: () => void }>]> = [
    ["todo-nocommit", () => tuiWorkspace("todo-nocommit")],
    ["todo-otherwf", () => tuiWorkspace("todo-otherwf")],
    ...(IS_ROOT ? [] : [["todo-rogh", readOnlyWorkflows] as [string, () => Promise<TuiWorkspace & { restore?: () => void }>]]),
  ];

  test.each(failing)("HO-05 an install that fails on %s stays under the recommendation, and Escape skips it", async (_kind, build) => {
    const ws = await build();
    try {
      const before = await porcelain(ws);
      const session = open(ws);
      try {
        await toRecommendation(session);
        session.sendKeys("Enter");
        const pane = await session.waitForPane("Could not add the workflow:");
        expect(pane).toContain(RECOMMENDATION);
        expect(await porcelain(ws)).toBe(before);
        session.sendKeys("Escape");
        await session.waitForPane(HANDOFF);
      } finally {
        session.kill();
      }
    } finally {
      ws.restore?.();
    }
  });

  test("HO-06 the agent menu lists every agent on PATH, then the copy", async () => {
    const ws = await tuiWorkspace("todo");
    const session = open(ws);
    try {
      const pane = await toHandoff(session);
      expect(pane).toMatch(/❯ Claude Code\n\n› Codex\n\n› Cursor\n\n› Copy prompt\n/);
    } finally {
      session.kill();
    }
  });

  test("HO-07 the menu holds only the agents PATH has, and the copy alone explains itself", async () => {
    const codexOnly = await tuiWorkspace("todo");
    let session = open(codexOnly, fakeEnv(codexOnly.sandbox, ["codex"]));
    try {
      const pane = await toHandoff(session);
      expect(pane).toMatch(/❯ Codex\n\n› Copy prompt\n/);
      expect(pane).not.toContain("Claude Code");
    } finally {
      session.kill();
    }
    const none = await tuiWorkspace("todo");
    session = open(none, fakeEnv(none.sandbox, "none"));
    try {
      const pane = await toHandoff(session);
      expect(pane).toContain("❯ Copy prompt");
      expect(pane).toContain("Paste into any agent or edit it first");
      expect(pane).not.toContain("Codex");
    } finally {
      session.kill();
    }
  });

  test("HO-08 Escape from the agent menu lands on the handoff entry", async () => {
    const ws = await tuiWorkspace("todo-git");
    const session = open(ws);
    try {
      await toRecommendation(session);
      session.sendKeys("Escape");
      await session.waitForPane(HANDOFF);
      session.sendKeys("Escape");
      expect(await session.waitForPane(MENU_HINT)).toContain("❯ Hand off to an agent");
    } finally {
      session.kill();
    }
  });

  test("HO-09 the first agent gets one argument, the workspace as cwd, and the skill reminder first", async () => {
    const ws = await tuiWorkspace("todo-git");
    const session = open(ws);
    try {
      await toRecommendation(session);
      session.sendKeys("Escape");
      await session.waitForPane(HANDOFF);
      session.sendKeys("Enter");
      expect(await session.waitForExit()).toBe(0);
      const calls = fakeCalls(ws.sandbox, "claude");
      expect(calls).toHaveLength(1);
      expect(calls[0]!.argv).toHaveLength(1);
      expect(calls[0]!.cwd).toBe(ws.path);
      const payload = calls[0]!.argv[0]!;
      expect(payload).toStartWith(`${SKILL_LINE}\n${PROMPT_START}\n`);
      expect(payload).toMatch(/\nAudit: \d+\/100 [A-Za-z ]+ \(authoritative\)\.\n/);
      expect(payload).toMatch(/\nTarget: repairing the rules below reaches a projected \d+\/100\.\n/);
      expect(payload).toEndWith("\nValidate with: rust-doctor . --yes\n");
      expect(await porcelain(ws)).toBe("");
    } finally {
      session.kill();
    }
  });

  test("HO-10 the agent inherits the terminal on every stream the report has", async () => {
    const run = async (redirect: boolean): Promise<string | undefined> => {
      const ws = await tuiWorkspace("todo");
      const env = { PATH: agentPath({ codex: probeAgent("codex") }), RD_AGENT_LOG: agentLog(ws.sandbox) };
      const { session } = redirect
        ? reportWithStderrFile(ws, { env })
        : { session: openReport(ws.path, { sandbox: ws.sandbox, env }) };
      try {
        await toHandoff(session);
        session.sendKeys("Enter");
        expect(await session.waitForExit()).toBe(0);
        return agentRecord(ws.sandbox, "codex", "tty");
      } finally {
        session.kill();
      }
    };
    expect(await run(false)).toBe("yyy");
    expect(await run(true)).toBe("yyn");
  });

  test("HO-11 an installed skill drops the reminder from the prompt", async () => {
    const ws = await tuiWorkspace("todo");
    const installed = await runDoctor(["skill", "install", ws.path, "--agent", "claude"], { sandbox: ws.sandbox });
    expect(installed.exitCode).toBe(0);
    const session = open(ws);
    try {
      await toHandoff(session);
      session.sendKeys("Enter");
      expect(await session.waitForExit()).toBe(0);
      const payload = fakeCalls(ws.sandbox, "claude")[0]!.argv[0]!;
      expect(payload).toStartWith(`${PROMPT_START}\n`);
      expect(payload).not.toContain("skill is not installed");
    } finally {
      session.kill();
    }
  });

  test("HO-12 an agent's own exit status never replaces the scan's", async () => {
    const ws = await tuiWorkspace("denied");
    const env = { PATH: agentPath({ codex: probeAgent("codex", { code: 9 }) }), RD_AGENT_LOG: agentLog(ws.sandbox) };
    const session = openReport(ws.path, { sandbox: ws.sandbox, env });
    try {
      await toHandoff(session);
      session.sendKeys("Enter");
      expect(await session.waitForExit()).toBe(1);
      expect(agentRecord(ws.sandbox, "codex", "payload")).toContain("clippy::todo");
      expect(session.capturePane({ history: true })).toContain("FAKE-AGENT codex RAN");
    } finally {
      session.kill();
    }
  });

  test("HO-13 an agent that cannot start prints the prompt instead, and the scan's code holds", async () => {
    const ws = await tuiWorkspace("todo");
    const env = { PATH: agentPath({ codex: "#!/nonexistent/interpreter\n" }) };
    const { session, stderr } = reportWithStderrFile(ws, { env });
    try {
      await toHandoff(session);
      session.sendKeys("Enter");
      expect(await session.waitForExit()).toBe(0);
      expect(said(stderr())).toMatch(/^Warning: could not launch \S+\/codex: .+ The prompt follows\.\n$/);
      const prompt = printedPrompt(session);
      expect(prompt).toStartWith(`${SKILL_LINE.replace("claude", "codex")}\n${PROMPT_START}\n`);
      expect(prompt).toEndWith("\nValidate with: rust-doctor . --yes");
    } finally {
      session.kill();
    }
  });

  test("HO-14 a copy after the workflow was written reports both, in order", async () => {
    const ws = await tuiWorkspace("todo-git");
    const { session, stderr } = reportWithStderrFile(ws, { env: fakeEnv(ws.sandbox, "all") });
    try {
      await toRecommendation(session);
      session.sendKeys("Enter");
      await session.waitForPane(HANDOFF);
      session.sendKeys("Down", "Down", "Down");
      await session.waitForPane("❯ Copy prompt");
      session.sendKeys("Enter");
      expect(await session.waitForExit()).toBe(0);
      expect(said(stderr())).toBe("Added .github/workflows/rust-doctor.yml.\nPrompt copied to clipboard.\n");
      const copied = fakeCalls(ws.sandbox, "wl-copy");
      expect(copied).toHaveLength(1);
      expect(copied[0]!.stdin).toStartWith(PROMPT_START);
      expect(copied[0]!.stdin).toEndWith("\nValidate with: rust-doctor . --yes\n");
      expect(fakeCalls(ws.sandbox, "claude")).toHaveLength(0);
    } finally {
      session.kill();
    }
  });

  test("HO-15 with no agent and no clipboard, Enter on the copy prints the prompt and the scan's code holds", async () => {
    const ws = await tuiWorkspace("denied");
    const { session, stderr } = reportWithStderrFile(ws, { env: { PATH: toolchainBin() } });
    try {
      await toHandoff(session);
      session.sendKeys("Enter");
      expect(await session.waitForExit()).toBe(1);
      expect(said(stderr())).toBe("Clipboard unavailable: no clipboard command on PATH. The prompt follows.\n");
      const prompt = printedPrompt(session);
      expect(prompt).toStartWith(PROMPT_START);
      expect(prompt).toContain("\n1. clippy::todo (error, 1 occurrence");
      expect(prompt).toEndWith("\nValidate with: rust-doctor . --yes");
    } finally {
      session.kill();
    }
  });

  test("HO-16 the rescan command repeats every policy and scope flag, shell-quoted", async () => {
    const ws = await tuiWorkspace("todo-git");
    const lib = join(ws.path, "src/lib.rs");
    writeFileSync(lib, `${readFileSync(lib, "utf8")}// touched\n`);
    const session = openReport(ws.path, {
      sandbox: ws.sandbox,
      env: fakeEnv(ws.sandbox, "clipboard"),
      args: [
        "--rule", "clippy::dbg_macro=off", "--category", "maintainability=error", "--blocking", "warning",
        "--package", "kernel-todo", "--scope", "files", "--base", "HEAD",
      ],
    });
    try {
      await toRecommendation(session);
      session.sendKeys("Escape");
      await session.waitForPane("❯ Copy prompt");
      session.sendKeys("Enter");
      expect(await session.waitForExit()).toBe(1);
      expect(fakeCalls(ws.sandbox, "wl-copy")[0]!.stdin).toEndWith(
        "\nValidate with: rust-doctor . --blocking warning --rule 'clippy::dbg_macro=off' --category 'maintainability=error'" +
          " --package kernel-todo --scope files --base HEAD --yes\n",
      );
    } finally {
      session.kill();
    }
  });

  test("HO-17 a prompt over 12 KiB is refused, and a clean scan then exits 2", async () => {
    // One module at a path of about 800 bytes holding thirty `todo!()`: the
    // 24 sites the prompt lists cannot fit in 12 KiB.
    const workspace = join(scratchDir("long-sites"), "long-sites");
    const dirs = ["a", "b", "c", "d"].map((letter) => letter.repeat(200));
    const module = join(...dirs, "m.rs");
    mkdirSync(join(workspace, "src", ...dirs), { recursive: true });
    writeFileSync(join(workspace, "Cargo.toml"), '[package]\nname = "long-sites"\nversion = "0.1.0"\nedition = "2024"\npublish = false\n\n[workspace]\n');
    writeFileSync(join(workspace, "src/lib.rs"), `#[path = "${module}"]\npub mod m;\n`);
    writeFileSync(join(workspace, "src", module), Array.from({ length: 30 }, (_, n) => `pub fn f${n}() -> u8 {\n    todo!()\n}\n`).join("\n"));
    const ws = { path: workspace, sandbox: sandbox("ho-17") };
    const { session, stderr } = reportWithStderrFile(ws, { env: { PATH: pathProfile("none") } });
    try {
      await session.waitForPane(MENU_HINT, 30_000);
      session.sendKeys("Down");
      await session.waitForPane("❯ Hand off to an agent");
      session.sendKeys("Enter");
      await session.waitForPane(HANDOFF);
      session.sendKeys("Enter");
      expect(await session.waitForExit()).toBe(2);
      expect(said(stderr())).toBe("Handoff prompt exceeds the 12 KiB limit.\n");
      expect(logText(session)).not.toContain(RULER);
    } finally {
      session.kill();
    }
  }, 90_000);
});
