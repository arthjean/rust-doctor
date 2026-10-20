// `tasks/e2e-catalog/scopes.md` sections A and A2: scope arguments clap
// refuses and the closed base selector grammar. Every refusal runs with
// neither cargo nor git on PATH, so a row that passes proves the argument was
// refused before either could start. The rows whose selector gets past the
// grammar (A12, A40 to A49) run with the toolchain: discovery runs `cargo
// metadata` before any base is resolved, so without cargo they would only
// ever report `execution/cargo-unavailable`.
import { beforeAll, describe, expect, test } from "bun:test";
import { unlinkSync } from "node:fs";
import { join } from "node:path";
import { pathWithout, runDoctor, treeFingerprint } from "./e2e-helpers";
import { gitScenario, type Repo } from "./git-scenario";
import { errorCodes, MINI_BASE, scan, scopeLine, sha256Mini } from "./scope-scan-support";

// The suite's tool directory without cargo and without git.
function pathWithoutCargoOrGit(): string {
  const dir = pathWithout("cargo");
  unlinkSync(join(dir, "git"));
  return dir;
}

let repo: Repo;
let bare: string;

beforeAll(async () => {
  repo = await gitScenario("arguments").template("mini").commit("base").build();
  bare = pathWithoutCargoOrGit();
});

const BASE = "error: --base <REF> requires a files, lines, contextual or baseline scope";
const STAGED = "error: --staged cannot judge the full codebase: pass --scope files, lines, contextual or baseline";
const UNTRACKED = "error: --include-untracked requires a files, lines or contextual scope";
const UNTRACKED_STAGED = "error: --include-untracked cannot be combined with --staged: the index holds no untracked file";

describe("scope arguments clap refuses", () => {
  test.each([
    ["A01", ["--base", "main"], BASE],
    ["A02", ["--scope", "full", "--base", "main"], BASE],
    ["A03", ["--staged"], STAGED],
    ["A04", ["--scope", "full", "--staged"], STAGED],
    ["A05", ["--include-untracked"], UNTRACKED],
    ["A06", ["--scope", "baseline", "--include-untracked"], UNTRACKED],
    ["A07", ["--scope", "lines", "--staged", "--include-untracked"], UNTRACKED_STAGED],
    ["A08", ["--scope", "files", "--staged", "--include-untracked"], UNTRACKED_STAGED],
    ["A09", ["--scope", "nope"], "error: invalid value 'nope' for '--scope <SCOPE>'"],
    ["A10", ["--scope"], "error: a value is required for '--scope <SCOPE>'"],
    ["A11", ["--base"], "error: a value is required for '--base <REF>'"],
    ["A13", ["--scope", "full", "--base", "main", "--staged"], BASE],
    ["A14", ["--scope", "contextual", "--include-untracked", "--staged"], UNTRACKED_STAGED],
  ] as const)("%s %j exits 2 with one error line and no report", async (_id, args, message) => {
    const before = treeFingerprint(repo.path);
    const run = await runDoctor([repo.path, "--yes", "--json", ...args], { env: { PATH: bare } });
    expect(run.exitCode).toBe(2);
    expect(run.stdout).toBe("");
    expect(run.stderr.split("\n")[0]).toStartWith(message);
    expect(run.stderr.split("\n").filter((line) => line.startsWith("error:"))).toHaveLength(1);
    expect(treeFingerprint(repo.path)).toBe(before);
  });

  test("A12 --scope full judges the whole codebase", async () => {
    const { run, report } = await scan(repo.path, ["--scope", "full"]);
    expect(run.exitCode).toBe(0);
    expect(report.scope).toMatchObject({
      mode: "full",
      comparison_base: null,
      base_ref: null,
      files: null,
      staged: false,
      untracked_unreported: null,
    });
    expect(await scopeLine(repo.path, ["--scope", "full"])).toBe("Scope: full codebase");
  });
});

describe("base selector grammar", () => {
  async function expectInvalidBase(path: string, args: string[]): Promise<void> {
    const before = treeFingerprint(repo.path);
    const run = await runDoctor([path, "--yes", "--json", ...args], { env: { PATH: bare } });
    expect(run.exitCode).toBe(2);
    const report = run.json as { status: string; scope: unknown; project: unknown; errors: unknown[] };
    expect(report.status).toBe("failed");
    expect(report.scope).toBeNull();
    expect(report.project).toBeNull();
    // The message is fixed: the refused selector is never echoed back.
    expect(report.errors).toEqual([{ stage: "scope", code: "invalid-base", message: "Invalid Git base selector." }]);
    const human = await runDoctor([path, "--yes", ...args], { env: { PATH: bare } });
    expect(human.exitCode).toBe(2);
    expect(human.stdout).toBe("Scope: files (not resolved)\nScan failed: Invalid Git base selector. (scope/invalid-base)\n");
    expect(human.stderr).toBe("");
    expect(treeFingerprint(repo.path)).toBe(before);
  }

  test.each([
    ["A20", "HEAD~1"],
    ["A21", "HEAD^"],
    ["A22", "main^{commit}"],
    ["A23", "-x"],
    ["A24", "--"],
    ["A25", "a..b"],
    ["A26", "a//b"],
    ["A27", ".hidden"],
    ["A28", "x.lock"],
    ["A29", "x."],
    ["A30", "@"],
    ["A31", "main@{1}"],
    ["A32", "café"],
    ["A33", "a b"],
    ["A34", "a:b"],
    ["A35", "refs/heads/"],
    ["A36", "x".repeat(256)],
    ["A37", ""],
  ])("%s --base=%j is refused before discovery", async (_id, base) => {
    await expectInvalidBase(repo.path, ["--scope", "files", `--base=${base}`]);
  });

  test("A38 a refused selector on a missing path is refused before discovery", async () => {
    await expectInvalidBase(join(repo.path, "nope"), ["--scope", "files", "--base=HEAD~1"]);
  });

  test("A39 a refused selector wins over an unknown package", async () => {
    await expectInvalidBase(repo.path, ["--scope", "files", "--base=HEAD~1", "--package", "nope"]);
  });

  test.each([
    ["A40", "refs/heads/main"],
    ["A41", "HEAD"],
    ["A42", "main"],
    ["A43", MINI_BASE.slice(0, 7)],
    ["A44", MINI_BASE],
    ["A45", MINI_BASE.slice(0, 39)],
  ])("%s --base=%s is accepted and resolved", async (_id, base) => {
    const { run, report } = await scan(repo.path, ["--scope", "files", `--base=${base}`]);
    expect(run.exitCode).toBe(0);
    expect(report.scope).toMatchObject({ mode: "files", comparison_base: MINI_BASE, base_ref: null, files: [] });
  });

  test.each([
    ["A46", "v1.0"],
    ["A47", "MAIN"],
    ["A48", "origin/main"],
  ])("%s --base=%s passes the grammar and names an unavailable base", async (_id, base) => {
    const { run, report } = await scan(repo.path, ["--scope", "files", `--base=${base}`]);
    expect(run.exitCode).toBe(2);
    expect(report.scope).toBeNull();
    expect(errorCodes(report)).toEqual(["scope/base-unavailable"]);
  });

  test("A49 a 64-hex object name is accepted in a SHA-256 repository", async () => {
    const sha256 = await sha256Mini("a49");
    const head = (await sha256.git(["rev-parse", "HEAD"])).trim();
    expect(head).toMatch(/^[0-9a-f]{64}$/);
    const { run, report } = await scan(sha256.path, ["--scope", "files", `--base=${head}`], { sandbox: sha256.sandbox });
    expect(run.exitCode).toBe(0);
    expect(report.scope).toMatchObject({ mode: "files", comparison_base: head, base_ref: null, files: [] });
  });
});
