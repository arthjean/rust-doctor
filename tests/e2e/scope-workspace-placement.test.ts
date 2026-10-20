// `tasks/e2e-catalog/scopes.md` section G with H06: where the Cargo workspace
// sits in its repository. A workspace below the root, beside another, inside
// a nested repository, a submodule or a linked worktree, under a path with a
// space or reached through a symlink.
import { describe, expect, test } from "bun:test";
import { symlinkSync } from "node:fs";
import { dirname, join } from "node:path";
import { runCommand, type Sandbox } from "./e2e-helpers";
import { gitScenario, miniTemplate, NEW_FN } from "./git-scenario";
import { deltaSummary, errorCodes, MINI_BASE, scan, sites, writeFiles, type Report } from "./scope-scan-support";

const LIB = miniTemplate()["src/lib.rs"]!;

function prefixed(dir: string): Record<string, string> {
  return Object.fromEntries(Object.entries(miniTemplate()).map(([path, body]) => [join(dir, path), body]));
}

// `nested`: the workspace at `crates/app`, and a branch that edits it, the
// root README and the stray `top.rs`.
function g01() {
  return gitScenario("g01")
    .nested("crates/app", "mini").commit("base")
    .branch("feature")
    .append("crates/app/src/lib.rs", NEW_FN)
    .append("README.md", "More.\n")
    .append("top.rs", "fn more() {}\n")
    .commit("feature")
    .build();
}

describe("a workspace below the repository root", () => {
  test.each([
    ["G01", "crates/app", ["--scope", "files"]],
    ["G01e", ".", ["--scope", "files"]],
    ["G01f", "crates/app/src", ["--scope", "files"]],
  ] as const)("%s from %s selects only the workspace's change, workspace-relative", async (id, at, args) => {
    const repo = await g01();
    const { run, report } = await scan(join(repo.path, at), [...args], { sandbox: repo.sandbox, tree: repo.path });
    expect(run.exitCode).toBe(0);
    expect(report.scope?.files).toEqual(["src/lib.rs"]);
    expect(sites(report)).toEqual(["src/lib.rs:12", "src/lib.rs:4"]);
    if (id === "G01e") expect(run.stderr).toContain("No Cargo workspace at this path: scanning the one in crates/app/.");
  });

  test("G01b a lines scope keeps the branch's todo only", async () => {
    const repo = await g01();
    const { run, report } = await scan(join(repo.path, "crates/app"), ["--scope", "lines"], { sandbox: repo.sandbox, tree: repo.path });
    expect(run.exitCode).toBe(0);
    expect(sites(report)).toEqual(["src/lib.rs:12"]);
  });

  // Release 1 changed the catalog's D[2,1,0,0,0]: the credential the branch
  // never touched is pre-existing (scopes/BUG-A, fixed by US-002).
  test("G01c a baseline materializes the whole tree and introduces the todo alone", async () => {
    const repo = await g01();
    const { run, report } = await scan(join(repo.path, "crates/app"), ["--scope", "baseline"], { sandbox: repo.sandbox, tree: repo.path });
    expect(run.exitCode).toBe(0);
    expect(deltaSummary(report)).toEqual([1, 2, 0, 0, 0]);
  });

  test("G01d a staged scan with nothing staged selects nothing", async () => {
    const repo = await g01();
    const { run, report } = await scan(join(repo.path, "crates/app"), ["--scope", "files", "--staged"], { sandbox: repo.sandbox, tree: repo.path });
    expect(run.exitCode).toBe(0);
    expect(report.scope?.files).toEqual([]);
  });
});

describe("two workspaces in one repository", () => {
  function g04() {
    return gitScenario("g04").write({ ...prefixed("a"), ...prefixed("b") }).commit("base").build();
  }

  test("G04 the repository root names no single workspace", async () => {
    const repo = await g04();
    const { run, report } = await scan(repo.path, ["--scope", "files"], { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(2);
    expect(errorCodes(report)).toEqual(["discovery/ambiguous-workspace"]);
  });

  test("G04b naming one workspace scans it", async () => {
    const repo = await g04();
    const { run } = await scan(join(repo.path, "b"), ["--scope", "files"], { sandbox: repo.sandbox, tree: repo.path });
    expect(run.exitCode).toBe(0);
  });
});

// An outer repository around `inner`, an independent `mini` repository the
// outer one does not track, with NEWFN unstaged in `inner`.
async function g05() {
  const inner = await gitScenario("inner").template("mini").commit("base").append("src/lib.rs", NEW_FN).build();
  const outer = dirname(inner.path);
  writeFiles(outer, { "README.md": "# outer\n" });
  await plainGit(outer, inner.sandbox, ["init", "-q"]);
  await plainGit(outer, inner.sandbox, ["add", "README.md"]);
  await plainGit(outer, inner.sandbox, ["commit", "-q", "-m", "outer"]);
  return { inner, outer };
}

async function plainGit(cwd: string, box: Sandbox, args: string[]): Promise<string> {
  const run = await runCommand(["git", "-c", "init.defaultBranch=main", "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null", ...args], {
    sandbox: box,
    cwd,
    env: { GIT_AUTHOR_NAME: "e2e", GIT_AUTHOR_EMAIL: "e2e@example.invalid", GIT_COMMITTER_NAME: "e2e", GIT_COMMITTER_EMAIL: "e2e@example.invalid" },
  });
  if (run.exitCode !== 0) throw new Error(`git ${args.join(" ")} failed in ${cwd}: ${run.stderr.trim()}`);
  return run.stdout;
}

describe("a repository inside another", () => {
  test("G05 the inner repository is the one compared", async () => {
    const { inner, outer } = await g05();
    const { run, report } = await scan(inner.path, ["--scope", "files"], { sandbox: inner.sandbox, tree: outer });
    expect(run.exitCode).toBe(0);
    expect(report.scope?.comparison_base).toBe(MINI_BASE);
    expect(report.scope?.base_ref).toBe("HEAD");
    expect(report.scope?.files).toEqual(["src/lib.rs"]);
  });

  test("G05b a baseline of the inner repository introduces the todo alone", async () => {
    const { inner, outer } = await g05();
    const { run, report } = await scan(inner.path, ["--scope", "baseline", "--base", "HEAD"], { sandbox: inner.sandbox, tree: outer });
    expect(run.exitCode).toBe(0);
    expect(deltaSummary(report)).toEqual([1, 2, 0, 0, 0]);
  });

  test("G05c the outer root finds no manifest it tracks", async () => {
    const { inner, outer } = await g05();
    const { run, report } = await scan(outer, [], { sandbox: inner.sandbox });
    expect(run.exitCode).toBe(2);
    expect(errorCodes(report)).toEqual(["discovery/no-manifest"]);
  });
});

// `mini` as the submodule `sub` of an outer repository (`sub/.git` is a
// file), with NEWFN unstaged in the submodule.
async function g06() {
  const sub = await gitScenario("g06-sub").template("mini").commit("base").build();
  return gitScenario("g06")
    .write({ "README.md": "# outer\n" })
    .submodule("sub", sub)
    .commit("base")
    .append("sub/src/lib.rs", NEW_FN)
    .build();
}

describe("a workspace in a submodule", () => {
  test("G06 a files scope compares against the submodule's HEAD", async () => {
    const repo = await g06();
    const { run, report } = await scan(join(repo.path, "sub"), ["--scope", "files"], { sandbox: repo.sandbox, tree: repo.path });
    expect(run.exitCode).toBe(0);
    expect(report.scope?.comparison_base).toBe(MINI_BASE);
    expect(report.scope?.files).toEqual(["src/lib.rs"]);
  });

  test("G06b a baseline of the submodule introduces the todo alone", async () => {
    const repo = await g06();
    const { run, report } = await scan(join(repo.path, "sub"), ["--scope", "baseline", "--base", "HEAD"], { sandbox: repo.sandbox, tree: repo.path });
    expect(run.exitCode).toBe(0);
    expect(deltaSummary(report)).toEqual([1, 2, 0, 0, 0]);
  });

  test("G06c a staged scan reads the submodule's index", async () => {
    const repo = await g06();
    const { run, report } = await scan(join(repo.path, "sub"), ["--scope", "lines", "--staged"], { sandbox: repo.sandbox, tree: repo.path });
    expect(run.exitCode).toBe(0);
    expect(report.scope?.staged).toBe(true);
  });
});

// A linked worktree `wt` on branch `wt`, with NEWFN unstaged in it.
async function g07() {
  const repo = await gitScenario("g07").template("mini").commit("base").worktree("wt", "wt").build();
  const worktree = repo.worktrees.wt!;
  writeFiles(worktree, { "src/lib.rs": LIB + NEW_FN });
  // The repository and its worktree are siblings, so one fingerprint holds both.
  return { repo, worktree, tree: dirname(repo.path) };
}

describe("a linked worktree", () => {
  test("G07 a files scope detects main from the worktree", async () => {
    const { repo, worktree, tree } = await g07();
    const { run, report } = await scan(worktree, ["--scope", "files"], { sandbox: repo.sandbox, tree });
    expect(run.exitCode).toBe(0);
    expect(report.scope?.base_ref).toBe("main");
    expect(report.scope?.files).toEqual(["src/lib.rs"]);
  });

  test("G07b a baseline from the worktree introduces the todo alone", async () => {
    const { repo, worktree, tree } = await g07();
    const { run, report } = await scan(worktree, ["--scope", "baseline"], { sandbox: repo.sandbox, tree });
    expect(run.exitCode).toBe(0);
    expect(deltaSummary(report)).toEqual([1, 2, 0, 0, 0]);
  });

  test("G07c a staged scan reads the worktree's own index", async () => {
    const { repo, worktree, tree } = await g07();
    const { run, report } = await scan(worktree, ["--scope", "files", "--staged"], { sandbox: repo.sandbox, tree });
    expect(run.exitCode).toBe(0);
    expect(report.scope?.files).toEqual([]);
  });
});

describe("paths a shell has to quote or a link resolves", () => {
  test("G08 a repository directory holding a space scans in lines and baseline scopes", async () => {
    const repo = await gitScenario("g08 dir").template("mini").commit("base").stage({ write: { "src/lib.rs": LIB + NEW_FN } }).build();
    expect(repo.path).toContain(" ");
    const lines = await scan(repo.path, ["--scope", "lines"], { sandbox: repo.sandbox });
    expect(lines.run.exitCode).toBe(0);
    expect(sites(lines.report)).toEqual(["src/lib.rs:12"]);
    const compared = await scan(repo.path, ["--scope", "baseline", "--base", "HEAD"], { sandbox: repo.sandbox });
    expect(compared.run.exitCode).toBe(0);
    expect(deltaSummary(compared.report)).toEqual([1, 2, 0, 0, 0]);
  });

  test("H06 a symlink to the repository gives the results of the real path", async () => {
    const repo = await gitScenario("h06").template("mini").commit("base").stage({ write: { "src/lib.rs": LIB + NEW_FN } }).build();
    const link = join(repo.sandbox.dir, "h06-link");
    symlinkSync(repo.path, link);
    const view = (report: Report) => ({ scope: report.scope, delta: report.delta?.summary ?? null, todo: sites(report), errors: report.errors });
    for (const args of [["--scope", "files"], ["--scope", "lines", "--staged"], ["--scope", "baseline", "--base", "HEAD"]]) {
      const real = await scan(repo.path, args, { sandbox: repo.sandbox });
      const linked = await scan(link, args, { sandbox: repo.sandbox, tree: repo.path });
      expect(linked.run.exitCode).toBe(0);
      expect(view(linked.report)).toEqual(view(real.report));
    }
  });
});
