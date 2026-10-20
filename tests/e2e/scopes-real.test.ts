// `tasks/e2e-catalog/real-codebases.md` section 2, invariant 13, and section R
// of `tasks/e2e-catalog/scopes.md`: every scope on a clone of this repository,
// clean and with one seeded change. The clone's `origin/main` is pinned to the
// commit it was cloned at, so a detected base is the same on every machine and
// whatever branch the working copy is on. Only the seeded findings are named:
// what this repository already holds is compared, never counted.
import { beforeAll, describe, expect, test } from "bun:test";
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { cloneThisRepository, type SelfClone } from "./self-clone";
import {
  introduced,
  report,
  scanSelf,
  seedUnwrap,
  SEEDED_PATH,
  TEST_TIMEOUT_MS,
  type Report,
} from "./self-scan-support";

let clone: SelfClone;
let base: string;
let target: string;

beforeAll(async () => {
  clone = await cloneThisRepository("scopes-real");
  target = clone.target("scan");
  base = await clone.sha();
  await clone.git(["update-ref", "refs/remotes/origin/main", base]);
  await clone.git(["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/main"]);
  await clone.git(["switch", "-q", "-c", "e2e-base"]);
}, TEST_TIMEOUT_MS);

const branches = new Map<string, Promise<void>>();

// Checks out `name`, creating it from the base commit with `change` the first
// time it is asked for.
async function onBranch(name: string, change?: () => Promise<void>): Promise<void> {
  await clone.git(["switch", "-q", "e2e-base"]);
  if (name === "e2e-base") return;
  if (!branches.has(name)) {
    branches.set(name, (async () => {
      if (change) await change();
      else await clone.git(["switch", "-q", "-c", name]);
    })());
    await branches.get(name);
    return;
  }
  await clone.git(["switch", "-q", name]);
}

async function scan(args: string[], env?: Record<string, string>): Promise<{ exitCode: number | null; report: Report; stdout: string }> {
  const run = await scanSelf(clone, ["--json", ...args], { target, env });
  return { exitCode: run.exitCode, report: report(run), stdout: run.stdout };
}

// R04's change: a private function holding a `todo!()` appended to
// `src/git.rs`, which rustc also reports as dead code.
const TODO_PATH = "src/git.rs";
let todoLine = 0;

async function todoBranch(): Promise<void> {
  await onBranch("e2e-todo", async () => {
    await clone.git(["switch", "-q", "-c", "e2e-todo"]);
    const file = join(clone.path, TODO_PATH);
    const before = readFileSync(file, "utf8");
    writeFileSync(file, `${before}\nfn e2e_todo() -> u8 {\n    todo!()\n}\n`);
    todoLine = before.split("\n").length + 2;
    await clone.git(["commit", "-q", "-am", "seed a todo"]);
  });
}

// The findings a scan places on the seeded function.
function seeded(value: Report, path: string, from: number): Array<[string, number]> {
  return value.diagnostics
    .filter((diagnostic) => diagnostic.path === path && (diagnostic.span?.line_start ?? 0) >= from - 1)
    .map((diagnostic): [string, number] => [diagnostic.code, diagnostic.span!.line_start])
    .sort();
}

const TODO_FINDINGS = () => [["clippy::todo", todoLine], ["dead_code", todoLine - 1]].sort() as Array<[string, number]>;

describe("scopes on this repository", () => {
  test("INV-13 on a clean tree files and lines publish nothing, and a baseline of HEAD against itself introduces and fixes nothing", async () => {
    await onBranch("e2e-base");
    for (const scope of ["files", "lines"]) {
      const run = await scan(["--scope", scope, "--base", "HEAD"]);
      expect(run.exitCode).toBe(0);
      expect(run.report.status).toBe("complete");
      expect(run.report.diagnostics).toEqual([]);
    }
    const baseline = await scan(["--scope", "baseline", "--base", base]);
    expect(baseline.exitCode).toBe(0);
    expect(baseline.report.status).toBe("complete");
    expect(baseline.report.delta?.summary).toMatchObject({ introduced: 0, fixed: 0 });
  }, TEST_TIMEOUT_MS);

  test("a seeded unwrap() committed on a scratch branch is the one finding a baseline introduces", async () => {
    let line = 0;
    await onBranch("e2e-seeded", async () => {
      line = await seedUnwrap(clone, "e2e-seeded");
    });
    const run = await scan(["--scope", "baseline", "--base", base]);
    expect(run.exitCode).toBe(0);
    expect(run.report.delta?.summary).toMatchObject({ introduced: 1, fixed: 0 });
    expect(introduced(run.report).map((diagnostic) => [diagnostic.code, diagnostic.path, diagnostic.span?.line_start]))
      .toEqual([["clippy::unwrap_used", SEEDED_PATH, line]]);
  }, TEST_TIMEOUT_MS);

  test("R01 the full scan of the main line is complete and passes its gate", async () => {
    await onBranch("e2e-base");
    const run = await scan([]);
    expect(run.exitCode).toBe(0);
    expect(run.report.status).toBe("complete");
    expect(run.report.gate.status).toBe("passed");
    expect(run.report.scope).toMatchObject({ mode: "full", comparison_base: null });
  }, TEST_TIMEOUT_MS);

  test.each([
    ["R02", "files"],
    ["R02b", "lines"],
  ])("%s a branch with no change selects no file against the detected origin/main (--scope %s)", async (_id, scope) => {
    await onBranch("e2e-noop");
    const run = await scan(["--scope", scope]);
    expect(run.exitCode).toBe(0);
    expect(run.report.scope).toMatchObject({ mode: scope, base_ref: "origin/main", comparison_base: base, files: [] });
    expect(run.report.diagnostics).toEqual([]);
  }, TEST_TIMEOUT_MS);

  test("R03 a baseline of a branch with no change introduces and fixes nothing, every finding pre-existing", async () => {
    await onBranch("e2e-noop");
    const full = await scan([]);
    const run = await scan(["--scope", "baseline"]);
    expect(run.exitCode).toBe(0);
    expect(run.report.scope).toMatchObject({ mode: "baseline", base_ref: "origin/main", comparison_base: base });
    expect(run.report.delta?.summary).toMatchObject({ introduced: 0, fixed: 0, pre_existing: full.report.summary.total });
  }, TEST_TIMEOUT_MS);

  test("R03c the base side rescanned without its cache gives the same delta", async () => {
    await onBranch("e2e-noop");
    const delta = (await scan(["--scope", "baseline"])).report.delta!.summary;
    const fresh = await scanSelf(clone, ["--scope", "baseline", "--verbose"], { target, env: { RUST_DOCTOR_NO_CACHE: "1" } });
    expect(fresh.exitCode).toBe(0);
    expect(fresh.stdout).toContain("Base side cache: disabled");
    expect(fresh.stdout).toContain(`Delta: +${delta.introduced} introduced; =${delta.pre_existing} pre-existing; -${delta.fixed} fixed`);
  }, TEST_TIMEOUT_MS);

  test.each([
    ["R04", "files"],
    ["R04b", "lines"],
    ["R04c", "contextual"],
  ])("%s a committed todo!() function is what --scope %s reports, in the one file it touched", async (_id, scope) => {
    await todoBranch();
    const run = await scan(["--scope", scope]);
    expect(run.exitCode).toBe(0);
    expect(run.report.scope).toMatchObject({ mode: scope, comparison_base: base, files: [TODO_PATH] });
    expect(seeded(run.report, TODO_PATH, todoLine - 1)).toEqual(TODO_FINDINGS());
    expect(run.report.diagnostics.every((diagnostic) => diagnostic.path === TODO_PATH)).toBe(true);
  }, TEST_TIMEOUT_MS);

  test("R04d a baseline introduces exactly the seeded todo!() and its dead code", async () => {
    await todoBranch();
    const run = await scan(["--scope", "baseline"]);
    expect(run.exitCode).toBe(0);
    expect(run.report.delta?.summary).toMatchObject({ introduced: 2, fixed: 0 });
    expect(introduced(run.report).map((diagnostic): [string, number] => [diagnostic.code, diagnostic.span!.line_start]).sort())
      .toEqual(TODO_FINDINGS());
  }, TEST_TIMEOUT_MS);

  // A shallow checkout carries no tag: CI checks out with full history.
  const hasTag = () => clone.git(["tag", "--list", "v0.7.0"]).then((out) => out.trim() === "v0.7.0");

  test("R04e a baseline against the v0.7.0 tag completes and counts the seeded todo!() as introduced", async () => {
    if (!(await hasTag())) throw new Error("the clone carries no v0.7.0 tag: check out with full history (fetch-depth: 0)");
    await todoBranch();
    const run = await scan(["--scope", "baseline", "--base", "v0.7.0"]);
    expect(run.exitCode).toBe(0);
    const tag = await clone.sha("v0.7.0^{commit}");
    expect(run.report.scope).toMatchObject({ mode: "baseline", comparison_base: tag });
    expect(run.report.delta).not.toBeNull();
    const codes = introduced(run.report).filter((diagnostic) => diagnostic.path === TODO_PATH && diagnostic.span?.line_start === todoLine);
    expect(codes.map((diagnostic) => diagnostic.code)).toEqual(["clippy::todo"]);
    expect(run.report.errors.every((error) => error.stage === "delta")).toBe(true);
  }, TEST_TIMEOUT_MS);

  test("R04f --scope files against v0.7.0 selects exactly the files git reports changed since the tag, the seeded one among them", async () => {
    if (!(await hasTag())) throw new Error("the clone carries no v0.7.0 tag: check out with full history (fetch-depth: 0)");
    await todoBranch();
    const run = await scan(["--scope", "files", "--base", "v0.7.0"]);
    expect(run.exitCode).toBe(0);
    const changed = (await clone.git(["diff", "--name-only", "--diff-filter=d", "v0.7.0", "HEAD"])).split("\n").filter(Boolean).sort();
    const files = (run.report.scope?.files as string[]).slice().sort();
    expect(files).toContain(TODO_PATH);
    expect(files).toEqual(changed);
  }, TEST_TIMEOUT_MS);

  // A pub function in a public module: the staged todo!() is the only
  // finding the staged change can carry.
  test.each([
    ["R05", "lines"],
    ["R05b", "baseline"],
  ])("%s a staged todo!() is judged by --scope %s --staged", async (_id, scope) => {
    await onBranch("e2e-staged");
    const file = join(clone.path, SEEDED_PATH);
    const before = readFileSync(file, "utf8");
    writeFileSync(file, `${before}\npub fn e2e_staged() -> u8 {\n    todo!()\n}\n`);
    const line = before.split("\n").length + 2;
    await clone.git(["add", "--", SEEDED_PATH]);
    try {
      const run = await scan(["--scope", scope, "--staged", "--base", base]);
      expect(run.exitCode).toBe(0);
      expect(run.report.scope).toMatchObject({ mode: scope, staged: true });
      const found = run.report.diagnostics.filter((diagnostic) => diagnostic.path === SEEDED_PATH && diagnostic.span?.line_start === line);
      expect(found.map((diagnostic) => diagnostic.code)).toEqual(["clippy::todo"]);
      if (scope === "baseline") expect(introduced(run.report).map((diagnostic) => diagnostic.code)).toEqual(["clippy::todo"]);
    } finally {
      await clone.git(["reset", "-q", "--hard"]);
    }
  }, TEST_TIMEOUT_MS);
});
