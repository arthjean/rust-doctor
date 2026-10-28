// `tasks/e2e-catalog/real-codebases.md` section 4, its static checks:
// action.yml pins every action it uses to a commit, interpolates nothing into
// a script, installs the crate's own version by default, and the workflow
// `ci install` writes into users' repositories agrees with it.
import { describe, expect, test } from "bun:test";
import { cargoVersion, runDoctor } from "./e2e-helpers";
import { gitScenario } from "./git-scenario";
import { COMMENT_ACTION_PATH, actionYaml } from "./action-support";

type Step = { name?: string; id?: string; uses?: string; run?: string; env?: Record<string, string>; with?: Record<string, unknown> };

const SAVE_IF = "${{ github.ref == format('refs/heads/{0}', github.event.repository.default_branch) }}";

const PREVIEW = "Would write .github/workflows/rust-doctor.yml:\n\n";
const PINNED = /^[\w.-]+\/[\w.-]+@[0-9a-f]{40}$/;

const steps = (): Step[] => actionYaml().runs.steps as Step[];

async function installedWorkflow(): Promise<{ text: string; steps: Step[]; comment: Step[] }> {
  const repo = await gitScenario("ci-template").fixture("projects/clean").write({ ".gitignore": "/target\n" }).commit("base").build();
  const run = await runDoctor(["ci", "install", "--dry-run", "--comment", repo.path], { sandbox: repo.sandbox });
  expect(run.exitCode).toBe(0);
  expect(run.stdout).toStartWith(PREVIEW);
  const text = run.stdout.slice(PREVIEW.length);
  const workflow = Bun.YAML.parse(text) as { jobs: { inspect: { steps: Step[] }; comment: { steps: Step[] } } };
  return { text, steps: workflow.jobs.inspect.steps, comment: workflow.jobs.comment.steps };
}

describe("action.yml", () => {
  test("every uses: is pinned to a 40-character commit", () => {
    const uses = steps().flatMap((step) => (step.uses ? [step.uses] : []));
    expect(uses.length).toBeGreaterThan(0);
    expect(uses.filter((ref) => !PINNED.test(ref))).toEqual([]);
  });

  test("no run: block interpolates an expression", () => {
    const runs = steps().filter((step) => step.run !== undefined);
    expect(runs.length).toBeGreaterThan(0);
    expect(runs.filter((step) => step.run!.includes("${{")).map((step) => step.name)).toEqual([]);
  });

  test("inputs.version defaults to the crate's version", () => {
    expect(actionYaml().inputs.version!.default).toBe(cargoVersion());
  });

  // US-019: only the default branch saves the cache, `~/.cargo/bin` is never
  // cached, and the launcher runs by the absolute path npm installed it at.
  test("the cache saves from the default branch only, and the launcher runs by its absolute path", () => {
    const cache = steps().find((step) => step.uses?.startsWith("Swatinem/rust-cache@"));
    expect(cache?.with).toMatchObject({ "cache-on-failure": true, "cache-bin": false, "save-if": SAVE_IF });
    const install = steps().find((step) => step.name === "Install rust-doctor")!;
    expect(install.id).toBe("install");
    expect(install.run).toContain('launcher="$(npm prefix -g)/bin/rust-doctor"\n');
    expect(install.run).toContain('echo "launcher=$launcher" >> "$GITHUB_OUTPUT"\n');
    for (const name of ["Scan", "Summarize"]) {
      const found = steps().find((step) => step.name === name)!;
      expect(found.env?.LAUNCHER).toBe("${{ steps.install.outputs.launcher }}");
      expect(found.run).toContain('"${LAUNCHER:-rust-doctor}" ');
    }
  });

  // US-020: action.yml posts nothing; it uploads what the comment job reads.
  test("action.yml posts no comment and uploads the report, the summary and the clean flag", () => {
    expect(Object.keys(actionYaml().inputs)).not.toContain("comment");
    expect(Object.keys(actionYaml().inputs)).not.toContain("comment-on-clean");
    expect(steps().some((step) => step.run?.includes("gh api"))).toBe(false);
    const upload = steps().find((step) => step.uses?.startsWith("actions/upload-artifact@"))!;
    expect(String(upload.with?.path).trim().split("\n")).toEqual([
      "${{ runner.temp }}/rust-doctor/report.json",
      "${{ runner.temp }}/rust-doctor/summary.md",
      "${{ runner.temp }}/rust-doctor/clean",
    ]);
  });
});

describe("comment/action.yml", () => {
  test("every uses: is pinned to a 40-character commit, and comment-on-clean defaults to true", () => {
    const action = actionYaml(COMMENT_ACTION_PATH);
    const uses = action.runs.steps.flatMap((step) => (step.uses ? [step.uses] : []));
    expect(uses.map((ref) => ref.split("@")[0])).toEqual(["actions/download-artifact"]);
    expect(uses.filter((ref) => !PINNED.test(ref))).toEqual([]);
    expect(action.inputs["comment-on-clean"]!.default).toBe("true");
  });
});

describe("the workflow ci install writes agrees with action.yml", () => {
  test("the same actions at the same commits, the same toolchain, the same version", async () => {
    const workflow = await installedWorkflow();
    const pins = (list: Step[]) => new Map(list.flatMap((step) => (step.uses ? [[step.uses.split("@")[0]!, step.uses]] : [])));
    const action = pins(steps());
    for (const [name, ref] of pins(workflow.steps)) {
      expect(ref).toMatch(PINNED);
      if (action.has(name)) expect(ref).toBe(action.get(name)!);
    }
    const toolchain = workflow.steps.find((step) => step.uses?.startsWith("dtolnay/rust-toolchain@"));
    expect(toolchain?.with?.toolchain).toBe(actionYaml().inputs.toolchain!.default);
    expect(workflow.steps.find((step) => step.name === "Install rust-doctor")?.run?.trim()).toBe(`npm install -g rust-doctor@${cargoVersion()}`);
    const cache = workflow.steps.find((step) => step.uses?.startsWith("Swatinem/rust-cache@"));
    expect(cache?.with).toMatchObject({ "cache-on-failure": true, "cache-bin": false, "save-if": SAVE_IF });
    expect(workflow.steps.find((step) => step.name === "Inspect the workspace")?.run).toContain('launcher="$(npm prefix -g)/bin/rust-doctor"\n');
    const download = workflow.comment.find((step) => step.uses?.startsWith("actions/download-artifact@"));
    expect(download?.uses).toBe(actionYaml(COMMENT_ACTION_PATH).runs.steps[0]!.uses);
  });

  test("the same comment script as comment/action.yml", async () => {
    const workflow = await installedWorkflow();
    const script = (list: Step[]) => list.find((step) => step.name === "Comment on the pull request")?.run;
    expect(script(workflow.comment)).toBeDefined();
    expect(script(workflow.comment)).toBe(script(actionYaml(COMMENT_ACTION_PATH).runs.steps as Step[]));
  });

  test("a baseline needs the history, and the template says why without a worktree (real/D1)", async () => {
    const workflow = await installedWorkflow();
    const checkout = workflow.steps.find((step) => step.uses?.startsWith("actions/checkout@"));
    expect(checkout?.with?.["fetch-depth"]).toBe(0);
    expect(workflow.text).not.toContain("worktree");
  });
});
