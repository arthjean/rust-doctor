// `tasks/e2e-catalog/scopes.md` section C with H02, H03 and L01: how a scope
// finds its base on every repository shape, and how it fails when it cannot.
// Each shape is built once and shared by its rows, which never write it.
import { describe, expect, test } from "bun:test";
import { join } from "node:path";
import { sandbox, scratchDir } from "./e2e-helpers";
import { gitScenario, miniTemplate, NEW_FN, type Repo } from "./git-scenario";
import {
  count,
  deltaSummary,
  errorCodes,
  introduced,
  MINI_BASE,
  scan,
  scopeLine,
  sha256Mini,
  sites,
  writeFiles,
  type PlainRepo,
} from "./scope-scan-support";

function once<T>(build: () => Promise<T>): () => Promise<T> {
  let built: Promise<T> | undefined;
  return () => (built ??= build());
}

async function failsWith(repo: PlainRepo, args: string[], code: string): Promise<void> {
  const { run, report } = await scan(repo.path, args, { sandbox: repo.sandbox });
  expect(run.exitCode).toBe(2);
  expect(report.status).toBe("failed");
  expect(report.scope).toBeNull();
  expect(errorCodes(report)).toEqual([code]);
}

const FILES = ["--scope", "files"];

// A clone under the origin's sandbox, for the shapes `clone()` has no option
// for (`--no-single-branch`).
async function cloneOf(origin: Repo, name: string, args: string[]): Promise<PlainRepo> {
  const path = join(scratchDir(name), name);
  await origin.git(["clone", "-q", "--no-hardlinks", ...args, `file://${origin.path}`, path]);
  return { path, sandbox: origin.sandbox, git: (more) => origin.git(["-C", path, ...more]) };
}

describe("detected bases", () => {
  const clean = once(() => gitScenario("c01").template("mini").commit("base").build());
  const dirty = once(async () => {
    const repo = await gitScenario("c01b").template("mini").commit("base").append("src/lib.rs", NEW_FN).build();
    return repo;
  });
  const feature = once(() =>
    gitScenario("c02").template("mini").commit("base").branch("feature").append("src/lib.rs", NEW_FN).commit("feature").build(),
  );

  test("C01 on the default branch the base is HEAD", async () => {
    const repo = await clean();
    const { run, report } = await scan(repo.path, FILES, { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(0);
    expect(report.scope).toMatchObject({
      mode: "files",
      comparison_base: MINI_BASE,
      base_ref: "HEAD",
      files: [],
      staged: false,
      untracked_unreported: 0,
    });
    expect(await scopeLine(repo.path, FILES, { sandbox: repo.sandbox })).toBe(
      "Scope: changed files (0 selected, base HEAD at af441c9b9f96)",
    );
  });

  test("C01b an unstaged edit on the default branch is judged against HEAD", async () => {
    const repo = await dirty();
    const { report } = await scan(repo.path, FILES, { sandbox: repo.sandbox });
    expect(report.scope).toMatchObject({ base_ref: "HEAD", files: ["src/lib.rs"] });
    expect(sites(report)).toEqual(["src/lib.rs:12", "src/lib.rs:4"]);
  });

  test("C02 a feature branch detects main", async () => {
    const repo = await feature();
    const { run, report } = await scan(repo.path, FILES, { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(0);
    expect(report.scope).toMatchObject({ comparison_base: MINI_BASE, base_ref: "main", files: ["src/lib.rs"] });
    expect(await scopeLine(repo.path, FILES, { sandbox: repo.sandbox })).toBe(
      "Scope: changed files (1 selected, base main at af441c9b9f96)",
    );
  });

  test("C02b a named base publishes no ref", async () => {
    const repo = await feature();
    const args = [...FILES, "--base", "main"];
    const { report } = await scan(repo.path, args, { sandbox: repo.sandbox });
    expect(report.scope).toMatchObject({ comparison_base: MINI_BASE, base_ref: null, files: ["src/lib.rs"] });
    expect(await scopeLine(repo.path, args, { sandbox: repo.sandbox })).toBe(
      "Scope: changed files (1 selected, base af441c9b9f96)",
    );
  });

  test("C02c --base HEAD on a feature branch judges only uncommitted work", async () => {
    const repo = await feature();
    const { report } = await scan(repo.path, [...FILES, "--base", "HEAD"], { sandbox: repo.sandbox });
    expect(report.scope).toMatchObject({ comparison_base: await repo.sha(), files: [] });
  });

  test("C02d the lines scope keeps only the branch's line", async () => {
    const repo = await feature();
    const { report } = await scan(repo.path, ["--scope", "lines"], { sandbox: repo.sandbox });
    expect(report.scope).toMatchObject({ mode: "lines", comparison_base: MINI_BASE, base_ref: "main", files: ["src/lib.rs"] });
    expect(sites(report)).toEqual(["src/lib.rs:12"]);
  });

  test("C02e the contextual scope names itself", async () => {
    const repo = await feature();
    const { report } = await scan(repo.path, ["--scope", "contextual"], { sandbox: repo.sandbox });
    expect(report.scope?.mode).toBe("contextual");
    expect(await scopeLine(repo.path, ["--scope", "contextual"], { sandbox: repo.sandbox })).toBe(
      "Scope: changed lines and the functions holding them (1 files, base main at af441c9b9f96)",
    );
  });

  test("C02f a baseline of a branch that adds no finding introduces none", async () => {
    const repo = await gitScenario("c02f")
      .template("mini").commit("base")
      .branch("feature").append("src/lib.rs", "\npub fn f() {}\n").commit("feature")
      .build();
    const { run, report } = await scan(repo.path, ["--scope", "baseline"], { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(0);
    expect(report.scope).toMatchObject({
      mode: "baseline",
      comparison_base: MINI_BASE,
      base_ref: "main",
      files: null,
      staged: false,
      untracked_unreported: null,
    });
    expect(deltaSummary(report)).toEqual([0, 2, 0, 0, 0]);
    expect(await scopeLine(repo.path, ["--scope", "baseline"], { sandbox: repo.sandbox })).toBe(
      "Scope: baseline comparison (base main at af441c9b9f96)",
    );
  });
});

describe("default branch names", () => {
  const master = once(async () => {
    const repo = await gitScenario("c03").template("mini").commit("base").build();
    await repo.git(["branch", "-q", "-m", "main", "master"]);
    return repo;
  });
  const masterFeature = once(async () => {
    const repo = await gitScenario("c03b").template("mini").commit("base").branch("feature").append("src/lib.rs", NEW_FN).commit("feature").build();
    await repo.git(["branch", "-q", "-m", "main", "master"]);
    return repo;
  });
  const trunk = once(async () => {
    const repo = await gitScenario("c04").template("mini").commit("base").build();
    await repo.git(["branch", "-q", "-m", "main", "trunk"]);
    return repo;
  });

  test("C03 on master the base is HEAD", async () => {
    const repo = await master();
    const { report } = await scan(repo.path, FILES, { sandbox: repo.sandbox });
    expect(report.scope?.base_ref).toBe("HEAD");
  });

  test("C03b a branch off master detects master", async () => {
    const repo = await masterFeature();
    const { report } = await scan(repo.path, FILES, { sandbox: repo.sandbox });
    expect(report.scope?.base_ref).toBe("master");
  });

  test("C04 a trunk repository has no base to detect", async () => {
    const repo = await trunk();
    await failsWith(repo, FILES, "scope/base-undetected");
    const { report } = await scan(repo.path, FILES, { sandbox: repo.sandbox });
    expect(report.errors[0]?.message).toBe("No base branch found: pass --base <REF>.");
    expect(await scopeLine(repo.path, FILES, { sandbox: repo.sandbox })).toBe("Scope: files (not resolved)");
  });

  test("C04b a baseline on trunk has no base to detect", async () => {
    const repo = await trunk();
    await failsWith(repo, ["--scope", "baseline"], "scope/base-undetected");
    expect(await scopeLine(repo.path, ["--scope", "baseline"], { sandbox: repo.sandbox })).toBe("Scope: baseline (not resolved)");
  });

  test("C04c a staged scan never detects a branch", async () => {
    const repo = await trunk();
    const { run, report } = await scan(repo.path, [...FILES, "--staged"], { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(0);
    expect(report.scope).toMatchObject({ base_ref: "HEAD", staged: true, untracked_unreported: null });
  });

  test("C04d naming trunk resolves it", async () => {
    const repo = await trunk();
    const { run, report } = await scan(repo.path, [...FILES, "--base", "trunk"], { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(0);
    expect(report.scope?.base_ref).toBeNull();
  });
});

describe("remotes", () => {
  const develop = gitScenario("c05")
    .template("mini").commit("base")
    .branch("develop").write({ "README.md": "# develop\n" }).commit("develop");
  const onDevelop = once(() => develop.clone().build());
  const topic = () =>
    gitScenario("c05b")
      .template("mini").commit("base")
      .branch("develop").write({ "README.md": "# develop\n" }).commit("develop")
      .clone()
      .branch("topic").append("src/lib.rs", NEW_FN).commit("topic");
  const onTopic = once(() => topic().build());

  test("C05 on the branch origin/HEAD names, the base is HEAD", async () => {
    const repo = await onDevelop();
    const { report } = await scan(repo.path, FILES, { sandbox: repo.sandbox });
    expect(report.scope).toMatchObject({ base_ref: "HEAD", comparison_base: await repo.sha("develop") });
  });

  test("C05b origin/HEAD wins over origin/main", async () => {
    const repo = await onTopic();
    const { report } = await scan(repo.path, FILES, { sandbox: repo.sandbox });
    expect(report.scope?.base_ref).toBe("origin/develop");
  });

  test("C05c a moved origin/HEAD is followed", async () => {
    const repo = await topic().setHead("main").build();
    const { report } = await scan(repo.path, FILES, { sandbox: repo.sandbox });
    expect(report.scope).toMatchObject({ base_ref: "origin/main", comparison_base: MINI_BASE });
  });

  test("C05d without origin/HEAD the base falls back to origin/main", async () => {
    const repo = await topic().build();
    await repo.git(["remote", "set-head", "origin", "-d"]);
    const { report } = await scan(repo.path, FILES, { sandbox: repo.sandbox });
    expect(report.scope?.base_ref).toBe("origin/main");
  });

  test("C05e a named remote branch publishes no ref", async () => {
    const repo = await onTopic();
    const { report } = await scan(repo.path, [...FILES, "--base", "origin/develop"], { sandbox: repo.sandbox });
    expect(report.scope).toMatchObject({ base_ref: null, comparison_base: await repo.sha("origin/develop") });
  });

  test("C05f an origin/HEAD the grammar refuses is skipped for the next candidate", async () => {
    const repo = await gitScenario("c05f")
      .template("mini").commit("base")
      .branch("feat@x").write({ "README.md": "# feat\n" }).commit("feat")
      .clone()
      .branch("topic").append("src/lib.rs", NEW_FN).commit("topic")
      .build();
    expect((await repo.git(["symbolic-ref", "refs/remotes/origin/HEAD"])).trim()).toBe("refs/remotes/origin/feat@x");
    const { run, report } = await scan(repo.path, FILES, { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(0);
    expect(report.scope).toMatchObject({ base_ref: "origin/main", comparison_base: MINI_BASE });
  });
});

describe("detached HEAD", () => {
  const detached = once(() => gitScenario("c06").template("mini").commit("base").write({ "README.md": "# c06\n" }).commit("readme").detach().build());
  const detachedCommit = once(() =>
    gitScenario("c06b")
      .template("mini").commit("base")
      .write({ "README.md": "# c06\n" }).commit("readme")
      .detach().append("src/lib.rs", "\npub fn f() {}\n").commit("detached")
      .build(),
  );

  test("C06 detached at the main tip the base is main, not HEAD", async () => {
    const repo = await detached();
    const { report } = await scan(repo.path, FILES, { sandbox: repo.sandbox });
    expect(report.scope).toMatchObject({ base_ref: "main", files: [] });
  });

  test("C06b a commit made detached is judged against main", async () => {
    const repo = await detachedCommit();
    const { report } = await scan(repo.path, FILES, { sandbox: repo.sandbox });
    expect(report.scope).toMatchObject({ base_ref: "main", files: ["src/lib.rs"] });
  });

  test("C06c a baseline of a detached commit that adds no finding introduces none", async () => {
    const repo = await detachedCommit();
    const { report } = await scan(repo.path, ["--scope", "baseline"], { sandbox: repo.sandbox });
    expect(report.scope?.base_ref).toBe("main");
    expect(deltaSummary(report)).toEqual([0, 2, 0, 0, 0]);
  });

  test("C09 a remote base without a remote is unavailable", async () => {
    await failsWith(await detached(), [...FILES, "--base", "origin/main"], "scope/base-unavailable");
  });

  test("C09b an unknown branch is unavailable", async () => {
    await failsWith(await detached(), [...FILES, "--base", "nosuch"], "scope/base-unavailable");
  });

  test("C09c a baseline against an unknown branch is unavailable", async () => {
    await failsWith(await detached(), ["--scope", "baseline", "--base", "nosuch"], "scope/base-unavailable");
  });
});

describe("shallow clones", () => {
  // main holds four commits; feature adds NEWFN on top of them, and the
  // origin keeps main checked out.
  const origin = once(() =>
    gitScenario("c08")
      .template("mini").commit("base")
      .write({ "README.md": "1\n" }).commit("two")
      .write({ "README.md": "2\n" }).commit("three")
      .write({ "README.md": "3\n" }).commit("four")
      .branch("feature").append("src/lib.rs", NEW_FN).commit("feature")
      .switch("main")
      .build(),
  );
  const singleBranch = once(async () => cloneOf(await origin(), "c08-single", ["--depth", "1", "--branch", "feature"]));
  const bothTips = once(async () => cloneOf(await origin(), "c08-both", ["--depth", "1", "--no-single-branch", "--branch", "feature"]));

  test("C08 a depth-1 single-branch clone has no base to detect", async () => {
    await failsWith(await singleBranch(), FILES, "scope/base-undetected");
  });

  test("C08b a named base missing from a shallow clone names the fetch", async () => {
    const repo = await singleBranch();
    await failsWith(repo, [...FILES, "--base", "origin/main"], "scope/shallow-clone");
    const { report } = await scan(repo.path, [...FILES, "--base", "origin/main"], { sandbox: repo.sandbox });
    expect(report.errors[0]?.message).toBe("This clone is shallow: fetch with fetch-depth: 0, or pass --base to a fetched ref.");
  });

  test("C08c --base HEAD works in a shallow clone", async () => {
    const repo = await singleBranch();
    const { run, report } = await scan(repo.path, [...FILES, "--base", "HEAD"], { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(0);
    expect(report.scope?.files).toEqual([]);
  });

  test("C08d a baseline against HEAD in a shallow clone introduces nothing", async () => {
    const repo = await singleBranch();
    const { run, report } = await scan(repo.path, ["--scope", "baseline", "--base", "HEAD"], { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(0);
    expect(deltaSummary(report)).toEqual([0, 3, 0, 0, 0]);
  });

  test("C08e both tips without their merge base name the shallow clone", async () => {
    await failsWith(await bothTips(), FILES, "scope/shallow-clone");
  });

  test("C08f a baseline without the merge base names the shallow clone", async () => {
    await failsWith(await bothTips(), ["--scope", "baseline"], "scope/shallow-clone");
  });

  test("C08g a clone deep enough to hold the merge base resolves it", async () => {
    const repo = await cloneOf(await origin(), "c08g", ["--depth", "3", "--no-single-branch", "--branch", "feature"]);
    const { run, report } = await scan(repo.path, FILES, { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(0);
    expect(report.scope).toMatchObject({ base_ref: "origin/main", files: ["src/lib.rs"] });
  });
});

describe("merge bases", () => {
  const crissCross = once(() => gitScenario("c10").template("mini").commit("base").crissCross("a", "b").build());

  test("C10 two merge bases are ambiguous", async () => {
    await failsWith(await crissCross(), [...FILES, "--base", "a"], "scope/merge-base-ambiguous");
  });

  test("C10b a baseline across two merge bases is ambiguous", async () => {
    await failsWith(await crissCross(), ["--scope", "baseline", "--base", "a"], "scope/merge-base-ambiguous");
  });

  const merged = once(async () => {
    const repo = await gitScenario("c10c")
      .template("mini").commit("base")
      .branch("side").write({ "side.txt": "side\n" }).commit("side")
      .switch("main")
      .build();
    await repo.git(["merge", "--no-ff", "-q", "-m", "merge side", "side"]);
    await repo.git(["switch", "-q", "-c", "after"]);
    writeFiles(repo.path, { "src/lib.rs": miniTemplate()["src/lib.rs"]! + NEW_FN });
    await repo.git(["commit", "-q", "-am", "after"]);
    return repo;
  });

  test("C10c a merge commit base is compared from the merge", async () => {
    const repo = await merged();
    const { run, report } = await scan(repo.path, [...FILES, "--base", "main"], { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(0);
    expect(report.scope).toMatchObject({ comparison_base: await repo.sha("main"), files: ["src/lib.rs"] });
  });

  test("C10d a parent selector is refused by the grammar", async () => {
    const repo = await merged();
    const { run, report } = await scan(repo.path, [...FILES, "--base", "main~1"], { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(2);
    expect(errorCodes(report)).toEqual(["scope/invalid-base"]);
  });

  test("C11 unrelated histories have no merge base", async () => {
    const repo = await gitScenario("c11")
      .template("mini").commit("base")
      .orphan("other").write({ "README.md": "# other\n" }).commit("other")
      .switch("main")
      .build();
    await failsWith(repo, [...FILES, "--base", "other"], "scope/merge-base-unavailable");
  });

  test("C12 a base ahead of HEAD merges back to HEAD", async () => {
    const repo = await gitScenario("c12")
      .template("mini").commit("base")
      .branch("ahead").append("src/lib.rs", NEW_FN).commit("ahead")
      .switch("main")
      .build();
    const { run, report } = await scan(repo.path, [...FILES, "--base", "ahead"], { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(0);
    expect(report.scope).toMatchObject({ comparison_base: MINI_BASE, files: [] });
  });
});

describe("repositories without a commit", () => {
  const unborn = once(() => gitScenario("c13").template("mini").build());
  const indexed = once(async () => {
    const repo = await gitScenario("c13d").template("mini").build();
    await repo.git(["add", "-A"]);
    return repo;
  });

  test("C13 an unborn repository has no base to detect", async () => {
    await failsWith(await unborn(), FILES, "scope/base-undetected");
  });

  test("C13b HEAD is unavailable before the first commit", async () => {
    await failsWith(await unborn(), [...FILES, "--base", "HEAD"], "scope/base-unavailable");
  });

  test("C13c a staged scan without an index says so", async () => {
    await failsWith(await unborn(), [...FILES, "--staged"], "scope/index-unavailable");
  });

  test("C13d a staged scan on an unborn HEAD has no base", async () => {
    await failsWith(await indexed(), [...FILES, "--staged"], "scope/base-unavailable");
  });

  test("C13e a baseline on an unborn HEAD has no base to detect", async () => {
    await failsWith(await indexed(), ["--scope", "baseline"], "scope/base-undetected");
  });

  test("C13f a full scan counts files only the index tracks", async () => {
    const repo = await indexed();
    const { run, report } = await scan(repo.path, [], { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(0);
    expect(count(report, "rust_doctor::repo::hardcoded_credential")).toBe(1);
  });
});

describe("outside git", () => {
  const plain = once(async (): Promise<PlainRepo> => {
    const path = join(scratchDir("c14"), "mini");
    writeFiles(path, miniTemplate());
    return { path, sandbox: sandbox("c14"), git: async () => "" };
  });

  test("C14 a changed scope outside git has no base to detect", async () => {
    await failsWith(await plain(), FILES, "scope/base-undetected");
  });

  test("C14b a named base outside git is unavailable", async () => {
    await failsWith(await plain(), ["--scope", "lines", "--base", "HEAD"], "scope/base-unavailable");
  });

  test("C14c a staged scan outside git has no index", async () => {
    await failsWith(await plain(), [...FILES, "--staged"], "scope/index-unavailable");
  });

  test("C14d a baseline outside git is unavailable", async () => {
    await failsWith(await plain(), ["--scope", "baseline", "--base", "main"], "scope/base-unavailable");
  });

  test("C14e a full scan outside git reports no repository finding", async () => {
    const repo = await plain();
    const { run, report } = await scan(repo.path, [], { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(0);
    expect(report.diagnostics.filter((diagnostic) => diagnostic.code.startsWith("rust_doctor::repo::"))).toEqual([]);
  });
});

describe("tags and object names", () => {
  const tagged = once(async () => {
    const repo = await gitScenario("c15")
      .template("mini").commit("base")
      .append("src/lib.rs", NEW_FN).commit("edit")
      .write({ "src/extra.rs": "pub fn extra() -> u8 {\n    2\n}\n" }).append("src/lib.rs", "pub mod extra;\n").commit("extra")
      .build();
    await repo.git(["tag", "-a", "-m", "v1.0", "v1.0", MINI_BASE]);
    return repo;
  });

  test("C15 an annotated tag is peeled to its commit", async () => {
    const repo = await tagged();
    const { run, report } = await scan(repo.path, [...FILES, "--base", "v1.0"], { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(0);
    expect(report.scope).toMatchObject({ comparison_base: MINI_BASE, files: ["src/extra.rs", "src/lib.rs"] });
  });

  test("C15b the tag's commit by object name gives the same range", async () => {
    const repo = await tagged();
    const commit = await repo.sha("v1.0^{commit}");
    const { report } = await scan(repo.path, [...FILES, "--base", commit], { sandbox: repo.sandbox });
    expect(report.scope).toMatchObject({ comparison_base: MINI_BASE, files: ["src/extra.rs", "src/lib.rs"] });
  });

  test("C15c the lines scope against a tag keeps only the changed lines", async () => {
    const repo = await tagged();
    const { report } = await scan(repo.path, ["--scope", "lines", "--base", "v1.0"], { sandbox: repo.sandbox });
    expect(report.scope).toMatchObject({ mode: "lines", files: ["src/extra.rs", "src/lib.rs"] });
    expect(sites(report)).toEqual(["src/lib.rs:12"]);
  });

  test("H02 a tree object is not a commit base", async () => {
    const repo = await tagged();
    await failsWith(repo, [...FILES, "--base", await repo.sha("HEAD^{tree}")], "scope/base-unavailable");
  });

  test("H02b a blob object is not a commit base", async () => {
    const repo = await tagged();
    await failsWith(repo, [...FILES, "--base", await repo.sha("HEAD:src/lib.rs")], "scope/base-unavailable");
  });

  test("H03 an orphan branch with an emptied worktree has no manifest", async () => {
    const repo = await gitScenario("h03").template("mini").commit("base").orphan("fresh").build();
    const { run, report } = await scan(repo.path, FILES, { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(2);
    expect(errorCodes(report)).toEqual(["discovery/no-manifest"]);
  });
});

describe("SHA-256 repositories", () => {
  const repo = once(() => sha256Mini("l01", NEW_FN));

  test("L01 a SHA-256 repository resolves a 64-hex base", async () => {
    const sha256 = await repo();
    const { run, report } = await scan(sha256.path, FILES, { sandbox: sha256.sandbox });
    expect(run.exitCode).toBe(0);
    expect(report.scope).toMatchObject({ base_ref: "main", comparison_base: (await sha256.git(["rev-parse", "main"])).trim() });
    expect(report.scope?.comparison_base).toMatch(/^[0-9a-f]{64}$/);
  });

  test("L01c a baseline in a SHA-256 repository introduces only the branch's finding", async () => {
    const sha256 = await repo();
    const { run, report } = await scan(sha256.path, ["--scope", "baseline"], { sandbox: sha256.sandbox });
    expect(run.exitCode).toBe(0);
    expect(deltaSummary(report)).toEqual([1, 2, 0, 0, 0]);
    expect(introduced(report)).toEqual(["clippy::todo@src/lib.rs:12"]);
  });

  test("L01d a staged scan in a SHA-256 repository compares with HEAD", async () => {
    const sha256 = await repo();
    const { run, report } = await scan(sha256.path, [...FILES, "--staged"], { sandbox: sha256.sandbox });
    expect(run.exitCode).toBe(0);
    expect(report.scope?.base_ref).toBe("HEAD");
  });
});
