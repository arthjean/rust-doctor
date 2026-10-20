// HA-01 to HA-32 of `tasks/e2e-catalog/subcommands.md` section 6: `hook
// install claude|cursor` merging one entry into an agent's settings file.
// Every run has HOME at the sandbox's scratch home, which stays empty: the
// installers write only under the workspace. US-007 made a missing PATH a
// refusal, so HA-29 asserts that rather than the catalog's 0.7.0 behavior.
import { describe, expect, test } from "bun:test";
import { chmodSync, existsSync, mkdirSync, readFileSync, symlinkSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import {
  copyWorkspace,
  isEmptyDir,
  pathWithout,
  runDoctor,
  sandbox,
  scratchDir,
  treeFingerprint,
  type Sandbox,
} from "./e2e-helpers";
import { gitScenario } from "./git-scenario";

type Workspace = { path: string; sandbox: Sandbox };

const CLAUDE = ".claude/settings.local.json";
const SHARED = ".claude/settings.json";
const CURSOR = ".cursor/hooks.json";
const CLAUDE_HOOK = { command: "rust-doctor hook run claude", timeout: 600, type: "command" };
const CURSOR_HOOK = { command: "rust-doctor hook run cursor" };
const CLAUDE_ENTRY = JSON.stringify({ hooks: [CLAUDE_HOOK] });
const SHAPE = (file: string) => `rust-doctor: ${file} does not hold the shape its agent reads: nothing was written.\n`;

function workspace(label: string): Workspace {
  const box = sandbox(label);
  return { path: copyWorkspace("projects/clean", join(box.dir, "ws")), sandbox: box };
}

function install(ws: Workspace, agent: "claude" | "cursor", flags: string[] = [], options: { path?: string; cwd?: string; env?: Record<string, string> } = {}) {
  return runDoctor(["hook", "install", agent, ...flags, options.path ?? ws.path], { sandbox: ws.sandbox, cwd: options.cwd, env: options.env });
}

function seed(ws: Workspace, file: string, body: unknown): string {
  const path = join(ws.path, file);
  mkdirSync(join(path, ".."), { recursive: true });
  writeFileSync(path, typeof body === "string" ? body : `${JSON.stringify(body, null, 2)}\n`);
  return path;
}

const read = (ws: Workspace, file: string) => JSON.parse(readFileSync(join(ws.path, file), "utf8"));

describe("hook install claude", () => {
  test("HA-01 --dry-run prints the entry and creates no .claude", async () => {
    const ws = workspace("ha-01");
    const before = treeFingerprint(ws.path);
    const run = await install(ws, "claude", ["--dry-run"]);
    expect(run.exitCode).toBe(0);
    expect(run.stdout).toBe(`Would add a Stop hook to ${CLAUDE}: ${CLAUDE_ENTRY}\n`);
    expect(existsSync(join(ws.path, ".claude"))).toBe(false);
    expect(treeFingerprint(ws.path)).toBe(before);
  });

  test("HA-02 install writes pretty JSON with a trailing newline and nothing under HOME", async () => {
    const ws = workspace("ha-02");
    const run = await install(ws, "claude");
    expect(run.exitCode).toBe(0);
    expect(run.stderr).toBe("");
    expect(run.stdout).toBe(`Added a Stop hook to ${CLAUDE}: ${CLAUDE_ENTRY}\n`);
    const expected = { hooks: { Stop: [{ hooks: [CLAUDE_HOOK] }] } };
    expect(readFileSync(join(ws.path, CLAUDE), "utf8")).toBe(`${JSON.stringify(expected, null, 2)}\n`);
    expect(isEmptyDir(ws.sandbox.home)).toBe(true);
  });

  test("HA-03 a second install says the hook is already there and keeps the bytes", async () => {
    const ws = workspace("ha-03");
    expect((await install(ws, "claude")).exitCode).toBe(0);
    const before = treeFingerprint(ws.path);
    const run = await install(ws, "claude");
    expect(run.exitCode).toBe(0);
    expect(run.stdout).toBe(`The rust-doctor Stop hook is already in ${CLAUDE}.\n`);
    expect(treeFingerprint(ws.path)).toBe(before);
  });

  test("HA-04 --update over the current hook says it is already there", async () => {
    const ws = workspace("ha-04");
    expect((await install(ws, "claude")).exitCode).toBe(0);
    const before = treeFingerprint(ws.path);
    const run = await install(ws, "claude", ["--update"]);
    expect(run.exitCode).toBe(0);
    expect(run.stdout).toBe(`The rust-doctor Stop hook is already in ${CLAUDE}.\n`);
    expect(treeFingerprint(ws.path)).toBe(before);
  });

  test("HA-05 --shared writes .claude/settings.json beside the local file", async () => {
    const ws = workspace("ha-05");
    expect((await install(ws, "claude")).exitCode).toBe(0);
    const run = await install(ws, "claude", ["--shared"]);
    expect(run.exitCode).toBe(0);
    expect(run.stdout).toBe(`Added a Stop hook to ${SHARED}: ${CLAUDE_ENTRY}\n`);
    expect(read(ws, SHARED)).toEqual(read(ws, CLAUDE));
    const again = await install(ws, "claude", ["--shared"]);
    expect([again.exitCode, again.stdout]).toEqual([0, `The rust-doctor Stop hook is already in ${SHARED}.\n`]);
  });

  test("HA-06 extra keys on the hook and the file leave it current", async () => {
    const ws = workspace("ha-06");
    seed(ws, CLAUDE, { model: "opus", hooks: { Stop: [{ hooks: [{ ...CLAUDE_HOOK, statusMessage: "scanning" }] }] } });
    const before = treeFingerprint(ws.path);
    const run = await install(ws, "claude");
    expect(run.exitCode).toBe(0);
    expect(run.stdout).toBe(`The rust-doctor Stop hook is already in ${CLAUDE}.\n`);
    expect(treeFingerprint(ws.path)).toBe(before);
  });

  const stale = { model: "opus", hooks: { Stop: [{ hooks: [{ ...CLAUDE_HOOK, timeout: 60, statusMessage: "scanning" }] }] } };

  test("HA-07 a hook with another timeout is outdated without --update", async () => {
    const ws = workspace("ha-07");
    seed(ws, CLAUDE, stale);
    const before = treeFingerprint(ws.path);
    const run = await install(ws, "claude");
    expect(run.exitCode).toBe(1);
    expect(run.stdout).toBe(`The rust-doctor Stop hook in ${CLAUDE} is outdated: pass --update to rewrite it.\n`);
    expect(treeFingerprint(ws.path)).toBe(before);
  });

  test("HA-08 --update --dry-run names the old and new hook and writes nothing", async () => {
    const ws = workspace("ha-08");
    seed(ws, CLAUDE, stale);
    const before = treeFingerprint(ws.path);
    const run = await install(ws, "claude", ["--update", "--dry-run"]);
    expect(run.exitCode).toBe(0);
    const old = JSON.stringify({ command: CLAUDE_HOOK.command, statusMessage: "scanning", timeout: 60, type: "command" });
    const next = JSON.stringify({ command: CLAUDE_HOOK.command, statusMessage: "scanning", timeout: 600, type: "command" });
    expect(run.stdout).toBe(`Would update the Stop hook in ${CLAUDE}: ${old} becomes ${next}\n`);
    expect(treeFingerprint(ws.path)).toBe(before);
  });

  test("HA-09 --update rewrites the hook and keeps the user's keys", async () => {
    const ws = workspace("ha-09");
    seed(ws, CLAUDE, stale);
    const run = await install(ws, "claude", ["--update"]);
    expect(run.exitCode).toBe(0);
    expect(run.stdout).toStartWith(`Updated the Stop hook in ${CLAUDE}: `);
    const settings = read(ws, CLAUDE);
    expect(settings.model).toBe("opus");
    expect(settings.hooks.Stop).toEqual([{ hooks: [{ ...CLAUDE_HOOK, statusMessage: "scanning" }] }]);
  });

  test("HA-10 an edited command is outdated, and --update resets it", async () => {
    const ws = workspace("ha-10");
    seed(ws, CLAUDE, { hooks: { Stop: [{ hooks: [{ ...CLAUDE_HOOK, command: `${CLAUDE_HOOK.command} --verbose` }] }] } });
    const refused = await install(ws, "claude");
    expect(refused.exitCode).toBe(1);
    const run = await install(ws, "claude", ["--update"]);
    expect(run.exitCode).toBe(0);
    expect(read(ws, CLAUDE).hooks.Stop).toEqual([{ hooks: [CLAUDE_HOOK] }]);
  });

  test("HA-11 a foreign Stop group is kept and ours is appended", async () => {
    const ws = workspace("ha-11");
    const foreign = { hooks: [{ type: "command", command: "echo hi" }] };
    seed(ws, CLAUDE, { hooks: { Stop: [foreign] } });
    const run = await install(ws, "claude");
    expect(run.exitCode).toBe(0);
    expect(read(ws, CLAUDE).hooks.Stop).toEqual([foreign, { hooks: [CLAUDE_HOOK] }]);
  });

  test.each(['[]', '{"hooks":[]}', '{"hooks":{"Stop":{}}}', '{"hooks":{"Stop":"x"}}', '"str"', "null"])(
    "HA-12 settings %s are refused as the wrong shape",
    async (body) => {
      const ws = workspace("ha-12");
      seed(ws, CLAUDE, body);
      const before = treeFingerprint(ws.path);
      const run = await install(ws, "claude");
      expect(run.exitCode).toBe(2);
      expect(run.stderr).toBe(SHAPE(CLAUDE));
      expect(treeFingerprint(ws.path)).toBe(before);
    },
  );

  test.each([
    ["HA-13", "a syntax error", '{"hooks": {,}}', 1, 12],
    ["HA-14", "an empty file", "", 1, 0],
    ["HA-15", "a comment", "// comment\n{}", 1, 1],
  ] as const)("%s %s is refused with its position", async (_id, _label, body, line, column) => {
    const ws = workspace("ha-parse");
    seed(ws, CLAUDE, body);
    const before = treeFingerprint(ws.path);
    const run = await install(ws, "claude");
    expect(run.exitCode).toBe(2);
    expect(run.stderr).toBe(`Could not parse ${CLAUDE} at line ${line}, column ${column}: nothing was written.\n`);
    expect(treeFingerprint(ws.path)).toBe(before);
  });

  test("HA-16 a duplicate key is accepted and the last value kept", async () => {
    const ws = workspace("ha-16");
    seed(ws, CLAUDE, '{"a":1,"a":2}');
    const run = await install(ws, "claude");
    expect(run.exitCode).toBe(0);
    const text = readFileSync(join(ws.path, CLAUDE), "utf8");
    expect(text).toContain('"a": 2');
    expect(text).not.toContain('"a": 1');
  });

  test.failing("HA-17 subcommands/F16 merging keeps the settings file's key order and indentation", async () => {
    const repo = await gitScenario("ha-17").fixture("projects/clean").write({ ".gitignore": "/target\n" }).commit("base").build();
    const settings = join(repo.path, ".claude/settings.local.json");
    mkdirSync(join(repo.path, ".claude"));
    writeFileSync(settings, '{\n    "zeta": 1,\n    "alpha": 2\n}\n');
    const run = await runDoctor(["hook", "install", "claude", repo.path], { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(0);
    const text = readFileSync(settings, "utf8");
    expect(JSON.parse(text).hooks.Stop).toHaveLength(1);
    expect(text.indexOf('"zeta"')).toBeLessThan(text.indexOf('"alpha"'));
    expect(text).toStartWith('{\n    "zeta": 1');
  });

  test("HA-18 a settings path that is a directory is refused as unreadable", async () => {
    const ws = workspace("ha-18");
    mkdirSync(join(ws.path, CLAUDE), { recursive: true });
    const before = treeFingerprint(ws.path);
    const run = await install(ws, "claude");
    expect(run.exitCode).toBe(2);
    expect(run.stderr).toBe(`rust-doctor: ${CLAUDE} could not be read: Is a directory (os error 21). Nothing was written.\n`);
    expect(treeFingerprint(ws.path)).toBe(before);
  });

  test("HA-19 an unreadable settings file is refused", async () => {
    const ws = workspace("ha-19");
    const settings = seed(ws, CLAUDE, {});
    chmodSync(settings, 0o000);
    try {
      const run = await install(ws, "claude");
      expect(run.exitCode).toBe(2);
      expect(run.stderr).toBe(`rust-doctor: ${CLAUDE} could not be read: Permission denied (os error 13). Nothing was written.\n`);
    } finally {
      chmodSync(settings, 0o644);
    }
    expect(readFileSync(settings, "utf8")).toBe("{}\n");
  });

  test.failing("HA-20 subcommands/F7 a write that fails prints no success line", async () => {
    const repo = await gitScenario("ha-20").fixture("projects/clean").write({ ".gitignore": "/target\n" }).commit("base").build();
    mkdirSync(join(repo.path, ".claude"));
    chmodSync(join(repo.path, ".claude"), 0o555);
    try {
      const run = await runDoctor(["hook", "install", "claude", repo.path], { sandbox: repo.sandbox });
      expect(run.exitCode).not.toBe(0);
      expect(run.stderr).toContain("could not be written");
      expect(run.stdout).not.toContain("Added a Stop hook");
    } finally {
      chmodSync(join(repo.path, ".claude"), 0o755);
    }
  });

  test("HA-21 a settings file symlinked outside is refused and its target kept", async () => {
    const ws = workspace("ha-21");
    const outside = join(scratchDir("ha-21-outside"), "settings.json");
    writeFileSync(outside, "{}\n");
    mkdirSync(join(ws.path, ".claude"));
    symlinkSync(outside, join(ws.path, CLAUDE));
    const before = treeFingerprint(ws.path);
    const run = await install(ws, "claude");
    expect(run.exitCode).toBe(2);
    expect(run.stderr).toBe(`rust-doctor: ${CLAUDE} is a symlink, and nothing is written through one.\n`);
    expect(readFileSync(outside, "utf8")).toBe("{}\n");
    expect(treeFingerprint(ws.path)).toBe(before);
  });

  test("HA-22 a symlinked .claude directory is refused", async () => {
    const ws = workspace("ha-22");
    const outside = scratchDir("ha-22-outside");
    symlinkSync(outside, join(ws.path, ".claude"));
    const before = treeFingerprint(ws.path);
    const run = await install(ws, "claude");
    expect(run.exitCode).toBe(2);
    expect(run.stderr).toBe("rust-doctor: .claude is a symlink, and nothing is written through one.\n");
    expect(isEmptyDir(outside)).toBe(true);
    expect(treeFingerprint(ws.path)).toBe(before);
  });
});

describe("hook install cursor", () => {
  const entry = JSON.stringify(CURSOR_HOOK);

  test("HA-23 --dry-run then install add the stop hook with version 1", async () => {
    const ws = workspace("ha-23");
    const before = treeFingerprint(ws.path);
    const dry = await install(ws, "cursor", ["--dry-run"]);
    expect([dry.exitCode, dry.stdout]).toEqual([0, `Would add a stop hook to ${CURSOR}: ${entry}\n`]);
    expect(treeFingerprint(ws.path)).toBe(before);
    const run = await install(ws, "cursor");
    expect([run.exitCode, run.stdout]).toEqual([0, `Added a stop hook to ${CURSOR}: ${entry}\n`]);
    expect(readFileSync(join(ws.path, CURSOR), "utf8")).toBe(`${JSON.stringify({ hooks: { stop: [CURSOR_HOOK] }, version: 1 }, null, 2)}\n`);
  });

  test("HA-24 a second install says the stop hook is already there", async () => {
    const ws = workspace("ha-24");
    expect((await install(ws, "cursor")).exitCode).toBe(0);
    const before = treeFingerprint(ws.path);
    const run = await install(ws, "cursor");
    expect([run.exitCode, run.stdout]).toEqual([0, `The rust-doctor stop hook is already in ${CURSOR}.\n`]);
    expect(treeFingerprint(ws.path)).toBe(before);
  });

  test("HA-25 an entry with an extra timeout stays current", async () => {
    const ws = workspace("ha-25");
    seed(ws, CURSOR, { version: 1, hooks: { stop: [{ ...CURSOR_HOOK, timeout: 5 }] } });
    const before = treeFingerprint(ws.path);
    const run = await install(ws, "cursor");
    expect([run.exitCode, run.stdout]).toEqual([0, `The rust-doctor stop hook is already in ${CURSOR}.\n`]);
    expect(treeFingerprint(ws.path)).toBe(before);
  });

  test("HA-26 a version 2 file keeps its version and other events", async () => {
    const ws = workspace("ha-26");
    seed(ws, CURSOR, { version: 2, hooks: { afterFileEdit: [{ command: "x" }] } });
    const run = await install(ws, "cursor");
    expect(run.exitCode).toBe(0);
    expect(read(ws, CURSOR)).toEqual({ version: 2, hooks: { afterFileEdit: [{ command: "x" }], stop: [CURSOR_HOOK] } });
  });

  test("HA-27 an edited cursor command is outdated, and --update resets it", async () => {
    const ws = workspace("ha-27");
    seed(ws, CURSOR, { version: 1, hooks: { stop: [{ command: `${CURSOR_HOOK.command} --old` }] } });
    const refused = await install(ws, "cursor");
    expect([refused.exitCode, refused.stdout]).toEqual([1, `The rust-doctor stop hook in ${CURSOR} is outdated: pass --update to rewrite it.\n`]);
    const run = await install(ws, "cursor", ["--update"]);
    expect(run.exitCode).toBe(0);
    expect(run.stdout).toBe(`Updated the stop hook in ${CURSOR}: ${JSON.stringify({ command: `${CURSOR_HOOK.command} --old` })} becomes ${entry}\n`);
    expect(read(ws, CURSOR).hooks.stop).toEqual([CURSOR_HOOK]);
  });

  test("HA-28 --shared is not a cursor flag", async () => {
    const ws = workspace("ha-28");
    const before = treeFingerprint(ws.path);
    const run = await install(ws, "cursor", ["--shared"]);
    expect(run.exitCode).toBe(2);
    expect(run.stderr).toContain("unexpected argument '--shared'");
    expect(treeFingerprint(ws.path)).toBe(before);
  });
});

describe("hook install paths", () => {
  test.each(["claude", "cursor"] as const)("HA-29 %s on a path that does not exist is refused and creates nothing", async (agent) => {
    const box = sandbox("ha-29");
    const missing = join(box.dir, "nonexistent/does/not/exist");
    const before = treeFingerprint(box.dir);
    const run = await runDoctor(["hook", "install", agent, missing], { sandbox: box });
    expect(run.exitCode).toBe(2);
    expect(run.stderr).toBe(`rust-doctor: PATH: ${missing} does not exist.\n`);
    expect(existsSync(join(box.dir, "nonexistent"))).toBe(false);
    expect(treeFingerprint(box.dir)).toBe(before);
  });

  test.each([
    ["claude", CLAUDE],
    ["cursor", CURSOR],
  ] as const)("HA-30 %s without rust-doctor on PATH writes the file and warns", async (agent, file) => {
    const ws = workspace("ha-30");
    const run = await install(ws, agent, [], { env: { PATH: pathWithout("rust-doctor") } });
    expect(run.exitCode).toBe(0);
    expect(existsSync(join(ws.path, file))).toBe(true);
    expect(run.stderr).toBe("Warning: `rust-doctor` is not on PATH, and the hook runs it by name.\n");
  });

  test("HA-31 a relative nested path installs into that member", async () => {
    const repo = await gitScenario("ha-31").nested("crates/x", "mini").commit("base").build();
    const run = await runDoctor(["hook", "install", "claude", "crates/x"], { sandbox: repo.sandbox, cwd: repo.path });
    expect(run.exitCode).toBe(0);
    expect(existsSync(join(repo.path, "crates/x", CLAUDE))).toBe(true);
    expect(existsSync(join(repo.path, ".claude"))).toBe(false);
  });

  test("HA-32 every refusal leaves the tree unchanged", async () => {
    const refusals: Array<[string, "claude" | "cursor", (ws: Workspace) => void, string[], number]> = [
      ["outdated", "claude", (ws) => void seed(ws, CLAUDE, outdated()), [], 1],
      ["outdated --update --dry-run", "claude", (ws) => void seed(ws, CLAUDE, outdated()), ["--update", "--dry-run"], 0],
      ["wrong shape", "claude", (ws) => void seed(ws, CLAUDE, "[]"), [], 2],
      ["unparsable", "claude", (ws) => void seed(ws, CLAUDE, "{,}"), [], 2],
      ["directory", "claude", (ws) => mkdirSync(join(ws.path, CLAUDE), { recursive: true }), [], 2],
      ["symlinked settings", "claude", (ws) => {
        mkdirSync(join(ws.path, ".claude"));
        symlinkSync(join(ws.sandbox.dir, "elsewhere.json"), join(ws.path, CLAUDE));
      }, [], 2],
      ["symlinked .claude", "claude", (ws) => symlinkSync(ws.sandbox.tmp, join(ws.path, ".claude")), [], 2],
      ["cursor outdated", "cursor", (ws) => void seed(ws, CURSOR, { version: 1, hooks: { stop: [{ command: "rust-doctor hook run cursor --old" }] } }), [], 1],
      ["cursor wrong shape", "cursor", (ws) => void seed(ws, CURSOR, '{"hooks":{"stop":{}}}'), [], 2],
      ["cursor --shared", "cursor", () => {}, ["--shared"], 2],
    ];
    for (const [label, agent, setup, flags, code] of refusals) {
      const ws = workspace(`ha-32-${label.replace(/\W+/g, "-")}`);
      setup(ws);
      const before = treeFingerprint(ws.path);
      const run = await install(ws, agent, flags);
      expect([label, run.exitCode, treeFingerprint(ws.path)]).toEqual([label, code, before]);
    }
  });
});

function outdated() {
  return { hooks: { Stop: [{ hooks: [{ ...CLAUDE_HOOK, timeout: 60 }] }] } };
}
