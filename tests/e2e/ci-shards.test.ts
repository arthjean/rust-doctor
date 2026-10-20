// The CI planner and the per-file runner: deterministic weighted shards that
// refuse a file without a weight, and a retry that passes only with a visible
// warning.
import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { buildShardPlan, discoverTestFiles, selectShard } from "./ci-shards";
import { runCommand, sandbox, toolchainBin } from "./e2e-helpers";

const files = ["a.test.ts", "b.test.ts", "c.test.ts", "d.test.ts"];

describe("CI shard planner", () => {
  test("assigns descending weights to the lightest shard, whatever the input order", () => {
    const manifest = [
      { file: "d.test.ts", weight: 3 },
      { file: "b.test.ts", weight: 7 },
      { file: "a.test.ts", weight: 9 },
      { file: "c.test.ts", weight: 5 },
    ];
    const plan = buildShardPlan(files, manifest, 2);
    expect(plan).toEqual({ shards: [["a.test.ts", "d.test.ts"], ["b.test.ts", "c.test.ts"]], totals: [12, 12] });
    expect(buildShardPlan([...files].reverse(), [...manifest].reverse(), 2)).toEqual(plan);
    expect(selectShard(plan, 1)).toEqual(["b.test.ts", "c.test.ts"]);
    expect(() => selectShard(plan, 2)).toThrow("invalid shard index: 2");
  });

  test("a test file without a weight fails the plan naming it", () => {
    expect(() => buildShardPlan(files, files.slice(0, 3).map((file) => ({ file, weight: 1 })), 2)).toThrow(
      "missing manifest entry: d.test.ts",
    );
  });

  test("a weight without a file, a duplicate and a bad weight fail the plan naming them", () => {
    const one = files.map((file) => ({ file, weight: 1 }));
    expect(() => buildShardPlan(files, [...one, { file: "gone.test.ts", weight: 1 }], 2)).toThrow("stale manifest entry: gone.test.ts");
    expect(() => buildShardPlan(files, [...one, { file: "a.test.ts", weight: 2 }], 2)).toThrow("duplicate manifest entry: a.test.ts");
    for (const weight of [0, -1, 1.5, Number.NaN]) {
      expect(() => buildShardPlan(files, one.map((entry) => (entry.file === "a.test.ts" ? { ...entry, weight } : entry)), 2)).toThrow(
        "weight must be a positive integer: a.test.ts",
      );
    }
    expect(() => buildShardPlan(files, one, 5)).toThrow("shard 4 is empty");
    expect(() => buildShardPlan(files, one, 0)).toThrow("shard count must be a positive integer");
  });

  test("the checked-in weights cover this directory's test files exactly, in three shards", () => {
    const manifest: unknown = JSON.parse(readFileSync(join(import.meta.dir, "ci-shard-weights.json"), "utf8"));
    const plan = buildShardPlan(discoverTestFiles(import.meta.dir), manifest, 3);
    expect(plan.shards.flat().sort()).toEqual(discoverTestFiles(import.meta.dir).sort());
  });
});

describe("CI file runner", () => {
  async function runFiles(...paths: string[]) {
    const box = sandbox("ci-run-files");
    const run = await runCommand(["bash", join(import.meta.dir, "ci-run-files.sh"), ...paths], {
      sandbox: box,
      env: {
        PATH: `${dirname(process.execPath)}:${toolchainBin()}`,
        PROBE_STATE: join(box.tmp, "probe-state"),
        BUN_INSTALL_CACHE_DIR: join(box.tmp, "bun-cache"),
      },
      timeoutMs: 60_000,
    });
    return { ...run, output: run.stdout + run.stderr };
  }

  test("a file that passes only on retry passes with a visible warning", async () => {
    const run = await runFiles("probes/flaky.probe.ts");
    expect(run.exitCode).toBe(0);
    expect(run.output).toContain("Retrying failed e2e file after resetting tmux: probes/flaky.probe.ts");
    expect(run.output).toContain("::warning title=E2E passed only on retry::probes/flaky.probe.ts failed and then passed on retry.");
  });

  test("a file that fails twice fails the shard", async () => {
    const run = await runFiles("probes/broken.probe.ts");
    expect(run.exitCode).toBe(1);
    expect(run.output).toContain("Retrying failed e2e file after resetting tmux: probes/broken.probe.ts");
    expect(run.output).not.toContain("::warning");
  });
});
