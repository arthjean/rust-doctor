// The git scenario builder, proven before any scope test relies on it: stable
// hashes, refused orderings, every helper's git state, and credentials that
// exist only at runtime.
import { describe, expect, test } from "bun:test";
import { lstatSync, mkdirSync, readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { runDoctor, sandbox } from "./e2e-helpers";
import { credential, gitScenario, NEW_FN, restoreFrom, waitForSnapshot } from "./git-scenario";

// The hash `tasks/e2e-catalog/scopes.md` recorded for the `mini` template's
// first commit: the template reproduces the catalog's bytes, so a test can
// assert `comparison_base` literally.
const MINI_BASE_SHA = "af441c9b9f96faa3f098b61e401b7475f31e56d7";

describe("git scenario builder", () => {
  test("the mini template's base commit has the same hash on two runs", async () => {
    const first = await gitScenario("stable-a").template("mini").commit("base").build();
    const second = await gitScenario("stable-b").template("mini").commit("base").build();
    expect(await first.sha()).toBe(await second.sha());
    expect(await first.sha()).toBe(MINI_BASE_SHA);
  });

  test("a step called before the history can hold it throws naming the step", async () => {
    await expect(gitScenario("early-stage").template("mini").stage({ write: { "a.rs": "" } }).build()).rejects.toThrow(
      "git scenario early-stage: stage needs a commit first",
    );
    await expect(gitScenario("early-commit").commit("base").build()).rejects.toThrow(
      "git scenario early-commit: commit(base) needs a file to commit first",
    );
    await expect(gitScenario("early-head").template("mini").commit("base").setHead("main").build()).rejects.toThrow(
      "git scenario early-head: setHead(main) needs a clone first",
    );
  });

  test("branch, stage, unstaged and untracked leave the git state they name", async () => {
    const repo = await gitScenario("states")
      .template("mini").commit("base")
      .branch("feature").append("src/lib.rs", NEW_FN).commit("feature")
      .stage({ write: { "src/leak.rs": "pub fn leak() {}\n" } })
      .unstaged({ "src/lib.rs": restoreFrom("main") })
      .untracked({ "src/fresh.rs": "pub fn fresh() {}\n" })
      .build();
    expect((await repo.git(["branch", "--show-current"])).trim()).toBe("feature");
    expect(await repo.git(["status", "--porcelain"])).toBe("A  src/leak.rs\n M src/lib.rs\n?? src/fresh.rs\n");
    expect(readFileSync(join(repo.path, "src/lib.rs"), "utf8")).not.toContain("new_debt");
  });

  test("clone, set-head, worktree, detach and orphan", async () => {
    const repo = await gitScenario("shapes")
      .template("mini").commit("base")
      .branch("feature").append("src/lib.rs", NEW_FN).commit("feature")
      .clone({ depth: 1, branch: "feature", singleBranch: true })
      .build();
    expect(repo.origin).toBeDefined();
    expect((await repo.git(["rev-list", "--count", "HEAD"])).trim()).toBe("1");
    expect((await repo.git(["branch", "-r"])).trim()).toBe("origin/feature");

    const full = await gitScenario("head").template("mini").commit("base").clone().setHead("main")
      .worktree("side", "side").detach().build();
    expect((await full.git(["symbolic-ref", "refs/remotes/origin/HEAD"])).trim()).toBe("refs/remotes/origin/main");
    expect((await full.git(["status", "--porcelain", "--branch"])).split("\n")[0]).toBe("## HEAD (no branch)");
    expect(statSync(join(full.worktrees.side!, "Cargo.toml")).isFile()).toBe(true);

    const orphan = await gitScenario("orphan").template("mini").commit("base").orphan("fresh").build();
    expect((await orphan.git(["branch", "--show-current"])).trim()).toBe("fresh");
    expect(await orphan.git(["log", "--oneline"]).catch(() => "no commit")).toBe("no commit");
  });

  test("criss-cross gives two merge bases, symlink and submodule land in the tree", async () => {
    const sub = await gitScenario("sub").template("mini").commit("base").build();
    const repo = await gitScenario("criss")
      .template("mini").commit("base")
      .crissCross()
      .switch("main")
      .symlink("src/alias.rs", "creds.rs")
      .submodule("vendor/sub", sub)
      .commit("links")
      .build();
    expect((await repo.git(["merge-base", "--all", "left", "right"])).trim().split("\n")).toHaveLength(2);
    expect(lstatSync(join(repo.path, "src/alias.rs")).isSymbolicLink()).toBe(true);
    expect(await repo.git(["ls-files", "--stage", "vendor/sub"])).toStartWith("160000 ");
  });

  test("racyIndex leaves a clean file the index's stat data cannot vouch for", async () => {
    const repo = await gitScenario("racy").template("mini").commit("base").racyIndex("src/lib.rs").build();
    const index = statSync(join(repo.path, ".git/index"));
    const file = statSync(join(repo.path, "src/lib.rs"));
    expect(file.mtimeMs).toBeGreaterThanOrEqual(index.mtimeMs);
    expect(await repo.git(["diff-index", "--name-only", "HEAD"])).toBe("");
  });

  test("waitForSnapshot resolves on the snapshot directory and fails naming the TMPDIR", async () => {
    const box = sandbox("snapshot");
    const appearing = waitForSnapshot(box.tmp);
    setTimeout(() => mkdirSync(join(box.tmp, "rust-doctor-baseline-1-0")), 100);
    expect(await appearing).toBe(join(box.tmp, "rust-doctor-baseline-1-0"));
    await expect(waitForSnapshot(sandbox("none").tmp, 100)).rejects.toThrow("no rust-doctor-baseline-* directory appeared");
  });

  test("the runtime credentials are the ones the repository pass reports", async () => {
    const repo = await gitScenario("creds").template("mini")
      .write({
        "src/token.rs": `pub fn token() -> &'static str {\n    "${credential("token")}"\n}\n`,
        "src/db.rs": `pub fn url() -> &'static str {\n    "${credential("database-url")}"\n}\n`,
      })
      .append("src/lib.rs", "pub mod token;\npub mod db;\n")
      .commit("base").build();
    const run = await runDoctor([repo.path, "--yes", "--json"], { sandbox: repo.sandbox });
    const report = run.json as { diagnostics: Array<{ code: string; path: string; span: { line_start: number } | null }> };
    const found = report.diagnostics
      .filter((diagnostic) => diagnostic.code === "rust_doctor::repo::hardcoded_credential")
      .map((diagnostic) => `${diagnostic.path}:${diagnostic.span?.line_start}`)
      .sort();
    expect(found).toEqual(["src/creds.rs:2", "src/db.rs:2", "src/token.rs:2"]);
    expect(run.stdout).not.toContain(credential("access-key"));
  });

  test("no credential-shaped literal is committed under tests/e2e", () => {
    const shapes = /AKIA[A-Z0-9]{16}|ghp_[A-Za-z0-9]{36}|xoxb-[A-Za-z0-9-]{10}|(?:postgres|mysql|redis|mongodb):\/\/[^:\s"'`]+:[^@\s"'`]+@/;
    const offenders: string[] = [];
    const walk = (dir: string) => {
      for (const name of readdirSync(dir)) {
        if (name === "node_modules") continue;
        const path = join(dir, name);
        if (lstatSync(path).isDirectory()) walk(path);
        else if (shapes.test(readFileSync(path, "utf8"))) offenders.push(relative(import.meta.dir, path));
      }
    };
    walk(import.meta.dir);
    expect(offenders).toEqual([]);
  });
});
