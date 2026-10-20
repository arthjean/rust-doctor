// `tasks/e2e-catalog/failures.md` section EN: what the terminal profile makes
// of the streams and the environment. Every terminal run goes through
// pty.ts; a run that opens the interactive report is told apart from a linear
// one by the menu hint, and quits with `q` once it shows.
import { beforeAll, describe, expect, test } from "bun:test";
import { writeFileSync } from "node:fs";
import { join } from "node:path";
import {
  BIN,
  copyWorkspace,
  NON_INTERACTIVE_MARKERS,
  normalizeReport,
  runDoctor,
  sandbox,
  scratchDir,
  targetDir,
  type EnvOverrides,
} from "./e2e-helpers";
import { gitScenario } from "./git-scenario";
import { spawnPty, type PtyExit } from "./pty";
import { MENU_HINT } from "./tmux-helpers";

const SGR = /\x1b\[[0-9;]*m/g;
const ERASE = /\r\x1b\[2K/g;
const REWIND = /\x1b\[\d+A/g;
const LINEAR = "Scope: full codebase";
const PHASES = ["Running native passes...", "Compiling dependencies...", "Linting clippy-warning (1/1)"];

let workspace = "";
let target = "";

beforeAll(async () => {
  workspace = copyWorkspace("projects/clippy-warning");
  target = targetDir("clippy-warning");
  const warm = await runDoctor([workspace, "--yes", "--json"], { target });
  expect(warm.exitCode).toBe(0);
});

type Terminal = PtyExit & { interactive: boolean };

// The binary on a terminal of `cols` columns, `argv` around it when the test
// needs a shell between the two. An interactive report is quit with `q` once
// its menu shows; a linear one runs to its end.
async function onTerminal(
  args: string[],
  options: { cols?: number; env?: EnvOverrides; wrap?: (command: string[]) => string[] } = {},
): Promise<Terminal> {
  const command = [BIN, ...args, "--advisory-db", "/dev/null"];
  const session = spawnPty(options.wrap ? options.wrap(command) : command, {
    cols: options.cols ?? 100,
    rows: 40,
    env: { CARGO_TARGET_DIR: target, ...options.env },
  });
  let interactive = false;
  let done = false;
  const exited = session.exited.finally(() => (done = true));
  while (!done) {
    if (!interactive && session.output().includes(MENU_HINT)) {
      interactive = true;
      session.write("q");
    }
    await Bun.sleep(25);
  }
  return { ...(await exited), interactive };
}

function scan(env: EnvOverrides = {}, args: string[] = []): Promise<Terminal> {
  return onTerminal([workspace, ...args], { env });
}

function visibleLines(output: string): string[] {
  const report = output.slice(output.indexOf(LINEAR));
  return report.replace(/\x1b\[[0-9;?]*[A-Za-z]/g, "").replace(/\x1b\]8;;[^\x07\x1b]*(\x07|\x1b\\)/g, "").split("\n").map((line) => line.replace(/\r/g, ""));
}

function widest(output: string): number {
  return Math.max(...visibleLines(output).filter((line) => !line.includes("https://")).map((line) => [...line].length));
}

const HELP = "Help: Propagate the error with ? or provide a default instead of panicking on the absent value.";

describe("terminal profile", () => {
  test("EN-01 --yes on a terminal prints the colored linear report, rewrites progress and draws no frame", async () => {
    const run = await scan({}, ["--yes"]);
    expect(run.exitCode).toBe(0);
    expect(run.interactive).toBe(false);
    expect(run.output).toContain(LINEAR);
    expect((run.output.match(SGR) ?? []).length).toBeGreaterThan(0);
    expect((run.output.match(ERASE) ?? []).length).toBeGreaterThanOrEqual(PHASES.length);
    expect(run.output.match(REWIND)).toBeNull();
  });

  test.each([
    ["1", 0],
    ["", 1],
  ] as const)("EN-02 NO_COLOR=%p turns color off only when it is not empty", async (value, minimum) => {
    const run = await scan({ NO_COLOR: value }, ["--yes"]);
    expect(run.exitCode).toBe(0);
    const sgr = (run.output.match(SGR) ?? []).length;
    if (minimum === 0) expect(sgr).toBe(0);
    else expect(sgr).toBeGreaterThan(0);
  });

  test("EN-03 TERM=dumb prints no color, no rewritten progress, at 80 columns", async () => {
    const run = await scan({ TERM: "dumb" }, ["--yes"]);
    expect(run.exitCode).toBe(0);
    expect(run.output.match(SGR)).toBeNull();
    expect(run.output.match(ERASE)).toBeNull();
    expect(widest(run.output)).toBeLessThanOrEqual(80);
    expect(visibleLines(run.output)).not.toContain(HELP);
  });

  test.each([
    [60, 60],
    [30, 40],
    [200, 200],
  ] as const)("EN-04 a %i-column terminal wraps at %i, and never cuts a URL line", async (cols, width) => {
    const run = await onTerminal([workspace, "--yes"], { cols });
    expect(run.exitCode).toBe(0);
    const lines = visibleLines(run.output);
    expect(widest(run.output)).toBeLessThanOrEqual(width);
    if (cols === 30) expect(widest(run.output)).toBeGreaterThan(30);
    if (cols === 200) expect(lines).toContain(HELP);
    else expect(lines).not.toContain(HELP);
    expect(lines.some((line) => line.endsWith("https://rust-doctor.com/rules/clippy%3A%3Aunwrap_used"))).toBe(true);
  });

  test("EN-05 a terminal reporting zero columns takes COLUMNS", async () => {
    const run = await onTerminal([workspace, "--yes"], {
      env: { COLUMNS: "70" },
      wrap: (command) => ["sh", "-c", 'stty cols 0 && exec "$@"', "sh", ...command],
    });
    expect(run.exitCode).toBe(0);
    expect(widest(run.output)).toBeLessThanOrEqual(70);
    expect(widest(run.output)).toBeGreaterThan(60);
  });

  test("EN-06 COLUMNS is ignored off a terminal", async () => {
    const run = await runDoctor([workspace, "--yes"], { target, env: { COLUMNS: "120" } });
    expect(run.exitCode).toBe(0);
    const lines = visibleLines(run.stdout);
    expect(widest(run.stdout)).toBeLessThanOrEqual(80);
    expect(lines).not.toContain(HELP);
  });

  test("EN-07 without --yes a terminal opens the interactive report, and q leaves with the scan's code", async () => {
    const run = await scan();
    expect(run.interactive).toBe(true);
    expect(run.exitCode).toBe(0);
  });

  test("EN-08 q leaves the interactive report with the code --blocking warning gives the scan", async () => {
    const run = await scan({}, ["--blocking", "warning"]);
    expect(run.interactive).toBe(true);
    expect(run.exitCode).toBe(1);
  });

  test.each(["true", "no"])("EN-09 CI=%s keeps the linear report on a terminal", async (value) => {
    const run = await scan({ CI: value });
    expect(run.interactive).toBe(false);
    expect(run.output).toContain(LINEAR);
    expect(run.exitCode).toBe(0);
  });

  test.each(["false", "FALSE", "0", ""])("EN-10 CI=%p is not a CI run: the interactive report opens", async (value) => {
    const run = await scan({ CI: value });
    expect(run.interactive).toBe(true);
    expect(run.exitCode).toBe(0);
  });

  test.each(NON_INTERACTIVE_MARKERS)("EN-11 %s=1 keeps the linear report on a terminal", async (marker) => {
    const run = await scan({ [marker]: "1" });
    expect(run.interactive).toBe(false);
    expect(run.output).toContain(LINEAR);
    expect(run.exitCode).toBe(0);
  });

  test("EN-12 an empty agent marker counts as unset: the interactive report opens", async () => {
    const run = await scan({ CLAUDECODE: "" });
    expect(run.interactive).toBe(true);
    expect(run.exitCode).toBe(0);
  });

  test("EN-13 the three agent markers name their missing skill once on stderr, and no other marker does", async () => {
    const agents: Record<string, string> = { CLAUDECODE: "claude", CODEX_SANDBOX: "codex", CURSOR_AGENT: "cursor" };
    for (const marker of NON_INTERACTIVE_MARKERS) {
      const run = await runDoctor([workspace, "--yes"], { target, env: { [marker]: "1" } });
      expect(run.exitCode).toBe(0);
      const agent = agents[marker];
      const notices = run.stderr.split("\n").filter((line) => line.startsWith("rust-doctor:"));
      if (agent) {
        expect(notices).toEqual([
          `rust-doctor: no rust-doctor skill is installed for ${agent}; run \`rust-doctor skill install --agent ${agent}\`.`,
        ]);
        expect(run.stderr.trimEnd().endsWith(notices[0]!)).toBe(true);
      } else {
        expect(notices).toEqual([]);
      }
    }
  });

  test("EN-14 an agent marker leaves a --json run's stderr empty and its report unchanged", async () => {
    const plain = await runDoctor([workspace, "--yes", "--json"], { target });
    const agent = await runDoctor([workspace, "--yes", "--json"], { target, env: { CLAUDECODE: "1" } });
    expect(agent.exitCode).toBe(0);
    expect(agent.stderr).toBe("");
    expect(normalizeReport(agent.json)).toEqual(normalizeReport(plain.json));
  });

  test("EN-15 TERM=dumb without --yes prints the linear report without color", async () => {
    const run = await scan({ TERM: "dumb" });
    expect(run.interactive).toBe(false);
    expect(run.output).toContain(LINEAR);
    expect(run.output.match(SGR)).toBeNull();
  });

  test("EN-16 --verbose on a terminal keeps the linear report, and --yes beside it draws no frame", async () => {
    const verbose = await scan({}, ["--verbose"]);
    expect(verbose.interactive).toBe(false);
    expect(verbose.exitCode).toBe(0);
    expect(verbose.output).toContain(LINEAR);
    const yes = await scan({}, ["--verbose", "--yes"]);
    expect(yes.exitCode).toBe(0);
    expect(yes.output.match(REWIND)).toBeNull();
  });

  test("EN-17 a failed scan on a terminal prints the linear failure and exits 2", async () => {
    const dir = scratchDir("not-cargo");
    writeFileSync(join(dir, "README"), "not a crate\n");
    const run = await onTerminal([dir]);
    expect(run.interactive).toBe(false);
    expect(run.output).toContain("Scan failed:");
    expect(run.exitCode).toBe(2);
  });

  test("EN-18 without --yes, a redirected stdin and stdout get the linear report and no wait", async () => {
    const run = await runDoctor([workspace], { target, timeoutMs: 20_000 });
    expect(run.exitCode).toBe(0);
    expect(run.stdout).toContain(LINEAR);
    expect(run.stdout).not.toContain(MENU_HINT);
  });

  test("EN-19 off a terminal, progress is one line per phase on stderr", async () => {
    const run = await runDoctor([workspace, "--yes"], { target });
    expect(run.exitCode).toBe(0);
    expect(run.stderr.split("\n").filter(Boolean)).toEqual(PHASES);
  });

  test("EN-20 a --json run prints no progress", async () => {
    const run = await runDoctor([workspace, "--yes", "--json"], { target });
    expect(run.exitCode).toBe(0);
    expect(run.stderr).toBe("");
  });

  test("EN-21 a GIT_DIR the caller exported does not reach the repository pass", async () => {
    const repo = await gitScenario("en-21")
      .fixture("projects/clippy-warning")
      .write({ ".gitignore": "/target\n", ".env": "MODE=dev\n" })
      .commit("base")
      .build();
    const run = await runDoctor([repo.path, "--yes", "--json"], { sandbox: repo.sandbox, env: { GIT_DIR: "/nonexistent" } });
    const report = run.json as { status: string; errors: unknown[]; diagnostics: Array<{ code: string | null; path: string | null }> };
    expect(report.status).toBe("complete");
    expect(report.errors).toEqual([]);
    expect(report.diagnostics.some((entry) => entry.code === "rust_doctor::repo::tracked_secret_file" && entry.path === ".env")).toBe(true);
  });

  test("EN-22 with stderr redirected, a terminal on stdin and stdout still opens the interactive report", async () => {
    const box = sandbox("en-22");
    const stderr = join(box.dir, "stderr.txt");
    const run = await onTerminal([workspace], {
      wrap: (command) => ["sh", "-c", 'exec "$@" 2>"$RD_STDERR"', "sh", ...command],
      env: { RD_STDERR: stderr },
    });
    expect(run.interactive).toBe(true);
    expect(run.exitCode).toBe(0);
    expect(await Bun.file(stderr).text()).toContain("Linting clippy-warning (1/1)");
  });
});
