// `tasks/e2e-catalog/failures.md` sections EX and AR: the exit status matrix
// and argument validation. The precedence 2, then 1, then the warning code,
// then 0 is one table; an argument error stops before Cargo starts, which an
// empty CARGO_TARGET_DIR proves.
import { describe, expect, test } from "bun:test";
import { existsSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import {
  copyWorkspace,
  isEmptyDir,
  runCommand,
  runDoctor,
  sandbox,
  scratchDir,
  spawnDoctor,
  treeFingerprint,
  type DoctorOptions,
  type DoctorRun,
} from "./e2e-helpers";

type ScanError = { stage: string; code: string; message: string };
type Score = { value: number; label: string; authoritative: boolean; reasons: string[] };
type Report = {
  schema_version: number;
  status: string;
  audit: { score: Score | null };
  policy: { rules: Array<{ id: string; not_evaluated?: string }> } | null;
  scope: unknown;
  project: unknown;
  diagnostics: Array<{ code: string | null; source: string; severity: string }>;
  errors: ScanError[];
  summary: { errors: number; warnings: number };
  gate: { blocking: string; status: string; blocking_diagnostics: number | null };
};

// A scan of `workspace` that must leave it exactly as it found it.
async function scan(workspace: string, args: string[], options: DoctorOptions = {}): Promise<DoctorRun & { report: Report }> {
  const before = treeFingerprint(workspace);
  const run = await runDoctor([workspace, "--yes", ...args], options);
  expect(treeFingerprint(workspace)).toBe(before);
  return { ...run, report: run.json as Report };
}

function codes(report: Report): string[] {
  return report.errors.map((error) => `${error.stage}/${error.code}`);
}

type MatrixRow = [id: string, fixture: string, args: string[], exit: number, check: (run: DoctorRun & { report: Report }) => void];

const matrix: MatrixRow[] = [
  ["EX-01", "projects/clean", [], 0, ({ report, stderr }) => {
    expect(report.status).toBe("complete");
    expect(report.gate.status).toBe("passed");
    expect(report.audit.score).toMatchObject({ value: 100, authoritative: true });
    expect(stderr).toBe("");
  }],
  ["EX-02", "projects/clippy-warning", [], 0, ({ report }) => {
    expect(report.gate).toMatchObject({ blocking: "error", status: "passed" });
    expect(report.summary.warnings).toBe(1);
  }],
  ["EX-03", "projects/clippy-warning", ["--blocking", "warning"], 1, ({ report }) => {
    expect(report.status).toBe("complete");
    expect(report.gate).toEqual({ blocking: "warning", status: "failed", blocking_diagnostics: 1 });
  }],
  ["EX-04", "projects/clippy-warning", ["--warning-exit-code", "42"], 42, ({ report }) => {
    expect(report.gate.status).toBe("passed");
  }],
  ["EX-05", "projects/clean", ["--warning-exit-code", "42"], 0, ({ report }) => {
    expect(report.gate.status).toBe("passed");
  }],
  ["EX-06", "projects/clippy-warning", ["--blocking", "none", "--warning-exit-code", "42"], 0, ({ report }) => {
    expect(report.gate.blocking).toBe("none");
  }],
  ["EX-07", "projects/compile-error", [], 1, ({ report }) => {
    expect(report.status).toBe("incomplete");
    expect(report.gate.status).toBe("not-evaluated");
    expect(codes(report)).toEqual(["clippy/packages-unlinted", "execution/build-failed", "execution/clippy-exit"]);
    expect(report.diagnostics.map((diagnostic) => diagnostic.code)).toContain("E0308");
  }],
  ["EX-08", "projects/compile-error", ["--warning-exit-code", "42"], 1, ({ report }) => {
    expect(report.status).toBe("incomplete");
  }],
  ["EX-09", "scan-failures/sleeping-build", ["--max-duration", "3"], 2, ({ report, durationMs }) => {
    expect(durationMs).toBeGreaterThanOrEqual(3_000);
    expect(durationMs).toBeLessThan(10_000);
    expect(report.status).toBe("incomplete");
    expect(report.errors).toEqual([{
      stage: "clippy",
      code: "deadline-exceeded",
      message: "The scan stopped at the 3 s limit set by --max-duration. Not linted: sleeper.",
    }]);
    expect(report.diagnostics.map((diagnostic) => diagnostic.code)).toContain("clippy::todo");
    expect(report.audit.score).toMatchObject({ value: 94, authoritative: false });
  }],
  ["EX-10", "scan-failures/sleeping-build", ["--max-duration", "2", "--warning-exit-code", "9"], 2, ({ report }) => {
    expect(codes(report)).toEqual(["clippy/deadline-exceeded"]);
  }],
];

describe("exit status matrix", () => {
  test.each(matrix)("%s exits with the code the precedence 2, 1, warning, 0 gives", async (_id, fixture, args, exit, check) => {
    const run = await scan(copyWorkspace(fixture), ["--json", ...args]);
    expect(run.exitCode).toBe(exit);
    check(run);
  });

  const discovery: Array<[id: string, path: (dir: string) => string, message: string]> = [
    ["EX-11", (dir) => dir, "no Cargo.toml found in this path or its ancestors"],
    ["EX-12", (dir) => join(dir, "does-not-exist"), "path is not a directory"],
    ["EX-13", (dir) => join(copyWorkspace("projects/clean", join(dir, "clean")), "src/lib.rs"), "path is not a directory"],
  ];
  test.each(discovery)("%s a path holding no workspace fails with no-manifest and exit 2", async (_id, path, message) => {
    const dir = scratchDir("not-cargo");
    writeFileSync(join(dir, "README"), "not a crate\n");
    const scanned = path(dir);
    const before = treeFingerprint(dir);
    const run = await runDoctor([scanned, "--yes", "--json"]);
    expect(treeFingerprint(dir)).toBe(before);
    const report = run.json as Report;
    expect(run.exitCode).toBe(2);
    expect(run.stderr).toBe("");
    expect(report.status).toBe("failed");
    expect(report.errors).toEqual([{ stage: "discovery", code: "no-manifest", message }]);
    expect(report.audit.score).toBeNull();
    expect([report.policy, report.scope, report.project]).toEqual([null, null, null]);
  });

  test("EX-14 a report that cannot be written exits 2 even when the gate failed", async () => {
    const workspace = copyWorkspace("projects/clippy-warning");
    const box = sandbox("dev-full");
    const before = treeFingerprint(workspace);
    const run = await runCommand(
      ["bash", "-c", 'exec rust-doctor "$@" >/dev/full', "bash", workspace, "--yes", "--json", "--blocking", "warning", "--advisory-db", "/dev/null"],
      { sandbox: box, env: { CARGO_TARGET_DIR: join(box.dir, "target") } },
    );
    expect(treeFingerprint(workspace)).toBe(before);
    expect(run.exitCode).toBe(2);
    expect(run.stderr).toBe("Failed to write report: could not serialize report: No space left on device (os error 28)\n");
  });

  test("EX-15 a SIGINT during the build exits 130 with a failed report", async () => {
    const workspace = copyWorkspace("scan-failures/sleeping-build");
    const box = sandbox("sigint");
    const pidFile = join(box.dir, "sleeper.pid");
    const before = treeFingerprint(workspace);
    const spawned = spawnDoctor([workspace, "--yes", "--json"], { sandbox: box, env: { RUST_DOCTOR_SLEEPER_PID: pidFile } });
    const deadline = Date.now() + 30_000;
    while (!existsSync(pidFile) && Date.now() < deadline) await Bun.sleep(25);
    expect(existsSync(pidFile)).toBe(true);
    spawned.proc.kill("SIGINT");
    const run = await spawned.result;
    expect(treeFingerprint(workspace)).toBe(before);
    expect(run.exitCode).toBe(130);
    const report = run.json as Report;
    expect(report.status).toBe("failed");
    expect(codes(report)).toEqual(["clippy/interrupted"]);
  });

  test("EX-16 --help lists the five exit statuses and their precedence", async () => {
    const run = await runDoctor(["--help"]);
    expect(run.exitCode).toBe(0);
    expect(run.stderr).toBe("");
    const statuses = run.stdout.slice(run.stdout.indexOf("Exit status:"));
    for (const line of [
      "0       the scan completed and the gate passed",
      "1       the gate failed, or the scan is incomplete",
      "2       the scan failed, ran past --max-duration, or the arguments are invalid",
      "3-125   --warning-exit-code: the gate passed with a finding --blocking warning would block",
      "130     a SIGINT, SIGTERM or SIGHUP stopped the scan",
    ]) {
      expect(statuses).toContain(line);
    }
    expect(statuses).toContain("The precedence is 2, 1, the warning code, then 0.");
  });
});

// An argument error: exit 2, nothing on stdout, and no Cargo started.
async function refused(args: string[], fixture = "projects/clean"): Promise<DoctorRun> {
  const workspace = copyWorkspace(fixture);
  const target = scratchDir("target");
  const before = treeFingerprint(workspace);
  const run = await runDoctor([workspace, "--yes", ...args], { target });
  expect(treeFingerprint(workspace)).toBe(before);
  expect(run.exitCode).toBe(2);
  expect(isEmptyDir(target)).toBe(true);
  return run;
}

const HELP_HINT = "For more information, try '--help'.\n";

describe("argument validation", () => {
  const clap: Array<[id: string, variants: Array<[args: string[], message: string]>]> = [
    ["AR-01", [[["--max-duration", "0"], "error: invalid value '0' for '--max-duration <SECONDS>': 0 is not in 1..=86400\n"]]],
    ["AR-02", [[["--max-duration", "86401"], "error: invalid value '86401' for '--max-duration <SECONDS>': 86401 is not in 1..=86400\n"]]],
    ["AR-03", [
      [["--warning-exit-code", "2"], "error: invalid value '2' for '--warning-exit-code <CODE>': 2 is not in 3..=125\n"],
      [["--warning-exit-code", "126"], "error: invalid value '126' for '--warning-exit-code <CODE>': 126 is not in 3..=125\n"],
    ]],
    ["AR-04", [[["--bogus"], "error: unexpected argument '--bogus' found\n\n  tip: to pass '--bogus' as a value, use '-- --bogus'\n"]]],
    ["AR-05", [[["--blocking", "fatal"], "error: invalid value 'fatal' for '--blocking <BLOCKING>'\n  [possible values: none, error, warning]\n"]]],
  ];
  test.each(clap)("%s the parser refuses the value before any process starts", async (_id, variants) => {
    for (const [args, message] of variants) {
      const run = await refused([...args, "--json"]);
      expect(run.stdout).toBe("");
      expect(run.stderr).toStartWith(message);
      expect(run.stderr).toEndWith(HELP_HINT);
    }
  });

  test.failing("AR-06 failures/BUG-10 a malformed --rule reads like every other argument error", async () => {
    const workspace = copyWorkspace("kernel-contract/todo");
    const clap = await runDoctor([workspace, "--bogus"]);
    expect(clap.stderr).toEndWith("For more information, try '--help'.\n");
    const run = await runDoctor([workspace, "--yes", "--rule", "nonsense"]);
    expect(run.exitCode).toBe(2);
    expect(run.stdout).toBe("");
    expect(run.stderr).toStartWith("error: expected KEY=LEVEL");
    expect(run.stderr).toEndWith("For more information, try '--help'.\n");
  });

  const policy: Array<[id: string, args: string[], code: string, message: string]> = [
    ["AR-07", ["--rule", "clippy::does_not_exist=warn"], "unknown-rule", "Unknown rule selector."],
    ["AR-08", ["--rule", "clippy::todo=off", "--rule", "clippy::todo=warn"], "duplicate-rule-override", "Duplicate rule override."],
    ["AR-09", ["--category", "nope=warn"], "unknown-category", "Unknown category selector."],
    ["AR-10", ["--rule", "clippy::*=off"], "invalid-rule-selector", "Invalid rule selector."],
  ];
  test.each(policy)("%s fails the policy stage without echoing the selector", async (_id, args, code, message) => {
    const run = await refused(["--json", ...args]);
    const report = run.json as Report;
    expect(run.stderr).toBe("");
    expect(report.status).toBe("failed");
    expect(report.errors).toEqual([{ stage: "policy", code, message }]);
    expect(report.audit.score).toBeNull();
    for (const selector of ["does_not_exist", "nope", "clippy::*"]) expect(run.stdout).not.toContain(selector);
  });

  test("AR-11 --base without a scope that takes one is a usage error", async () => {
    const run = await refused(["--json", "--base", "HEAD"]);
    expect(run.stdout).toBe("");
    expect(run.stderr).toBe(
      "error: --base <REF> requires a files, lines, contextual or baseline scope\n\n"
        + "Usage: rust-doctor [OPTIONS] [PATH] [COMMAND]\n\n" + HELP_HINT,
    );
  });

  test("AR-12 --staged and --include-untracked outside the scopes that take them are usage errors", async () => {
    const cases: Array<[string[], string]> = [
      [["--staged"], "error: --staged cannot judge the full codebase: pass --scope files, lines, contextual or baseline\n"],
      [["--include-untracked"], "error: --include-untracked requires a files, lines or contextual scope\n"],
      [["--scope", "full", "--staged"], "error: --staged cannot judge the full codebase: pass --scope files, lines, contextual or baseline\n"],
      [["--scope", "baseline", "--include-untracked"], "error: --include-untracked requires a files, lines or contextual scope\n"],
    ];
    for (const [args, message] of cases) {
      const run = await refused(["--json", ...args]);
      expect(run.stdout).toBe("");
      expect(run.stderr).toStartWith(message);
      expect(run.stderr).toContain("Usage: rust-doctor [OPTIONS] [PATH] [COMMAND]");
      expect(run.stderr).toEndWith(HELP_HINT);
    }
  });

  const unresolved: Array<[id: string, fixture: string, args: string[], error: ScanError]> = [
    // Outside git the hint to pass --base cannot help (AR-14); the catalog
    // records the wording, scopes C14 asserts the same code.
    ["AR-13", "projects/clean", ["--scope", "files"], { stage: "scope", code: "base-undetected", message: "No base branch found: pass --base <REF>." }],
    ["AR-14", "projects/clean", ["--scope", "files", "--base", "HEAD"], { stage: "scope", code: "base-unavailable", message: "Git base commit is unavailable." }],
    ["AR-15", "projects/clean", ["--package", "nope"], { stage: "policy", code: "unknown-package", message: "--package names no workspace member. Members: clean." }],
    ["AR-16", "scan-failures/broken-member", ["--package", "broken-a", "--package", "nope"], {
      stage: "policy",
      code: "unknown-package",
      message: "--package names no workspace member. Members: broken-a, dependent-c, healthy-b.",
    }],
  ];
  test.each(unresolved)("%s fails with one JSON error before Clippy runs", async (_id, fixture, args, error) => {
    const run = await refused(["--json", ...args], fixture);
    const report = run.json as Report;
    expect(run.stderr).toBe("");
    expect(report.status).toBe("failed");
    expect(report.errors).toEqual([error]);
    expect(report.audit.score).toBeNull();
  });

  test("AR-17 --package naming the only member scans it with a partial score", async () => {
    const run = await scan(copyWorkspace("projects/clean"), ["--json", "--package", "clean"]);
    expect(run.exitCode).toBe(0);
    expect(run.report.status).toBe("complete");
    expect(run.report.audit.score).toMatchObject({ value: 100, authoritative: false, reasons: ["rules-not-evaluated"] });
    const repoRules = run.report.policy!.rules.filter((rule) => rule.id.startsWith("rust_doctor::repo::"));
    expect(repoRules.length).toBeGreaterThan(0);
    for (const rule of repoRules) expect(rule.not_evaluated).toBe("package-scoped");
  });

  test("AR-18 the historical inspect alias prints the same report as the root form", async () => {
    const workspace = copyWorkspace("projects/clippy-warning");
    const box = sandbox("alias");
    const target = scratchDir("target");
    const before = treeFingerprint(workspace);
    const root = await runDoctor([workspace, "--yes", "--json"], { sandbox: box, target });
    const alias = await runDoctor(["inspect", workspace, "--yes", "--json"], { sandbox: box, target });
    expect(treeFingerprint(workspace)).toBe(before);
    expect(alias.exitCode).toBe(root.exitCode);
    expect(root.stdout.length).toBeGreaterThan(0);
    expect(alias.stdout).toBe(root.stdout);
  });
});
