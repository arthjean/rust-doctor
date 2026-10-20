// `tasks/e2e-catalog/subcommands.md` section 6: `hook install claude|cursor`.
// EP-005 fills it; the rows here are the open agent-installer defects of the
// ledger.
import { describe, expect, test } from "bun:test";
import { chmodSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { runDoctor } from "./e2e-helpers";
import { gitScenario } from "./git-scenario";

const clean = (name: string) => gitScenario(name).fixture("projects/clean").write({ ".gitignore": "/target\n" }).commit("base").build();

describe("hook install claude", () => {
  test.failing("HA-20 subcommands/F7 a write that fails prints no success line", async () => {
    const repo = await clean("ha-20");
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

  test.failing("HA-17 subcommands/F16 merging keeps the settings file's key order and indentation", async () => {
    const repo = await clean("ha-17");
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
});
