// Run by ci-shards.test.ts through ci-run-files.sh: fails on its first run
// and passes on the next, keeping its count in $PROBE_STATE.
import { expect, test } from "bun:test";
import { existsSync, writeFileSync } from "node:fs";

test("passes only on retry", () => {
  const state = process.env.PROBE_STATE!;
  const first = !existsSync(state);
  writeFileSync(state, "ran");
  expect(first).toBe(false);
});
