// This repository as a real codebase: a `git clone --local --no-hardlinks` of
// HEAD in the test's scratch root, never the working tree, so a scan judges
// what is committed and can write nothing into the live checkout. A test that
// only reads shares one clone per process; a test that commits takes its own.
import { join } from "node:path";
import { REPO_ROOT, runCommand, sandbox, scratchDir, targetDir, type Sandbox } from "./e2e-helpers";

export type SelfClone = {
  path: string;
  sandbox: Sandbox;
  git(args: string[]): Promise<string>;
  sha(rev?: string): Promise<string>;
  // A named scan target, kept across runs, so only the first run is cold.
  target(key: string): string;
};

const IDENTITY = {
  GIT_AUTHOR_NAME: "e2e",
  GIT_AUTHOR_EMAIL: "e2e@example.invalid",
  GIT_COMMITTER_NAME: "e2e",
  GIT_COMMITTER_EMAIL: "e2e@example.invalid",
  GIT_AUTHOR_DATE: "2026-01-01T00:00:00Z",
  GIT_COMMITTER_DATE: "2026-01-01T00:00:00Z",
};

const FLAGS = ["-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null", "-c", "advice.detachedHead=false"];

async function git(cwd: string, box: Sandbox, args: string[]): Promise<string> {
  const run = await runCommand(["git", ...FLAGS, ...args], { cwd, sandbox: box, env: IDENTITY });
  if (run.exitCode !== 0) throw new Error(`git ${args.join(" ")} failed in ${cwd} (exit ${run.exitCode}): ${run.stderr.trim()}`);
  return run.stdout;
}

export async function cloneThisRepository(label = "self"): Promise<SelfClone> {
  const box = sandbox(`clone-${label}`);
  const path = join(scratchDir(`${label}-repo`), "rust-doctor");
  await git(box.dir, box, ["clone", "-q", "--local", "--no-hardlinks", REPO_ROOT, path]);
  return {
    path,
    sandbox: box,
    git: (args) => git(path, box, args),
    sha: async (rev = "HEAD") => (await git(path, box, ["rev-parse", rev])).trim(),
    target: (key) => targetDir(`self-${key}`),
  };
}

let shared: Promise<SelfClone> | undefined;

// One read-only clone for every test of the file: a test that writes into it,
// commits or switches branch takes its own with `cloneThisRepository`.
export function sharedClone(): Promise<SelfClone> {
  shared ??= cloneThisRepository("shared");
  return shared;
}
