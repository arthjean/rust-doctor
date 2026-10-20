// HR-01 to HR-27 of `tasks/e2e-catalog/subcommands.md` section 7: `hook run
// claude|cursor` fed the payloads Claude Code and Cursor send at the end of a
// turn, on the `mini` history, whose pre-existing `todo!()` the lines scope
// never judges. Every run gets its own CARGO_TARGET_DIR outside the repository
// so HR-25's claim, the hook writes nothing in the tree, holds for every row.
import { describe, expect, test } from "bun:test";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { BIN, copyWorkspace, runDoctor, sandbox, scratchDir, treeFingerprint, type DoctorRun, type Sandbox } from "./e2e-helpers";
import { gitScenario, NEW_FN, type Repo } from "./git-scenario";
import { runPty } from "./pty";

const RESCAN = "Fix them, then rescan: rust-doctor . --scope lines --base HEAD --include-untracked --blocking warning";
const ONE_FINDING =
  "rust-doctor found 1 finding in the lines this turn changed:\n" +
  "- clippy::todo src/lib.rs:12 `todo` should not be present in production code\n" +
  `${RESCAN}\n`;
const unscanned = (reason: string) => `rust-doctor could not scan (${reason}); not blocking this turn.\n`;

function claudePayload(repo: { path: string; sandbox: Sandbox }, extra: Record<string, unknown> = {}): string {
  return JSON.stringify({
    session_id: "e2e-session",
    transcript_path: join(repo.sandbox.home, "t.jsonl"),
    cwd: repo.path,
    permission_mode: "default",
    hook_event_name: "Stop",
    stop_hook_active: false,
    ...extra,
  });
}

function cursorPayload(repo: { path: string }, extra: Record<string, unknown> = {}): string {
  return JSON.stringify({
    status: "completed",
    loop_count: 0,
    conversation_id: "c",
    generation_id: "g",
    hook_event_name: "stop",
    workspace_roots: [repo.path],
    ...extra,
  });
}

// `mini` with the turn's change: NEW_FN appended to the tracked lib, unstaged.
async function changed(name: string): Promise<Repo> {
  const repo = await gitScenario(name).template("mini").commit("base").build();
  const lib = join(repo.path, "src/lib.rs");
  writeFileSync(lib, readFileSync(lib, "utf8") + NEW_FN);
  return repo;
}

// `mini` with an untracked module of twelve `todo!()`, declared from the lib.
async function many(name: string): Promise<Repo> {
  const repo = await gitScenario(name).template("mini").commit("base").build();
  const lib = join(repo.path, "src/lib.rs");
  writeFileSync(lib, `${readFileSync(lib, "utf8")}pub mod many;\n`);
  const fns = Array.from({ length: 12 }, (_, n) => `pub fn f${n}() -> u8 {\n    todo!()\n}\n`).join("\n");
  writeFileSync(join(repo.path, "src/many.rs"), fns);
  return repo;
}

type HookOptions = { cwd?: string; env?: Record<string, string | undefined>; stdin?: string; target?: string };

// One hook run under the repository's sandbox, asserting it wrote nothing in
// the tree it judged.
async function hook(agent: "claude" | "cursor", repo: { path: string; sandbox: Sandbox }, options: HookOptions = {}): Promise<DoctorRun> {
  const before = treeFingerprint(repo.path);
  const run = await runDoctor(["hook", "run", agent], {
    sandbox: repo.sandbox,
    cwd: options.cwd ?? repo.path,
    env: agent === "claude" ? { CLAUDE_PROJECT_DIR: repo.path, ...options.env } : options.env,
    stdin: options.stdin ?? (agent === "claude" ? claudePayload(repo) : cursorPayload(repo)),
    target: options.target ?? join(scratchDir("hook-target"), "target"),
    timeoutMs: 120_000,
  });
  expect(treeFingerprint(repo.path)).toBe(before);
  return run;
}

const outcome = (run: DoctorRun) => ({ exitCode: run.exitCode, stdout: run.stdout, stderr: run.stderr });

describe("hook run claude", () => {
  test("HR-01 an unchanged tree passes silently", async () => {
    const repo = await gitScenario("hr-01").template("mini").commit("base").build();
    expect(outcome(await hook("claude", repo))).toEqual({ exitCode: 0, stdout: "", stderr: "" });
  });

  test("HR-02 a todo!() the turn added blocks with the finding and the rescan line", async () => {
    const repo = await changed("hr-02");
    expect(outcome(await hook("claude", repo))).toEqual({ exitCode: 2, stdout: "", stderr: ONE_FINDING });
  });

  test("HR-03 stop_hook_active ends the hook before any scan", async () => {
    const repo = await changed("hr-03");
    const target = join(scratchDir("hr-03"), "target");
    const run = await hook("claude", repo, { stdin: claudePayload(repo, { stop_hook_active: true }), target });
    expect(outcome(run)).toEqual({ exitCode: 0, stdout: "", stderr: "" });
    expect(existsSync(target)).toBe(false);
  });

  test("HR-04 without CLAUDE_PROJECT_DIR the current directory is scanned", async () => {
    const repo = await changed("hr-04");
    expect(outcome(await hook("claude", repo, { env: { CLAUDE_PROJECT_DIR: undefined } }))).toEqual({ exitCode: 2, stdout: "", stderr: ONE_FINDING });
  });

  test("HR-05 an empty CLAUDE_PROJECT_DIR falls back to the current directory", async () => {
    const repo = await changed("hr-05");
    expect(outcome(await hook("claude", repo, { env: { CLAUDE_PROJECT_DIR: "" } }))).toEqual({ exitCode: 2, stdout: "", stderr: ONE_FINDING });
  });

  test("HR-06 CLAUDE_PROJECT_DIR wins over the current directory", async () => {
    const repo = await changed("hr-06");
    expect(outcome(await hook("claude", repo, { cwd: "/" }))).toEqual({ exitCode: 2, stdout: "", stderr: ONE_FINDING });
  });

  test("HR-07 from / without CLAUDE_PROJECT_DIR the hook cannot scan and lets the turn end", async () => {
    const repo = await changed("hr-07");
    const run = await hook("claude", repo, { cwd: "/", env: { CLAUDE_PROJECT_DIR: undefined } });
    expect(outcome(run)).toEqual({ exitCode: 0, stdout: "", stderr: unscanned("discovery: ambiguous-workspace") });
  });

  test.each([
    ["HR-08", "input that is not JSON", "not json"],
    ["HR-09", "empty input", ""],
    ["HR-10", "a string stop_hook_active", '{"stop_hook_active":"true"}'],
  ])("%s %s is no guard: the hook scans and blocks", async (id, _label, stdin) => {
    const repo = await changed(id.toLowerCase());
    expect(outcome(await hook("claude", repo, { stdin }))).toEqual({ exitCode: 2, stdout: "", stderr: ONE_FINDING });
  });

  test.failing("HR-11 subcommands/F17 a Stop payload over 1 MiB keeps the stop_hook_active loop guard", async () => {
    const repo = await gitScenario("hr-11").template("mini").commit("base").build();
    const lib = join(repo.path, "src/lib.rs");
    writeFileSync(lib, `${readFileSync(lib, "utf8")}\npub fn turn() -> u8 {\n    todo!()\n}\n`);
    const env = { CLAUDE_PROJECT_DIR: repo.path };
    const guarded = JSON.stringify({ stop_hook_active: true });
    const small = await runDoctor(["hook", "run", "claude"], { sandbox: repo.sandbox, cwd: repo.path, env, stdin: guarded });
    expect([small.exitCode, small.stderr]).toEqual([0, ""]);
    const large = JSON.stringify({ stop_hook_active: true, pad: "x".repeat(1100 * 1024) });
    const run = await runDoctor(["hook", "run", "claude"], { sandbox: repo.sandbox, cwd: repo.path, env, stdin: large });
    expect([run.exitCode, run.stderr]).toEqual([0, ""]);
  });

  test("HR-12 blocking = \"none\" in the configuration does not unblock the hook", async () => {
    const repo = await changed("hr-12");
    writeFileSync(join(repo.path, "rust-doctor.toml"), 'blocking = "none"\n');
    const run = await hook("claude", repo);
    expect(run.exitCode).toBe(2);
    expect(run.stderr).toStartWith("rust-doctor found 1 finding");
  });

  test("HR-13 a rule the configuration turns off does not block", async () => {
    const repo = await changed("hr-13");
    writeFileSync(join(repo.path, "rust-doctor.toml"), '[rules]\n"clippy::todo" = "off"\n');
    expect(outcome(await hook("claude", repo))).toEqual({ exitCode: 0, stdout: "", stderr: "" });
  });

  test("HR-14 an invalid configuration lets the turn end and says why", async () => {
    const repo = await changed("hr-14");
    writeFileSync(join(repo.path, "rust-doctor.toml"), "nope = 1\n");
    expect(outcome(await hook("claude", repo))).toEqual({ exitCode: 0, stdout: "", stderr: unscanned("configuration: config-invalid") });
  });

  test("HR-15 twelve findings list ten, count the rest, and end with the rescan line", async () => {
    const repo = await many("hr-15");
    const run = await hook("claude", repo);
    expect(run.exitCode).toBe(2);
    const lines = run.stderr.trimEnd().split("\n");
    expect(lines[0]).toBe("rust-doctor found 12 findings in the lines this turn changed:");
    expect(lines.slice(1, 11).every((line) => line.startsWith("- clippy::todo src/many.rs:"))).toBe(true);
    expect(lines.slice(11)).toEqual(["- and 2 more", RESCAN]);
  });

  test("HR-16 a todo!() already committed does not block", async () => {
    const repo = await gitScenario("hr-16").template("mini").append("src/lib.rs", NEW_FN).commit("base").build();
    expect(outcome(await hook("claude", repo))).toEqual({ exitCode: 0, stdout: "", stderr: "" });
  });

  test("HR-17 a repository without a commit has no base, and the turn ends", async () => {
    const repo = await gitScenario("hr-17").template("mini").build();
    expect(outcome(await hook("claude", repo))).toEqual({ exitCode: 0, stdout: "", stderr: unscanned("scope: base-unavailable") });
  });

  test("HR-18 a crate outside git has no base, and the turn ends", async () => {
    const box = sandbox("hr-18");
    const ws = { path: copyWorkspace("projects/clean"), sandbox: box };
    expect(outcome(await hook("claude", ws))).toEqual({ exitCode: 0, stdout: "", stderr: unscanned("scope: base-unavailable") });
  });

  test("HR-19 a CLAUDE_PROJECT_DIR that does not exist lets the turn end", async () => {
    const repo = await changed("hr-19");
    const run = await hook("claude", repo, { env: { CLAUDE_PROJECT_DIR: join(repo.sandbox.dir, "nonexistent") } });
    expect(outcome(run)).toEqual({ exitCode: 0, stdout: "", stderr: unscanned("discovery: no-manifest") });
  });

  test("HR-24 a todo!() inside a test module does not block", async () => {
    const repo = await gitScenario("hr-24").template("mini").commit("base").build();
    const lib = join(repo.path, "src/lib.rs");
    writeFileSync(lib, `${readFileSync(lib, "utf8")}\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn later() {\n        todo!()\n    }\n}\n`);
    expect(outcome(await hook("claude", repo))).toEqual({ exitCode: 0, stdout: "", stderr: "" });
  });

  test("HR-25 a blocking run writes nothing in the tree and keeps the index untouched", async () => {
    const repo = await changed("hr-25");
    const index = join(repo.path, ".git/index");
    const before = readFileSync(index);
    const run = await hook("claude", repo);
    expect(run.exitCode).toBe(2);
    expect(readFileSync(index).equals(before)).toBe(true);
    expect(existsSync(join(repo.path, "target"))).toBe(false);
  });

  test("HR-26 a terminal on stdin is not waited on: the hook scans", async () => {
    const repo = await changed("hr-26");
    const before = treeFingerprint(repo.path);
    const run = await runPty([BIN, "hook", "run", "claude"], {
      sandbox: repo.sandbox,
      cwd: repo.path,
      env: { CLAUDE_PROJECT_DIR: repo.path, CARGO_TARGET_DIR: join(scratchDir("hr-26"), "target") },
      timeoutMs: 60_000,
    });
    expect(run.exitCode).toBe(2);
    expect(run.output).toContain("rust-doctor found 1 finding in the lines this turn changed:");
    expect(treeFingerprint(repo.path)).toBe(before);
  });

  test("HR-27 under CLAUDECODE=1 the hook prints no skill notice", async () => {
    const repo = await changed("hr-27");
    expect(outcome(await hook("claude", repo, { env: { CLAUDECODE: "1" } }))).toEqual({ exitCode: 2, stdout: "", stderr: ONE_FINDING });
  });
});

describe("hook run cursor", () => {
  test("HR-20 a completed turn with findings gets one follow-up message", async () => {
    const repo = await many("hr-20");
    const run = await hook("cursor", repo);
    expect([run.exitCode, run.stderr]).toEqual([0, ""]);
    const lines = run.stdout.split("\n");
    expect(lines).toHaveLength(2);
    expect(lines[1]).toBe("");
    const followup = JSON.parse(lines[0]!).followup_message as string;
    expect(followup).toStartWith("rust-doctor found 12 findings in the lines this turn changed:\n");
    expect(followup).toEndWith(RESCAN);
  });

  test("HR-21 an aborted, errored or already sent-back turn ends before any scan", async () => {
    const repo = await many("hr-21");
    for (const extra of [{ status: "aborted" }, { status: "error" }, { loop_count: 1 }]) {
      const target = join(scratchDir("hr-21"), "target");
      const run = await hook("cursor", repo, { stdin: cursorPayload(repo, extra), target });
      expect([extra, outcome(run)]).toEqual([extra, { exitCode: 0, stdout: "", stderr: "" }]);
      expect(existsSync(target)).toBe(false);
    }
  });

  test("HR-22 an empty object is no guard: the hook scans and sends a follow-up", async () => {
    const repo = await many("hr-22");
    const run = await hook("cursor", repo, { stdin: "{}" });
    expect([run.exitCode, run.stderr]).toEqual([0, ""]);
    expect(JSON.parse(run.stdout).followup_message).toStartWith("rust-doctor found 12 findings");
  });

  test("HR-23 CLAUDE_PROJECT_DIR is ignored: cursor scans its working directory", async () => {
    const repo = await many("hr-23");
    const run = await hook("cursor", repo, { env: { CLAUDE_PROJECT_DIR: "/" } });
    expect([run.exitCode, run.stderr]).toEqual([0, ""]);
    expect(JSON.parse(run.stdout).followup_message).toStartWith("rust-doctor found 12 findings");
  });
});
