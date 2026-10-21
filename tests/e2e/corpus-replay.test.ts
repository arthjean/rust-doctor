// The 18 pinned subjects of `tests/corpus.json`, replayed through the shipped
// CLI rather than in process: each cold scan of a fresh copy reproduces its
// record, a second scan of the same copy publishes the same report once
// `timings` is removed, and the copy is byte-identical afterwards (US-005: a
// lockless subject's first scan used to write the `Cargo.lock` its second one
// read). Runs in the `replay` job of `.github/workflows/corpus.yml` only.
import { describe, expect, test } from "bun:test";
import { join } from "node:path";
import { runDoctor, scratchDir, treeFingerprint } from "./e2e-helpers";
import { materialize, observe, recordEnv, recorded, requireCorpus, SUBJECTS } from "./corpus-support";

function withoutTimings(report: unknown): unknown {
  const copy = structuredClone(report) as Record<string, unknown>;
  delete copy.timings;
  return copy;
}

describe.skipIf(!requireCorpus())("corpus replay", () => {
  test.each(SUBJECTS.map((entry) => [entry.name, entry] as const))("%s reproduces its record, twice, and leaves its copy as found", async (_name, entry) => {
    const copy = await materialize(entry);
    const before = treeFingerprint(copy.path);
    const target = join(scratchDir(`${entry.name}-target`), "target");
    const scan = () => runDoctor(["inspect", copy.path, ...entry.args], {
      sandbox: copy.sandbox,
      target,
      env: recordEnv(),
      timeoutMs: 600_000,
    });

    const cold = await scan();
    expect(cold.json, cold.stderr).toBeDefined();
    expect(observe(cold.exitCode, cold.json)).toEqual(recorded(entry));

    const rerun = await scan();
    expect(rerun.exitCode).toBe(cold.exitCode);
    expect(withoutTimings(rerun.json)).toEqual(withoutTimings(cold.json));
    expect(treeFingerprint(copy.path)).toBe(before);
  }, 900_000);
});
