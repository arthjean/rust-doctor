// `tasks/e2e-catalog/scopes.md` section S: staged scans judge the index from a
// private tree under TMPDIR, the repository pass included (US-003), and refuse
// an index they cannot read.
import { describe, expect, test } from "bun:test";
import { copyFileSync, mkdirSync, readdirSync, utimesSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { runCommand, runDoctor, treeFingerprint } from "./e2e-helpers";
import { credential, gitScenario, miniTemplate, NEW_FN, restoreFrom } from "./git-scenario";
import { count, deltaSummary, errorCodes, MINI_BASE, scan, scopeLine, sites, type Report } from "./scope-scan-support";

const LIB = miniTemplate()["src/lib.rs"]!;
const CREDENTIAL = "rust_doctor::repo::hardcoded_credential";
const SECRET_FILE = "rust_doctor::repo::tracked_secret_file";
const TODO = "clippy::todo";

// S01: `mini` with a harmless `creds.rs` committed, then a staged access key
// in `src/leak.rs` and a force-added `.env` holding a token.
function s01() {
  return gitScenario("s01")
    .template("mini")
    .write({ "src/creds.rs": "pub fn key() -> &'static str {\n    \"none\"\n}\n", ".gitignore": "/target\n.env\n" })
    .commit("base")
    .stage({
      write: {
        "src/leak.rs": `pub fn leak() -> &'static str {\n    "${credential("access-key")}"\n}\n`,
        "src/lib.rs": `${LIB}pub mod leak;\n`,
        ".env": `GITHUB_TOKEN=${credential("token")}\n`,
      },
    })
    .build();
}

function repoFindings(report: Report): [number, number] {
  return [count(report, CREDENTIAL), count(report, SECRET_FILE)];
}

describe("staged secrets", () => {
  // Every row sees two credentials (`src/leak.rs`, `.env`) and one tracked
  // secret file; a files or full scope also keeps the pre-existing todo.
  test.each([
    ["S01", ["--scope", "files", "--staged"], 1],
    ["S01b", ["--scope", "lines", "--staged"], 0],
    ["S01d", ["--scope", "files"], 1],
    ["S01e", ["--scope", "lines"], 0],
    ["S01g", [], 1],
  ] as const)("%s %p reports both credentials and the tracked secret file", async (_id, args, todos) => {
    const built = await s01();
    const scope: readonly string[] = args;
    const { run, report } = await scan(built.path, [...scope], { sandbox: built.sandbox });
    expect(run.exitCode).toBe(0);
    expect(report.status).toBe("complete");
    expect(repoFindings(report)).toEqual([2, 1]);
    expect(count(report, TODO)).toBe(todos);
    expect(report.scope?.staged ?? false).toBe(scope.includes("--staged"));
    if (scope[1] === "files") expect(report.scope?.files).toEqual([".env", "src/leak.rs", "src/lib.rs"]);
  });

  test.each([
    ["S01c", ["--scope", "baseline", "--staged"]],
    ["S01f", ["--scope", "baseline", "--base", "HEAD"]],
  ] as const)("%s %p introduces the two credentials and the secret file", async (_id, args) => {
    const built = await s01();
    const { run, report } = await scan(built.path, [...args], { sandbox: built.sandbox });
    expect(run.exitCode).toBe(0);
    expect(deltaSummary(report)).toEqual([3, 1, 0, 0, 0]);
  });
});

describe("what the index holds", () => {
  test("D11b a staged README alone selects one file and says no Rust change is staged", async () => {
    const repo = await gitScenario("d11b").template("mini").commit("base").stage({ write: { "README.md": "# mini\n" } }).build();
    const { run, report } = await scan(repo.path, ["--scope", "files", "--staged"], { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(0);
    expect(report.scope?.files).toEqual(["README.md"]);
    const before = treeFingerprint(repo.path);
    const human = await runDoctor([repo.path, "--yes", "--scope", "files", "--staged"], { sandbox: repo.sandbox });
    expect(treeFingerprint(repo.path)).toBe(before);
    const lines = human.stdout.split("\n");
    expect(lines[0]).toBe(`Scope: staged files (1 selected, base HEAD at ${MINI_BASE.slice(0, 12)})`);
    expect(lines).toContain("No staged Rust change to judge.");
  });

  // Each row's own repository: `d.rs` and `r.rs` committed with a todo each,
  // then `d.rs` removed and `r.rs` moved to `r2.rs` in the index.
  test.each([
    ["H04", ["--scope", "files", "--staged"], ["src/lib.rs", "src/r2.rs"]],
    ["H04b", ["--scope", "lines", "--staged"], ["src/lib.rs"]],
  ] as const)("%s %p lists what the index adds or modifies, never a deletion", async (id, args, files) => {
    const repo = await h04(id);
    const { run, report } = await scan(repo.path, [...args], { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(0);
    expect(report.scope?.files).toEqual([...files]);
  });

  // Release 1 changed the catalog's D[0,2,1,1,0]: the credential is now seen
  // on both sides of a staged comparison (US-002, US-003), so it is
  // pre-existing.
  test("H04c a staged deletion is fixed and a staged rename pairs across files", async () => {
    const repo = await h04("H04c");
    const { run, report } = await scan(repo.path, ["--scope", "baseline", "--staged"], { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(0);
    expect(deltaSummary(report)).toEqual([0, 3, 1, 1, 0]);
  });
});

function h04(id: string) {
  const todo = (name: string) => `pub fn ${name}() -> u8 {\n    todo!()\n}\n`;
  return gitScenario(id.toLowerCase())
    .template("mini")
    .write({ "src/d.rs": todo("d"), "src/r.rs": todo("r") })
    .append("src/lib.rs", "pub mod d;\npub mod r;\n")
    .commit("base")
    .stage({ write: { "src/r2.rs": todo("r"), "src/lib.rs": `${LIB}pub mod r2;\n` }, remove: ["src/d.rs", "src/r.rs"] })
    .build();
}

describe("a conflicted index", () => {
  // `main` and `side` change the same line, and the merge stops on it with
  // three unmerged entries for `src/lib.rs`.
  async function f01() {
    const repo = await gitScenario("f01")
      .template("mini").commit("base")
      .branch("side").write({ "src/lib.rs": LIB.replace("    1\n", "    2\n") }).commit("side")
      .switch("main").write({ "src/lib.rs": LIB.replace("    1\n", "    3\n") }).commit("main")
      .build();
    await repo.git(["merge", "side"]).catch(() => undefined);
    expect((await repo.git(["ls-files", "-u"])).trim().split("\n")).toHaveLength(3);
    return repo;
  }

  test("F01 a staged scan refuses the conflicted index", async () => {
    const repo = await f01();
    const { run, report } = await scan(repo.path, ["--scope", "lines", "--staged"], { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(2);
    expect(errorCodes(report)).toEqual(["scope/index-conflicted"]);
  });

  test("F01b a files scan judges the conflict markers and ends incomplete", async () => {
    const repo = await f01();
    const head = await repo.sha();
    const { run, report } = await scan(repo.path, ["--scope", "files"], { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(1);
    expect(report.complete).toBe(false);
    expect(await scopeLine(repo.path, ["--scope", "files"], { sandbox: repo.sandbox })).toBe(
      `Scope: changed files (1 selected, base HEAD at ${head.slice(0, 12)})`,
    );
  });

  test("F01c a baseline against HEAD ends incomplete", async () => {
    const repo = await f01();
    const { run, report } = await scan(repo.path, ["--scope", "baseline", "--base", "HEAD"], { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(1);
    expect(report.complete).toBe(false);
  });
});

describe("GIT_INDEX_FILE and GIT_DIR", () => {
  // NEWFN in the working tree, staged only in an alternate index outside the
  // repository: the default index still matches HEAD.
  async function f02() {
    const repo = await gitScenario("f02").template("mini").commit("base").unstaged({ "src/lib.rs": LIB + NEW_FN }).build();
    const alternate = join(repo.sandbox.dir, "alternate-index");
    copyFileSync(join(repo.path, ".git/index"), alternate);
    const add = await runCommand(["git", "add", "src/lib.rs"], { sandbox: repo.sandbox, cwd: repo.path, env: { GIT_INDEX_FILE: alternate } });
    expect(add.exitCode).toBe(0);
    return { repo, alternate };
  }

  test("F02 a staged scan judges the alternate index GIT_INDEX_FILE names", async () => {
    const { repo, alternate } = await f02();
    const { run, report } = await scan(repo.path, ["--scope", "lines", "--staged"], { sandbox: repo.sandbox, env: { GIT_INDEX_FILE: alternate } });
    expect(run.exitCode).toBe(0);
    expect(report.scope?.staged).toBe(true);
    expect(sites(report)).toEqual(["src/lib.rs:12"]);
  });

  test("F02b without GIT_INDEX_FILE the default index stages nothing", async () => {
    const { repo } = await f02();
    const { run, report } = await scan(repo.path, ["--scope", "lines", "--staged"], { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(0);
    expect(report.scope?.files).toEqual([]);
    expect(sites(report)).toEqual([]);
  });

  test("F02c a GIT_INDEX_FILE that does not exist is an unavailable index", async () => {
    const { repo } = await f02();
    const missing = join(repo.sandbox.dir, "no-such-index");
    const { run, report } = await scan(repo.path, ["--scope", "lines", "--staged"], { sandbox: repo.sandbox, env: { GIT_INDEX_FILE: missing } });
    expect(run.exitCode).toBe(2);
    expect(errorCodes(report)).toEqual(["scope/index-unavailable"]);
  });

  test("F02d a working-tree scope ignores GIT_INDEX_FILE", async () => {
    const { repo, alternate } = await f02();
    const { run, report } = await scan(repo.path, ["--scope", "lines"], { sandbox: repo.sandbox, env: { GIT_INDEX_FILE: alternate } });
    expect(run.exitCode).toBe(0);
    expect(report.scope?.staged).toBe(false);
    expect(sites(report)).toEqual(["src/lib.rs:12"]);
  });

  test("F02e a GIT_DIR naming nothing is removed and the scan runs normally", async () => {
    const { repo } = await f02();
    const { run, report } = await scan(repo.path, ["--scope", "lines"], { sandbox: repo.sandbox, env: { GIT_DIR: "/nonexistent" } });
    expect(run.exitCode).toBe(0);
    expect(report.errors).toEqual([]);
    expect(sites(report)).toEqual(["src/lib.rs:12"]);
  });

  // The form a git hook runs under: cwd at the repository root and a
  // relative GIT_INDEX_FILE. The working tree is back at HEAD, so only the
  // index holds NEWFN.
  test("F02f a relative GIT_INDEX_FILE resolves against the hook's cwd", async () => {
    const repo = await gitScenario("f02f")
      .template("mini").commit("base")
      .stage({ write: { "src/lib.rs": LIB + NEW_FN } })
      .unstaged({ "src/lib.rs": restoreFrom("HEAD") })
      .build();
    const { run, report } = await scan(".", ["--scope", "lines", "--staged"], {
      sandbox: repo.sandbox,
      cwd: repo.path,
      tree: repo.path,
      env: { GIT_INDEX_FILE: ".git/index" },
    });
    expect(run.exitCode).toBe(0);
    expect(sites(report)).toEqual(["src/lib.rs:12"]);
  });
});

describe("an index another git process holds", () => {
  // A stat change the index has not recorded, and `index.lock` taken: no git
  // command can write the index while the lock stands.
  async function i01() {
    const repo = await gitScenario("i01").template("mini").commit("base").build();
    const later = new Date(Date.now() + 5_000);
    utimesSync(join(repo.path, "src/lib.rs"), later, later);
    writeFileSync(join(repo.path, ".git/index.lock"), "");
    return repo;
  }

  test("I01 a staged scan refuses a locked index", async () => {
    const repo = await i01();
    const { run, report } = await scan(repo.path, ["--scope", "files", "--staged"], { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(2);
    expect(errorCodes(report)).toEqual(["scope/index-unavailable"]);
  });

  test.each([
    ["I01b", ["--scope", "files"]],
    ["I01c", ["--scope", "baseline", "--base", "HEAD"]],
  ] as const)("%s %p runs beside a locked index", async (_id, args) => {
    const repo = await i01();
    const { run, report } = await scan(repo.path, [...args], { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(0);
    expect(report.errors).toEqual([]);
  });
});

describe("an unusable TMPDIR", () => {
  // The snapshot tree must live outside the repository it copies. A TMPDIR
  // that cannot hold it is skipped for the next private candidate, so these
  // rows fail only once HOME offers none either (US-018).
  test.each([
    ["H01", "inside", ["--scope", "baseline", "--base", "HEAD"]],
    ["H01b", "inside", ["--scope", "lines", "--staged"]],
    ["H01c", "missing", ["--scope", "baseline", "--base", "HEAD"]],
  ] as const)("%s a TMPDIR %s the repository fails %p before any snapshot", async (_id, where, args) => {
    const repo = await gitScenario("h01").template("mini").commit("base").stage({ write: { "src/lib.rs": LIB + NEW_FN } }).build();
    const tmp = where === "inside" ? join(repo.path, "tmpin") : "/nonexistent";
    if (where === "inside") mkdirSync(tmp);
    const env = { TMPDIR: tmp, HOME: join(repo.sandbox.dir, "no-home") };
    const { run, report } = await scan(repo.path, [...args], { sandbox: repo.sandbox, env });
    expect(run.exitCode).toBe(2);
    expect(errorCodes(report)).toEqual(["baseline/baseline-temp-unavailable"]);
  });

  test.each([
    ["H01d", ["--scope", "baseline", "--base", "HEAD"]],
    ["H01e", ["--scope", "lines", "--staged"]],
  ] as const)("%s a TMPDIR inside the repository is skipped for the private cache home on %p", async (_id, args) => {
    const repo = await gitScenario("h01d").template("mini").commit("base").stage({ write: { "src/lib.rs": LIB + NEW_FN } }).build();
    const tmp = join(repo.path, "tmpin");
    mkdirSync(tmp);
    const { report } = await scan(repo.path, [...args], { sandbox: repo.sandbox, env: { TMPDIR: tmp } });
    expect(errorCodes(report)).toEqual([]);
    expect(report.status).toBe("complete");
    expect(readdirSync(tmp)).toEqual([]);
    expect(readdirSync(join(repo.sandbox.home, ".cache/rust-doctor"))).toEqual([]);
  });
});
