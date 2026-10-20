// Run by harness.test.ts in its own bun process: a test that starts a process
// and returns without waiting for it, which the preload must kill and name.
import { test } from "bun:test";
import { runCommand } from "../e2e-helpers";

test("leaves a sleeper running", () => {
  void runCommand(["sleep", "30"]).catch(() => {});
});
