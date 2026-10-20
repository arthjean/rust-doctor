// `tasks/e2e-catalog/scopes.md` sections N and K: integrity, cleanup and
// signals. EP-005 fills it; the row here is the open index-integrity defect
// of the ledger.
import { describe, expect, test } from "bun:test";
import { utimesSync } from "node:fs";
import { join } from "node:path";
import { runDoctor, treeFingerprint } from "./e2e-helpers";
import { gitScenario } from "./git-scenario";

describe("repository integrity", () => {
  test.failing.each(["files", "lines", "contextual"])(
    "N02 scopes/BUG-C a %s scan leaves .git/index as it found it",
    async (scope) => {
      const repo = await gitScenario(`n02-${scope}`).template("mini").commit("base").build();
      // A stat change the index has not recorded: a refreshing `git diff`
      // writes the index back, a `git diff-index` does not.
      await Bun.sleep(1_100);
      const now = new Date();
      utimesSync(join(repo.path, "src/lib.rs"), now, now);
      const before = treeFingerprint(repo.path);
      const run = await runDoctor([repo.path, "--yes", "--json", "--scope", scope], { sandbox: repo.sandbox });
      expect(run.exitCode).toBe(0);
      expect(treeFingerprint(repo.path)).toBe(before);
    },
  );
});
