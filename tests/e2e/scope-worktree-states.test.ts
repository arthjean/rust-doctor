// `tasks/e2e-catalog/scopes.md` section D: working-tree states under the
// files, lines and contextual scopes. EP-005 fills it; the rows here are the
// open working-tree defects of the ledger.
import { describe, expect, test } from "bun:test";
import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { runDoctor } from "./e2e-helpers";
import { gitScenario } from "./git-scenario";

type Report = { status: string; diagnostics: Array<{ code: string; path: string; span: { line_start: number } | null }> };

function todoSites(report: Report): string[] {
  return report.diagnostics
    .filter((diagnostic) => diagnostic.code === "clippy::todo")
    .map((diagnostic) => `${diagnostic.path}:${diagnostic.span?.line_start}`)
    .sort();
}

describe("working-tree states", () => {
  test.failing.each(["files", "lines"])(
    "H07 scopes/BUG-G a %s scan keeps an edit made through a tracked symlinked module",
    async (scope) => {
      const repo = await gitScenario(`h07-${scope}`)
        .template("mini")
        .write({ "src/impls/real.rs": "pub fn real() -> u8 {\n    3\n}\n" })
        .symlink("src/alias.rs", "impls/real.rs")
        .append("src/lib.rs", "pub mod alias;\n")
        .commit("base")
        .build();
      writeFileSync(join(repo.path, "src/impls/real.rs"), "pub fn real() -> u8 {\n    3\n}\n\npub fn more() -> u8 {\n    todo!()\n}\n");
      const baseline = await runDoctor([repo.path, "--yes", "--json", "--scope", "baseline", "--base", "main"], { sandbox: repo.sandbox });
      expect(todoSites(baseline.json as Report)).toContain("src/alias.rs:6");
      const run = await runDoctor([repo.path, "--yes", "--json", "--scope", scope, "--base", "main"], { sandbox: repo.sandbox });
      expect((run.json as Report).status).toBe("complete");
      expect(todoSites(run.json as Report)).toEqual(["src/alias.rs:6"]);
    },
  );
});
