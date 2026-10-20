// `tasks/e2e-catalog/subcommands.md` section 7: `hook run claude|cursor`.
// EP-005 fills it; the row here is the open Stop-hook defect of the ledger.
import { describe, expect, test } from "bun:test";
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { runDoctor } from "./e2e-helpers";
import { gitScenario } from "./git-scenario";

describe("hook run claude", () => {
  test.failing("HR-11 subcommands/F17 a Stop payload over 1 MiB keeps the stop_hook_active loop guard", async () => {
    const repo = await gitScenario("hr-11").template("mini").commit("base").build();
    const lib = join(repo.path, "src/lib.rs");
    writeFileSync(lib, `${readFileSync(lib, "utf8")}\npub fn turn() -> u8 {\n    todo!()\n}\n`);
    const env = { CLAUDE_PROJECT_DIR: repo.path };
    const guarded = JSON.stringify({ stop_hook_active: true });
    const small = await runDoctor(["hook", "run", "claude"], { sandbox: repo.sandbox, cwd: repo.path, env, stdin: guarded });
    expect([small.exitCode, small.stderr]).toEqual([0, ""]);
    const large = JSON.stringify({ stop_hook_active: true, pad: "x".repeat(1100 * 1024) });
    const run = await runDoctor(["hook", "run", "claude"], { sandbox: repo.sandbox, cwd: repo.path, env, stdin: large });
    expect([run.exitCode, run.stderr]).toEqual([0, ""]);
  });
});
