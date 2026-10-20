// `tasks/e2e-catalog/failures.md` sections EX and AR: the exit status matrix
// and argument validation. EP-005 fills it; the row here is the open
// argument-error defect of the ledger.
import { describe, expect, test } from "bun:test";
import { copyWorkspace, runDoctor } from "./e2e-helpers";

describe("argument validation", () => {
  test.failing("AR-06 failures/BUG-10 a malformed --rule reads like every other argument error", async () => {
    const workspace = copyWorkspace("kernel-contract/todo");
    const clap = await runDoctor([workspace, "--bogus"]);
    expect(clap.stderr).toEndWith("For more information, try '--help'.\n");
    const run = await runDoctor([workspace, "--yes", "--rule", "nonsense"]);
    expect(run.exitCode).toBe(2);
    expect(run.stdout).toBe("");
    expect(run.stderr).toStartWith("error: expected KEY=LEVEL");
    expect(run.stderr).toEndWith("For more information, try '--help'.\n");
  });
});
