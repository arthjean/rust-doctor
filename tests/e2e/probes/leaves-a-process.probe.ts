import { test } from "bun:test";
import { runCommand } from "../e2e-helpers";

test("leaves a sleeper running", () => {
  void runCommand(["sleep", "30"]).catch(() => {});
});
