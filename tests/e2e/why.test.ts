// `tasks/e2e-catalog/subcommands.md` section 3: `why`. EP-005 fills it; the
// rows here are the open explainer defects of the ledger.
import { describe, expect, test } from "bun:test";
import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { runDoctor } from "./e2e-helpers";
import { gitScenario } from "./git-scenario";

describe("why", () => {
  test.failing("WY-27 subcommands/F13 a saved report answers as the live scan does on a declared-generated file", async () => {
    const repo = await gitScenario("wy-27")
      .fixture("projects/clean")
      .write({
        ".gitignore": "/target\n",
        ".gitattributes": "src/out/** linguist-generated\n",
        "src/out/mod.rs": "pub fn generated() -> u8 {\n    todo!()\n}\n",
      })
      .append("src/lib.rs", "pub mod out;\n")
      .commit("base")
      .build();
    const report = join(repo.sandbox.dir, "report.json");
    writeFileSync(report, (await runDoctor([repo.path, "--yes", "--json"], { sandbox: repo.sandbox })).stdout);
    const live = await runDoctor(["why", "src/out/mod.rs:2"], { sandbox: repo.sandbox, cwd: repo.path });
    const saved = await runDoctor(["why", "src/out/mod.rs:2", "--report", report], { sandbox: repo.sandbox, cwd: repo.path });
    expect(live.stdout).toContain("declared generated or vendored");
    expect(saved.stdout).toBe(live.stdout);
  });

  test.failing("WY-29 subcommands/F14 a line an #[expect] silences names the attribute", async () => {
    const repo = await gitScenario("wy-29")
      .fixture("projects/clean")
      .write({ ".gitignore": "/target\n" })
      .append("src/lib.rs", '\n#[expect(clippy::todo, reason = "staged rollout")]\npub fn later() -> u8 {\n    todo!()\n}\n')
      .commit("base")
      .build();
    const run = await runDoctor(["why", "src/lib.rs:9"], { sandbox: repo.sandbox, cwd: repo.path });
    expect(run.exitCode).toBe(0);
    expect(run.stdout).toContain("expect");
  });
});
