// `tasks/e2e-catalog/real-codebases.md` section 2, invariant 7: the catalog
// the binary publishes is closed. Every catalogued finding a scan of this
// repository carries is a rule `rules list` names, the list agrees with the
// measured record, and `rules explain` answers for every rule it lists.
import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { REPO_ROOT, runCommand } from "./e2e-helpers";
import { sharedClone } from "./self-clone";
import { report, scanSelf, TEST_TIMEOUT_MS } from "./self-scan-support";

const PREFIXES = ["clippy::", "rust_doctor::source::", "rust_doctor::cargo::", "rust_doctor::structure::", "rust_doctor::repo::"];

describe("the published catalog", () => {
  test("INV-07 every catalogued finding is a listed rule, the list matches the record, and rules explain answers for each", async () => {
    const listed = await runCommand(["rust-doctor", "rules", "list", "--json"]);
    expect(listed.exitCode).toBe(0);
    const ids = (JSON.parse(listed.stdout) as Array<{ id: string }>).map((rule) => rule.id);
    expect(new Set(ids).size).toBe(ids.length);
    expect(ids.filter((id) => !PREFIXES.some((prefix) => id.startsWith(prefix)))).toEqual([]);
    const record = JSON.parse(readFileSync(join(REPO_ROOT, "tests/corpus.json"), "utf8")) as { catalog: unknown[] };
    expect(ids.length).toBe(record.catalog.length);

    const clone = await sharedClone();
    const scan = report(await scanSelf(clone, ["--json"], { target: clone.target("scan") }));
    const known = new Set(ids);
    const strays = scan.diagnostics.filter((diagnostic) => diagnostic.category !== null && !known.has(diagnostic.code));
    expect(strays.map((diagnostic) => diagnostic.code)).toEqual([]);

    const failures: string[] = [];
    for (let start = 0; start < ids.length; start += 16) {
      const batch = ids.slice(start, start + 16);
      const runs = await Promise.all(batch.map((id) => runCommand(["rust-doctor", "rules", "explain", id])));
      runs.forEach((run, index) => {
        if (run.exitCode !== 0 || !run.stdout.startsWith(`${batch[index]}\n`)) failures.push(`${batch[index]}: exit ${run.exitCode}`);
      });
    }
    expect(failures).toEqual([]);
    const unknown = await runCommand(["rust-doctor", "rules", "explain", "clippy::no_such_rule_e2e"]);
    expect(unknown.exitCode).toBe(2);
    expect(unknown.stdout).toBe("");
  }, TEST_TIMEOUT_MS);
});
