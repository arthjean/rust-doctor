// Loaded before every test file (bunfig.toml), so its hooks hold for the
// whole suite rather than for the first file that imports the helpers. It
// also loads the helpers themselves, which is what makes a missing binary
// fail every file with the build instruction.
import { afterAll, afterEach, setDefaultTimeout } from "bun:test";
import { rmSync } from "node:fs";
import { killLiveProcesses, RUN_ROOT } from "./e2e-helpers";

// Every test spawns the real binary, and a cold scan of a fixture compiles it
// first, so Bun's five-second default would fail on a slow runner rather than
// on a defect. A test that needs longer names its own timeout. Set here
// because bun 1.4.2 ignores `[test] timeout` in bunfig.toml.
setDefaultTimeout(60_000);

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
