// Run by ci-shards.test.ts through ci-run-files.sh: fails on every run.
import { expect, test } from "bun:test";

test("always fails", () => {
  expect(1).toBe(2);
});
