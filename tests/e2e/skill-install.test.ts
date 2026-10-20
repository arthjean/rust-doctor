// SK-01 to SK-28 of `tasks/e2e-catalog/subcommands.md` section 8: `skill
// install`, its refusals and agent detection, and the notice a scan prints
// for an agent whose skill is missing.
import { describe, expect, test } from "bun:test";
import { chmodSync, existsSync, mkdirSync, readdirSync, readFileSync, symlinkSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import {
  cargoVersion,
  copyWorkspace,
  isEmptyDir,
  REPO_ROOT,
  runDoctor,
  sandbox,
  scratchDir,
  treeFingerprint,
  type DoctorOptions,
} from "./e2e-helpers";
import { fakeEnv } from "./fakes";
import { gitScenario, type Repo } from "./git-scenario";

const VERSION = cargoVersion();
const SHIPPED = join(REPO_ROOT, "skills/rust-doctor");
const STAMP = `<!-- Installed by rust-doctor ${VERSION}. Refresh with \`rust-doctor skill install --update\`. -->\n`;
// The shipped SKILL.md with the version stamp right after its front matter.
const INSTALLED = readFileSync(join(SHIPPED, "SKILL.md"), "utf8").replace(/^(---\n[\s\S]*?\n---\n)/, `$1${STAMP}`);
const CLAUDE = ".claude/skills/rust-doctor";
const SKILL = `${CLAUDE}/SKILL.md`;
const paths = (dir: string) => [`${dir}/SKILL.md`, `${dir}/references/expert-review.md`];
const wouldWrite = (dir: string) => paths(dir).map((path) => `Would write ${path}`);
const lines = (...rows: string[]) => `${rows.join("\n")}\n`;

const clean = (name: string) => gitScenario(name).fixture("projects/clean").write({ ".gitignore": "/target\n" }).commit("base").build();

function skill(repo: Repo, args: string[], options: DoctorOptions = {}) {
  return runDoctor(["skill", "install", ...args, repo.path], { sandbox: repo.sandbox, ...options });
}

async function installed(name: string, args: string[] = []): Promise<Repo> {
  const repo = await clean(name);
  expect((await skill(repo, args)).exitCode).toBe(0);
  return repo;
}

// An empty workspace directory, for the detection rows that need no crate.
function emptyRepo(name: string): Repo {
  const box = sandbox(name);
  const path = join(scratchDir(`${name}-ws`), "ws");
  mkdirSync(path);
  return { name, path, sandbox: box, worktrees: {}, git: async () => "", sha: async () => "", target: () => "" };
}

function setStamp(repo: Repo, replacement: string) {
  const path = join(repo.path, SKILL);
  const body = readFileSync(path, "utf8");
  expect(body).toContain(STAMP);
  writeFileSync(path, body.replace(STAMP, replacement));
}

describe("skill install", () => {
  test("SK-01 --dry-run names the two files and writes nothing", async () => {
    const repo = await clean("sk-01");
    const before = treeFingerprint(repo.path);
    const run = await skill(repo, ["--dry-run"]);
    expect(run.exitCode).toBe(0);
    expect(run.stdout).toBe(lines(...wouldWrite(CLAUDE)));
    expect(treeFingerprint(repo.path)).toBe(before);
  });

  test("SK-02 --agent all --dry-run names six files under three agents", async () => {
    const repo = await clean("sk-02");
    const before = treeFingerprint(repo.path);
    const run = await skill(repo, ["--agent", "all", "--dry-run"]);
    expect(run.exitCode).toBe(0);
    expect(run.stdout).toBe(
      lines(...wouldWrite(CLAUDE), ...wouldWrite(".agents/skills/rust-doctor"), ...wouldWrite(".cursor/skills/rust-doctor")),
    );
    expect(treeFingerprint(repo.path)).toBe(before);
  });

  test("SK-03 install writes the shipped skill with a version stamp after its front matter", async () => {
    const repo = await clean("sk-03");
    const run = await skill(repo, []);
    expect(run.exitCode).toBe(0);
    expect(run.stdout).toBe(lines(...paths(CLAUDE)));
    expect(readFileSync(join(repo.path, SKILL), "utf8")).toBe(INSTALLED);
    expect(readFileSync(join(repo.path, CLAUDE, "references/expert-review.md"))).toEqual(
      readFileSync(join(SHIPPED, "references/expert-review.md")),
    );
  });

  test("SK-04 a second install refuses and points at --update", async () => {
    const repo = await installed("sk-04");
    const before = treeFingerprint(repo.path);
    const run = await skill(repo, []);
    expect(run.exitCode).toBe(1);
    expect(run.stdout).toBe("");
    expect(run.stderr).toBe(
      `rust-doctor: the skill was not installed, ${CLAUDE} already exists; pass --update to refresh it.\n`,
    );
    expect(treeFingerprint(repo.path)).toBe(before);
  });

  test("SK-05 --update over the current copy replaces it with itself", async () => {
    const repo = await installed("sk-05");
    const run = await skill(repo, ["--update"]);
    expect(run.exitCode).toBe(0);
    expect(run.stdout).toBe(lines(`Replaced rust-doctor ${VERSION} with ${VERSION} in ${CLAUDE}.`, ...paths(CLAUDE)));
    expect(readFileSync(join(repo.path, SKILL), "utf8")).toBe(INSTALLED);
  });

  test("SK-06 --update --dry-run previews the replacement and writes nothing", async () => {
    const repo = await installed("sk-06");
    setStamp(repo, STAMP.replace(VERSION, "0.1.0"));
    const before = treeFingerprint(repo.path);
    const run = await skill(repo, ["--update", "--dry-run"]);
    expect(run.exitCode).toBe(0);
    expect(run.stdout).toBe(lines(`Would replace rust-doctor 0.1.0 with ${VERSION} in ${CLAUDE}.`, ...wouldWrite(CLAUDE)));
    expect(treeFingerprint(repo.path)).toBe(before);
  });

  test("SK-07 --update over an older copy names both versions and drops the local edit", async () => {
    const repo = await installed("sk-07");
    setStamp(repo, STAMP.replace(VERSION, "0.1.0"));
    writeFileSync(join(repo.path, SKILL), `${readFileSync(join(repo.path, SKILL), "utf8")}a local line\n`);
    const run = await skill(repo, ["--update"]);
    expect(run.exitCode).toBe(0);
    expect(run.stdout.split("\n")[0]).toBe(`Replaced rust-doctor 0.1.0 with ${VERSION} in ${CLAUDE}.`);
    expect(readFileSync(join(repo.path, SKILL), "utf8")).toBe(INSTALLED);
  });

  test("SK-08 --update over a copy without a stamp says no version was recorded", async () => {
    const repo = await installed("sk-08");
    setStamp(repo, "");
    const run = await skill(repo, ["--update"]);
    expect(run.exitCode).toBe(0);
    expect(run.stdout.split("\n")[0]).toBe(`Replaced a copy that recorded no version with ${VERSION} in ${CLAUDE}.`);
    expect(readFileSync(join(repo.path, SKILL), "utf8")).toBe(INSTALLED);
  });

  test("SK-09 --update refuses a directory holding another skill", async () => {
    const repo = await clean("sk-09");
    mkdirSync(join(repo.path, CLAUDE), { recursive: true });
    writeFileSync(join(repo.path, SKILL), "---\nname: other\n---\nbody\n");
    const before = treeFingerprint(repo.path);
    const run = await skill(repo, ["--update"]);
    expect(run.exitCode).toBe(1);
    expect(run.stderr).toBe(`rust-doctor: the skill was not installed, ${CLAUDE} holds another skill: nothing was written.\n`);
    expect(treeFingerprint(repo.path)).toBe(before);
  });

  test("SK-10 --update refuses a skill directory without SKILL.md the same way", async () => {
    const repo = await clean("sk-10");
    mkdirSync(join(repo.path, CLAUDE), { recursive: true });
    const before = treeFingerprint(repo.path);
    const run = await skill(repo, ["--update"]);
    expect(run.exitCode).toBe(1);
    expect(run.stderr).toBe(`rust-doctor: the skill was not installed, ${CLAUDE} holds another skill: nothing was written.\n`);
    expect(treeFingerprint(repo.path)).toBe(before);
  });

  test("SK-11 a stamp carrying an escape sequence reads as no recorded version and is never echoed", async () => {
    const repo = await installed("sk-11");
    setStamp(repo, STAMP.replace(`${VERSION}.`, "0.1\x1b[2J."));
    const run = await skill(repo, ["--update"]);
    expect(run.exitCode).toBe(0);
    expect(run.stdout.split("\n")[0]).toBe(`Replaced a copy that recorded no version with ${VERSION} in ${CLAUDE}.`);
    expect(run.stdout + run.stderr).not.toContain("\x1b");
  });

  test("SK-12 --update keeps a file the user added to the skill directory", async () => {
    const repo = await installed("sk-12");
    writeFileSync(join(repo.path, CLAUDE, "NOTES.md"), "mine\n");
    expect((await skill(repo, ["--update"])).exitCode).toBe(0);
    expect(readFileSync(join(repo.path, CLAUDE, "NOTES.md"), "utf8")).toBe("mine\n");
  });

  test("SK-13 --agent all refuses on the existing claude copy before creating the others", async () => {
    const repo = await installed("sk-13");
    const before = treeFingerprint(repo.path);
    const run = await skill(repo, ["--agent", "all"]);
    expect(run.exitCode).toBe(1);
    expect(run.stderr).toContain(`${CLAUDE} already exists`);
    expect(existsSync(join(repo.path, ".agents"))).toBe(false);
    expect(existsSync(join(repo.path, ".cursor"))).toBe(false);
    expect(treeFingerprint(repo.path)).toBe(before);
  });

  test("SK-14 --agent all --update replaces the claude copy and creates the two others", async () => {
    const repo = await installed("sk-14");
    const run = await skill(repo, ["--agent", "all", "--update"]);
    expect(run.exitCode).toBe(0);
    expect(run.stdout).toBe(
      lines(
        `Replaced rust-doctor ${VERSION} with ${VERSION} in ${CLAUDE}.`,
        ...paths(CLAUDE),
        ...paths(".agents/skills/rust-doctor"),
        ...paths(".cursor/skills/rust-doctor"),
      ),
    );
    for (const dir of [CLAUDE, ".agents/skills/rust-doctor", ".cursor/skills/rust-doctor"]) {
      expect(readFileSync(join(repo.path, dir, "SKILL.md"), "utf8")).toBe(INSTALLED);
    }
  });

  test("SK-15 --agent codex over the copy --agent all wrote refuses as already existing", async () => {
    const repo = await installed("sk-15", ["--agent", "all"]);
    const before = treeFingerprint(repo.path);
    const run = await skill(repo, ["--agent", "codex"]);
    expect(run.exitCode).toBe(1);
    expect(run.stderr).toBe(
      "rust-doctor: the skill was not installed, .agents/skills/rust-doctor already exists; pass --update to refresh it.\n",
    );
    expect(treeFingerprint(repo.path)).toBe(before);
  });

  test("SK-16 --agent detected with no agent anywhere names everything it probed", async () => {
    const repo = emptyRepo("sk-16");
    const run = await skill(repo, ["--agent", "detected"]);
    expect(run.exitCode).toBe(2);
    expect(run.stderr).toBe(
      "rust-doctor: the skill was not installed, no agent was detected: the `claude`, `codex` and `cursor-agent` commands on PATH, and the .claude/, .agents/, .codex/ and .cursor/ directories of the workspace.\n",
    );
    expect(isEmptyDir(repo.path)).toBe(true);
  });

  const detected: Array<[string, string, string]> = [
    ["SK-17", ".codex/", ".agents/skills/rust-doctor"],
    ["SK-18", ".agents/", ".agents/skills/rust-doctor"],
    ["SK-19", "cursor-agent", ".cursor/skills/rust-doctor"],
  ];
  test.each(detected)("%s --agent detected --dry-run follows %s", async (id, marker, target) => {
    const repo = emptyRepo(id.toLowerCase());
    const directory = marker.endsWith("/");
    if (directory) mkdirSync(join(repo.path, marker));
    const before = treeFingerprint(repo.path);
    const run = await skill(repo, ["--agent", "detected", "--dry-run"], {
      env: directory ? {} : fakeEnv(repo.sandbox, [marker]),
    });
    expect(run.exitCode).toBe(0);
    expect(run.stdout).toBe(lines(...wouldWrite(target)));
    expect(treeFingerprint(repo.path)).toBe(before);
  });

  test("SK-20 --agent detected with a .cursor directory writes the cursor copy only", async () => {
    const repo = emptyRepo("sk-20");
    mkdirSync(join(repo.path, ".cursor"));
    const run = await skill(repo, ["--agent", "detected"]);
    expect(run.exitCode).toBe(0);
    expect(run.stdout).toBe(lines(...paths(".cursor/skills/rust-doctor")));
    expect(readdirSync(repo.path)).toEqual([".cursor"]);
  });

  test("SK-21 a PATH that does not exist is refused, and nothing is created", async () => {
    const box = sandbox("sk-21");
    const before = treeFingerprint(box.dir);
    const run = await runDoctor(["skill", "install", "missing/deep"], { sandbox: box, cwd: box.dir });
    expect(run.exitCode).toBe(2);
    expect(run.stderr).toBe("rust-doctor: PATH: missing/deep does not exist.\n");
    expect(existsSync(join(box.dir, "missing"))).toBe(false);
    expect(treeFingerprint(box.dir)).toBe(before);
  });

  test("SK-22 a symlinked .claude is refused, and its target stays empty", async () => {
    const repo = await clean("sk-22");
    const outside = scratchDir("sk-22-outside");
    symlinkSync(outside, join(repo.path, ".claude"));
    const before = treeFingerprint(repo.path);
    const run = await skill(repo, []);
    expect(run.exitCode).toBe(1);
    expect(run.stderr).toBe("rust-doctor: the skill was not installed, .claude is a symlink, and nothing is written through one.\n");
    expect(isEmptyDir(outside)).toBe(true);
    expect(treeFingerprint(repo.path)).toBe(before);
  });

  test("SK-23 --update refuses a symlinked skill directory", async () => {
    const repo = await clean("sk-23");
    const outside = scratchDir("sk-23-outside");
    mkdirSync(join(repo.path, ".claude/skills"), { recursive: true });
    symlinkSync(outside, join(repo.path, CLAUDE));
    const before = treeFingerprint(repo.path);
    const run = await skill(repo, ["--update"]);
    expect(run.exitCode).toBe(1);
    expect(run.stderr).toBe(`rust-doctor: the skill was not installed, ${CLAUDE} is a symlink, and nothing is written through one.\n`);
    expect(isEmptyDir(outside)).toBe(true);
    expect(treeFingerprint(repo.path)).toBe(before);
  });

  test.failing("SK-24 subcommands/F8 a write failure exits as the hook and ci installers do", async () => {
    const repo = await gitScenario("sk-24").fixture("projects/clean").write({ ".gitignore": "/target\n" }).commit("base").build();
    const skills = join(repo.path, ".claude/skills");
    const workflows = join(repo.path, ".github/workflows");
    mkdirSync(skills, { recursive: true });
    mkdirSync(workflows, { recursive: true });
    chmodSync(skills, 0o555);
    chmodSync(workflows, 0o555);
    try {
      const skill = await runDoctor(["skill", "install", repo.path], { sandbox: repo.sandbox });
      const ci = await runDoctor(["ci", "install", repo.path], { sandbox: repo.sandbox });
      chmodSync(skills, 0o755);
      chmodSync(join(repo.path, ".claude"), 0o555);
      const hook = await runDoctor(["hook", "install", "claude", repo.path], { sandbox: repo.sandbox });
      expect([ci.exitCode, hook.exitCode]).toEqual([2, 2]);
      expect(skill.stderr).toContain("the skill was not installed");
      expect(skill.exitCode).toBe(2);
    } finally {
      chmodSync(join(repo.path, ".claude"), 0o755);
      chmodSync(skills, 0o755);
      chmodSync(workflows, 0o755);
    }
  });

  test("SK-25 install writes into the workspace and never into HOME", async () => {
    const repo = await clean("sk-25");
    expect((await skill(repo, ["--agent", "all"])).exitCode).toBe(0);
    expect(isEmptyDir(repo.sandbox.home)).toBe(true);
  });
});

// The notice a text scan prints on stderr for the agent whose marker is set,
// and the silence of a JSON scan and of an empty marker.
describe("skill notice", () => {
  const scan = async (name: string, env: Record<string, string>, json = false) => {
    const box = sandbox(name);
    const workspace = copyWorkspace("projects/clean");
    const before = treeFingerprint(workspace);
    const run = await runDoctor([workspace, "--yes", ...(json ? ["--json"] : [])], { sandbox: box, env });
    expect(run.exitCode).toBe(0);
    expect(treeFingerprint(workspace)).toBe(before);
    return run;
  };
  const notice = (agent: string) =>
    `rust-doctor: no rust-doctor skill is installed for ${agent}; run \`rust-doctor skill install --agent ${agent}\`.`;
  const skillLines = (stderr: string) => stderr.split("\n").filter((line) => line.includes("skill"));

  test("SK-26 CLAUDECODE=1 without a skill prints the claude notice once", async () => {
    const run = await scan("sk-26", { CLAUDECODE: "1" });
    expect(skillLines(run.stderr)).toEqual([notice("claude")]);
  });

  test.each([
    ["CODEX_SANDBOX", "codex"],
    ["CURSOR_AGENT", "cursor"],
  ])("SK-27 %s=1 prints the notice naming %s", async (marker, agent) => {
    const run = await scan(`sk-27-${agent}`, { [marker]: "1" });
    expect(skillLines(run.stderr)).toEqual([notice(agent)]);
  });

  test("SK-28 a --json scan and an empty CLAUDECODE print no notice", async () => {
    const json = await scan("sk-28-json", { CLAUDECODE: "1" }, true);
    expect(skillLines(json.stderr)).toEqual([]);
    expect((json.json as { schema_version?: unknown }).schema_version).toBeDefined();
    const empty = await scan("sk-28-empty", { CLAUDECODE: "" });
    expect(skillLines(empty.stderr)).toEqual([]);
  });
});
