import { afterAll, afterEach, setDefaultTimeout } from "bun:test";
import { rmSync } from "node:fs";
import { killLiveProcesses, RUN_ROOT } from "./e2e-helpers";

setDefaultTimeout(60_000);

afterEach(() => {
  const left = killLiveProcesses();
  if (left.length > 0) {
    throw new Error(`killed a process the test left running: ${left.join(", ")}`);
  }
});

afterAll(() => {
  killLiveProcesses();
  rmSync(RUN_ROOT, { recursive: true, force: true });
});
