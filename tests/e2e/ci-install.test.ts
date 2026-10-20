// `tasks/e2e-catalog/subcommands.md` section 10: `ci install`. EP-005 fills
// it; the rows here are the open workflow-installer defects of the ledger.
import { describe, expect, test } from "bun:test";
import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { runDoctor } from "./e2e-helpers";
import { gitScenario } from "./git-scenario";

const clean = (name: string) => gitScenario(name).fixture("projects/clean").write({ ".gitignore": "/target\n" }).commit("base").build();

describe("ci install", () => {
  test.failing("CI-05 subcommands/F11 a rerun over the workflow it wrote exits 0, as the hook installers do", async () => {
    const repo = await clean("ci-05");
    const first = await runDoctor(["ci", "install", repo.path], { sandbox: repo.sandbox });
    expect(first.stdout).toBe("Wrote .github/workflows/rust-doctor.yml\n");
    const rerun = await runDoctor(["ci", "install", repo.path], { sandbox: repo.sandbox });
    expect(rerun.exitCode).toBe(0);
  });

  test.failing("CI-18 subcommands/F18 a leftover staging file does not fail every later --update", async () => {
    const repo = await clean("ci-18");
    expect((await runDoctor(["ci", "install", repo.path], { sandbox: repo.sandbox })).exitCode).toBe(0);
    writeFileSync(join(repo.path, ".github/workflows/rust-doctor.yml.rust-doctor"), "leftover\n");
    const update = await runDoctor(["ci", "install", "--update", repo.path], { sandbox: repo.sandbox });
    expect(update.exitCode).toBe(0);
  });
});
