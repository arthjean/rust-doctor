// Two real external codebases through the path a user takes: hexyl (one
// binary and library crate) and ripgrep (a ten-member workspace), both with a
// committed lockfile, scanned through the installed npm launcher and through
// the direct binary. The two must publish the same report and the same exit
// code: the launcher forwards, it never judges. Runs in the corpus job only.
import { describe, expect, test } from "bun:test";
import { join } from "node:path";
import { BIN, doctorArgs, hermeticEnv, scratchDir, spawnTracked } from "./e2e-helpers";
import { materialize, recordEnv, requireCorpus, subject } from "./corpus-support";
import { installedLauncher, launcherEnv } from "./launcher-support";

function withoutTimings(stdout: string): unknown {
  const report = JSON.parse(stdout) as Record<string, unknown>;
  delete report.timings;
  return report;
}

describe.skipIf(!requireCorpus())("real subjects through the launcher", () => {
  test.each(["hexyl", "ripgrep"])("%s scans the same through the launcher as through the binary", async (name) => {
    const entry = subject(name);
    const copy = await materialize(entry);
    const target = join(scratchDir(`${name}-target`), "target");
    const args = doctorArgs([copy.path, ...entry.args]);
    const launcher = launcherEnv(copy.sandbox);
    const env = hermeticEnv(copy.sandbox, {
      ...recordEnv(),
      ...launcher,
      PATH: `${launcher.PATH}:${recordEnv().PATH}`,
      CARGO_TARGET_DIR: target,
    });
    const scan = (program: string) => spawnTracked([program, ...args], { cwd: copy.sandbox.dir, env, timeoutMs: 600_000 }).exited;

    const direct = await scan(BIN);
    const launched = await scan(await installedLauncher());
    expect(direct.exitCode).toBe(0);
    expect(launched.exitCode).toBe(direct.exitCode);
    expect(launched.stderr).toBe(direct.stderr);
    expect(withoutTimings(launched.stdout)).toEqual(withoutTimings(direct.stdout));
  }, 900_000);
});
