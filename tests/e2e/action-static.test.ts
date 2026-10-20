// `tasks/e2e-catalog/real-codebases.md` section 4, its static checks:
// action.yml pins every action it uses to a commit, interpolates nothing into
// a script, installs the crate's own version by default, and the workflow
// `ci install` writes into users' repositories agrees with it.
import { describe, expect, test } from "bun:test";
import { cargoVersion, runDoctor } from "./e2e-helpers";
import { gitScenario } from "./git-scenario";
import { actionYaml } from "./action-support";

type Step = { name?: string; uses?: string; run?: string; with?: Record<string, unknown> };

const PREVIEW = "Would write .github/workflows/rust-doctor.yml:\n\n";
const PINNED = /^[\w.-]+\/[\w.-]+@[0-9a-f]{40}$/;

const steps = (): Step[] => actionYaml().runs.steps as Step[];

async function installedWorkflow(): Promise<{ text: string; steps: Step[] }> {
  const repo = await gitScenario("ci-template").fixture("projects/clean").write({ ".gitignore": "/target\n" }).commit("base").build();
  const run = await runDoctor(["ci", "install", "--dry-run", "--comment", repo.path], { sandbox: repo.sandbox });
  expect(run.exitCode).toBe(0);
  expect(run.stdout).toStartWith(PREVIEW);
  const text = run.stdout.slice(PREVIEW.length);
  const workflow = Bun.YAML.parse(text) as { jobs: { inspect: { steps: Step[] } } };
  return { text, steps: workflow.jobs.inspect.steps };
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
  });

  test("the same comment script", async () => {
    const workflow = await installedWorkflow();
    const script = (list: Step[]) => list.find((step) => step.name === "Comment on the pull request")?.run;
    expect(script(workflow.steps)).toBeDefined();
    expect(script(workflow.steps)).toBe(script(steps()));
  });

  test("a baseline needs the history, and the template says why without a worktree (real/D1)", async () => {
    const workflow = await installedWorkflow();
    const checkout = workflow.steps.find((step) => step.uses?.startsWith("actions/checkout@"));
    expect(checkout?.with?.["fetch-depth"]).toBe(0);
    expect(workflow.text).not.toContain("worktree");
  });
});
