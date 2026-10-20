// `tasks/e2e-catalog/scopes.md` section E: what a baseline comparison calls
// new. EP-005 fills it; the rows here are the open baseline defects of the
// ledger.
import { describe, expect, test } from "bun:test";
import { runDoctor } from "./e2e-helpers";
import { gitScenario, NEW_FN } from "./git-scenario";

type Report = { status: string; errors: Array<{ stage: string; code: string }>; delta: { summary: { introduced: number } } | null };

describe("baseline comparison", () => {
  test.failing("E06 scopes/BUG-E a submodule in the base tree does not refuse the baseline", async () => {
    const sub = await gitScenario("e06-sub").template("mini").commit("base").build();
    const repo = await gitScenario("e06")
      .template("mini").submodule("vendor/sub", sub).commit("base")
      .branch("feature").append("src/lib.rs", NEW_FN).commit("feature")
      .build();
    const run = await runDoctor([repo.path, "--yes", "--json", "--scope", "baseline", "--base", "main"], { sandbox: repo.sandbox });
    const report = run.json as Report;
    expect(report.errors).toEqual([]);
    expect(report.delta?.summary.introduced).toBe(1);
  });

  test.failing("E07 scopes/BUG-F a relative symlink through .. that stays inside the repository is accepted", async () => {
    const repo = await gitScenario("e07")
      .template("mini")
      .write({ "shared/s.rs": "pub fn s() -> u8 {\n    2\n}\n" })
      .symlink("src/s.rs", "../shared/s.rs")
      .append("src/lib.rs", "pub mod s;\n")
      .commit("base")
      .branch("feature").append("src/lib.rs", NEW_FN).commit("feature")
      .build();
    const run = await runDoctor([repo.path, "--yes", "--json", "--scope", "baseline", "--base", "main"], { sandbox: repo.sandbox });
    const report = run.json as Report;
    expect(report.errors).toEqual([]);
    expect(report.delta?.summary.introduced).toBe(1);
  });
});
