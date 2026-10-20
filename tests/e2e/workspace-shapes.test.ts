// `tasks/e2e-catalog/failures.md` section WS: workspace shapes and
// filesystem conditions. EP-006 fills it; the row here is the open
// missing-git defect of the ledger.
import { describe, expect, test } from "bun:test";
import { copyWorkspace, pathWithout, runDoctor } from "./e2e-helpers";

type Report = { status: string };

describe("workspace shapes", () => {
  test.failing("WS-34 failures/BUG-08 a workspace outside git scans complete without git on PATH", async () => {
    const workspace = copyWorkspace("kernel-contract/todo");
    expect(((await runDoctor([workspace, "--yes", "--json"])).json as Report).status).toBe("complete");
    const run = await runDoctor([workspace, "--yes", "--json"], { env: { PATH: pathWithout("git") } });
    expect((run.json as Report).status).toBe("complete");
    expect(run.exitCode).toBe(0);
  });
});
