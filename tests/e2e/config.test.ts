// `tasks/e2e-catalog/subcommands.md` section 11: `rust-doctor.toml`. EP-005
// fills it; the row here is the open rule-name defect of the ledger.
import { describe, expect, test } from "bun:test";
import { copyWorkspace, runDoctor } from "./e2e-helpers";

describe("rule names", () => {
  test.failing("CF-18 subcommands/F19 --rule takes the suffix rules explain takes", async () => {
    const workspace = copyWorkspace("kernel-contract/todo");
    expect((await runDoctor(["rules", "explain", "todo", workspace])).exitCode).toBe(0);
    const run = await runDoctor([workspace, "--yes", "--json", "--rule", "todo=off"]);
    expect(run.exitCode).toBe(0);
    const report = run.json as { diagnostics: Array<{ code: string }> };
    expect(report.diagnostics.map((diagnostic) => diagnostic.code)).not.toContain("clippy::todo");
  });
});
