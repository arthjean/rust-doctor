// `tasks/e2e-catalog/scopes.md` section D with F03, F05, H05 and H07:
// working-tree states under the files, lines and contextual scopes, and the
// baseline of the same states. Each state is built once and shared by its
// rows, which never write it.
import { describe, expect, test } from "bun:test";
import { chmodSync, readFileSync, utimesSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { FIXTURES, runDoctor, treeFingerprint, type DoctorRun } from "./e2e-helpers";
import { gitScenario, miniTemplate, NEW_FN, restoreFrom, type Repo } from "./git-scenario";
import {
  count,
  deltaSummary,
  errorCodes,
  human,
  introduced,
  MINI_BASE,
  scan,
  scopeLine,
  sites,
  unwrapped,
  type Report,
} from "./scope-scan-support";

function once<T>(build: () => Promise<T>): () => Promise<T> {
  let built: Promise<T> | undefined;
  return () => (built ??= build());
}

const MINI_LIB = miniTemplate()["src/lib.rs"]!;
const TODO_FN = (name: string) => `pub fn ${name}() -> u8 {\n    todo!()\n}\n`;

function run(repo: Repo, args: string[]) {
  return scan(repo.path, args, { sandbox: repo.sandbox });
}

// The report of a run that may change the index, which `scan` would refuse.
function jsonReport(result: DoctorRun): Report {
  if (result.json === undefined) throw new Error(`no JSON report (exit ${result.exitCode}): ${result.stderr}`);
  return result.json as Report;
}

describe("uncommitted and staged work", () => {
  const unstaged = once(() => gitScenario("d01").template("mini").commit("base").append("src/lib.rs", NEW_FN).build());
  const staged = once(() =>
    gitScenario("d02").template("mini").commit("base").stage({ write: { "src/lib.rs": MINI_LIB + NEW_FN } }).build(),
  );
  const stagedOnly = once(() =>
    gitScenario("d04")
      .template("mini").commit("base")
      .stage({ write: { "src/lib.rs": MINI_LIB + NEW_FN } })
      .unstaged({ "src/lib.rs": restoreFrom("HEAD") })
      .build(),
  );

  test("D01 the files scope judges every line of an edited file", async () => {
    const { report } = await run(await unstaged(), ["--scope", "files"]);
    expect(report.scope?.files).toEqual(["src/lib.rs"]);
    expect(sites(report)).toEqual(["src/lib.rs:12", "src/lib.rs:4"]);
  });

  test("D01l the lines scope judges only the added line", async () => {
    const { report } = await run(await unstaged(), ["--scope", "lines"]);
    expect(sites(report)).toEqual(["src/lib.rs:12"]);
  });

  test("D01c the contextual scope keeps a line-reach finding on its line", async () => {
    const { report } = await run(await unstaged(), ["--scope", "contextual"]);
    expect(sites(report)).toEqual(["src/lib.rs:12"]);
  });

  test("D01s a staged scan ignores unstaged work", async () => {
    const repo = await unstaged();
    const args = ["--scope", "files", "--staged"];
    const { report } = await run(repo, args);
    expect(report.scope).toMatchObject({ files: [], staged: true, untracked_unreported: null });
    expect(await scopeLine(repo.path, args, { sandbox: repo.sandbox })).toBe(
      "Scope: staged files (0 selected, base HEAD at af441c9b9f96)",
    );
  });

  test("D02 the worktree lines scope includes staged work", async () => {
    const { report } = await run(await staged(), ["--scope", "lines"]);
    expect(sites(report)).toEqual(["src/lib.rs:12"]);
  });

  test("D02b a staged lines scan judges the index", async () => {
    const repo = await staged();
    const args = ["--scope", "lines", "--staged"];
    const { report } = await run(repo, args);
    expect(report.scope?.staged).toBe(true);
    expect(sites(report)).toEqual(["src/lib.rs:12"]);
    expect(await scopeLine(repo.path, args, { sandbox: repo.sandbox })).toBe(
      "Scope: staged lines (1 files, base HEAD at af441c9b9f96)",
    );
  });

  const restored = () =>
    gitScenario("d04-index")
      .template("mini").commit("base")
      .stage({ write: { "src/lib.rs": MINI_LIB + NEW_FN } })
      .unstaged({ "src/lib.rs": restoreFrom("HEAD") })
      .build();

  // The index half of D04 is the open defect, kept apart so it cannot hide a
  // regression of the scope.
  test("D04 a lines scan of a worktree restored under a staged change lists no file and reports nothing", async () => {
    const repo = await restored();
    const report = jsonReport(await runDoctor([repo.path, "--yes", "--json", "--scope", "lines"], { sandbox: repo.sandbox }));
    expect(report.scope?.files).toEqual([]);
    expect(report.diagnostics).toEqual([]);
  });

  test.failing("scopes/BUG-C a lines scan of a worktree restored under a staged change leaves the index alone (D04)", async () => {
    const repo = await restored();
    const before = treeFingerprint(repo.path);
    jsonReport(await runDoctor([repo.path, "--yes", "--json", "--scope", "lines"], { sandbox: repo.sandbox }));
    expect(treeFingerprint(repo.path)).toBe(before);
  });

  test("D04b a staged lines scan judges the index, not the restored worktree", async () => {
    const { report } = await run(await stagedOnly(), ["--scope", "lines", "--staged"]);
    expect(sites(report)).toEqual(["src/lib.rs:12"]);
  });

  test("D04c a staged baseline compares the index with HEAD", async () => {
    const repo = await stagedOnly();
    const args = ["--scope", "baseline", "--staged"];
    const { report } = await run(repo, args);
    expect(report.scope).toMatchObject({ mode: "baseline", comparison_base: MINI_BASE, base_ref: "HEAD", staged: true });
    expect(deltaSummary(report)).toEqual([1, 2, 0, 0, 0]);
    expect(introduced(report)).toEqual(["clippy::todo@src/lib.rs:12"]);
    expect(await scopeLine(repo.path, args, { sandbox: repo.sandbox })).toBe(
      "Scope: staged baseline comparison (base HEAD at af441c9b9f96)",
    );
  });

  test("D04d a worktree baseline of the restored file introduces nothing", async () => {
    const { report } = await run(await stagedOnly(), ["--scope", "baseline", "--base", "HEAD"]);
    expect(deltaSummary(report)).toEqual([0, 2, 0, 0, 0]);
  });
});

describe("untracked files", () => {
  const untracked = once(async () => {
    const repo = await gitScenario("d05")
      .template("mini").append("src/lib.rs", "pub mod fresh;\n").commit("base")
      .untracked({ "src/fresh.rs": TODO_FN("fresh"), "notes.txt": "notes\n", "ignored.log": "log\n" })
      .build();
    writeFileSync(join(repo.path, ".git/info/exclude"), "*.log\n");
    return repo;
  });

  test("D05 untracked Rust files are compiled, counted and not reported", async () => {
    const repo = await untracked();
    const { report } = await run(repo, ["--scope", "files"]);
    expect(report.scope).toMatchObject({ files: [], untracked_unreported: 1 });
    expect(sites(report)).toEqual([]);
    const text = (await human(repo.path, ["--scope", "files"], { sandbox: repo.sandbox })).stdout;
    expect(unwrapped(text)).toContain(
      "1 untracked Rust file was compiled but not reported: pass --include-untracked to judge them.",
    );
  });

  test.each([
    ["D05b", "files"],
    ["D05c", "lines"],
    ["D05d", "contextual"],
  ])("%s a %s scan with --include-untracked judges untracked files whole", async (_id, scope) => {
    const { report } = await run(await untracked(), ["--scope", scope, "--include-untracked"]);
    expect(report.scope).toMatchObject({ files: ["notes.txt", "src/fresh.rs"], untracked_unreported: null });
    expect(sites(report)).toEqual(["src/fresh.rs:2"]);
  });

  test("D05e a base that declares a module only the worktree holds does not compile", async () => {
    const { run: result, report } = await run(await untracked(), ["--scope", "baseline", "--base", "HEAD"]);
    expect(result.exitCode).toBe(2);
    expect(errorCodes(report)).toEqual(["baseline/baseline-scan-incomplete"]);
  });

  test("D11 an untracked file that is not Rust is not counted", async () => {
    const repo = await gitScenario("d11").template("mini").commit("base").untracked({ "README.md": "# mini\n" }).build();
    const { report } = await run(repo, ["--scope", "files"]);
    expect(report.scope).toMatchObject({ files: [], untracked_unreported: 0 });
  });

  test("D19 more untracked files than the scope bound refuse the scan", async () => {
    const files: Record<string, string> = {};
    for (let index = 0; index <= 10_000; index += 1) files[`many/f${index}.txt`] = "";
    const repo = await gitScenario("d19").template("mini").commit("base").untracked(files).build();
    const { run: result, report } = await run(repo, ["--scope", "files", "--include-untracked"]);
    expect(result.exitCode).toBe(2);
    expect(errorCodes(report)).toEqual(["scope/too-many-files"]);
  });
});

describe("deletions, renames and modes", () => {
  // Long enough that a one-line edit keeps the file above git's rename
  // similarity threshold.
  const BEFORE = `${TODO_FN("before")}\npub fn one() -> u8 {\n    1\n}\n\npub fn two() -> u8 {\n    2\n}\n\npub fn three() -> u8 {\n    3\n}\n`;
  const withModules = () =>
    gitScenario("d07")
      .template("mini")
      .append("src/lib.rs", "pub mod gone;\npub mod before;\n")
      .write({ "src/gone.rs": TODO_FN("gone"), "src/before.rs": BEFORE })
      .commit("base")
      .branch("work")
      .stage({ write: { "src/lib.rs": `${MINI_LIB}pub mod before;\n` }, remove: ["src/gone.rs"] })
      .commit("delete");
  const deleted = once(() => withModules().build());
  const renamedLib = `${MINI_LIB}#[path = "after.rs"]\npub mod before;\n`;
  const renamed = () =>
    withModules()
      .stage({ write: { "src/lib.rs": renamedLib, "src/after.rs": BEFORE }, remove: ["src/before.rs"] })
      .commit("rename");
  const moved = once(() => renamed().build());
  const edited = once(() =>
    renamed().write({ "src/after.rs": BEFORE.replace("    3\n", "    4\n") }).commit("edit").build(),
  );

  test("D07 a deleted file is not listed", async () => {
    const { report } = await run(await deleted(), ["--scope", "files"]);
    expect(report.scope?.files).toEqual(["src/lib.rs"]);
  });

  test("D07b a deletion point in lines finds nothing", async () => {
    const { report } = await run(await deleted(), ["--scope", "lines"]);
    expect(report.scope?.files).toEqual(["src/lib.rs"]);
    expect(report.diagnostics).toEqual([]);
  });

  test("D07c a baseline counts the deleted file's finding as fixed", async () => {
    const { report } = await run(await deleted(), ["--scope", "baseline"]);
    expect(deltaSummary(report)).toEqual([0, 3, 1, 0, 0]);
  });

  test("D08 a rename lists the new path", async () => {
    const repo = await moved();
    const { report } = await run(repo, ["--scope", "files", "--base", await repo.sha("work~1")]);
    expect(report.scope?.files).toEqual(["src/after.rs", "src/lib.rs"]);
  });

  test("D08b a pure rename has no hunk", async () => {
    const repo = await moved();
    const { report } = await run(repo, ["--scope", "lines", "--base", await repo.sha("work~1")]);
    expect(report.scope?.files).toEqual(["src/lib.rs"]);
  });

  test("D08c a baseline pairs the moved finding across files", async () => {
    const repo = await moved();
    const { report } = await run(repo, ["--scope", "baseline", "--base", await repo.sha("work~1")]);
    expect(deltaSummary(report)).toEqual([0, 3, 0, 1, 0]);
  });

  test("D09 an edited rename keeps only the edited line", async () => {
    const repo = await edited();
    const { report } = await run(repo, ["--scope", "lines", "--base", await repo.sha("work~2")]);
    expect(report.scope?.files).toEqual(["src/after.rs", "src/lib.rs"]);
    expect(sites(report)).toEqual([]);
  });

  test("D09b a baseline pairs an edited rename across files", async () => {
    const repo = await edited();
    const { report } = await run(repo, ["--scope", "baseline", "--base", await repo.sha("work~2")]);
    expect(deltaSummary(report)).toEqual([0, 3, 0, 1, 0]);
  });

  const executable = once(async () => {
    const repo = await gitScenario("d13").template("mini").commit("base").write({ "README.md": "# mini\n" }).commit("readme").build();
    chmodSync(join(repo.path, "src/lib.rs"), 0o755);
    return repo;
  });

  test("D13 a mode change lists the file", async () => {
    const repo = await executable();
    const { report } = await run(repo, ["--scope", "files", "--base", await repo.sha("HEAD~1")]);
    expect(report.scope?.files).toEqual(["README.md", "src/lib.rs"]);
  });

  test("D13b a mode change has no hunk", async () => {
    const { report } = await run(await executable(), ["--scope", "lines", "--base", "HEAD"]);
    expect(report.scope?.files).toEqual([]);
  });

  const binary = () => gitScenario("d14").template("mini").commit("base").stage({ write: { "blob.bin": "\u0000\u0001bin\u0000" } });

  // A stat change on a file still equal to HEAD: porcelain `git diff` drops
  // the pair and writes the refreshed index back, `git diff-index` does not.
  const touchedBinary = async () => {
    const repo = await binary().build();
    const past = new Date(Date.UTC(2025, 0, 1));
    utimesSync(join(repo.path, "src/lib.rs"), past, past);
    return repo;
  };

  // The index half of D14 is the open defect, kept apart so it cannot hide a
  // regression of the scope.
  test("D14 a files scan of a staged binary lists the binary", async () => {
    const repo = await touchedBinary();
    const report = jsonReport(await runDoctor([repo.path, "--yes", "--json", "--scope", "files", "--base", "HEAD"], { sandbox: repo.sandbox }));
    expect(report.scope?.files).toEqual(["blob.bin"]);
  });

  test.failing("scopes/BUG-C a files scan of a staged binary leaves the index alone (D14)", async () => {
    const repo = await touchedBinary();
    const before = treeFingerprint(repo.path);
    jsonReport(await runDoctor([repo.path, "--yes", "--json", "--scope", "files", "--base", "HEAD"], { sandbox: repo.sandbox }));
    expect(treeFingerprint(repo.path)).toBe(before);
  });

  test("D14b a binary file has no hunk", async () => {
    const repo = await binary().build();
    const { report } = await run(repo, ["--scope", "lines", "--base", "HEAD"]);
    expect(report.scope?.files).toEqual([]);
  });
});

describe("path encoding", () => {
  const CAFE = "café.rs";
  const encodedLib = `${MINI_LIB}#[path = "sp ace.rs"]\npub mod space;\n#[path = "pct%41.rs"]\npub mod pct;\n#[path = "${CAFE}"]\npub mod cafe;\n`;
  const encoded = () =>
    gitScenario("d15")
      .template("mini").commit("base")
      .stage({
        write: {
          "src/lib.rs": encodedLib,
          "src/sp ace.rs": TODO_FN("space"),
          "src/pct%41.rs": TODO_FN("pct"),
          [`src/${CAFE}`]: TODO_FN("cafe"),
        },
      });
  const staged = once(() => encoded().build());
  const published = [`src/${CAFE}`, "src/lib.rs", "src/pct%2541.rs", "src/sp ace.rs"];

  test("D15 the files scope publishes percent-encoded and UTF-8 paths", async () => {
    const { report } = await run(await staged(), ["--scope", "files", "--base", "HEAD"]);
    expect(report.scope?.files).toEqual(published);
    expect(sites(report)).toEqual([`src/${CAFE}:2`, "src/lib.rs:4", "src/pct%2541.rs:2", "src/sp ace.rs:2"]);
  });

  test("D15b the lines scope keeps the three new findings", async () => {
    const { report } = await run(await staged(), ["--scope", "lines", "--base", "HEAD"]);
    expect(sites(report)).toEqual([`src/${CAFE}:2`, "src/pct%2541.rs:2", "src/sp ace.rs:2"]);
  });

  test("D15c a staged lines scan keeps the same findings", async () => {
    const { report } = await run(await staged(), ["--scope", "lines", "--staged"]);
    expect(report.scope?.staged).toBe(true);
    expect(sites(report)).toEqual([`src/${CAFE}:2`, "src/pct%2541.rs:2", "src/sp ace.rs:2"]);
  });

  test("D15d a baseline introduces the three new findings by their published paths", async () => {
    const { report } = await run(await staged(), ["--scope", "baseline", "--base", "HEAD"]);
    expect(deltaSummary(report)).toEqual([3, 2, 0, 0, 0]);
    expect(introduced(report)).toEqual([
      `clippy::todo@src/${CAFE}:2`,
      "clippy::todo@src/pct%2541.rs:2",
      "clippy::todo@src/sp ace.rs:2",
    ]);
  });

  test("D15e a baseline pairs a percent-named move across files", async () => {
    const repo = await encoded()
      .commit("encoded")
      .stage({ write: { "src/lib.rs": encodedLib.replace("pct%41.rs", "moved%42.rs"), "src/moved%42.rs": TODO_FN("pct") }, remove: ["src/pct%41.rs"] })
      .build();
    const { report } = await run(repo, ["--scope", "baseline", "--base", "HEAD"]);
    expect(deltaSummary(report)).toEqual([0, 5, 0, 1, 0]);
  });

  test("D16 a changed path that is not UTF-8 is refused, a control character is encoded", async () => {
    const control = await gitScenario("d16-control").template("mini").commit("base").stage({ write: { "notes\u0001.txt": "notes\n" } }).build();
    const encodedRun = await run(control, ["--scope", "files", "--base", "HEAD"]);
    expect(encodedRun.run.exitCode).toBe(0);
    expect(encodedRun.report.scope?.files).toEqual(["notes%01.txt"]);
    const invalid = await gitScenario("d16").template("mini").commit("base").build();
    writeFileSync(Buffer.concat([Buffer.from(join(invalid.path, "notes")), Buffer.from([0xff]), Buffer.from(".txt")]), "notes\n");
    await invalid.git(["add", "-A"]);
    const { run: result, report } = await scan(invalid.path, ["--scope", "files", "--base", "HEAD"], { sandbox: invalid.sandbox });
    expect(result.exitCode).toBe(2);
    expect(errorCodes(report)).toEqual(["scope/git-path-invalid"]);
  });
});

describe("hunks and reach", () => {
  const body = once(() =>
    gitScenario("d17")
      .template("mini")
      .append("src/lib.rs", "pub mod body;\n")
      .write({ "src/body.rs": "pub fn body() -> u8 {\n    let x = 1;\n    let y = 2;\n    todo!()\n}\n" })
      .commit("base")
      .unstaged({ "src/body.rs": "pub fn body() -> u8 {\n    let x = 1;\n    todo!()\n}\n" })
      .build(),
  );

  test("D17 a deletion point alone finds nothing", async () => {
    const { report } = await run(await body(), ["--scope", "lines", "--base", "HEAD"]);
    expect(report.scope?.files).toEqual(["src/body.rs"]);
    expect(report.diagnostics).toEqual([]);
  });

  test("D17b the contextual scope never widens a line-reach finding", async () => {
    const { report } = await run(await body(), ["--scope", "contextual", "--base", "HEAD"]);
    expect(report.diagnostics).toEqual([]);
  });

  test("D17c the files scope judges the whole file", async () => {
    const { report } = await run(await body(), ["--scope", "files", "--base", "HEAD"]);
    expect(sites(report)).toEqual(["src/body.rs:3"]);
    expect(count(report, "unused_variables")).toBe(1);
  });

  const topic = once(() =>
    gitScenario("d18")
      .template("mini").commit("base")
      .branch("topic").write({ "src/c.rs": TODO_FN("c") }).append("src/lib.rs", "pub mod c;\n").commit("topic")
      .append("src/creds.rs", NEW_FN)
      .build(),
  );

  test("D18 the files scope joins the committed range and dirty work", async () => {
    const { report } = await run(await topic(), ["--scope", "files"]);
    expect(report.scope?.files).toEqual(["src/c.rs", "src/creds.rs", "src/lib.rs"]);
  });

  test("D18b the lines scope keeps the branch's and the worktree's lines", async () => {
    const { report } = await run(await topic(), ["--scope", "lines"]);
    expect(sites(report)).toEqual(["src/c.rs:2", "src/creds.rs:6"]);
  });

  test("D18c a baseline introduces both todo and nothing else", async () => {
    const { report } = await run(await topic(), ["--scope", "baseline"]);
    expect(deltaSummary(report)).toEqual([2, 2, 0, 0, 0]);
    expect(introduced(report)).toEqual(["clippy::todo@src/c.rs:2", "clippy::todo@src/creds.rs:6"]);
  });
});

describe("oversized diffs", () => {
  const big = once(() => {
    const lines = Array.from({ length: 40_000 }, (_, index) => `pub const BIG_${index}: u32 = ${index};`);
    return gitScenario("f03")
      .template("mini").commit("base")
      .stage({ write: { "src/lib.rs": `${MINI_LIB}pub mod big;\n`, "src/big.rs": `${lines.join("\n")}\n` } })
      .build();
  });

  test.each([
    ["F03", "lines"],
    ["F03b", "contextual"],
  ])("%s a %s scan past the diff bound names the scopes that fit", async (_id, scope) => {
    const repo = await big();
    const args = ["--scope", scope, "--base", "HEAD"];
    const { run: result, report } = await run(repo, args);
    expect(result.exitCode).toBe(2);
    expect(errorCodes(report)).toEqual(["scope/diff-too-large"]);
    expect(report.errors[0]?.message).toEndWith("use --scope files or baseline.");
    expect(await scopeLine(repo.path, args, { sandbox: repo.sandbox })).toBe(`Scope: ${scope} (not resolved)`);
  });

  test("F03c the files scope judges the oversized file", async () => {
    const { run: result, report } = await run(await big(), ["--scope", "files", "--base", "HEAD"]);
    expect(result.exitCode).toBe(0);
    expect(count(report, "rust_doctor::structure::oversized_unit")).toBe(1);
  });
});

describe("contextual reach", () => {
  const HELD = "    let value = guarded.lock().unwrap();\n    *value\n";
  const AWAITED = "    let value = guarded.lock().unwrap();\n    ready(*value).await\n";
  const fixture = readFileSync(join(FIXTURES, "score-credibility/packs/concurrency/src/lib.rs"), "utf8");
  // The concurrency pack committed with the guard held but not awaited on,
  // then the worktree awaiting under it again.
  const awaited = once(() =>
    gitScenario("f05")
      .fixture("score-credibility/packs/concurrency")
      .write({ "src/lib.rs": fixture.replace(AWAITED, HELD), ".gitignore": "/target\n" })
      .commit("held")
      .unstaged({ "src/lib.rs": fixture })
      .build(),
  );

  test("F05 the contextual scope widens an item-reach finding to its function", async () => {
    const { report } = await run(await awaited(), ["--scope", "contextual"]);
    expect(count(report, "clippy::await_holding_lock")).toBe(1);
  });

  test("F05b the lines scope does not widen", async () => {
    const { report } = await run(await awaited(), ["--scope", "lines"]);
    expect(count(report, "clippy::await_holding_lock")).toBe(0);
  });

  test("F05c the files scope judges the whole file", async () => {
    const { report } = await run(await awaited(), ["--scope", "files"]);
    expect(report.diagnostics.filter((diagnostic) => diagnostic.path === "src/lib.rs").map((diagnostic) => diagnostic.code).sort()).toEqual([
      "clippy::arc_with_non_send_sync",
      "clippy::await_holding_lock",
      "clippy::await_holding_refcell_ref",
      "clippy::mut_mutex_lock",
      "clippy::rc_mutex",
      "clippy::unused_async",
    ]);
  });

  test("F05d a baseline introduces the await under the lock", async () => {
    const { report } = await run(await awaited(), ["--scope", "baseline", "--base", "HEAD"]);
    expect(deltaSummary(report)).toEqual([1, 5, 1, 0, 0]);
    expect(introduced(report).map((site) => site.split("@")[0])).toEqual(["clippy::await_holding_lock"]);
  });
});

describe("nested workspaces", () => {
  const nested = once(() =>
    gitScenario("h05")
      .nested("crates/app", "mini").commit("base")
      .untracked({ "outside.rs": "fn outside() {}\n", "crates/app/src/stray.rs": "pub fn stray() {}\n" })
      .build(),
  );

  test("H05 only untracked files inside the workspace are counted", async () => {
    const repo = await nested();
    const { report } = await scan(join(repo.path, "crates/app"), ["--scope", "files"], { sandbox: repo.sandbox, tree: repo.path });
    expect(report.scope?.untracked_unreported).toBe(1);
  });

  test("H05b an included untracked file the crate never declares is an orphan", async () => {
    const repo = await nested();
    const { report } = await scan(join(repo.path, "crates/app"), ["--scope", "files", "--include-untracked"], {
      sandbox: repo.sandbox,
      tree: repo.path,
    });
    expect(report.scope?.files).toEqual(["src/stray.rs"]);
    expect(count(report, "rust_doctor::structure::orphan_module_file")).toBe(1);
  });
});

describe("symlinked modules", () => {
  // `src/alias.rs` is a tracked link to `impls/real.rs`; the worktree adds a
  // todo to the target, which rustc reports at the link.
  const linked = () =>
    gitScenario("h07")
      .template("mini")
      .write({ "src/impls/real.rs": "pub fn real() -> u8 {\n    3\n}\n" })
      .symlink("src/alias.rs", "impls/real.rs")
      .append("src/lib.rs", "pub mod alias;\n")
      .commit("base")
      .unstaged({ "src/impls/real.rs": "pub fn real() -> u8 {\n    3\n}\n\npub fn more() -> u8 {\n    todo!()\n}\n" });

  // What the scan does today, apart from the dropped finding of the open
  // defect, so the defect cannot hide a regression of the scope.
  test.each([
    ["H07", "files"],
    ["H07b", "lines"],
  ])("%s a %s scan of an edit made through a tracked symlinked module judges the target", async (_id, scope) => {
    const repo = await linked().build();
    const { report } = await run(repo, ["--scope", scope, "--base", "HEAD"]);
    expect(report.status).toBe("complete");
    expect(report.scope?.files).toEqual(["src/impls/real.rs"]);
  });

  test.failing.each(["files", "lines"])("scopes/BUG-G a %s scan keeps an edit made through a tracked symlinked module (H07, H07b)", async (scope) => {
    const repo = await linked().build();
    const { report } = await run(repo, ["--scope", scope, "--base", "HEAD"]);
    expect(sites(report)).toEqual(["src/alias.rs:6"]);
  });

  test("H07c a baseline introduces the edit at the link", async () => {
    const repo = await linked().build();
    const { report } = await run(repo, ["--scope", "baseline", "--base", "HEAD"]);
    expect(deltaSummary(report)).toEqual([1, 2, 0, 0, 0]);
    expect(introduced(report)).toEqual(["clippy::todo@src/alias.rs:6"]);
  });
});
