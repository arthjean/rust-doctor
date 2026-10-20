// HG-01 to HG-30 of `tasks/e2e-catalog/subcommands.md` section 5: `hook
// install git` in every hook layout git and the hook managers know, and a
// real `git commit` through the hook it writes. US-011 changed the hook body
// (`--blocking warning`, a failed scan lets the commit through) and US-007
// made relative and missing paths behave, so HG-02, HG-07, HG-22, HG-26 and
// HG-27 assert that behavior rather than the catalog's 0.7.0 one.
import { describe, expect, test } from "bun:test";
import { existsSync, mkdirSync, readFileSync, rmSync, statSync, symlinkSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import {
  copyWorkspace,
  isEmptyDir,
  pathWithout,
  runCommand,
  runDoctor,
  sandbox,
  scratchDir,
  treeFingerprint,
} from "./e2e-helpers";
import { gitScenario, type Repo } from "./git-scenario";

const clean = (name: string) => gitScenario(name).fixture("projects/clean").write({ ".gitignore": "/target\n" }).commit("base").build();
const nested = (name: string, dir = "crates/x") => gitScenario(name).nested(dir, "mini").commit("base").build();

const command = (workspace = ".") => `rust-doctor ${workspace} --yes --staged --scope lines --blocking warning`;
const OLD_HOOK = "#!/bin/sh\n# rust-doctor pre-commit hook: judges what this commit records.\nexec rust-doctor . --yes --staged\n";
const FOREIGN_HOOK = "#!/bin/sh\nmake lint\n";
const OUTSIDE = "The hooks directory is outside this working tree, so nothing was written.\n";
const NOT_A_REPO = "rust-doctor: not a git repository, so no pre-commit hook was installed.\n";

const hookPath = (repo: Repo) => join(repo.path, ".git/hooks/pre-commit");

function install(repo: Repo, flags: string[] = [], path = repo.path, cwd?: string, env?: Record<string, string>) {
  return runDoctor(["hook", "install", "git", ...flags, path], { sandbox: repo.sandbox, cwd, env });
}

function plant(repo: Repo, body: string): string {
  const hook = hookPath(repo);
  writeFileSync(hook, body, { mode: 0o755 });
  return hook;
}

function snippet(config: string, workspace = "."): string {
  return config.includes("lefthook")
    ? `pre-commit:\n  commands:\n    rust-doctor:\n      run: ${command(workspace)}\n`
    : `- repo: local\n  hooks:\n    - id: rust-doctor\n      name: rust-doctor\n      entry: ${command(workspace)}\n      language: system\n      pass_filenames: false\n`;
}

function managerOutput(config: string): string {
  return `${config} runs this repository's hooks, and rust-doctor never edits it.\nAdd this hook to it:\n\n${snippet(config)}\n`;
}

describe("hook install git", () => {
  test("HG-01 --dry-run prints the script and writes nothing", async () => {
    const repo = await clean("hg-01");
    const before = treeFingerprint(repo.path);
    const run = await install(repo, ["--dry-run"]);
    expect(run.exitCode).toBe(0);
    expect(run.stdout).toStartWith("Would write .git/hooks/pre-commit:\n\n#!/bin/sh\n");
    expect(run.stdout).toContain(`\n${command()}\n`);
    expect(existsSync(hookPath(repo))).toBe(false);
    expect(treeFingerprint(repo.path)).toBe(before);
  });

  test("HG-02 install writes an executable hook running a staged lines scan that blocks on warnings", async () => {
    const repo = await clean("hg-02");
    const dry = await install(repo, ["--dry-run"]);
    const run = await install(repo);
    expect(run.exitCode).toBe(0);
    expect(run.stderr).toBe("");
    const script = readFileSync(hookPath(repo), "utf8");
    expect(run.stdout).toBe(`Wrote .git/hooks/pre-commit:\n\n${script}`);
    expect(dry.stdout).toBe(`Would write .git/hooks/pre-commit:\n\n${script}`);
    expect(statSync(hookPath(repo)).mode & 0o777).toBe(0o755);
    expect(script).toStartWith("#!/bin/sh\n# rust-doctor pre-commit hook");
    expect(script.split("\n")).toContain(command());
    expect(script).not.toContain("exec rust-doctor");
    expect((await runCommand(["sh", "-n", hookPath(repo)], { sandbox: repo.sandbox })).exitCode).toBe(0);
  });

  test("HG-03 a second install says it is already installed and writes nothing", async () => {
    const repo = await clean("hg-03");
    expect((await install(repo)).exitCode).toBe(0);
    const before = treeFingerprint(repo.path);
    const run = await install(repo);
    expect(run.exitCode).toBe(0);
    expect(run.stdout).toBe("The rust-doctor pre-commit hook is already installed in .git/hooks/pre-commit.\n");
    expect(treeFingerprint(repo.path)).toBe(before);
  });

  test("HG-04 --update and --update --dry-run over the current hook say it is already installed", async () => {
    const repo = await clean("hg-04");
    expect((await install(repo)).exitCode).toBe(0);
    const before = treeFingerprint(repo.path);
    for (const flags of [["--update"], ["--update", "--dry-run"]]) {
      const run = await install(repo, flags);
      expect([flags, run.exitCode, run.stdout]).toEqual([
        flags,
        0,
        "The rust-doctor pre-commit hook is already installed in .git/hooks/pre-commit.\n",
      ]);
    }
    expect(treeFingerprint(repo.path)).toBe(before);
  });

  test("HG-05 install without rust-doctor on PATH writes the hook and warns", async () => {
    const repo = await clean("hg-05");
    const run = await install(repo, [], repo.path, undefined, { PATH: pathWithout("rust-doctor") });
    expect(run.exitCode).toBe(0);
    expect(existsSync(hookPath(repo))).toBe(true);
    expect(run.stderr).toBe("Warning: `rust-doctor` is not on PATH, and the hook runs it by name.\n");
  });

  test("HG-06 an outdated rust-doctor hook is refused without --update", async () => {
    const repo = await clean("hg-06");
    plant(repo, OLD_HOOK);
    const before = treeFingerprint(repo.path);
    const run = await install(repo);
    expect(run.exitCode).toBe(1);
    expect(run.stdout).toBe(".git/hooks/pre-commit holds an outdated rust-doctor pre-commit hook: pass --update to rewrite it.\n");
    expect(treeFingerprint(repo.path)).toBe(before);
  });

  test("HG-07 --update --dry-run names the old and new command and writes nothing", async () => {
    const repo = await clean("hg-07");
    plant(repo, OLD_HOOK);
    const before = treeFingerprint(repo.path);
    const run = await install(repo, ["--update", "--dry-run"]);
    expect(run.exitCode).toBe(0);
    expect(run.stdout).toStartWith(
      `Would update .git/hooks/pre-commit: \`rust-doctor . --yes --staged\` becomes \`${command()}\`.\nWould write .git/hooks/pre-commit:\n\n#!/bin/sh\n`,
    );
    expect(treeFingerprint(repo.path)).toBe(before);
  });

  test("HG-08 --update rewrites an outdated hook to the current one", async () => {
    const repo = await clean("hg-08");
    const fresh = await clean("hg-08-fresh");
    expect((await install(fresh)).exitCode).toBe(0);
    plant(repo, OLD_HOOK);
    const run = await install(repo, ["--update"]);
    expect(run.exitCode).toBe(0);
    expect(run.stdout).toStartWith(`Updating .git/hooks/pre-commit: \`rust-doctor . --yes --staged\` becomes \`${command()}\`.\nWrote .git/hooks/pre-commit:\n\n`);
    expect(readFileSync(hookPath(repo), "utf8")).toBe(readFileSync(hookPath(fresh), "utf8"));
  });

  const foreign = `.git/hooks/pre-commit exists and was not written by rust-doctor, so nothing was written.\nAdd this line to it:\n\n${command()}\n`;
  test.each([
    ["HG-09", "install", [] as string[], 1],
    ["HG-10", "--update", ["--update"], 2],
    ["HG-11", "--dry-run", ["--dry-run"], 1],
  ] as const)("%s %s over a foreign hook prints the line to add and writes nothing", async (id, _label, flags, code) => {
    const repo = await clean(id.toLowerCase());
    plant(repo, FOREIGN_HOOK);
    const before = treeFingerprint(repo.path);
    const run = await install(repo, [...flags]);
    expect(run.exitCode).toBe(code);
    expect(run.stdout).toBe(foreign);
    expect(readFileSync(hookPath(repo), "utf8")).toBe(FOREIGN_HOOK);
    expect(treeFingerprint(repo.path)).toBe(before);
  });

  test.failing("HG-12 subcommands/F15 --update over a hand-edited hook keeps the user's lines", async () => {
    const repo = await gitScenario("hg-12").fixture("projects/clean").write({ ".gitignore": "/target\n" }).commit("base").build();
    expect((await runDoctor(["hook", "install", "git", repo.path], { sandbox: repo.sandbox })).exitCode).toBe(0);
    const hook = join(repo.path, ".git/hooks/pre-commit");
    writeFileSync(hook, `${readFileSync(hook, "utf8")}make lint\n`);
    const update = await runDoctor(["hook", "install", "git", "--update", repo.path], { sandbox: repo.sandbox });
    expect(update.exitCode).toBe(0);
    expect(readFileSync(hook, "utf8")).toContain("make lint\n");
  });

  test("HG-13 a pre-commit symlinked outside the repository is refused", async () => {
    const repo = await clean("hg-13");
    const outside = scratchDir("hg-13-outside");
    symlinkSync(join(outside, "pre-commit"), hookPath(repo));
    const before = treeFingerprint(repo.path);
    const run = await install(repo);
    expect(run.exitCode).toBe(2);
    expect(run.stderr).toBe("rust-doctor: .git/hooks/pre-commit is a symlink, and nothing is written through one.\n");
    expect(isEmptyDir(outside)).toBe(true);
    expect(treeFingerprint(repo.path)).toBe(before);
  });

  test("HG-14 a hooks directory symlinked outside the repository is refused", async () => {
    const repo = await clean("hg-14");
    const outside = scratchDir("hg-14-outside");
    rmSync(join(repo.path, ".git/hooks"), { recursive: true });
    symlinkSync(outside, join(repo.path, ".git/hooks"));
    const before = treeFingerprint(repo.path);
    const run = await install(repo);
    expect(run.exitCode).toBe(1);
    expect(run.stdout).toBe(`${OUTSIDE}Add this line to its pre-commit hook:\n\n${command()}\n`);
    expect(isEmptyDir(outside)).toBe(true);
    expect(treeFingerprint(repo.path)).toBe(before);
  });

  test("HG-15 core.hooksPath inside the repository receives the hook", async () => {
    const repo = await clean("hg-15");
    await repo.git(["config", "core.hooksPath", ".githooks"]);
    const run = await install(repo);
    expect(run.exitCode).toBe(0);
    expect(run.stdout).toStartWith("Wrote .githooks/pre-commit:\n\n");
    expect(readFileSync(join(repo.path, ".githooks/pre-commit"), "utf8").split("\n")).toContain(command());
    expect(existsSync(hookPath(repo))).toBe(false);
  });

  test("HG-16 core.hooksPath outside the repository is refused", async () => {
    const repo = await clean("hg-16");
    const shared = scratchDir("hg-16-shared");
    await repo.git(["config", "core.hooksPath", shared]);
    const before = treeFingerprint(repo.path);
    const run = await install(repo);
    expect(run.exitCode).toBe(1);
    expect(run.stdout).toStartWith(OUTSIDE);
    expect(isEmptyDir(shared)).toBe(true);
    expect(treeFingerprint(repo.path)).toBe(before);
  });

  test("HG-17 .cargo-husky/hooks wins over .pre-commit-config.yaml", async () => {
    const repo = await clean("hg-17");
    mkdirSync(join(repo.path, ".cargo-husky/hooks"), { recursive: true });
    writeFileSync(join(repo.path, ".pre-commit-config.yaml"), "repos: []\n");
    const run = await install(repo);
    expect(run.exitCode).toBe(0);
    expect(run.stdout).toStartWith("Wrote .cargo-husky/hooks/pre-commit:\n\n");
    expect(existsSync(join(repo.path, ".cargo-husky/hooks/pre-commit"))).toBe(true);
    expect(readFileSync(join(repo.path, ".pre-commit-config.yaml"), "utf8")).toBe("repos: []\n");
  });

  test("HG-18 .pre-commit-config.yaml gets a printed local repo entry and is never edited", async () => {
    const repo = await clean("hg-18");
    writeFileSync(join(repo.path, ".pre-commit-config.yaml"), "repos: []\n");
    const before = treeFingerprint(repo.path);
    const run = await install(repo);
    expect(run.exitCode).toBe(0);
    expect(run.stdout).toBe(managerOutput(".pre-commit-config.yaml"));
    expect(existsSync(hookPath(repo))).toBe(false);
    expect(treeFingerprint(repo.path)).toBe(before);
  });

  test.each(["lefthook.yml", "lefthook.yaml", ".lefthook.yml", ".lefthook.yaml"])(
    "HG-19 %s gets a printed lefthook command and is never edited",
    async (config) => {
      const repo = await clean(`hg-19-${config}`);
      writeFileSync(join(repo.path, config), "pre-push:\n  commands: {}\n");
      const before = treeFingerprint(repo.path);
      const run = await install(repo);
      expect(run.exitCode).toBe(0);
      expect(run.stdout).toBe(managerOutput(config));
      expect(treeFingerprint(repo.path)).toBe(before);
    },
  );

  test("HG-20 lefthook with --dry-run --update prints the same snippet", async () => {
    const repo = await clean("hg-20");
    writeFileSync(join(repo.path, "lefthook.yml"), "pre-push:\n  commands: {}\n");
    const before = treeFingerprint(repo.path);
    const run = await install(repo, ["--dry-run", "--update"]);
    expect(run.exitCode).toBe(0);
    expect(run.stdout).toBe(managerOutput("lefthook.yml"));
    expect(treeFingerprint(repo.path)).toBe(before);
  });

  test("HG-21 a crate outside git is refused", async () => {
    const box = sandbox("hg-21");
    const workspace = copyWorkspace("projects/clean");
    const before = treeFingerprint(workspace);
    const run = await runDoctor(["hook", "install", "git", workspace], { sandbox: box });
    expect(run.exitCode).toBe(2);
    expect(run.stderr).toBe(NOT_A_REPO);
    expect(treeFingerprint(workspace)).toBe(before);
  });

  test("HG-22 a path that does not exist is refused naming it, and nothing is created", async () => {
    const box = sandbox("hg-22");
    const missing = join(box.dir, "nonexistent/dir");
    const before = treeFingerprint(box.dir);
    const run = await runDoctor(["hook", "install", "git", missing], { sandbox: box });
    expect(run.exitCode).toBe(2);
    expect(run.stderr).toBe(`rust-doctor: PATH: ${missing} does not exist.\n`);
    expect(treeFingerprint(box.dir)).toBe(before);
  });

  test("HG-23 a linked worktree's hooks live in the common directory, outside its tree", async () => {
    const repo = await gitScenario("hg-23").fixture("projects/clean").write({ ".gitignore": "/target\n" }).commit("base").worktree("wt", "feature").build();
    const worktree = repo.worktrees.wt!;
    const before = [treeFingerprint(repo.path), treeFingerprint(worktree)];
    const run = await install(repo, [], worktree);
    expect(run.exitCode).toBe(1);
    expect(run.stdout).toStartWith(OUTSIDE);
    expect([treeFingerprint(repo.path), treeFingerprint(worktree)]).toEqual(before);
  });

  test("HG-24 a bare repository is refused as not a git repository", async () => {
    const box = sandbox("hg-24");
    const bare = join(box.dir, "bare.git");
    expect((await runCommand(["git", "init", "-q", "--bare", bare], { sandbox: box })).exitCode).toBe(0);
    const before = treeFingerprint(bare);
    const run = await runDoctor(["hook", "install", "git", bare], { sandbox: box });
    expect(run.exitCode).toBe(2);
    expect(run.stderr).toBe(NOT_A_REPO);
    expect(treeFingerprint(bare)).toBe(before);
  });

  test("HG-25 a nested workspace is named relative to the repository root", async () => {
    const repo = await nested("hg-25");
    const before = treeFingerprint(repo.path);
    const run = await install(repo, ["--dry-run"], join(repo.path, "crates/x"));
    expect(run.exitCode).toBe(0);
    expect(run.stdout.split("\n")).toContain(command("crates/x"));
    expect(treeFingerprint(repo.path)).toBe(before);
  });

  test.each(["crates/x", "./crates/x"])("HG-26 %s from the repository root installs as its absolute form does", async (relative) => {
    const repo = await nested(`hg-26-${relative.length}`);
    const absolute = await install(repo, ["--dry-run"], join(repo.path, "crates/x"));
    const run = await install(repo, [], relative, repo.path);
    expect(run.exitCode).toBe(0);
    expect(run.stdout).toBe(absolute.stdout.replace("Would write", "Wrote"));
    expect(readFileSync(hookPath(repo), "utf8").split("\n")).toContain(command("crates/x"));
  });

  test("HG-27 .. and ../.. from inside a nested workspace resolve against the current directory", async () => {
    const repo = await nested("hg-27");
    const before = treeFingerprint(repo.path);
    const cwd = join(repo.path, "crates/x");
    const parent = await install(repo, ["--dry-run"], "..", cwd);
    expect(parent.exitCode).toBe(0);
    expect(parent.stdout.split("\n")).toContain(command("crates"));
    const root = await install(repo, ["--dry-run"], "../..", cwd);
    expect(root.exitCode).toBe(0);
    expect(root.stdout.split("\n")).toContain(command("."));
    expect(treeFingerprint(repo.path)).toBe(before);
  });

  test("HG-28 a workspace path with a space is quoted for the shell", async () => {
    const repo = await nested("hg-28", "my app");
    const before = treeFingerprint(repo.path);
    const run = await install(repo, ["--dry-run"], join(repo.path, "my app"));
    expect(run.exitCode).toBe(0);
    expect(run.stdout.split("\n")).toContain(command("'my app'"));
    expect(treeFingerprint(repo.path)).toBe(before);
  });

  test("HG-29 GIT_DIR and GIT_INDEX_FILE naming another repository do not move the hook", async () => {
    const repo = await clean("hg-29");
    const other = await clean("hg-29-other");
    const otherBefore = treeFingerprint(other.path);
    const run = await install(repo, [], repo.path, undefined, {
      GIT_DIR: join(other.path, ".git"),
      GIT_INDEX_FILE: join(other.path, ".git/index"),
    });
    expect(run.exitCode).toBe(0);
    expect(existsSync(hookPath(repo))).toBe(true);
    expect(existsSync(hookPath(other))).toBe(false);
    expect(treeFingerprint(other.path)).toBe(otherBefore);
  });

  test("HG-30 every refusal leaves the tree unchanged", async () => {
    const refusals: Array<[string, (repo: Repo) => void, string[], number]> = [
      ["outdated", (repo) => void plant(repo, OLD_HOOK), [], 1],
      ["outdated --update --dry-run", (repo) => void plant(repo, OLD_HOOK), ["--update", "--dry-run"], 0],
      ["foreign", (repo) => void plant(repo, FOREIGN_HOOK), [], 1],
      ["foreign --update", (repo) => void plant(repo, FOREIGN_HOOK), ["--update"], 2],
      ["foreign --dry-run", (repo) => void plant(repo, FOREIGN_HOOK), ["--dry-run"], 1],
      ["symlinked hook", (repo) => symlinkSync(join(repo.sandbox.dir, "elsewhere"), hookPath(repo)), [], 2],
      ["pre-commit framework", (repo) => writeFileSync(join(repo.path, ".pre-commit-config.yaml"), "repos: []\n"), [], 0],
      ["lefthook", (repo) => writeFileSync(join(repo.path, "lefthook.yml"), "{}\n"), ["--update"], 0],
    ];
    for (const [label, setup, flags, code] of refusals) {
      const repo = await clean(`hg-30-${label.replace(/\W+/g, "-")}`);
      setup(repo);
      const before = treeFingerprint(repo.path);
      const run = await install(repo, flags);
      expect([label, run.exitCode, treeFingerprint(repo.path)]).toEqual([label, code, before]);
    }
  });
});

// US-011: the hook git runs on `git commit`, with hooks on (git-scenario
// turns them off for its own calls), PATH holding the `rust-doctor` link.
describe("a commit through the installed hook", () => {
  test("US-011 a staged warning is refused by git commit, and --no-verify lets it through", async () => {
    const repo = await clean("hg-commit");
    expect((await install(repo)).exitCode).toBe(0);
    const lib = join(repo.path, "src/lib.rs");
    writeFileSync(lib, `${readFileSync(lib, "utf8")}\npub fn later() -> u8 {\n    todo!()\n}\n`);
    await repo.git(["add", "src/lib.rs"]);
    const head = await repo.sha();
    const env = {
      GIT_AUTHOR_NAME: "e2e",
      GIT_AUTHOR_EMAIL: "e2e@example.invalid",
      GIT_COMMITTER_NAME: "e2e",
      GIT_COMMITTER_EMAIL: "e2e@example.invalid",
      CARGO_TARGET_DIR: join(repo.sandbox.dir, "hook-target"),
    };
    const commit = (extra: string[]) =>
      runCommand(["git", "-c", "commit.gpgsign=false", "commit", "-q", "-m", "add later", ...extra], {
        sandbox: repo.sandbox,
        cwd: repo.path,
        env,
        timeoutMs: 120_000,
      });
    const refused = await commit([]);
    expect(refused.exitCode).not.toBe(0);
    expect(`${refused.stdout}${refused.stderr}`).toContain("clippy::todo");
    expect(`${refused.stdout}${refused.stderr}`).not.toContain("this commit was not judged");
    expect(await repo.sha()).toBe(head);
    const forced = await commit(["--no-verify"]);
    expect(forced.exitCode).toBe(0);
    expect(await repo.sha()).not.toBe(head);
  }, 150_000);
});
