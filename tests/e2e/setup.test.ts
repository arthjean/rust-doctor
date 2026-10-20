// SU-01 to SU-19 of `tasks/e2e-catalog/subcommands.md` section 9: `setup`,
// which detects the agents through the fake PATH profiles of `fakes.ts` and
// the workspace's agent directories, then installs the skill, the git hook
// and the agent Stop hooks in one pass.
import { describe, expect, test } from "bun:test";
import { chmodSync, existsSync, lstatSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join, relative } from "node:path";
import {
  cargoVersion,
  copyWorkspace,
  runDoctor,
  sandbox,
  scratchDir,
  toolchainBin,
  treeFingerprint,
  type DoctorOptions,
} from "./e2e-helpers";
import { fakeCalls, fakeEnv, type Profile } from "./fakes";
import { gitScenario, type Repo } from "./git-scenario";

const VERSION = cargoVersion();
const HOOK_COMMAND = "rust-doctor . --yes --staged --scope lines --blocking warning";
const NO_CI = "setup never adds a CI workflow: run `rust-doctor ci install` for GitHub Actions.";
const STOP_JSON = '{"hooks":[{"command":"rust-doctor hook run claude","timeout":600,"type":"command"}]}';
const NO_AGENT =
  "rust-doctor: no agent detected, so nothing was set up. Probed the `claude`, `codex` and `cursor-agent` commands on PATH, and the .claude/, .agents/, .codex/ and .cursor/ directories of the workspace.\n";
const skillFiles = (dir: string) => [`${dir}/SKILL.md`, `${dir}/references/expert-review.md`];

const clean = (name: string) => gitScenario(name).fixture("projects/clean").write({ ".gitignore": "/target\n" }).commit("base").build();

function setup(repo: Repo, args: string[], profile: Profile = "none", options: DoctorOptions = {}) {
  return runDoctor(["setup", ...args, repo.path], { sandbox: repo.sandbox, env: fakeEnv(repo.sandbox, profile), ...options });
}

// Every file of the workspace, `.git` left out but for the hook setup writes.
function files(root: string): string[] {
  const found: string[] = [];
  const walk = (dir: string) => {
    for (const name of readdirSync(dir)) {
      const path = join(dir, name);
      const rel = relative(root, path);
      if (rel === ".git" || rel === "target") continue;
      if (lstatSync(path).isDirectory()) walk(path);
      else found.push(rel);
    }
  };
  walk(root);
  if (existsSync(join(root, ".git/hooks/pre-commit"))) found.push(".git/hooks/pre-commit");
  return found.sort();
}

const added = (before: string[], after: string[]) => after.filter((path) => !before.includes(path));

function count(text: string, needle: string): number {
  return text.split(needle).length - 1;
}

describe("setup", () => {
  test("SU-01 no agent on PATH or in the workspace sets up nothing", async () => {
    const repo = await clean("su-01");
    const before = treeFingerprint(repo.path);
    const run = await setup(repo, []);
    expect(run.exitCode).toBe(2);
    expect(run.stdout).toBe("");
    expect(run.stderr).toBe(NO_AGENT);
    expect(treeFingerprint(repo.path)).toBe(before);
  });

  test("SU-02 --dry-run with a .claude directory prints the whole plan and writes nothing", async () => {
    const repo = await clean("su-02");
    mkdirSync(join(repo.path, ".claude"));
    const before = treeFingerprint(repo.path);
    const run = await setup(repo, ["--dry-run"]);
    expect(run.exitCode).toBe(0);
    expect(run.stdout).toStartWith(
      ["Detected: claude.", "Plan:", ...skillFiles(".claude/skills/rust-doctor").map((p) => `Would write ${p}`), "Would write .git/hooks/pre-commit:", "", "#!/bin/sh\n"].join("\n"),
    );
    expect(run.stdout).toContain(`\n${HOOK_COMMAND}\n`);
    expect(run.stdout).toEndWith(`Would add a Stop hook to .claude/settings.local.json: ${STOP_JSON}\n${NO_CI}\n`);
    expect(run.stdout).not.toContain("Writing:");
    expect(treeFingerprint(repo.path)).toBe(before);
  });

  test("SU-03 setup writes the skill, the git hook and the Stop hook, four files", async () => {
    const repo = await clean("su-03");
    mkdirSync(join(repo.path, ".claude"));
    const before = files(repo.path);
    const run = await setup(repo, []);
    expect(run.exitCode).toBe(0);
    const writing = run.stdout.slice(run.stdout.indexOf("Writing:\n"));
    expect(writing).toStartWith(["Writing:", ...skillFiles(".claude/skills/rust-doctor"), "Wrote .git/hooks/pre-commit:", ""].join("\n"));
    expect(writing).toEndWith(`Added a Stop hook to .claude/settings.local.json: ${STOP_JSON}\n${NO_CI}\n`);
    expect(added(before, files(repo.path))).toEqual([
      ".claude/settings.local.json",
      ".claude/skills/rust-doctor/SKILL.md",
      ".claude/skills/rust-doctor/references/expert-review.md",
      ".git/hooks/pre-commit",
    ]);
    expect(lstatSync(join(repo.path, ".git/hooks/pre-commit")).mode & 0o111).not.toBe(0);
    expect(readFileSync(join(repo.path, ".git/hooks/pre-commit"), "utf8")).toContain(`\n${HOOK_COMMAND}\n`);
    const settings = JSON.parse(readFileSync(join(repo.path, ".claude/settings.local.json"), "utf8"));
    expect(settings.hooks.Stop).toEqual(JSON.parse(`[${STOP_JSON}]`));
  });

  test("SU-04 a rerun reports everything installed and changes nothing", async () => {
    const repo = await clean("su-04");
    mkdirSync(join(repo.path, ".claude"));
    expect((await setup(repo, [])).exitCode).toBe(0);
    const before = treeFingerprint(repo.path);
    const run = await setup(repo, []);
    expect(run.exitCode).toBe(0);
    expect(count(run.stdout, `Would replace rust-doctor ${VERSION} with ${VERSION} in .claude/skills/rust-doctor.\n`)).toBe(1);
    expect(count(run.stdout, `Replaced rust-doctor ${VERSION} with ${VERSION} in .claude/skills/rust-doctor.\n`)).toBe(1);
    expect(count(run.stdout, "The rust-doctor pre-commit hook is already installed in .git/hooks/pre-commit.\n")).toBe(2);
    expect(count(run.stdout, "The rust-doctor Stop hook is already in .claude/settings.local.json.\n")).toBe(2);
    expect(treeFingerprint(repo.path)).toBe(before);
  });

  test("SU-05 every agent on PATH gets its skill and its hook, and none is started", async () => {
    const repo = await clean("su-05");
    const fingerprint = treeFingerprint(repo.path);
    const dry = await setup(repo, ["--dry-run"], "agents");
    expect(dry.exitCode).toBe(0);
    expect(dry.stdout).toStartWith("Detected: claude, codex, cursor.\nPlan:\n");
    expect(treeFingerprint(repo.path)).toBe(fingerprint);
    const before = files(repo.path);
    const run = await setup(repo, [], "agents");
    expect(run.exitCode).toBe(0);
    expect(added(before, files(repo.path))).toEqual([
      ".agents/skills/rust-doctor/SKILL.md",
      ".agents/skills/rust-doctor/references/expert-review.md",
      ".claude/settings.local.json",
      ".claude/skills/rust-doctor/SKILL.md",
      ".claude/skills/rust-doctor/references/expert-review.md",
      ".cursor/hooks.json",
      ".cursor/skills/rust-doctor/SKILL.md",
      ".cursor/skills/rust-doctor/references/expert-review.md",
      ".git/hooks/pre-commit",
    ]);
    expect(JSON.parse(readFileSync(join(repo.path, ".cursor/hooks.json"), "utf8")).hooks.stop).toEqual([
      { command: "rust-doctor hook run cursor" },
    ]);
    for (const agent of ["claude", "codex", "cursor-agent"]) expect(fakeCalls(repo.sandbox, agent)).toEqual([]);
  });

  test.failing("SU-06 subcommands/F8 a foreign pre-commit stops setup with the exit code hook install git gives it", async () => {
    const repo = await clean("su-06");
    mkdirSync(join(repo.path, ".cursor"));
    writeFileSync(join(repo.path, ".git/hooks/pre-commit"), "#!/bin/sh\nmake lint\n", { mode: 0o755 });
    const before = treeFingerprint(repo.path);
    const run = await setup(repo, []);
    expect(run.stdout).toContain(
      `.git/hooks/pre-commit exists and was not written by rust-doctor, so nothing was written.\nAdd this line to it:\n\n${HOOK_COMMAND}\n`,
    );
    expect(existsSync(join(repo.path, ".cursor/skills"))).toBe(false);
    expect(treeFingerprint(repo.path)).toBe(before);
    const standalone = await runDoctor(["hook", "install", "git", repo.path], { sandbox: repo.sandbox });
    expect(standalone.exitCode).toBe(1);
    expect(run.exitCode).toBe(standalone.exitCode);
  });

  test("SU-07 a crate outside git stops setup before the skill or the Stop hook is written", async () => {
    const box = sandbox("su-07");
    const workspace = copyWorkspace("projects/clean");
    mkdirSync(join(workspace, ".claude"));
    const before = treeFingerprint(workspace);
    const run = await runDoctor(["setup", workspace], { sandbox: box, env: fakeEnv(box, "none") });
    expect(run.exitCode).toBe(2);
    expect(run.stderr).toBe("rust-doctor: not a git repository, so no pre-commit hook was installed.\n");
    expect(readdirSync(join(workspace, ".claude"))).toEqual([]);
    expect(treeFingerprint(workspace)).toBe(before);
  });

  test("SU-08 a foreign skill directory refuses the whole setup", async () => {
    const repo = await clean("su-08");
    mkdirSync(join(repo.path, ".claude/skills/rust-doctor"), { recursive: true });
    writeFileSync(join(repo.path, ".claude/skills/rust-doctor/SKILL.md"), "---\nname: other\n---\nbody\n");
    const before = treeFingerprint(repo.path);
    const run = await setup(repo, []);
    expect(run.exitCode).toBe(1);
    expect(run.stdout).toStartWith("Detected: claude.\nPlan:\n");
    expect(run.stderr).toBe(
      "rust-doctor: the skill was not installed, .claude/skills/rust-doctor holds another skill: nothing was written.\n",
    );
    expect(treeFingerprint(repo.path)).toBe(before);
  });

  test("SU-09 an unparsable settings file refuses the whole setup", async () => {
    const repo = await clean("su-09");
    mkdirSync(join(repo.path, ".claude"));
    writeFileSync(join(repo.path, ".claude/settings.local.json"), "{,}\n");
    const before = treeFingerprint(repo.path);
    const run = await setup(repo, []);
    expect(run.exitCode).toBe(2);
    expect(run.stderr).toContain("Could not parse .claude/settings.local.json at line 1, column 2: nothing was written.");
    expect(treeFingerprint(repo.path)).toBe(before);
  });

  test("SU-10 a pre-commit framework gets a snippet, never a hook file", async () => {
    const repo = await clean("su-10");
    mkdirSync(join(repo.path, ".cursor"));
    writeFileSync(join(repo.path, ".pre-commit-config.yaml"), "repos: []\n");
    const before = files(repo.path);
    const run = await setup(repo, []);
    expect(run.exitCode).toBe(0);
    expect(count(run.stdout, ".pre-commit-config.yaml runs this repository's hooks, and rust-doctor never edits it.\n")).toBe(2);
    expect(count(run.stdout, `      entry: ${HOOK_COMMAND}\n`)).toBe(2);
    expect(added(before, files(repo.path))).toEqual([
      ".cursor/hooks.json",
      ".cursor/skills/rust-doctor/SKILL.md",
      ".cursor/skills/rust-doctor/references/expert-review.md",
    ]);
    expect(readFileSync(join(repo.path, ".pre-commit-config.yaml"), "utf8")).toBe("repos: []\n");
  });

  test("SU-11 a hooks directory outside the working tree refuses the whole setup", async () => {
    const repo = await clean("su-11");
    mkdirSync(join(repo.path, ".cursor"));
    await repo.git(["config", "core.hooksPath", scratchDir("su-11-hooks")]);
    const before = treeFingerprint(repo.path);
    const run = await setup(repo, []);
    expect(run.exitCode).toBe(1);
    expect(run.stdout).toContain(
      `The hooks directory is outside this working tree, so nothing was written.\nAdd this line to its pre-commit hook:\n\n${HOOK_COMMAND}\n`,
    );
    expect(treeFingerprint(repo.path)).toBe(before);
  });

  test("SU-12 a .codex directory gets the .agents skill and the git hook, and no agent hook", async () => {
    const repo = await clean("su-12");
    mkdirSync(join(repo.path, ".codex"));
    const before = files(repo.path);
    const run = await setup(repo, []);
    expect(run.exitCode).toBe(0);
    expect(run.stdout).toStartWith("Detected: codex.\n");
    expect(added(before, files(repo.path))).toEqual([
      ".agents/skills/rust-doctor/SKILL.md",
      ".agents/skills/rust-doctor/references/expert-review.md",
      ".git/hooks/pre-commit",
    ]);
  });

  test("SU-13 an outdated rust-doctor pre-commit is updated in place", async () => {
    const repo = await clean("su-13");
    mkdirSync(join(repo.path, ".claude"));
    const hook = join(repo.path, ".git/hooks/pre-commit");
    writeFileSync(hook, "#!/bin/sh\n# rust-doctor pre-commit hook: judges what this commit records.\nrust-doctor . --yes --staged --scope lines\n", { mode: 0o755 });
    const run = await setup(repo, []);
    expect(run.exitCode).toBe(0);
    const change = `.git/hooks/pre-commit: \`rust-doctor . --yes --staged --scope lines\` becomes \`${HOOK_COMMAND}\`.\n`;
    expect(run.stdout).toContain(`Would update ${change}`);
    expect(run.stdout).toContain(`Updating ${change}`);
    expect(readFileSync(hook, "utf8")).toContain(`\n${HOOK_COMMAND}\n`);
  });

  test("SU-14 a PATH that does not exist is refused before any plan, and nothing is created", async () => {
    const box = sandbox("su-14");
    const before = treeFingerprint(box.dir);
    const run = await runDoctor(["setup", "missing"], { sandbox: box, cwd: box.dir, env: fakeEnv(box, "agents") });
    expect(run.exitCode).toBe(2);
    expect(run.stdout).toBe("");
    expect(run.stderr).toBe("rust-doctor: PATH: missing does not exist.\n");
    expect(existsSync(join(box.dir, "missing"))).toBe(false);
    expect(treeFingerprint(box.dir)).toBe(before);
  });

  test("SU-15 a second positional argument is a usage error", async () => {
    const repo = await clean("su-15");
    const before = treeFingerprint(repo.path);
    const run = await runDoctor(["setup", ".", "extra"], { sandbox: repo.sandbox, cwd: repo.path, env: fakeEnv(repo.sandbox, "agents") });
    expect(run.exitCode).toBe(2);
    expect(run.stdout).toBe("");
    expect(run.stderr).toContain("unexpected argument 'extra' found");
    expect(run.stderr).toContain("Usage: rust-doctor setup [OPTIONS] [PATH]");
    expect(treeFingerprint(repo.path)).toBe(before);
  });

  const nested = async (name: string) => {
    const repo = await gitScenario(name).nested("crates/x", "mini").commit("base").build();
    mkdirSync(join(repo.path, "crates/x/.claude"));
    return repo;
  };

  test("SU-16 a relative member PATH from the repository root sets up what its absolute form does", async () => {
    const repo = await nested("su-16");
    const options = { sandbox: repo.sandbox, cwd: repo.path, env: fakeEnv(repo.sandbox, "none") };
    const absolute = await runDoctor(["setup", join(repo.path, "crates/x"), "--dry-run"], options);
    const before = files(repo.path);
    const run = await runDoctor(["setup", "crates/x"], options);
    expect(run.exitCode).toBe(0);
    expect(run.stdout.slice(0, run.stdout.indexOf("Writing:\n"))).toBe(absolute.stdout.slice(0, -NO_CI.length - 1));
    expect(added(before, files(repo.path))).toEqual([
      ".git/hooks/pre-commit",
      "crates/x/.claude/settings.local.json",
      "crates/x/.claude/skills/rust-doctor/SKILL.md",
      "crates/x/.claude/skills/rust-doctor/references/expert-review.md",
    ]);
    expect(readFileSync(join(repo.path, ".git/hooks/pre-commit"), "utf8")).toContain(
      "\nrust-doctor crates/x --yes --staged --scope lines --blocking warning\n",
    );
  });

  test("SU-17 an absolute member PATH --dry-run plans the hook on the member and its Stop hook", async () => {
    const repo = await nested("su-17");
    const before = treeFingerprint(repo.path);
    const run = await runDoctor(["setup", join(repo.path, "crates/x"), "--dry-run"], {
      sandbox: repo.sandbox,
      cwd: repo.path,
      env: fakeEnv(repo.sandbox, "none"),
    });
    expect(run.exitCode).toBe(0);
    expect(run.stdout).toStartWith("Detected: claude.\nPlan:\n");
    expect(run.stdout).toContain("\nrust-doctor crates/x --yes --staged --scope lines --blocking warning\n");
    expect(run.stdout).toContain(`Would add a Stop hook to .claude/settings.local.json: ${STOP_JSON}\n`);
    expect(treeFingerprint(repo.path)).toBe(before);
  });

  test("SU-18 a successful setup adds no CI workflow and says so last", async () => {
    const repo = await clean("su-18");
    const run = await setup(repo, [], "agents");
    expect(run.exitCode).toBe(0);
    expect(run.stdout).toEndWith(`\n${NO_CI}\n`);
    expect(existsSync(join(repo.path, ".github"))).toBe(false);
  });

  test("SU-19 a claude file on PATH that is not executable is not an agent", async () => {
    const repo = await clean("su-19");
    const dir = join(scratchDir("su-19-path"), "bin");
    mkdirSync(dir);
    writeFileSync(join(dir, "claude"), "#!/bin/sh\nexit 0\n");
    chmodSync(join(dir, "claude"), 0o644);
    const before = treeFingerprint(repo.path);
    const run = await setup(repo, [], "none", { env: { PATH: `${dir}:${toolchainBin()}` } });
    expect(run.exitCode).toBe(2);
    expect(run.stderr).toBe(NO_AGENT);
    expect(treeFingerprint(repo.path)).toBe(before);
  });
});
