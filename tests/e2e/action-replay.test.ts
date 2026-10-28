// `tasks/e2e-catalog/real-codebases.md` section 4: action.yml's own logic,
// and comment/action.yml's after it, replayed offline over their case matrix. The scans are real (the release
// binary on `mini` histories, cloned over file:// from a local origin the way
// actions/checkout hands a workflow its repository); `gh` is a recording stub,
// and the steps GitHub itself provides are skipped.
import { describe, expect, test } from "bun:test";
import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { cargoVersion, runCommand, scratchDir } from "./e2e-helpers";
import { gitScenario, NEW_FN, type Repo } from "./git-scenario";
import { COMMENT_ACTION_PATH, actionYaml, replayAction, step, type Event, type Replay, type ReplayOptions } from "./action-support";

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

// The comment job after the scan: comment/action.yml replayed in the same
// runner, where the scan left the report, summary and clean flag its
// artifact carries. The download step is the runner's and is skipped.
function comment(repo: Repo, options: Partial<Omit<ReplayOptions, "workspace" | "action">> = {}): Promise<Replay> {
  return replayAction({ event: "pull_request", sandbox: repo.sandbox, ...options, action: COMMENT_ACTION_PATH, workspace: repo.path });
}

const COMMENT_STEP = "Comment on the pull request";

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
  test("a pull request from a depth-1 checkout fetches its base, fails on the gate and the comment job still posts one sticky comment", async () => {
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
    expect(result.gh).toEqual([]);
    expect(result.failed).toBe(true);
    expect(step(result, EXIT_STEP).exitCode).toBe(1);
    const posted = await comment(repo);
    expect(writes(posted)).toHaveLength(1);
    const post = writes(posted)[0]!;
    expect(post.slice(0, 4)).toEqual(["api", "-X", "POST", "repos/octo/app/issues/7/comments"]);
    expect(post.at(-1)).toStartWith(`body=${STICKY}\n\n`);
    expect(posted.failed).toBe(false);
  });

  test("a push scans in full scope, reports no delta and does not fail on the gate", async () => {
    const repo = await checkout("push-full", "newfn");
    const result = await replay(repo, "push", { inputs: { blocking: "warning" } });
    expect(result.stepOutputs.scan!["exit-code"]).toBe("1");
    expect(report(result).scope).toMatchObject({ mode: "full", comparison_base: null });
    expect(result.outputs).toMatchObject({ introduced: "", fixed: "", authoritative: "true" });
    expect(result.steps.map((run) => run.name)).not.toContain(COMMENT_STEP);
    expect(result.gh).toEqual([]);
    expect(step(await comment(repo, { event: "push" }), COMMENT_STEP).ran).toBe(false);
    expect(step(result, EXIT_STEP).exitCode).toBe(0);
    expect(result.failed).toBe(false);
  });

  test("a base input replaces the pull request's base", async () => {
    const repo = await checkout("pr-base-input", "newfn");
    const head = await repo.sha();
    const result = await replay(repo, "pull_request", { inputs: { base: head } });
    expect(report(result).scope).toMatchObject({ mode: "baseline", comparison_base: head });
    expect(result.outputs).toMatchObject({ introduced: "0", fixed: "0" });
    expect(result.gh).toEqual([]);
  });

  test("a no-op pull request with an earlier sticky comment edits it", async () => {
    const repo = await checkout("pr-noop-patch", "none");
    const result = await replay(repo, "pull_request");
    expect(result.outputs).toMatchObject({ introduced: "0", fixed: "0" });
    expect(result.stepOutputs.summarize!.clean).toBe("true");
    const posted = await comment(repo, { comments: botComment(42) });
    expect(writes(posted).map((call) => call.slice(0, 4))).toEqual([["api", "-X", "PATCH", "repos/octo/app/issues/comments/42"]]);
  });

  test("a no-op pull request with comment-on-clean false posts no comment (US-002)", async () => {
    const repo = await checkout("pr-noop-quiet", "none");
    const result = await replay(repo, "pull_request");
    expect(result.stepOutputs.summarize!.clean).toBe("true");
    expect(readFileSync(join(result.runnerTemp, "rust-doctor/clean"), "utf8")).toBe("true\n");
    const quiet = await comment(repo, { inputs: { "comment-on-clean": "false" } });
    expect(writes(quiet)).toEqual([]);
    expect(step(quiet, COMMENT_STEP).stdout).toContain("comment-on-clean is false, so no comment was posted");
    expect(quiet.failed).toBe(false);
  });

  test("a no-op pull request posts a comment when comment-on-clean keeps its default", async () => {
    const repo = await checkout("pr-noop-post", "none");
    await replay(repo, "pull_request");
    expect(writes(await comment(repo)).map((call) => call[2])).toEqual(["POST"]);
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
    const result = await replay(repo, "pull_request", { inputs: { args: "--bogus" } });
    expect(result.stepOutputs.scan!["exit-code"]).toBe("2");
    expect(result.summary).toBe("rust-doctor wrote no report that could be rendered.\n");
    expect(result.stepOutputs.summarize!.clean).toBe("false");
    const skipped = await comment(repo, { comments: botComment(42) });
    expect(skipped.gh).toEqual([]);
    expect(step(skipped, COMMENT_STEP).stdout).toContain("rust-doctor wrote no report, so no comment was posted.");
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
    await replay(repo, "pull_request");
    const list = await comment(repo, { ghListExit: 1 });
    expect(step(list, COMMENT_STEP).exitCode).toBe(0);
    expect(step(list, COMMENT_STEP).stdout).toContain("::warning::rust-doctor could not read the pull request's comments");
    // A fork's read-only token: the write is refused, and the job stays green.
    const write = await comment(repo, { ghWriteExit: 1 });
    expect(step(write, COMMENT_STEP).exitCode).toBe(0);
    expect(step(write, COMMENT_STEP).stdout).toContain("::warning::rust-doctor could not post the pull request comment");
    expect(write.failed).toBe(false);
  });

  // action.yml downgrades every comment failure to a warning, but a comment
  // list that is not JSON fails jq under `set -e` and turns the step red.
  test.failing("real/B8 a comment list that is not JSON leaves a warning and a green step", async () => {
    const repo = await checkout("api-not-json", "none");
    await replay(repo, "pull_request");
    const result = await comment(repo, { comments: "<html>bad gateway</html>" });
    expect(step(result, COMMENT_STEP).exitCode).toBe(0);
    expect(step(result, COMMENT_STEP).stdout).toContain("::warning::");
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

describe("the launcher and the comment job (EP-005)", () => {
  // US-019: a runner's `~/.cargo/bin` comes first on PATH, and a build script
  // can plant a `rust-doctor` there. The Action runs the launcher npm
  // installed, by its absolute path, so the planted one never runs.
  test("a planted rust-doctor earlier on PATH never stands in for the pinned launcher", async () => {
    const repo = await checkout("planted-launcher", "none");
    const planted = join(repo.sandbox.dir, "cargo-bin");
    const marker = join(repo.sandbox.dir, "planted-ran");
    mkdirSync(planted);
    writeFileSync(join(planted, "rust-doctor"), `#!/bin/sh\ntouch "${marker}"\necho "rust-doctor 0.0.0-planted"\n`, { mode: 0o755 });
    const result = await replay(repo, "push", { inputs: { version: cargoVersion() }, pathPrefix: [planted] });
    const install = step(result, "Install rust-doctor");
    expect(install.exitCode).toBe(0);
    expect(install.stdout).toBe(`rust-doctor ${cargoVersion()}\n`);
    expect(result.stepOutputs.install!.launcher).toEndWith("/npm-prefix/bin/rust-doctor");
    expect(result.stepOutputs.scan!["exit-code"]).toBe("0");
    expect(report(result).status).toBe("complete");
    expect(result.summary.length).toBeGreaterThan(0);
    expect(existsSync(marker)).toBe(false);
  });

  // US-020: a scan that crashed before its upload leaves no artifact, and the
  // comment job says so and stays green.
  test("the comment job without an artifact prints the notice and exits 0", async () => {
    const repo = await checkout("no-artifact", "none");
    const result = await comment(repo);
    expect(step(result, COMMENT_STEP).exitCode).toBe(0);
    expect(step(result, COMMENT_STEP).stdout).toContain("rust-doctor wrote no report, so no comment was posted.");
    expect(result.gh).toEqual([]);
    expect(result.failed).toBe(false);
  });

  test("the comment action checks nothing out, installs nothing and downloads the scan's artifact", () => {
    const steps = actionYaml(COMMENT_ACTION_PATH).runs.steps;
    expect(steps.map((candidate) => candidate.uses?.split("@")[0] ?? candidate.name)).toEqual(["actions/download-artifact", COMMENT_STEP]);
    expect(steps[0]!.uses).toMatch(/@[0-9a-f]{40}$/);
    expect(JSON.stringify(steps)).not.toContain("npm install");
  });
});
