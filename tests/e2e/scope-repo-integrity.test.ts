import { describe, expect, test } from "bun:test";
import { existsSync, readdirSync, utimesSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { E2E_BASE, runDoctor, spawnDoctor, treeFingerprint } from "./e2e-helpers";
import { gitScenario, miniTemplate, NEW_FN, waitForSnapshot } from "./git-scenario";
import { errorCodes, scan, type Report } from "./scope-scan-support";
import { buildGroup, gone, slowRepo, snapshots, waitForBuildScript, type SlowRepo } from "./scope-snapshot-support";

const LIB = miniTemplate()["src/lib.rs"]!;

function busy(name: string) {
  return gitScenario(name)
    .template("mini").commit("base")
    .branch("feature").append("src/lib.rs", NEW_FN).commit("feature")
    .stage({ write: { "src/staged.rs": "pub fn staged() -> u8 {\n    todo!()\n}\n" } })
    .unstaged({ "src/creds.rs": `${miniTemplate()["src/creds.rs"]}// edited\n` })
    .untracked({ "notes.txt": "notes\n" });
}

function datedAfterTheIndex(path: string): void {
  const later = new Date(Date.now() + 3_600_000);
  utimesSync(path, later, later);
}

describe("the repository a scan reads", () => {
  test("N01 baseline, staged and full scans leave every byte, the index's mtime included", async () => {
    for (const racy of [false, true]) {
      const repo = await busy(`n01-${racy}`).build();
      if (racy) datedAfterTheIndex(join(repo.path, "src/lib.rs"));
      for (const args of [[], ["--scope", "baseline"], ["--scope", "baseline", "--staged"], ["--scope", "files", "--staged"], ["--scope", "lines", "--staged"], ["--scope", "contextual", "--staged"]]) {
        const { run } = await scan(repo.path, args, { sandbox: repo.sandbox });
        expect(run.exitCode).toBe(0);
      }
    }
  });

  async function statChanged(name: string) {
    const repo = await gitScenario(name).template("mini").commit("base").build();
    await Bun.sleep(1_100);
    const now = new Date();
    utimesSync(join(repo.path, "src/lib.rs"), now, now);
    return repo;
  }

  test.each(["files", "lines", "contextual"])("N02 a %s scan of a stat-only change completes and lists no file", async (scope) => {
    const repo = await statChanged(`n02-${scope}`);
    const run = await runDoctor([repo.path, "--yes", "--json", "--scope", scope], { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(0);
    expect((run.json as Report).status).toBe("complete");
    expect((run.json as Report).scope?.files).toEqual([]);
  });

  test.failing.each(["files", "lines", "contextual"])("scopes/BUG-C a %s scan leaves .git/index as it found it (N02)", async (scope) => {
    const repo = await statChanged(`n02-index-${scope}`);
    const before = treeFingerprint(repo.path);
    const run = await runDoctor([repo.path, "--yes", "--json", "--scope", scope], { sandbox: repo.sandbox });
    if (run.json === undefined) throw new Error(`no JSON report (exit ${run.exitCode}): ${run.stderr}`);
    expect(treeFingerprint(repo.path)).toBe(before);
  });

  test.failing("N03 scopes/BUG-C a racily clean index survives two files, lines and contextual scans", async () => {
    for (const scope of ["files", "lines", "contextual"]) {
      const repo = await gitScenario(`n03-${scope}`).template("mini").commit("base").build();
      datedAfterTheIndex(join(repo.path, "src/lib.rs"));
      for (let pass = 0; pass < 2; pass += 1) {
        const before = treeFingerprint(repo.path);
        const run = await runDoctor([repo.path, "--yes", "--json", "--scope", scope], { sandbox: repo.sandbox });
        expect(run.exitCode).toBe(0);
        expect(`${scope} pass ${pass}: ${treeFingerprint(repo.path)}`).toBe(`${scope} pass ${pass}: ${before}`);
      }
    }
  });

  test("N06 no report of any scope carries an absolute path", async () => {
    const repo = await busy("n06").build();
    for (const args of [[], ["--scope", "files"], ["--scope", "lines"], ["--scope", "contextual"], ["--scope", "baseline"], ["--scope", "lines", "--staged"], ["--scope", "baseline", "--staged"]]) {
      const { run } = await scan(repo.path, args, { sandbox: repo.sandbox });
      expect(run.exitCode).toBe(0);
      for (const absolute of [repo.path, E2E_BASE, homedir()]) expect(run.stdout).not.toContain(absolute);
    }
  });

  test("E11 concurrent baselines agree, change nothing and leave no snapshot", async () => {
    const repo = await gitScenario("e11").template("mini").commit("base").branch("feature").append("src/lib.rs", NEW_FN).commit("feature").build();
    const target = join(repo.sandbox.dir, "shared-target");
    const before = treeFingerprint(repo.path);
    const runs = await Promise.all(
      Array.from({ length: 4 }, () =>
        runDoctor([repo.path, "--yes", "--json", "--scope", "baseline"], { sandbox: repo.sandbox, target, env: { RUST_DOCTOR_NO_CACHE: "1" } }),
      ),
    );
    expect(treeFingerprint(repo.path)).toBe(before);
    expect(snapshots(repo.sandbox.tmp)).toEqual([]);
    const views = runs.map((run) => {
      expect(run.exitCode).toBe(0);
      const report = run.json as Report & { delta: unknown };
      return { delta: report.delta, ids: report.diagnostics.map((diagnostic) => diagnostic.id).sort() };
    });
    for (const view of views.slice(1)) expect(view).toEqual(views[0]!);
  });
});

describe("snapshots in TMPDIR", () => {
  test("N04 no snapshot is left after exit 0, 1 or 2 of a baseline or staged run", async () => {
    const noop = await gitScenario("n04-noop").template("mini").commit("base").branch("feature").build();
    const todo = await gitScenario("n04-todo")
      .template("mini").commit("base")
      .branch("feature").append("src/lib.rs", NEW_FN).commit("feature")
      .stage({ write: { "src/lib.rs": LIB + NEW_FN + NEW_FN.replace("new_debt", "newer_debt") } })
      .build();
    const broken = await gitScenario("n04-broken")
      .template("mini").append("src/lib.rs", "pub fn broken( {\n").commit("base")
      .branch("feature").write({ "src/lib.rs": LIB }).commit("repair")
      .build();
    const slow = await slowRepo("n04-slow", "stage");
    const cases = [
      { repo: noop, args: ["--scope", "baseline"], exit: 0 },
      { repo: noop, args: ["--scope", "lines", "--staged"], exit: 0 },
      { repo: todo, args: ["--scope", "baseline", "--blocking", "warning"], exit: 1 },
      { repo: todo, args: ["--scope", "lines", "--staged", "--blocking", "warning"], exit: 1 },
      { repo: broken, args: ["--scope", "baseline"], exit: 2 },
      { repo: slow, args: ["--scope", "lines", "--staged", "--max-duration", "1"], exit: 2, env: slow.env },
    ];
    for (const { repo, args, exit, env } of cases) {
      const { run } = await scan(repo.path, args, { sandbox: repo.sandbox, env });
      expect(`${args.join(" ")}: exit ${run.exitCode}`).toBe(`${args.join(" ")}: exit ${exit}`);
      expect(snapshots(repo.sandbox.tmp)).toEqual([]);
    }
  });

  test("N05 with the default TMPDIR, baseline and staged baseline runs leave no snapshot of theirs", async () => {
    const repo = await gitScenario("n05")
      .template("mini").commit("base")
      .branch("feature").append("src/lib.rs", NEW_FN).commit("feature")
      .stage({ write: { "src/extra.rs": "pub fn extra() {}\n" } })
      .build();
    for (const args of [["--scope", "baseline"], ["--scope", "baseline", "--staged"]]) {
      const before = treeFingerprint(repo.path);
      const spawned = spawnDoctor([repo.path, "--yes", "--json", ...args], { sandbox: repo.sandbox, env: { TMPDIR: undefined } });
      const run = await spawned.result;
      expect(run.exitCode).toBe(0);
      expect(treeFingerprint(repo.path)).toBe(before);
      const own = `rust-doctor-baseline-${spawned.pid}-`;
      expect(readdirSync("/tmp").filter((name) => name.startsWith(own))).toEqual([]);
    }
  });
});

type Interrupted = Report & { audit: { score: unknown } };

async function interruptible(repo: SlowRepo, args: string[]) {
  const before = treeFingerprint(repo.path);
  const spawned = spawnDoctor([repo.path, "--yes", "--json", ...args], { sandbox: repo.sandbox, env: repo.env });
  const snapshot = await waitForSnapshot(repo.sandbox.tmp);
  return { spawned, snapshot, before };
}

async function expectInterrupted(
  repo: SlowRepo,
  started: Awaited<ReturnType<typeof interruptible>>,
  signal: NodeJS.Signals,
  mode: string,
  stage = "execution",
) {
  const sent = performance.now();
  started.spawned.proc.kill(signal);
  const run = await started.spawned.result;
  expect(performance.now() - sent).toBeLessThan(5_000);
  expect(run.exitCode).toBe(130);
  const report = run.json as Interrupted;
  expect(report.status).toBe("failed");
  expect(errorCodes(report)).toEqual([`${stage}/interrupted`]);
  expect(report.scope?.mode).toBe(mode);
  expect(report.audit.score).toBeNull();
  expect(existsSync(started.snapshot)).toBe(false);
  expect(snapshots(repo.sandbox.tmp)).toEqual([]);
  expect(treeFingerprint(repo.path)).toBe(started.before);
}

describe("signals", () => {
  test("K1 a SIGINT just after the snapshot appears stops the baseline and removes it", async () => {
    const repo = await slowRepo("k1");
    const started = await interruptible(repo, ["--scope", "baseline"]);
    await Bun.sleep(300);
    await expectInterrupted(repo, started, "SIGINT", "baseline");
  });

  test("K2 a SIGINT while the base side compiles stops Cargo and its build script", async () => {
    const repo = await slowRepo("k2");
    const started = await interruptible(repo, ["--scope", "baseline"]);
    const builds = await waitForBuildScript(repo);
    await expectInterrupted(repo, started, "SIGINT", "baseline");
    expect(await gone(builds)).toEqual([]);
  });

  test.each(["SIGTERM", "SIGHUP"] as const)("K3 a %s while the base side compiles stops it like SIGINT", async (signal) => {
    const repo = await slowRepo(`k3-${signal}`);
    const started = await interruptible(repo, ["--scope", "baseline"]);
    const builds = await waitForBuildScript(repo);
    await expectInterrupted(repo, started, signal, "baseline");
    expect(await gone(builds)).toEqual([]);
  });

  test("K4 a SIGKILLed run leaves its snapshot, and the next run sweeps it", async () => {
    const repo = await slowRepo("k4");
    const started = await interruptible(repo, ["--scope", "baseline"]);
    const builds = await waitForBuildScript(repo);
    started.spawned.proc.kill("SIGKILL");
    const killed = await started.spawned.proc.exited;
    try {
      expect(started.spawned.proc.signalCode).toBe("SIGKILL");
      expect(killed).not.toBe(0);
      expect(snapshots(repo.sandbox.tmp)).toEqual([`rust-doctor-baseline-${started.spawned.pid}-0`]);
    } finally {
      process.kill(-buildGroup(repo, builds[0]!), "SIGKILL");
      await started.spawned.result.catch(() => undefined);
    }
    expect(await gone(builds)).toEqual([]);
    const next = await gitScenario("k4-next").template("mini").commit("base").branch("feature").build();
    const { run } = await scan(next.path, ["--scope", "baseline"], { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(0);
    expect(snapshots(repo.sandbox.tmp)).toEqual([]);
  });

  test("K5 a SIGINT during a staged scan stops it and removes the staged snapshot", async () => {
    const repo = await slowRepo("k5", "stage");
    const started = await interruptible(repo, ["--scope", "lines", "--staged"]);
    await waitForBuildScript(repo);
    await expectInterrupted(repo, started, "SIGINT", "lines", "clippy");
  });
});
