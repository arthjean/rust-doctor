import { expect, test } from "bun:test";
import { existsSync, writeFileSync } from "node:fs";

test("passes only on retry", () => {
  const state = process.env.PROBE_STATE!;
  const first = !existsSync(state);
  writeFileSync(state, "ran");
  expect(first).toBe(false);
});
