// `tasks/e2e-catalog/failures.md` section PG: a repository whose own git
// configuration names programs (fsmonitor, clean and smudge filters, a diff
// driver, an external diff, a pager, hooks), each of which writes its name to
// a log. Modeled on pf's poisoned-repository.test.ts. A control first proves
// plain git runs them; no scope runs any of them, since every filter driver
// the configuration names is neutralized and hooks are switched off (EP-001
// of the security audit remediation).
import { describe, expect, test } from "bun:test";
import { chmodSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { runCommand, runDoctor } from "./e2e-helpers";
import { gitScenario, miniTemplate, NEW_FN, type Repo } from "./git-scenario";
import { spawnPty } from "./pty";

type Poisoned = Repo & { log: string };

const FILTERS = { clean: "cat", smudge: "cat", textconv: 'cat "$1"', pager: "cat", external: "exit 0", fsmonitor: "exit 0" };
const HOOKS = ["post-checkout", "post-index-change", "reference-transaction", "pre-commit"];
const NEVER = ["fsmonitor", "textconv", "pager", "external"];

function program(path: string, name: string, log: string, body: string): void {
  writeFileSync(path, `#!/bin/sh\necho ${name} >> '${log}'\n${body}\n`);
  chmodSync(path, 0o755);
}

// `mini` with every `.rs` file under the planted filter and diff driver, and
// NEWFN appended to `src/lib.rs`, staged or not. The poison is configured
// after the history is built, so building it ran none of the programs.
async function poisoned(name: string, change: "unstaged" | "staged"): Promise<Poisoned> {
  const lib = miniTemplate()["src/lib.rs"]! + NEW_FN;
  const scenario = gitScenario(name).template("mini").write({ ".gitattributes": "*.rs filter=planted diff=planted\n" }).commit("base");
  const repo = await (change === "staged" ? scenario.stage({ write: { "src/lib.rs": lib } }) : scenario.unstaged({ "src/lib.rs": lib })).build();
  const bin = join(repo.sandbox.dir, "poison");
  const hooks = join(bin, "hooks");
  mkdirSync(hooks, { recursive: true });
  const log = join(repo.sandbox.dir, "poison.log");
  for (const [name, body] of Object.entries(FILTERS)) program(join(bin, name), name, log, body);
  for (const hook of HOOKS) program(join(hooks, hook), hook, log, "exit 0");
  for (const [key, value] of [
    ["core.fsmonitor", join(bin, "fsmonitor")],
    ["core.hooksPath", hooks],
    ["core.pager", join(bin, "pager")],
    ["filter.planted.clean", join(bin, "clean")],
    ["filter.planted.smudge", join(bin, "smudge")],
    ["diff.planted.textconv", join(bin, "textconv")],
    ["diff.external", join(bin, "external")],
  ] as const) {
    await repo.git(["config", key, value]);
  }
  return Object.assign(repo, { log });
}

function ran(repo: Poisoned): string[] {
  return existsSync(repo.log) ? readFileSync(repo.log, "utf8").split("\n").filter(Boolean) : [];
}

// The control: plain git, in the same sealed environment, runs every program
// a scope must never run. Then the log is emptied for the scan.
async function controlled(repo: Poisoned): Promise<void> {
  const status = await runCommand(["git", "status", "--short"], { cwd: repo.path, sandbox: repo.sandbox });
  expect(status.exitCode).toBe(0);
  const diff = await runCommand(["git", "-c", "core.pager=", "diff", "--textconv", "HEAD", "--", "src/lib.rs"], { cwd: repo.path, sandbox: repo.sandbox });
  expect(diff.exitCode).toBe(0);
  // Without the external diff, git converts the content through textconv.
  const textconv = await runCommand(["git", "-c", "core.pager=", "diff", "--no-ext-diff", "--textconv", "HEAD", "--", "src/lib.rs"], { cwd: repo.path, sandbox: repo.sandbox });
  expect(textconv.exitCode).toBe(0);
  // git pages only onto a terminal.
  const paged = await spawnPty(["git", "log", "-1"], { cwd: repo.path, sandbox: repo.sandbox }).exited;
  expect(paged.exitCode).toBe(0);
  for (const name of NEVER) expect(ran(repo)).toContain(name);
  writeFileSync(repo.log, "");
}

function counts(names: string[]): Record<string, number> {
  const out: Record<string, number> = {};
  for (const name of names) out[name] = (out[name] ?? 0) + 1;
  return out;
}

async function scanPoisoned(repo: Poisoned, args: string[]): Promise<Record<string, number>> {
  await controlled(repo);
  const run = await runDoctor([repo.path, "--yes", "--json", ...args], { sandbox: repo.sandbox, env: { RUST_DOCTOR_NO_CACHE: "1" } });
  const report = run.json as { status: string };
  expect(report.status).toBe("complete");
  return counts(ran(repo));
}

// How often a filter runs follows how many files and git calls a scope makes,
// so a row names which programs run, never how many times.
describe("poisoned repository configuration", () => {
  test("PG-01 a full scan runs none of the repository's programs", async () => {
    expect(await scanPoisoned(await poisoned("pg-01", "unstaged"), [])).toEqual({});
  });

  test("PG-02 a files scan runs none of the repository's programs", async () => {
    expect(Object.keys(await scanPoisoned(await poisoned("pg-02", "unstaged"), ["--scope", "files", "--base", "HEAD"]))).toEqual([]);
  });

  test("PG-03 a lines scan runs none of the repository's programs", async () => {
    expect(Object.keys(await scanPoisoned(await poisoned("pg-03", "unstaged"), ["--scope", "lines", "--base", "HEAD"]))).toEqual([]);
  });

  test("PG-04 a baseline scan runs none of the repository's programs", async () => {
    const programs = await scanPoisoned(await poisoned("pg-04", "unstaged"), ["--scope", "baseline", "--base", "HEAD"]);
    expect(Object.keys(programs)).toEqual([]);
  });

  test("PG-05 a staged baseline scan runs none of the repository's programs", async () => {
    const programs = await scanPoisoned(await poisoned("pg-05", "staged"), ["--scope", "baseline", "--base", "HEAD", "--staged"]);
    expect(Object.keys(programs)).toEqual([]);
  });

  test("PG-06 no scope runs the fsmonitor, the textconv driver, the pager or the external diff", async () => {
    for (const [change, args] of [
      ["unstaged", []],
      ["unstaged", ["--scope", "files", "--base", "HEAD"]],
      ["unstaged", ["--scope", "lines", "--base", "HEAD"]],
      ["unstaged", ["--scope", "contextual", "--base", "HEAD"]],
      ["unstaged", ["--scope", "baseline", "--base", "HEAD"]],
      ["staged", ["--staged", "--scope", "lines"]],
      ["staged", ["--scope", "baseline", "--base", "HEAD", "--staged"]],
    ] as const) {
      const programs = await scanPoisoned(await poisoned(`pg-06-${change}`, change), [...args]);
      for (const name of NEVER) expect(programs[name]).toBeUndefined();
    }
  });
});
