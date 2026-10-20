// `tasks/e2e-catalog/failures.md` section SG: signals, deadlines and the
// process lifecycle. A signal is sent once the sleeping build script has
// written its pid, the moment Clippy is provably running, and `kill -0
// -<pgid>` on the group that script ran in proves Cargo did not outlive the
// scan. SIGINT, SIGTERM and SIGHUP end it with a report and exit 130 (US-010);
// SIGKILL and SIGQUIT run no handler, so their group outlives the binary.
import { describe, expect, test } from "bun:test";
import { chmodSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import {
  copyWorkspace,
  isEmptyDir,
  REPO_ROOT,
  runDoctor,
  sandbox,
  scratchDir,
  spawnDoctor,
  toolchainBin,
  treeFingerprint,
  type DoctorRun,
  type Sandbox,
} from "./e2e-helpers";
import { gitScenario, NEW_FN } from "./git-scenario";
import { groupAlive, groupGone, groupOf, killGroup, spawnStreaming, waitForFile } from "./signals-support";
import { alive, gone, snapshots } from "./scope-snapshot-support";

type ScanError = { stage: string; code: string; message: string };
type Report = {
  schema_version: number;
  status: string;
  audit: { score: unknown };
  errors: ScanError[];
  delta?: { summary: Record<string, number> };
};

const SCHEMA_VERSION = Number(readFileSync(join(REPO_ROOT, "src/report.rs"), "utf8").match(/SCHEMA_VERSION: u8 = (\d+);/)![1]);
const INTERRUPTED = "The scan was interrupted by a signal.";

// stdout is one JSON document and one trailing newline.
function oneDocument(stdout: string): Report {
  expect(stdout.endsWith("}\n")).toBe(true);
  expect(stdout.indexOf("\n")).toBe(stdout.length - 1);
  const report = JSON.parse(stdout) as Report;
  expect(report.schema_version).toBe(SCHEMA_VERSION);
  return report;
}

function codes(report: Report): string[] {
  return report.errors.map((error) => `${error.stage}/${error.code}`);
}

type Sleeping = {
  workspace: string;
  box: Sandbox;
  before: string;
  spawned: ReturnType<typeof spawnDoctor>;
  sleeper: number;
  group: number;
};

// A scan of the sleeping build, returned once its build script runs.
async function sleeping(label: string, args: string[] = ["--json"], workspace = copyWorkspace("scan-failures/sleeping-build"), box = sandbox(label)): Promise<Sleeping> {
  const pidFile = join(box.dir, `${label}.pid`);
  const before = treeFingerprint(workspace);
  const spawned = spawnDoctor([workspace, "--yes", ...args], { sandbox: box, env: { RUST_DOCTOR_SLEEPER_PID: pidFile } });
  const sleeper = Number(await waitForFile(pidFile));
  return { workspace, box, before, spawned, sleeper, group: groupOf(sleeper) };
}

// What every handled signal leaves: exit 130 soon after the signal, Cargo's
// group gone, the workspace and TMPDIR as found.
async function expectHandled(started: Sleeping, signals: NodeJS.Signals[], gapMs = 0): Promise<DoctorRun> {
  const sent = performance.now();
  for (const [index, signal] of signals.entries()) {
    if (index > 0) await Bun.sleep(gapMs);
    started.spawned.proc.kill(signal);
  }
  const run = await started.spawned.result;
  expect(performance.now() - sent).toBeLessThan(5_000);
  expect(run.exitCode).toBe(130);
  expect(await groupGone(started.group)).toBe(true);
  expect(alive(started.sleeper)).toBe(false);
  expect(treeFingerprint(started.workspace)).toBe(started.before);
  expect(snapshots(started.box.tmp)).toEqual([]);
  return run;
}

// Every catalogued Clippy rule off: the scan runs the native passes only and
// compiles nothing.
async function clippyOff(): Promise<string> {
  const rules = JSON.parse((await runDoctor(["rules", "list", "--json"])).stdout) as Array<{ id: string }>;
  return `[rules]\n${rules.filter((rule) => rule.id.startsWith("clippy::")).map((rule) => `"${rule.id}" = "off"`).join("\n")}\n`;
}

// Enough source that the native passes run for seconds: 3000 modules of
// distinct functions, so a signal lands while they work.
async function bigNative(): Promise<string> {
  const workspace = join(scratchDir("bignative"), "bignative");
  mkdirSync(join(workspace, "src"), { recursive: true });
  writeFileSync(join(workspace, "Cargo.toml"), '[package]\nname = "bignative"\nversion = "0.1.0"\nedition = "2024"\npublish = false\n\n[workspace]\n');
  writeFileSync(join(workspace, "rust-doctor.toml"), await clippyOff());
  let lib = "";
  for (let module = 0; module < 3000; module += 1) {
    lib += `pub mod m${module};\n`;
    let body = "";
    for (let index = 0; index < 20; index += 1) {
      body += `pub fn f${index}(values: &[u32]) -> u32 {\n    let mut total = ${module * 20 + index};\n    for value in values {\n        if *value > ${index} {\n            total += *value;\n        }\n    }\n    total\n}\n\n`;
    }
    writeFileSync(join(workspace, "src", `m${module}.rs`), body);
  }
  writeFileSync(join(workspace, "src/lib.rs"), lib);
  return workspace;
}

// A PATH whose `rustc --version` hangs, recording its pid; every other rustc
// call reaches the real one.
function hangingRustc(box: Sandbox): { path: string; pidFile: string } {
  const dir = join(box.dir, "hanging-bin");
  mkdirSync(dir);
  const pidFile = join(box.dir, "probe.pid");
  writeFileSync(
    join(dir, "rustc"),
    `#!/bin/sh\nif [ "$1" = --version ]; then echo $$ > '${pidFile}'; exec sleep 30; fi\nexec '${toolchainBin()}/rustc' "$@"\n`,
  );
  chmodSync(join(dir, "rustc"), 0o755);
  return { path: `${dir}:${toolchainBin()}`, pidFile };
}

describe("signals during Clippy", () => {
  const handled: Array<[id: string, signal: NodeJS.Signals]> = [
    ["SG-01", "SIGINT"],
    ["SG-02", "SIGTERM"],
    ["SG-07", "SIGHUP"],
  ];
  test.each(handled)("%s a %s while the build script sleeps exits 130 with a failed report and no orphan", async (_id, signal) => {
    const run = await expectHandled(await sleeping(signal.toLowerCase()), [signal]);
    const report = oneDocument(run.stdout);
    expect(report.status).toBe("failed");
    expect(report.audit.score).toBeNull();
    expect(report.errors).toEqual([{ stage: "clippy", code: "interrupted", message: INTERRUPTED }]);
  });

  test("SG-03 two SIGINTs 50 ms apart are one delivery: one interrupted report, exit 130", async () => {
    const run = await expectHandled(await sleeping("sigint-twice"), ["SIGINT", "SIGINT"], 50);
    expect(codes(oneDocument(run.stdout))).toEqual(["clippy/interrupted"]);
  });

  test("SG-04 a SIGINT of the linear report prints the failure line and exits 130", async () => {
    const run = await expectHandled(await sleeping("sigint-linear", []), ["SIGINT"]);
    expect(run.stdout).toStartWith("Scope: full codebase\n");
    expect(run.stdout).toContain(`Scan failed: ${INTERRUPTED} (clippy/interrupted)`);
  });

  // SIGKILL runs no handler: the binary dies, and Cargo's group, which it
  // started apart, keeps running until someone kills it. Inherent, and the
  // reason `timeout -s KILL` in CI leaks builds.
  test("SG-12 a SIGKILL ends the binary at once and leaves Cargo's group running", async () => {
    const started = await sleeping("sigkill");
    try {
      started.spawned.proc.kill("SIGKILL");
      const run = await started.spawned.result;
      expect(run.exitCode).toBeNull();
      expect(run.signal).toBe("SIGKILL");
      expect(run.stdout).toBe("");
      expect(groupAlive(started.group)).toBe(true);
      expect(alive(started.sleeper)).toBe(true);
    } finally {
      killGroup(started.group);
    }
    expect(await groupGone(started.group)).toBe(true);
  });

  // SIGQUIT is not one of the handled signals: its default action ends the
  // binary without a report, and, as with SIGKILL, Cargo's group survives.
  // Bun.spawn starts the child with the default disposition, which a shell's
  // `&` would have replaced with "ignored".
  test("SG-13 a SIGQUIT takes the default action: no report, Cargo's group left running", async () => {
    const started = await sleeping("sigquit");
    try {
      started.spawned.proc.kill("SIGQUIT");
      const run = await started.spawned.result;
      expect(run.exitCode).toBeNull();
      expect(run.signal).toBe("SIGQUIT");
      expect(run.stdout).toBe("");
      expect(groupAlive(started.group)).toBe(true);
    } finally {
      killGroup(started.group);
    }
    expect(await groupGone(started.group)).toBe(true);
  });
});

describe("signals during the native passes", () => {
  let workspace: string | undefined;
  const native: Array<[id: string, signal: NodeJS.Signals]> = [
    ["SG-05", "SIGINT"],
    ["SG-06", "SIGTERM"],
  ];
  test.each(native)("%s a %s while the native passes run exits 130 at the execution stage", async (_id, signal) => {
    workspace ??= await bigNative();
    const scan = spawnStreaming([workspace, "--yes"]);
    await scan.waitForStderr("Running native passes...");
    await Bun.sleep(200);
    const sent = performance.now();
    scan.proc.kill(signal);
    const run = await scan.exited;
    expect(performance.now() - sent).toBeLessThan(5_000);
    expect(run.exitCode).toBe(130);
    expect(run.stdout).toContain(`Scan failed: ${INTERRUPTED} (execution/interrupted)`);
    expect(isEmptyDir(scan.sandbox.tmp)).toBe(true);
  }, 90_000);
});

describe("signals during a baseline", () => {
  // The base side's build script sleeps, so the signal lands while the
  // snapshot exists: SIGHUP removes it and ends Cargo as SIGINT does.
  test.each(["SIGHUP", "SIGINT"] as const)("SG-08 a %s during a baseline's base side leaves no snapshot and no orphan", async (signal) => {
    const repo = await gitScenario(`sg-08-${signal}`)
      .fixture("scan-failures/sleeping-build")
      .write({ ".gitignore": "/target\n" })
      .commit("base")
      .append("quick/src/lib.rs", "\npub fn more() -> u8 {\n    todo!()\n}\n")
      .build();
    const started = await sleeping(`baseline-${signal}`, ["--json", "--scope", "baseline", "--base", "HEAD"], repo.path, repo.sandbox);
    const run = await expectHandled(started, [signal]);
    const report = oneDocument(run.stdout);
    expect(report.status).toBe("failed");
    expect(report.errors.map((error) => error.code)).toEqual(["interrupted"]);
  });

  test("SG-14 two concurrent baseline runs on one workspace both complete", async () => {
    const repo = await gitScenario("sg-14").template("mini").commit("base").branch("feature").append("src/lib.rs", NEW_FN).commit("feature").build();
    const before = treeFingerprint(repo.path);
    const args = [repo.path, "--yes", "--json", "--scope", "baseline", "--base", "main"];
    const runs = await Promise.all([runDoctor(args, { sandbox: repo.sandbox }), runDoctor(args, { sandbox: repo.sandbox })]);
    const reports = runs.map((run) => {
      expect(run.exitCode).toBe(0);
      const report = oneDocument(run.stdout);
      expect(report.status).toBe("complete");
      return report;
    });
    expect(reports[0]!.delta?.summary).toEqual(reports[1]!.delta?.summary);
    expect(reports[0]!.delta?.summary.introduced).toBe(1);
    expect(treeFingerprint(repo.path)).toBe(before);
    expect(snapshots(repo.sandbox.tmp)).toEqual([]);
  });
});

describe("deadlines", () => {
  // Scan A holds the build directory lock of a shared target; scan B waits
  // on it until its own deadline, and says why.
  test("SG-09 a scan waiting on another Cargo's build lock names the lock at its deadline", async () => {
    const target = scratchDir("shared-target");
    const box = sandbox("lock-holder");
    const holder = copyWorkspace("scan-failures/sleeping-build");
    const pidFile = join(box.dir, "holder.pid");
    const first = spawnDoctor([holder, "--yes", "--json"], { sandbox: box, target, env: { RUST_DOCTOR_SLEEPER_PID: pidFile } });
    const sleeper = Number(await waitForFile(pidFile));
    const group = groupOf(sleeper);
    let held: DoctorRun | undefined;
    try {
      const run = await runDoctor([copyWorkspace("projects/clean"), "--yes", "--json", "--max-duration", "3"], { target });
      expect(run.exitCode).toBe(2);
      const report = oneDocument(run.stdout);
      expect(report.status).toBe("incomplete");
      const deadline = report.errors.find((error) => error.code === "deadline-exceeded");
      expect(deadline?.stage).toBe("clippy");
      expect(deadline?.message).toContain("Not linted: clean.");
      expect(deadline?.message).toContain("Cargo waited on the build directory lock held by another Cargo or rust-analyzer.");
    } finally {
      first.proc.kill("SIGINT");
      held = await first.result;
    }
    expect(held.exitCode).toBe(130);
    expect(await groupGone(group)).toBe(true);
  });

  test("SG-10 a version probe that hangs ends at the --max-duration deadline, its child killed", async () => {
    const box = sandbox("probe-deadline");
    const rustc = hangingRustc(box);
    const started = performance.now();
    const run = await runDoctor([copyWorkspace("projects/clean"), "--yes", "--json", "--max-duration", "2"], { sandbox: box, env: { PATH: rustc.path } });
    expect(performance.now() - started).toBeLessThan(6_000);
    expect(run.exitCode).toBe(2);
    const report = oneDocument(run.stdout);
    expect(report.status).toBe("failed");
    expect(codes(report)).toContain("toolchain/deadline-exceeded");
    expect(await gone([Number(await waitForFile(rustc.pidFile))])).toEqual([]);
  });

  test("SG-11 a version probe that hangs without a deadline times out at 10 s and says so", async () => {
    const box = sandbox("probe-timeout");
    const rustc = hangingRustc(box);
    const started = performance.now();
    const run = await runDoctor([copyWorkspace("projects/clean"), "--yes", "--json"], { sandbox: box, env: { PATH: rustc.path } });
    const elapsed = performance.now() - started;
    expect(elapsed).toBeGreaterThan(9_500);
    expect(elapsed).toBeLessThan(20_000);
    expect(run.exitCode).toBe(2);
    const report = oneDocument(run.stdout);
    const timeout = report.errors.find((error) => error.code === "probe-timeout");
    expect(timeout?.stage).toBe("toolchain");
    expect(timeout?.message).toStartWith("rustc did not report a version within 10 s. Install a Rust toolchain");
    expect(await gone([Number(await waitForFile(rustc.pidFile))])).toEqual([]);
  }, 30_000);
});
