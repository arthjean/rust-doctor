// Loaded before every test file (bunfig.toml), so its hooks hold for the
// whole suite rather than for the first file that imports the helpers. It
// also loads the helpers themselves, which is what makes a missing binary
// fail every file with the build instruction.
import { afterAll, afterEach } from "bun:test";
import { rmSync } from "node:fs";
import { killLiveProcesses, RUN_ROOT } from "./e2e-helpers";

afterEach(() => {
  const left = killLiveProcesses();
  if (left.length > 0) {
    throw new Error(`killed a process the test left running: ${left.join(", ")}`);
  }
});

// `bun test` exits without firing the process `exit` event the helpers listen
// to, so the run's scratch directory goes here, once every file has run.
afterAll(() => {
  killLiveProcesses();
  rmSync(RUN_ROOT, { recursive: true, force: true });
});
