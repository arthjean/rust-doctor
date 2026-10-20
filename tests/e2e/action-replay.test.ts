// `tasks/e2e-catalog/real-codebases.md` section 4: action.yml's own logic,
// replayed offline over its case matrix. The scans are real (the release
// binary on `mini` histories, cloned over file:// from a local origin the way
// actions/checkout hands a workflow its repository); `gh` is a recording stub,
// and the steps GitHub itself provides are skipped.
import { describe, expect, test } from "bun:test";
import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { runCommand, scratchDir } from "./e2e-helpers";
import { gitScenario, NEW_FN, type Repo } from "./git-scenario";
import { replayAction, step, type Event, type Replay } from "./action-support";

const TARGET = scratchDir("action-target");
const EXIT_STEP = "Exit with the scan's code";
const STICKY = "<!-- rust-doctor -->";

type Report = { status: string; scope: { mode: string; comparison_base: string | null } };

// `main` holds `mini`; `feature` adds NEWFN in a second commit, or nothing
// for a no-op pull request. The checkout is a clone on `feature`.
async function checkout(name: string, change: "newfn" | "none", clone: { depth?: number } = {}): Promise<Repo> {
  let scenario = gitScenario(name).template("mini").commit("base").branch("feature");
  if (change === "newfn") scenario = scenario.append("src/lib.rs", NEW_FN).commit("feature");
  return scenario.clone({ branch: "feature", ...(clone.depth ? { depth: clone.depth, singleBranch: true } : {}) }).build();
}

function replay(repo: Repo, event: Event, options: Omit<Parameters<typeof replayAction>[0], "workspace" | "event"> = {}): Promise<Replay> {
  return replayAction({ workspace: repo.path, event, sandbox: repo.sandbox, target: TARGET, ...options });
}

function report(result: Replay): Report {
  return JSON.parse(readFileSync(result.stepOutputs.scan!.report!, "utf8")) as Report;
}

function writes(result: Replay): string[][] {
  return result.gh.filter((call) => call.includes("-X"));
}

function botComment(id: number): string {
  return JSON.stringify([{ id, user: { login: "github-actions[bot]" }, body: `${STICKY}\n\nold` }]);
}

function namesUnder(dir: string): string[] {
  return readdirSync(dir, { recursive: true }).map(String);
}

describe("the exit policy", () => {
  // The warning code is 42. Only a pull request fails on the gate (1) or the
  // warning code; a missing code counts as 2, and 2 and 130 fail every event.
  const codes = ["0", "1", "2", "130", "42", undefined] as const;
  const cases = (["push", "pull_request", "workflow_dispatch"] as const).flatMap((event) =>
    codes.map((code) => {
      const scanned = code === undefined ? 2 : Number(code);
      const expected = event !== "pull_request" && (scanned === 1 || scanned === 42) ? 0 : scanned;
      return [event, code ?? "missing", expected] as const;
    }),
  );

  test.each(cases)("%s with scan exit %s ends with %d", async (event, code, expected) => {
    const repo = await gitScenario(`policy-${event}-${code}`).template("mini").commit("base").build();
    const result = await replay(repo, event, {
      only: [EXIT_STEP],
      inputs: { "warning-exit-code": "42" },
      stepOutputs: code === "missing" ? {} : { scan: { "exit-code": code } },
    });
    expect(step(result, EXIT_STEP).exitCode).toBe(expected);
    if (expected === 0 && code !== "0") expect(step(result, EXIT_STEP).stdout).toContain("::notice::The gate failed, and only a pull request fails on it");
  });
});

describe("a replayed run", () => {
  test("a pull request from a depth-1 checkout fetches its base, fails on the gate and posts one sticky comment", async () => {
    const repo = await checkout("pr-shallow", "newfn", { depth: 1 });
    expect((await runCommand(["git", "rev-parse", "--verify", "-q", "origin/main"], { cwd: repo.path, sandbox: repo.sandbox })).exitCode).not.toBe(0);
    const result = await replay(repo, "pull_request", { inputs: { blocking: "warning" } });
    expect(step(result, "Scan").exitCode).toBe(0);
    expect(result.stepOutputs.scan!["exit-code"]).toBe("1");
    const main = await runCommand(["git", "rev-parse", "--verify", "-q", "origin/main"], { cwd: repo.path, sandbox: repo.sandbox });
    expect(main.exitCode).toBe(0);
    expect(report(result).scope).toMatchObject({ mode: "baseline", comparison_base: main.stdout.trim() });
    expect(result.outputs).toMatchObject({ authoritative: "true", introduced: "1", fixed: "0", "exit-code": "1" });
    expect(result.outputs.score).toMatch(/^\d+$/);
    expect(result.summary.length).toBeGreaterThan(0);
    expect(writes(result)).toHaveLength(1);
    const post = writes(result)[0]!;
    expect(post.slice(0, 4)).toEqual(["api", "-X", "POST", "repos/octo/app/issues/7/comments"]);
    expect(post.at(-1)).toStartWith(`body=${STICKY}\n\n`);
    expect(result.failed).toBe(true);
    expect(step(result, EXIT_STEP).exitCode).toBe(1);
  });

  test("a push scans in full scope, reports no delta and does not fail on the gate", async () => {
    const repo = await checkout("push-full", "newfn");
    const result = await replay(repo, "push", { inputs: { blocking: "warning" } });
    expect(result.stepOutputs.scan!["exit-code"]).toBe("1");
    expect(report(result).scope).toMatchObject({ mode: "full", comparison_base: null });
    expect(result.outputs).toMatchObject({ introduced: "", fixed: "", authoritative: "true" });
    expect(step(result, "Comment on the pull request").ran).toBe(false);
    expect(result.gh).toEqual([]);
    expect(step(result, EXIT_STEP).exitCode).toBe(0);
    expect(result.failed).toBe(false);
  });

  test("a base input replaces the pull request's base", async () => {
    const repo = await checkout("pr-base-input", "newfn");
    const head = await repo.sha();
    const result = await replay(repo, "pull_request", { inputs: { base: head, comment: "false" } });
    expect(report(result).scope).toMatchObject({ mode: "baseline", comparison_base: head });
    expect(result.outputs).toMatchObject({ introduced: "0", fixed: "0" });
    expect(result.gh).toEqual([]);
  });

  test("a no-op pull request with an earlier sticky comment edits it", async () => {
    const repo = await checkout("pr-noop-patch", "none");
    const result = await replay(repo, "pull_request", { comments: botComment(42) });
    expect(result.outputs).toMatchObject({ introduced: "0", fixed: "0" });
    expect(result.stepOutputs.summarize!.clean).toBe("true");
    expect(writes(result).map((call) => call.slice(0, 4))).toEqual([["api", "-X", "PATCH", "repos/octo/app/issues/comments/42"]]);
  });

  test("a no-op pull request with comment-on-clean false posts no comment (US-002)", async () => {
    const repo = await checkout("pr-noop-quiet", "none");
    const result = await replay(repo, "pull_request", { inputs: { "comment-on-clean": "false" } });
    expect(result.stepOutputs.summarize!.clean).toBe("true");
    expect(writes(result)).toEqual([]);
    expect(step(result, "Comment on the pull request").stdout).toContain("comment-on-clean is false, so no comment was posted");
    expect(result.failed).toBe(false);
  });

  test("a no-op pull request posts a comment when comment-on-clean keeps its default", async () => {
    const repo = await checkout("pr-noop-post", "none");
    const result = await replay(repo, "pull_request");
    expect(writes(result).map((call) => call[2])).toEqual(["POST"]);
  });

  test("a working-directory below the checkout is the workspace scanned", async () => {
    const repo = await gitScenario("subdir").nested("crates/app", "mini").commit("base").build();
    const result = await replay(repo, "push", { inputs: { "working-directory": "crates/app" } });
    expect(result.stepOutputs.scan!["exit-code"]).toBe("0");
    expect(report(result).status).toBe("complete");
    expect(result.summary).toContain("| src/lib.rs:4 |");
    expect(result.failed).toBe(false);
  });

  test("args and inputs carrying shell syntax stay literal", async () => {
    const repo = await checkout("injection", "none");
    const result = await replay(repo, "push", {
      inputs: { args: "$(touch pwned1) ; touch pwned2 `touch pwned3`", base: "main;touch pwned4", blocking: "$(touch pwned5)" },
    });
    expect(result.stepOutputs.scan!["exit-code"]).toBe("2");
    for (const dir of [repo.sandbox.dir, repo.path]) {
      expect(namesUnder(dir).filter((name) => name.includes("pwned"))).toEqual([]);
    }
  });

  test("an unreadable report gets the summary's sentence, the fallback outputs and no comment", async () => {
    const repo = await checkout("unreadable", "newfn");
    const result = await replay(repo, "pull_request", { inputs: { args: "--bogus" }, comments: botComment(42) });
    expect(result.stepOutputs.scan!["exit-code"]).toBe("2");
    expect(result.summary).toBe("rust-doctor wrote no report that could be rendered.\n");
    expect(result.stepOutputs.summarize!.clean).toBe("false");
    expect(result.gh).toEqual([]);
    expect(step(result, "Comment on the pull request").stdout).toContain("rust-doctor wrote no report, so no comment was posted.");
    expect(result.stepOutputs.outputs!.score ?? "").toBe("");
    expect(step(result, EXIT_STEP).exitCode).toBe(2);
  });

  // The outputs step falls back to `authoritative=false` only when jq fails,
  // and jq reads an empty report, what a refused argument leaves, as no
  // input at all: it prints nothing and succeeds.
  test.failing("real/B9 an empty report still sets the authoritative output to false", async () => {
    const repo = await checkout("empty-report", "none");
    const result = await replay(repo, "push", { inputs: { args: "--bogus" } });
    expect(result.stepOutputs.scan!["exit-code"]).toBe("2");
    expect(result.outputs.authoritative).toBe("false");
  });

  test("a comment API that refuses to list or to write leaves a warning and a green step", async () => {
    const repo = await checkout("api-failure", "none");
    const list = await replay(repo, "pull_request", { ghListExit: 1 });
    expect(step(list, "Comment on the pull request").exitCode).toBe(0);
    expect(step(list, "Comment on the pull request").stdout).toContain("::warning::rust-doctor could not read the pull request's comments");
    const write = await replay(repo, "pull_request", { ghWriteExit: 1 });
    expect(step(write, "Comment on the pull request").exitCode).toBe(0);
    expect(step(write, "Comment on the pull request").stdout).toContain("::warning::rust-doctor could not post the pull request comment");
    expect(write.failed).toBe(false);
  });

  // action.yml downgrades every comment failure to a warning, but a comment
  // list that is not JSON fails jq under `set -e` and turns the step red.
  test.failing("real/B8 a comment list that is not JSON leaves a warning and a green step", async () => {
    const repo = await checkout("api-not-json", "none");
    const result = await replay(repo, "pull_request", { comments: "<html>bad gateway</html>" });
    expect(step(result, "Comment on the pull request").exitCode).toBe(0);
    expect(step(result, "Comment on the pull request").stdout).toContain("::warning::");
  });

  test("strings of the report never write an output line of their own", async () => {
    const repo = await gitScenario("outputs-injection").template("mini").commit("base").build();
    const crafted = join(repo.sandbox.dir, "crafted.json");
    writeFileSync(crafted, JSON.stringify({
      status: "complete\nclean=true",
      scope: { mode: "baseline\nx=1" },
      audit: { score: { value: "1\nscore=1", authoritative: "true\nscore=1" } },
      delta: { summary: { introduced: "0\nfixed=9", fixed: "9\nintroduced=0" } },
      diagnostics: [],
    }));
    const result = await replay(repo, "pull_request", {
      only: ["Summarize", "Set the outputs"],
      stepOutputs: { scan: { report: crafted } },
    });
    expect(result.stepOutputs.outputs).toEqual({ score: "", authoritative: "false", introduced: "", fixed: "" });
    expect(result.stepOutputs.summarize).toEqual({ clean: "false" });
  });
});
