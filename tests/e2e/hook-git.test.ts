// `tasks/e2e-catalog/subcommands.md` section 5: `hook install git`. EP-005
// fills it; the row here is the open git-hook installer defect of the ledger.
import { describe, expect, test } from "bun:test";
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { runDoctor } from "./e2e-helpers";
import { gitScenario } from "./git-scenario";

describe("hook install git", () => {
  test.failing("HG-12 subcommands/F15 --update over a hand-edited hook keeps the user's lines", async () => {
    const repo = await gitScenario("hg-12").fixture("projects/clean").write({ ".gitignore": "/target\n" }).commit("base").build();
    expect((await runDoctor(["hook", "install", "git", repo.path], { sandbox: repo.sandbox })).exitCode).toBe(0);
    const hook = join(repo.path, ".git/hooks/pre-commit");
    writeFileSync(hook, `${readFileSync(hook, "utf8")}make lint\n`);
    const update = await runDoctor(["hook", "install", "git", "--update", repo.path], { sandbox: repo.sandbox });
    expect(update.exitCode).toBe(0);
    expect(readFileSync(hook, "utf8")).toContain("make lint\n");
  });
});
