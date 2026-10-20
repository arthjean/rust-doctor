// `tasks/e2e-catalog/subcommands.md` section 8: `skill install`. EP-005 fills
// it; the row here is the open installer exit-code defect of the ledger.
import { describe, expect, test } from "bun:test";
import { chmodSync, mkdirSync } from "node:fs";
import { join } from "node:path";
import { runDoctor } from "./e2e-helpers";
import { gitScenario } from "./git-scenario";

describe("skill install", () => {
  test.failing("SK-24 subcommands/F8 a write failure exits as the hook and ci installers do", async () => {
    const repo = await gitScenario("sk-24").fixture("projects/clean").write({ ".gitignore": "/target\n" }).commit("base").build();
    const skills = join(repo.path, ".claude/skills");
    const workflows = join(repo.path, ".github/workflows");
    mkdirSync(skills, { recursive: true });
    mkdirSync(workflows, { recursive: true });
    chmodSync(skills, 0o555);
    chmodSync(workflows, 0o555);
    try {
      const skill = await runDoctor(["skill", "install", repo.path], { sandbox: repo.sandbox });
      const ci = await runDoctor(["ci", "install", repo.path], { sandbox: repo.sandbox });
      chmodSync(skills, 0o755);
      chmodSync(join(repo.path, ".claude"), 0o555);
      const hook = await runDoctor(["hook", "install", "claude", repo.path], { sandbox: repo.sandbox });
      expect([ci.exitCode, hook.exitCode]).toEqual([2, 2]);
      expect(skill.stderr).toContain("the skill was not installed");
      expect(skill.exitCode).toBe(2);
    } finally {
      chmodSync(join(repo.path, ".claude"), 0o755);
      chmodSync(skills, 0o755);
      chmodSync(workflows, 0o755);
    }
  });
});
