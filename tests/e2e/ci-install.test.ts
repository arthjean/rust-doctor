// CI-01 to CI-34 of `tasks/e2e-catalog/subcommands.md` section 10: `ci
// install`. The workflow it writes is parsed with `Bun.YAML`, every `run:`
// block goes through `bash -n`, and the two steps that matter on a pull
// request run under bash with a fake `gh`.
import { describe, expect, test } from "bun:test";
import { chmodSync, existsSync, mkdirSync, readFileSync, renameSync, symlinkSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import {
  cargoVersion,
  copyWorkspace,
  isEmptyDir,
  runCommand,
  runDoctor,
  sandbox,
  scratchDir,
  toolchainBin,
  treeFingerprint,
  type DoctorOptions,
} from "./e2e-helpers";
import { fakeCalls, fakeEnv } from "./fakes";
import { gitScenario, type Repo } from "./git-scenario";
import { actionTools } from "./action-support";

const WORKFLOW = ".github/workflows/rust-doctor.yml";
const MARKER = "# Written by rust-doctor ci install; `rust-doctor ci install --update` rewrites it.";
const PREVIEW = `Would write ${WORKFLOW}:\n\n`;
const NOT_INSTALLED = "rust-doctor: the workflow was not installed, ";
const TOOLCHAIN = "1.97.1";
const JQ = Bun.which("jq", { PATH: "/usr/local/bin:/usr/bin:/bin" });

type Step = { name?: string; uses?: string; if?: string; with?: Record<string, unknown>; env?: Record<string, string>; run?: string };
type Job = {
  "runs-on": string;
  "timeout-minutes": number;
  needs?: string;
  if?: string;
  permissions?: Record<string, string>;
  steps: Step[];
};
type Workflow = {
  name: string;
  on: { push: { branches: string[] }; pull_request: { types: string[] } };
  concurrency: { group: string; "cancel-in-progress": string };
  permissions: Record<string, string>;
  jobs: { inspect: Job; comment?: Job };
};

const SAVE_IF = "${{ github.ref == format('refs/heads/{0}', github.event.repository.default_branch) }}";

const clean = (name: string) => gitScenario(name).fixture("projects/clean").write({ ".gitignore": "/target\n" }).commit("base").build();

function ci(repo: Repo, args: string[], options: DoctorOptions = {}) {
  return runDoctor(["ci", "install", ...args, repo.path], { sandbox: repo.sandbox, ...options });
}

async function installed(name: string, args: string[] = []): Promise<Repo> {
  const repo = await clean(name);
  expect((await ci(repo, args)).stdout).toBe(`Wrote ${WORKFLOW}\n`);
  return repo;
}

const written = (repo: Repo) => readFileSync(join(repo.path, WORKFLOW), "utf8");
const parse = (text: string) => Bun.YAML.parse(text) as Workflow;
const steps = (text: string) => parse(text).jobs.inspect.steps;
// A run step of any job.
const step = (text: string, name: string) => {
  const found = Object.values(parse(text).jobs).flatMap((job) => job?.steps ?? []).find((candidate) => candidate.name === name);
  if (!found?.run) throw new Error(`no run step named ${name}`);
  return found;
};

// The workflow a dry run previews, without the preview's heading.
async function preview(repo: Repo, args: string[], options: DoctorOptions = {}): Promise<string> {
  const run = await ci(repo, [...args, "--dry-run"], options);
  expect(run.exitCode).toBe(0);
  expect(run.stdout).toStartWith(PREVIEW);
  expect(run.stdout).toEndWith("\n\n");
  return run.stdout.slice(PREVIEW.length, -1);
}

// A refusal: its exit code, its one stderr line, and a tree left as it was.
async function refused(repo: Repo, args: string[], exitCode: number, stderr: string, options: DoctorOptions = {}) {
  const before = treeFingerprint(repo.path);
  const run = await ci(repo, args, options);
  expect(run.exitCode).toBe(exitCode);
  expect(run.stdout).toBe("");
  expect(run.stderr).toBe(`${stderr}\n`);
  expect(treeFingerprint(repo.path)).toBe(before);
}

// Runs one step's script the way a GitHub runner does, in `cwd`.
async function runStep(script: string, cwd: string, env: Record<string, string>, timeoutMs?: number) {
  const dir = scratchDir("step");
  writeFileSync(join(dir, "step.sh"), script);
  return runCommand(["bash", "--noprofile", "--norc", "-eo", "pipefail", join(dir, "step.sh")], { cwd, env, timeoutMs });
}

// A `gh` that records every call through the shared fake, answers the
// comment listing with RD_GH_COMMENTS and fails a POST with RD_GH_POST_EXIT,
// next to the machine's `jq`.
function ghPath(): string {
  const dir = join(scratchDir("gh"), "bin");
  mkdirSync(dir);
  writeFileSync(
    join(dir, "gh"),
    `#!/bin/sh\n"${join(import.meta.dir, "fakes/gh")}" "$@" || exit $?\n` +
      `if [ "$2" = --paginate ]; then printf '%s' "\${RD_GH_COMMENTS:-[]}"; fi\n` +
      `if [ "$3" = POST ]; then exit "\${RD_GH_POST_EXIT:-0}"; fi\n`,
    { mode: 0o755 },
  );
  if (JQ) symlinkSync(JQ, join(dir, "jq"));
  return `${dir}:${toolchainBin()}`;
}

describe("ci install", () => {
  test("CI-01 --dry-run previews the workflow and writes nothing", async () => {
    const repo = await clean("ci-01");
    const before = treeFingerprint(repo.path);
    const run = await ci(repo, ["--dry-run"]);
    expect(run.exitCode).toBe(0);
    expect(run.stdout).toStartWith(`${PREVIEW}${MARKER}\nname: Rust Doctor\n`);
    expect(treeFingerprint(repo.path)).toBe(before);
  });

  test("CI-02 install writes the previewed workflow and prints one line", async () => {
    const repo = await clean("ci-02");
    const shown = await preview(repo, []);
    const run = await ci(repo, []);
    expect(run.exitCode).toBe(0);
    expect(run.stdout).toBe(`Wrote ${WORKFLOW}\n`);
    expect(run.stderr).toBe("");
    expect(written(repo)).toBe(shown);
  });

  test(`CI-03 the workflow parses, pins every action and pins toolchain ${TOOLCHAIN}`, async () => {
    const repo = await installed("ci-03");
    const doc = parse(written(repo));
    expect(doc.name).toBe("Rust Doctor");
    expect(doc.on.push.branches).toEqual(["main"]);
    expect(doc.on.pull_request.types).toEqual(["opened", "synchronize", "reopened", "ready_for_review"]);
    expect(doc.permissions).toEqual({ contents: "read" });
    expect(doc.concurrency.group).toBe("rust-doctor-${{ github.event.pull_request.number || github.ref }}");
    const job = doc.jobs.inspect;
    expect(job["runs-on"]).toBe("ubuntu-latest");
    expect(job["timeout-minutes"]).toBe(30);
    expect(job.steps).toHaveLength(6);
    const uses = job.steps.flatMap((s) => (s.uses ? [s.uses] : []));
    expect(uses.map((u) => u.split("@")[0])).toEqual([
      "actions/checkout",
      "dtolnay/rust-toolchain",
      "Swatinem/rust-cache",
      "actions/upload-artifact",
    ]);
    for (const u of uses) expect(u).toMatch(/@[0-9a-f]{40}$/);
    const by = (action: string) => job.steps.find((s) => s.uses?.startsWith(`${action}@`))!.with;
    expect(by("actions/checkout")).toEqual({ "fetch-depth": 0, "persist-credentials": false });
    expect(by("dtolnay/rust-toolchain")).toEqual({ toolchain: TOOLCHAIN, components: "clippy" });
    expect(by("Swatinem/rust-cache")).toEqual({ "cache-on-failure": true, "cache-bin": false, "save-if": SAVE_IF });
    expect(step(written(repo), "Install rust-doctor").run).toBe(`npm install -g rust-doctor@${cargoVersion()}`);
    const inspect = step(written(repo), "Inspect the workspace").run!;
    expect(inspect).toContain('launcher="$(npm prefix -g)/bin/rust-doctor"\n');
    expect(inspect).toContain('"$launcher" . --yes --json "${scan_arguments[@]}" > "$report"\n');
    expect(inspect).toContain('"$launcher" report markdown "$report" > "$summary" || true\n');
    expect(inspect).not.toMatch(/^rust-doctor /m);
    expect(doc.jobs.comment).toBeUndefined();
    expect(written(repo)).toContain(`toolchain: '${TOOLCHAIN}'\n`);
    expect(written(repo)).toContain("temporary snapshot");
    expect(written(repo)).not.toContain("temporary worktree");
  });

  test.each([
    ["default", []],
    ["--comment", ["--comment", "--blocking", "warning"]],
  ])("CI-04 every run block of the %s workflow passes bash -n and reads no ${{ expression", async (_label, args) => {
    const repo = await clean(`ci-04-${args.length}`);
    const doc = parse(await preview(repo, args));
    const blocks = Object.values(doc.jobs).flatMap((job) => job?.steps ?? []).flatMap((s) => (s.run ? [s.run] : []));
    expect(blocks.length).toBeGreaterThanOrEqual(args.length ? 3 : 2);
    for (const block of blocks) {
      expect(block).not.toContain("${{");
      const dir = scratchDir("bash-n");
      writeFileSync(join(dir, "block.sh"), block);
      const check = await runCommand(["bash", "-n", join(dir, "block.sh")]);
      expect({ exitCode: check.exitCode, stderr: check.stderr }).toEqual({ exitCode: 0, stderr: "" });
    }
  });

  test.failing("CI-05 subcommands/F11 a rerun over the workflow it wrote exits 0, as the hook installers do", async () => {
    const repo = await clean("ci-05");
    const first = await runDoctor(["ci", "install", repo.path], { sandbox: repo.sandbox });
    expect(first.stdout).toBe("Wrote .github/workflows/rust-doctor.yml\n");
    const rerun = await runDoctor(["ci", "install", repo.path], { sandbox: repo.sandbox });
    expect(rerun.exitCode).toBe(0);
  });

  test.failing("CI-06 subcommands/F8 a --dry-run rerun over the workflow it wrote previews without failing", async () => {
    const repo = await installed("ci-06");
    const before = treeFingerprint(repo.path);
    const run = await ci(repo, ["--dry-run"]);
    expect(treeFingerprint(repo.path)).toBe(before);
    expect(run.stderr).toBe("");
    expect(run.exitCode).toBe(0);
  });

  test("CI-07 --update rewrites the workflow it wrote to the same bytes", async () => {
    const repo = await installed("ci-07");
    const before = written(repo);
    const run = await ci(repo, ["--update"]);
    expect(run.exitCode).toBe(0);
    expect(run.stdout).toBe(`Rewrote ${WORKFLOW}\n`);
    expect(written(repo)).toBe(before);
  });

  test("CI-08 --update with --comment, --blocking, --branch and --toolchain carries each", async () => {
    const repo = await installed("ci-08");
    const run = await ci(repo, ["--update", "--comment", "--blocking", "warning", "--branch", "dev", "--toolchain", "stable"]);
    expect(run.exitCode).toBe(0);
    const text = written(repo);
    const doc = parse(text);
    expect(doc.permissions).toEqual({ contents: "read" });
    expect(doc.on.push.branches).toEqual(["dev"]);
    expect(doc.jobs.inspect.steps).toHaveLength(6);
    const job = doc.jobs.comment!;
    expect(job.needs).toBe("inspect");
    expect(job.if).toBe("always() && github.event_name == 'pull_request'");
    expect(job.permissions).toEqual({ "pull-requests": "write" });
    expect(job.steps.map((s) => s.uses?.split("@")[0] ?? s.name)).toEqual(["actions/download-artifact", "Comment on the pull request"]);
    const comment = step(text, "Comment on the pull request");
    expect(comment.if).toBeUndefined();
    expect(Object.keys(comment.env ?? {}).sort()).toEqual(["GH_TOKEN", "PULL_REQUEST", "REPOSITORY"]);
    expect(text).toContain("toolchain: 'stable'\n");
    expect(step(text, "Inspect the workspace").run).toContain(
      'scan_arguments=(--scope baseline --base "origin/$BASE_REF" --blocking warning)\n',
    );
  });

  test.each(["none", "error"])("CI-09 --blocking %s reaches the baseline arguments only", async (level) => {
    const repo = await installed(`ci-09-${level}`);
    expect((await ci(repo, ["--update", "--blocking", level])).exitCode).toBe(0);
    const run = step(written(repo), "Inspect the workspace").run!;
    expect(run).toContain(`scan_arguments=(--scope baseline --base "origin/$BASE_REF" --blocking ${level})\n`);
    expect(run).toContain("scan_arguments=(--scope full --blocking none)\n");
  });

  test("CI-10 without --blocking the baseline arguments leave blocking to the configuration", async () => {
    const repo = await clean("ci-10");
    const run = step(await preview(repo, []), "Inspect the workspace").run!;
    expect(run).toContain('scan_arguments=(--scope baseline --base "origin/$BASE_REF")\n');
  });

  test("CI-11 --blocking warn is a usage error that suggests warning", async () => {
    const repo = await clean("ci-11");
    const before = treeFingerprint(repo.path);
    const run = await ci(repo, ["--blocking", "warn"]);
    expect(run.exitCode).toBe(2);
    expect(run.stderr).toContain("invalid value 'warn' for '--blocking <LEVEL>'");
    expect(run.stderr).toContain("tip: a similar value exists: 'warning'");
    expect(treeFingerprint(repo.path)).toBe(before);
  });

  test.each(["", "1.97 x", "1.97.1'$(id)"])("CI-12 --toolchain %p is refused", async (toolchain) => {
    const repo = await clean("ci-12");
    await refused(repo, ["--toolchain", toolchain], 2, `${NOT_INSTALLED}--toolchain is not a toolchain name.`);
  });

  test("CI-13 a dated nightly toolchain is written as given", async () => {
    const repo = await clean("ci-13");
    expect(parse(await preview(repo, ["--toolchain", "nightly-2026-07-19"])).jobs.inspect.steps[1]!.with!.toolchain).toBe(
      "nightly-2026-07-19",
    );
  });

  const badBranches: string[][] = [["--branch", "ma'in"], ["--branch", "ma in"], ["--branch", "a\nb"], ["--branch", ""], ["--branch=-x"]];
  test.each(badBranches.map((args) => [args]))("CI-14 %p is refused as a branch name", async (args) => {
    const repo = await clean("ci-14");
    await refused(repo, args, 2, `${NOT_INSTALLED}--branch is not a branch name.`);
  });

  test("CI-15 --branch takes every name git takes, single-quoted", async () => {
    const repo = await clean("ci-15");
    for (const branch of ["feature/x", "a#b", "a`b$c", "fé", "a]b", "${{github.token}}"]) {
      const text = await preview(repo, ["--branch", branch]);
      expect(text).toContain(`    branches: ['${branch}']\n`);
      expect(parse(text).on.push.branches).toEqual([branch]);
    }
  });

  // The refusal half of CI-15, apart so the open defect cannot hide a
  // regression on the names above.
  test.failing("subcommands/F18 --branch refuses a..b, as git check-ref-format does (CI-15)", async () => {
    const repo = await clean("ci-15-dots");
    await refused(repo, ["--branch", "a..b"], 2, `${NOT_INSTALLED}--branch is not a branch name.`);
  });

  test("CI-16 --update rewrites a hand edit of the workflow it wrote", async () => {
    const repo = await installed("ci-16");
    const original = written(repo);
    writeFileSync(join(repo.path, WORKFLOW), original.replace("timeout-minutes: 30", "timeout-minutes: 99"));
    const run = await ci(repo, ["--update"]);
    expect(run.exitCode).toBe(0);
    expect(written(repo)).toBe(original);
  });

  test.each([
    ["removed", (text: string) => text.replace(`${MARKER}\n`, "")],
    ["on line 2", (text: string) => `# edited by hand\n${text}`],
  ])("CI-17 --update refuses a workflow whose marker is %s", async (label, edit) => {
    const repo = await installed(`ci-17-${label.length}`);
    writeFileSync(join(repo.path, WORKFLOW), edit(written(repo)));
    await refused(repo, ["--update"], 1, `${NOT_INSTALLED}${WORKFLOW} was not written by rust-doctor, so nothing was written.`);
  });

  test.failing("CI-18 subcommands/F18 a leftover staging file does not fail every later --update", async () => {
    const repo = await clean("ci-18");
    expect((await runDoctor(["ci", "install", repo.path], { sandbox: repo.sandbox })).exitCode).toBe(0);
    writeFileSync(join(repo.path, ".github/workflows/rust-doctor.yml.rust-doctor"), "leftover\n");
    const update = await runDoctor(["ci", "install", "--update", repo.path], { sandbox: repo.sandbox });
    expect(update.exitCode).toBe(0);
  });

  const lint = "on: push\njobs:\n  lint:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: arthjean/rust-doctor@v1\n";

  test.each([[[]], [["--update"]]])("CI-19 a workflow already using the action refuses install %p", async (args) => {
    const repo = await gitScenario("ci-19").fixture("projects/clean").write({ ".github/workflows/lint.yaml": lint }).commit("base").build();
    await refused(repo, args, 1, `${NOT_INSTALLED}.github/workflows/lint.yaml already runs rust-doctor, so nothing was written.`);
  });

  test.failing("CI-20 subcommands/F18 a comment that only mentions the action does not count as running it", async () => {
    const repo = await gitScenario("ci-20")
      .fixture("projects/clean")
      .write({ ".github/workflows/lint.yaml": "on: push\n# arthjean/rust-doctor@v1 is for later\njobs: {}\n" })
      .commit("base")
      .build();
    const run = await ci(repo, []);
    expect(run.stderr).toBe("");
    expect(run.exitCode).toBe(0);
    expect(existsSync(join(repo.path, WORKFLOW))).toBe(true);
  });

  test("CI-21 a renamed copy of the workflow it wrote is found by its marker", async () => {
    const repo = await installed("ci-21");
    renameSync(join(repo.path, WORKFLOW), join(repo.path, ".github/workflows/renamed.yml"));
    await refused(repo, [], 1, `${NOT_INSTALLED}.github/workflows/renamed.yml already runs rust-doctor, so nothing was written.`);
  });

  test("CI-22 a detached HEAD with no default branch asks for --branch", async () => {
    const repo = await gitScenario("ci-22").fixture("projects/clean").commit("base").detach().build();
    await repo.git(["branch", "-D", "main"]);
    await refused(repo, [], 2, `${NOT_INSTALLED}no default branch found: pass --branch <NAME>.`);
  });

  test("CI-23 a master default branch is kept from another branch", async () => {
    const repo = await clean("ci-23");
    await repo.git(["branch", "-m", "main", "master"]);
    expect(parse(await preview(repo, [])).on.push.branches).toEqual(["master"]);
    await repo.git(["switch", "-q", "-c", "feature"]);
    expect(parse(await preview(repo, [])).on.push.branches).toEqual(["master"]);
  });

  test("CI-24 origin/HEAD names the default branch", async () => {
    const repo = await gitScenario("ci-24")
      .fixture("projects/clean")
      .commit("base")
      .branch("develop")
      .switch("main")
      .clone()
      .setHead("develop")
      .branch("topic")
      .build();
    expect(parse(await preview(repo, [])).on.push.branches).toEqual(["develop"]);
  });

  test.each([
    ["a crate outside git", "outside", `${NOT_INSTALLED}the path is not inside a git repository, and GitHub reads workflows from one.`],
    ["a PATH that does not exist", "missing", "rust-doctor: PATH: missing does not exist."],
  ])("CI-25 %s is refused", async (_label, kind, stderr) => {
    const box = sandbox("ci-25");
    const workspace = copyWorkspace("projects/clean", join(box.dir, "outside"));
    const before = treeFingerprint(box.dir);
    const run = await runDoctor(["ci", "install", kind === "outside" ? workspace : "missing", "--branch", "main"], { sandbox: box, cwd: box.dir });
    expect(run.exitCode).toBe(2);
    expect(run.stderr).toBe(`${stderr}\n`);
    expect(treeFingerprint(box.dir)).toBe(before);
  });

  test("CI-26 a symlinked .github is refused, and its target stays empty", async () => {
    const repo = await clean("ci-26");
    const outside = scratchDir("ci-26-outside");
    symlinkSync(outside, join(repo.path, ".github"));
    await refused(repo, [], 2, `${NOT_INSTALLED}.github is a symlink, and nothing is written through one.`);
    expect(isEmptyDir(outside)).toBe(true);
  });

  test("CI-27 --update refuses a dangling symlink at the workflow path", async () => {
    const repo = await clean("ci-27");
    mkdirSync(join(repo.path, ".github/workflows"), { recursive: true });
    symlinkSync(join(scratchDir("ci-27-gone"), "absent.yml"), join(repo.path, WORKFLOW));
    await refused(repo, ["--update"], 2, `${NOT_INSTALLED}${WORKFLOW} is a symlink, and nothing is written through one.`);
  });

  const nested = (name: string) => gitScenario(name).nested("crates/x", "mini").commit("base").build();

  test("CI-28 an absolute member PATH caches and scans the member", async () => {
    const repo = await nested("ci-28");
    const before = treeFingerprint(repo.path);
    const text = await preview({ ...repo, path: join(repo.path, "crates/x") }, [], { cwd: repo.path });
    expect(steps(text)[2]!.with).toEqual({ workspaces: "crates/x", "cache-on-failure": true, "cache-bin": false, "save-if": SAVE_IF });
    expect(step(text, "Inspect the workspace").run).toContain('"$launcher" crates/x --yes --json "${scan_arguments[@]}" > "$report"\n');
    expect(treeFingerprint(repo.path)).toBe(before);
  });

  test.each([[[]], [["--branch", "main"]]])("CI-29 a relative member PATH previews what its absolute form does %p", async (args) => {
    const repo = await nested("ci-29");
    const absolute = await preview({ ...repo, path: join(repo.path, "crates/x") }, args, { cwd: repo.path });
    const relative = await preview({ ...repo, path: "crates/x" }, args, { cwd: repo.path });
    expect(relative).toBe(absolute);
  });

  test("CI-30 a read-only repository root fails the write with exit 2", async () => {
    const repo = await clean("ci-30");
    chmodSync(repo.path, 0o555);
    try {
      await refused(repo, [], 2, `${NOT_INSTALLED}could not write ${WORKFLOW} (permission denied).`);
    } finally {
      chmodSync(repo.path, 0o755);
    }
  });

  test("CI-31 a workspace path with a space is refused", async () => {
    const repo = await gitScenario("ci-31")
      .write({
        "my app/Cargo.toml": '[package]\nname = "my-app"\nversion = "0.1.0"\nedition = "2024"\n',
        "my app/src/lib.rs": "pub fn value() -> u8 {\n    1\n}\n",
      })
      .commit("base")
      .build();
    await refused(
      { ...repo, path: join(repo.path, "my app") },
      ["--branch", "main"],
      2,
      `${NOT_INSTALLED}the workspace's path from the repository's top is not one a workflow can carry.`,
    );
  });
});

describe("ci install steps", () => {
  test("CI-32 the inspect step fails a pull request that adds a warning and reports a push", async () => {
    const template = await clean("ci-32-template");
    const workflow = await preview(template, ["--blocking", "warning", "--comment"]);
    const repo = await gitScenario("ci-32")
      .fixture("projects/clean")
      .write({ ".gitignore": "/target\n", [WORKFLOW]: workflow })
      .commit("base")
      .clone()
      .branch("feature")
      .append("src/lib.rs", "\npub fn later() -> u8 {\n    todo!()\n}\n")
      .commit("todo")
      .build();
    const script = step(workflow, "Inspect the workspace").run!;
    const inspect = async (label: string, baseRef: string, eventName: string) => {
      const runner = scratchDir(`ci-32-${label}`);
      const summary = join(runner, "step-summary.md");
      const run = await runStep(
        script,
        repo.path,
        {
          RUNNER_TEMP: runner,
          GITHUB_STEP_SUMMARY: summary,
          BASE_REF: baseRef,
          EVENT_NAME: eventName,
          CARGO_TARGET_DIR: repo.target("ci-32"),
          // `npm prefix -g` answers a prefix whose launcher is the binary under test.
          PATH: `${actionTools()}:${toolchainBin()}`,
        },
        150_000,
      );
      const report = JSON.parse(readFileSync(join(runner, "rust-doctor/report.json"), "utf8"));
      return { run, report, summary: readFileSync(summary, "utf8") };
    };
    const before = treeFingerprint(repo.path);
    const pr = await inspect("pr", "main", "pull_request");
    expect(pr.run.exitCode).toBe(1);
    expect(pr.report.scope.mode).toBe("baseline");
    expect(pr.report.delta.summary.introduced).toBe(1);
    expect(pr.summary).toContain("**Against the base:** 1 introduced, 0 fixed.");
    const push = await inspect("push", "", "push");
    expect(push.run.exitCode).toBe(0);
    expect(push.report.scope.mode).toBe("full");
    expect(treeFingerprint(repo.path)).toBe(before);
  }, 360_000);

  const commentStep = async () => {
    const repo = await clean("ci-comment");
    return { repo, script: step(await preview(repo, ["--comment"]), "Comment on the pull request").run! };
  };

  const commentEnv = (runner: string, box: ReturnType<typeof sandbox>, extra: Record<string, string> = {}) => ({
    ...fakeEnv(box, ["gh"]),
    PATH: ghPath(),
    RUNNER_TEMP: runner,
    GH_TOKEN: "test",
    REPOSITORY: "owner/repo",
    PULL_REQUEST: "7",
    ...extra,
  });

  const withSummary = () => {
    const runner = scratchDir("runner");
    mkdirSync(join(runner, "rust-doctor"));
    writeFileSync(join(runner, "rust-doctor/summary.md"), "## rust-doctor\n\nScore 100.\n");
    return runner;
  };

  test.skipIf(!JQ)("CI-33 the comment step posts one sticky comment, then edits it", async () => {
    const { repo, script } = await commentStep();
    const box = sandbox("ci-33");
    const runner = withSummary();
    const first = await runStep(script, repo.path, commentEnv(runner, box));
    expect(first.exitCode).toBe(0);
    const ours = JSON.stringify([
      { id: 41, user: { login: "someone" }, body: "<!-- rust-doctor -->\n\nnot ours" },
      { id: 42, user: { login: "github-actions[bot]" }, body: "<!-- rust-doctor -->\n\nold" },
    ]);
    const second = await runStep(script, repo.path, commentEnv(runner, box, { RD_GH_COMMENTS: ours }));
    expect(second.exitCode).toBe(0);
    const calls = fakeCalls(box, "gh").map((call) => call.argv);
    expect(calls.map((argv) => argv.slice(0, 4))).toEqual([
      ["api", "--paginate", "repos/owner/repo/issues/7/comments"],
      ["api", "-X", "POST", "repos/owner/repo/issues/7/comments"],
      ["api", "--paginate", "repos/owner/repo/issues/7/comments"],
      ["api", "-X", "PATCH", "repos/owner/repo/issues/comments/42"],
    ]);
    for (const argv of [calls[1]!, calls[3]!]) {
      expect(argv.slice(4)).toEqual(["-f", "body=<!-- rust-doctor -->\n\n## rust-doctor\n\nScore 100."]);
    }
  });

  test.skipIf(!JQ).each([
    ["no summary", "::notice::rust-doctor wrote no report, so no comment was posted.\n", 0],
    [
      "a refused POST",
      "::warning::rust-doctor could not post the pull request comment. A pull request from a fork gets a read-only token.\n",
      2,
    ],
  ])("CI-34 the comment step with %s exits 0 with an annotation", async (label, annotation, ghCalls) => {
    const { repo, script } = await commentStep();
    const box = sandbox("ci-34");
    const runner = label === "no summary" ? scratchDir("runner") : withSummary();
    const run = await runStep(script, repo.path, commentEnv(runner, box, { RD_GH_POST_EXIT: "1" }));
    expect(run.exitCode).toBe(0);
    expect(run.stdout).toBe(annotation);
    expect(fakeCalls(box, "gh")).toHaveLength(ghCalls);
  });
});
