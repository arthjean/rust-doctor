// `tasks/e2e-catalog/scopes.md` sections N and K, with E11: what a scan leaves
// behind. The repository byte for byte, no snapshot in TMPDIR whatever the
// exit, no absolute path in the report, and a clean stop on SIGINT, SIGTERM
// and SIGHUP (US-010). A SIGKILLed run's snapshot is swept by the next one.
import { describe, expect, test } from "bun:test";
import { existsSync, readdirSync, utimesSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { E2E_BASE, runDoctor, spawnDoctor, treeFingerprint } from "./e2e-helpers";
import { gitScenario, miniTemplate, NEW_FN, waitForSnapshot } from "./git-scenario";
import { errorCodes, scan, type Report } from "./scope-scan-support";
import { buildGroup, gone, slowRepo, snapshots, waitForBuildScript, type SlowRepo } from "./scope-snapshot-support";

const LIB = miniTemplate()["src/lib.rs"]!;

// A branch with a committed, a staged, an unstaged and an untracked change.
function busy(name: string) {
  return gitScenario(name)
    .template("mini").commit("base")
    .branch("feature").append("src/lib.rs", NEW_FN).commit("feature")
    .stage({ write: { "src/staged.rs": "pub fn staged() -> u8 {\n    todo!()\n}\n" } })
    .unstaged({ "src/creds.rs": `${miniTemplate()["src/creds.rs"]}// edited\n` })
    .untracked({ "notes.txt": "notes\n" });
}

// A clean tracked file whose mtime is not older than the index: git cannot
// trust its stat data, and a refreshing command reads it again and records
// the new stat in the index. The `racyIndex()` state lasts only until the
// first refresh, so the file is dated an hour ahead to stay racy across every
// scan of a row.
function datedAfterTheIndex(path: string): void {
  const later = new Date(Date.now() + 3_600_000);
  utimesSync(path, later, later);
}

describe("the repository a scan reads", () => {
  // The scopes that never refresh the index, on a racily clean one too.
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

  // A stat change the index has not recorded: a refreshing `git diff` writes
  // the index back, a `git diff-index` does not.
  async function statChanged(name: string) {
    const repo = await gitScenario(name).template("mini").commit("base").build();
    await Bun.sleep(1_100);
    const now = new Date();
    utimesSync(join(repo.path, "src/lib.rs"), now, now);
    return repo;
  }

  // The index half of N02 is the open defect, kept apart so it cannot hide a
  // failing scan.
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

  // Every scope gets its own racily clean file, so each one's first run
  // meets the stat change the index has not recorded.
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

  // Four runs at once on one repository and one target, the cache off: each
  // run materializes its own base side.
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
  // Every exit a baseline or staged run can end on, each after its snapshot
  // existed: a pass, a failed gate, and a failure (a base that does not
  // compile, a staged scan cut by its deadline).
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

  // TMPDIR unset: the snapshot goes to `/tmp`, which other processes share,
  // so only the names carrying the run's own pid are its own.
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

// Starts a cold, uncached scan of `repo`, whose build script sleeps, and
// returns once its snapshot exists.
async function interruptible(repo: SlowRepo, args: string[]) {
  const before = treeFingerprint(repo.path);
  const spawned = spawnDoctor([repo.path, "--yes", "--json", ...args], { sandbox: repo.sandbox, env: repo.env });
  const snapshot = await waitForSnapshot(repo.sandbox.tmp);
  return { spawned, snapshot, before };
}

// `stage` is where the signal stopped the run: `clippy` when it stopped
// Clippy on the scanned tree, `execution` anywhere else, the base side
// included.
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

  // SIGHUP, a closed terminal tab, ends the scan as SIGTERM does (US-010).
  test.each(["SIGTERM", "SIGHUP"] as const)("K3 a %s while the base side compiles stops it like SIGINT", async (signal) => {
    const repo = await slowRepo(`k3-${signal}`);
    const started = await interruptible(repo, ["--scope", "baseline"]);
    const builds = await waitForBuildScript(repo);
    await expectInterrupted(repo, started, signal, "baseline");
    expect(await gone(builds)).toEqual([]);
  });

  // SIGKILL runs nothing: the snapshot stays, and Cargo's process group,
  // which the binary started apart, outlives it. The next baseline or staged
  // run in that TMPDIR sweeps the dead run's snapshot (scopes/K4, US-010).
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
