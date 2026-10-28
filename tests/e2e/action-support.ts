// action.yml, or comment/action.yml, replayed without GitHub: its `run:` steps
// execute as GitHub runs a composite action's `shell: bash` (`bash --noprofile
// --norc -eo pipefail`), under fake GITHUB_* files, with the release binary as
// the preinstalled `rust-doctor` and as the launcher a stub `npm` installed
// under its prefix, and a recording `gh` stub. Only the expressions action.yml
// uses are evaluated: `inputs.*`, `github.*`, `steps.<id>.outputs.*` and the
// `if:` forms built from `always()`, `==`, `!=` and `&&`. A `uses:` step
// (toolchain, cache, artifact upload) is the runner's and is skipped.
import { chmodSync, existsSync, mkdirSync, readFileSync, symlinkSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { BIN, hermeticEnv, REPO_ROOT, RUN_ROOT, sandbox, spawnTracked, toolchainBin, type Sandbox } from "./e2e-helpers";

type Step = {
  name?: string;
  id?: string;
  if?: string;
  uses?: string;
  shell?: string;
  env?: Record<string, string>;
  run?: string;
};

type ActionYaml = {
  inputs: Record<string, { default?: string }>;
  outputs?: Record<string, { value: string }>;
  runs: { using: string; steps: Step[] };
};

export const ACTION_PATH = join(REPO_ROOT, "action.yml");
export const COMMENT_ACTION_PATH = join(REPO_ROOT, "comment/action.yml");

export function actionYaml(path = ACTION_PATH): ActionYaml {
  return Bun.YAML.parse(readFileSync(path, "utf8")) as ActionYaml;
}

// The stand-in for `gh api`: every call is appended to $GH_LOG as one JSON
// array of its arguments. Listing comments prints $GH_COMMENTS, or fails with
// $GH_LIST_EXIT; a PATCH or POST exits $GH_WRITE_EXIT.
const GH_STUB = `#!/bin/sh
set -u
log=\${GH_LOG:?GH_LOG is not set}
first=1
{
  printf '['
  for arg in "$@"; do
    [ "$first" = 1 ] || printf ','
    first=0
    printf '%s' "$(printf '%s' "$arg" | jq -Rs .)"
  done
  printf ']\\n'
} >> "$log"
case " $* " in
  *" -X "*) exit "\${GH_WRITE_EXIT:-0}" ;;
esac
[ "\${GH_LIST_EXIT:-0}" = 0 ] || exit "$GH_LIST_EXIT"
cat "\${GH_COMMENTS:-/dev/null}"
`;

let toolsDir: string | undefined;

// The prefix the `npm` stub answers `npm prefix -g` with, whose
// `bin/rust-doctor` is the binary under test.
export const NPM_PREFIX = join(RUN_ROOT, "npm-prefix");

// `npm install -g` installs nothing (the launcher already sits under the
// prefix), and `npm prefix -g` prints the prefix.
const NPM_STUB = `#!/bin/sh
[ "\${1:-}" = prefix ] && echo "${NPM_PREFIX}"
exit 0
`;

// rust-doctor (the binary under test), npm, gh (the stubs) and jq, in front of
// the suite's toolchain directory.
export function actionTools(): string {
  if (toolsDir) return toolsDir;
  const dir = join(RUN_ROOT, "action-bin");
  mkdirSync(dir, { recursive: true });
  symlinkSync(BIN, join(dir, "rust-doctor"));
  mkdirSync(join(NPM_PREFIX, "bin"), { recursive: true });
  symlinkSync(BIN, join(NPM_PREFIX, "bin/rust-doctor"));
  writeFileSync(join(dir, "npm"), NPM_STUB);
  chmodSync(join(dir, "npm"), 0o755);
  const jq = Bun.which("jq", { PATH: "/usr/local/bin:/usr/bin:/bin" });
  if (!jq) throw new Error("jq is not installed: the Action replay needs it, as a hosted runner provides it");
  symlinkSync(jq, join(dir, "jq"));
  writeFileSync(join(dir, "gh"), GH_STUB);
  chmodSync(join(dir, "gh"), 0o755);
  toolsDir = dir;
  return dir;
}

export type Event = "push" | "pull_request" | "workflow_dispatch";

export type ReplayOptions = {
  // The action replayed, action.yml unless set.
  action?: string;
  // Directories put in front of PATH, as a runner's `~/.cargo/bin` is.
  pathPrefix?: string[];
  // The checkout, GITHUB_WORKSPACE and the steps' working directory.
  workspace: string;
  event: Event;
  inputs?: Record<string, string>;
  // github.base_ref, set on a pull request.
  baseRef?: string;
  // The comments `gh api` lists, as the JSON text it prints.
  comments?: string;
  ghListExit?: number;
  ghWriteExit?: number;
  // Only the steps with these names run; `stepOutputs` stands for the ones
  // left out.
  only?: string[];
  stepOutputs?: Record<string, Record<string, string>>;
  target?: string;
  sandbox?: Sandbox;
};

export type StepRun = { name: string; ran: boolean; exitCode: number | null; stdout: string; stderr: string };

export type Replay = {
  steps: StepRun[];
  stepOutputs: Record<string, Record<string, string>>;
  outputs: Record<string, string>;
  summary: string;
  gh: string[][];
  // The status the composite action ends with: the last failing step's.
  failed: boolean;
  exitCode: number;
  runnerTemp: string;
};

type Context = {
  inputs: Record<string, string>;
  github: Record<string, unknown>;
  steps: Record<string, { outputs: Record<string, string> }>;
};

function lookup(context: Context, path: string): string {
  let value: unknown = context;
  for (const key of path.split(".")) {
    if (value === null || typeof value !== "object") return "";
    value = (value as Record<string, unknown>)[key];
  }
  return value === undefined || value === null ? "" : String(value);
}

function literal(term: string, context: Context): string {
  const quoted = term.match(/^'(.*)'$/);
  return quoted ? quoted[1]! : lookup(context, term);
}

// `always()`, `a == 'b'`, `a != 'b'`, joined by `&&`: every form action.yml
// writes. Anything else fails, so a new form cannot be evaluated wrong.
export function evaluateIf(condition: string, context: Context): { value: boolean; always: boolean } {
  let always = false;
  const value = condition.split("&&").map((part) => part.trim()).every((term) => {
    if (term === "always()") {
      always = true;
      return true;
    }
    const comparison = term.match(/^([\w.-]+)\s*(==|!=)\s*('[^']*'|[\w.-]+)$/);
    if (!comparison) throw new Error(`unsupported expression in action.yml: ${term}`);
    const equal = literal(comparison[1]!, context) === literal(comparison[3]!, context);
    return comparison[2] === "==" ? equal : !equal;
  });
  return { value, always };
}

export function interpolate(text: string, context: Context): string {
  return text.replace(/\$\{\{\s*([\w.-]+)\s*\}\}/g, (_match, path: string) => lookup(context, path));
}

function parseOutputs(path: string): Record<string, string> {
  if (!existsSync(path)) return {};
  const outputs: Record<string, string> = {};
  for (const line of readFileSync(path, "utf8").split("\n")) {
    const equals = line.indexOf("=");
    if (equals > 0) outputs[line.slice(0, equals)] = line.slice(equals + 1);
  }
  return outputs;
}

export async function replayAction(options: ReplayOptions): Promise<Replay> {
  const action = actionYaml(options.action);
  const box = options.sandbox ?? sandbox("action");
  const runner = join(box.dir, "runner");
  const runnerTemp = join(runner, "temp");
  mkdirSync(runnerTemp, { recursive: true });
  const summaryPath = join(runner, "step-summary.md");
  const ghLog = join(runner, "gh.log");
  const comments = join(runner, "comments.json");
  writeFileSync(summaryPath, "");
  writeFileSync(comments, options.comments ?? "[]");
  const inputs: Record<string, string> = {};
  for (const [name, input] of Object.entries(action.inputs)) inputs[name] = input.default ?? "";
  Object.assign(inputs, options.action ? {} : { version: "preinstalled", cache: "false" }, options.inputs);
  const context: Context = {
    inputs,
    github: {
      event_name: options.event,
      base_ref: options.event === "pull_request" ? (options.baseRef ?? "main") : "",
      repository: "octo/app",
      token: "fake-github-token",
      workspace: options.workspace,
      event: options.event === "pull_request" ? { pull_request: { number: 7 } } : {},
    },
    steps: Object.fromEntries(Object.entries(options.stepOutputs ?? {}).map(([id, outputs]) => [id, { outputs }])),
  };
  const steps: StepRun[] = [];
  let failed = false;
  let exitCode = 0;
  for (const [index, step] of action.runs.steps.entries()) {
    const name = step.name ?? step.uses ?? `step ${index}`;
    const selected = !options.only || options.only.includes(name);
    const condition = step.if ? evaluateIf(step.if, context) : { value: true, always: false };
    const runs = selected && step.run !== undefined && condition.value && (condition.always || !failed);
    if (!runs) {
      steps.push({ name, ran: false, exitCode: null, stdout: "", stderr: "" });
      continue;
    }
    if (step.shell !== "bash") throw new Error(`${name}: only shell: bash is replayed`);
    const outputFile = join(runner, `output-${index}`);
    writeFileSync(outputFile, "");
    const script = join(runner, `step-${index}.sh`);
    writeFileSync(script, interpolate(step.run!, context));
    const env = hermeticEnv(box, {
      PATH: [...(options.pathPrefix ?? []), actionTools(), toolchainBin()].join(":"),
      CI: "true",
      GITHUB_ACTIONS: "true",
      GITHUB_EVENT_NAME: options.event,
      GITHUB_WORKSPACE: options.workspace,
      GITHUB_OUTPUT: outputFile,
      GITHUB_STEP_SUMMARY: summaryPath,
      RUNNER_TEMP: runnerTemp,
      GH_LOG: ghLog,
      GH_COMMENTS: comments,
      GH_LIST_EXIT: String(options.ghListExit ?? 0),
      GH_WRITE_EXIT: String(options.ghWriteExit ?? 0),
      ...(options.target ? { CARGO_TARGET_DIR: options.target } : {}),
      ...Object.fromEntries(Object.entries(step.env ?? {}).map(([key, value]) => [key, interpolate(value, context)])),
    });
    const run = await spawnTracked(["bash", "--noprofile", "--norc", "-eo", "pipefail", script], {
      cwd: options.workspace,
      env,
      timeoutMs: 90_000,
    }).exited;
    steps.push({ name, ran: true, exitCode: run.exitCode, stdout: run.stdout, stderr: run.stderr });
    if (step.id) context.steps[step.id] = { outputs: parseOutputs(outputFile) };
    if (run.exitCode !== 0) {
      failed = true;
      exitCode = run.exitCode ?? 1;
    }
  }
  const outputs = Object.fromEntries(Object.entries(action.outputs ?? {}).map(([name, output]) => [name, interpolate(output.value, context)]));
  const gh = existsSync(ghLog)
    ? readFileSync(ghLog, "utf8").split("\n").filter(Boolean).map((line) => JSON.parse(line) as string[])
    : [];
  return {
    steps,
    stepOutputs: Object.fromEntries(Object.entries(context.steps).map(([id, step]) => [id, step.outputs])),
    outputs,
    summary: readFileSync(summaryPath, "utf8"),
    gh,
    failed,
    exitCode,
    runnerTemp,
  };
}

export function step(replay: Replay, name: string): StepRun {
  const found = replay.steps.find((candidate) => candidate.name === name);
  if (!found) throw new Error(`action.yml has no step named ${name}`);
  return found;
}
