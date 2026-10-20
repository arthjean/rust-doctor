// What the scope files of US-021 share: a JSON scan that must leave the tree
// as it found it, the human scope line, and the few projections of a report
// the catalog's expectations are written in.
import { expect } from "bun:test";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { runCommand, runDoctor, sandbox, scratchDir, treeFingerprint, type DoctorOptions, type DoctorRun, type Sandbox } from "./e2e-helpers";
import { miniTemplate } from "./git-scenario";

// The base commit of `mini`, fixed by the scenario's identity and date.
export const MINI_BASE = "af441c9b9f96faa3f098b61e401b7475f31e56d7";

export type Scope = {
  mode: string;
  execution_scope: string;
  comparison_base: string | null;
  files: string[] | null;
  base_ref: string | null;
  staged: boolean;
  untracked_unreported: number | null;
};

export type Diagnostic = { id: string; code: string; path: string; span: { line_start: number } | null };

export type Report = {
  status: string;
  complete: boolean;
  scope: Scope | null;
  project: unknown;
  errors: Array<{ stage: string; code: string; message: string }>;
  diagnostics: Diagnostic[];
  delta: {
    introduced: string[];
    summary: { introduced: number; pre_existing: number; fixed: number; cross_file_matches: number; rule_count_matches: number };
  } | null;
};

export type Scan = { run: DoctorRun; report: Report };

export type ScanOptions = DoctorOptions & {
  // The tree whose fingerprint must not change, when it is not the scanned
  // path itself.
  tree?: string;
};

// A JSON scan of `path` that leaves the tree it inspects untouched.
export async function scan(path: string, args: string[], options: ScanOptions = {}): Promise<Scan> {
  const tree = options.tree ?? path;
  const before = treeFingerprint(tree);
  const run = await runDoctor([path, "--yes", "--json", ...args], options);
  expect(treeFingerprint(tree)).toBe(before);
  if (run.json === undefined) throw new Error(`no JSON report (exit ${run.exitCode}): ${run.stderr}`);
  return { run, report: run.json as Report };
}

// The human report of `path`, which leaves the tree it inspects untouched.
export async function human(path: string, args: string[], options: ScanOptions = {}): Promise<DoctorRun> {
  const tree = options.tree ?? path;
  const before = treeFingerprint(tree);
  const run = await runDoctor([path, "--yes", ...args], options);
  expect(treeFingerprint(tree)).toBe(before);
  return run;
}

// Text with every run of whitespace folded to one space, so a sentence the
// 80-column layout wrapped still reads as one.
export function unwrapped(text: string): string {
  return text.replace(/\s+/g, " ");
}

// The line of the human report that names the scope, unwrapped: off a
// terminal the report is laid out at 80 columns, and a scope line always ends
// with its closing parenthesis unless it names the full codebase.
export async function scopeLine(path: string, args: string[], options: ScanOptions = {}): Promise<string> {
  const lines = (await human(path, args, options)).stdout.split("\n");
  let line = lines.shift() ?? "";
  while (line.startsWith("Scope: ") && line !== "Scope: full codebase" && !line.endsWith(")") && lines.length > 0) {
    line += ` ${lines.shift()!.trim()}`;
  }
  return line;
}

export function errorCodes(report: Report): string[] {
  return report.errors.map((error) => `${error.stage}/${error.code}`);
}

// `path:line` of every finding of `code`, sorted.
export function sites(report: Report, code = "clippy::todo"): string[] {
  return report.diagnostics
    .filter((diagnostic) => diagnostic.code === code)
    .map((diagnostic) => `${diagnostic.path}:${diagnostic.span?.line_start}`)
    .sort();
}

export function count(report: Report, code: string): number {
  return report.diagnostics.filter((diagnostic) => diagnostic.code === code).length;
}

// The catalog's `D[i,p,f,x,r]`.
export function deltaSummary(report: Report): number[] {
  const summary = report.delta?.summary;
  if (!summary) throw new Error("the report carries no delta");
  return [summary.introduced, summary.pre_existing, summary.fixed, summary.cross_file_matches, summary.rule_count_matches];
}

// `code@path:line` of every introduced finding, sorted.
export function introduced(report: Report): string[] {
  const ids = new Set(report.delta?.introduced ?? []);
  return report.diagnostics
    .filter((diagnostic) => ids.has(diagnostic.id))
    .map((diagnostic) => `${diagnostic.code}@${diagnostic.path}:${diagnostic.span?.line_start}`)
    .sort();
}

export function writeFiles(root: string, files: Record<string, string>): void {
  for (const [path, body] of Object.entries(files)) {
    mkdirSync(dirname(join(root, path)), { recursive: true });
    writeFileSync(join(root, path), body);
  }
}

export type PlainRepo = { path: string; sandbox: Sandbox; git(args: string[]): Promise<string> };

// `mini` in a SHA-256 repository, which `gitScenario` cannot initialize; with
// `extra`, a branch `feature` holding one commit that appends it to the lib.
export async function sha256Mini(name: string, extra?: string): Promise<PlainRepo> {
  const box = sandbox(`git-${name}`);
  const path = join(scratchDir(name), name);
  mkdirSync(path, { recursive: true });
  const git = async (args: string[]) => {
    const run = await runCommand(
      ["git", "-c", "init.defaultBranch=main", "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null", ...args],
      {
        sandbox: box,
        cwd: path,
        env: {
          GIT_AUTHOR_NAME: "e2e",
          GIT_AUTHOR_EMAIL: "e2e@example.invalid",
          GIT_COMMITTER_NAME: "e2e",
          GIT_COMMITTER_EMAIL: "e2e@example.invalid",
          GIT_AUTHOR_DATE: "2026-01-01T00:00:00Z",
          GIT_COMMITTER_DATE: "2026-01-01T00:00:00Z",
        },
      },
    );
    if (run.exitCode !== 0) throw new Error(`git ${args.join(" ")} failed: ${run.stderr.trim()}`);
    return run.stdout;
  };
  await git(["init", "-q", "--object-format=sha256"]);
  writeFiles(path, miniTemplate());
  await git(["add", "-A"]);
  await git(["commit", "-q", "-m", "base"]);
  if (extra !== undefined) {
    await git(["switch", "-q", "-c", "feature"]);
    writeFileSync(join(path, "src/lib.rs"), (await git(["show", "HEAD:src/lib.rs"])) + extra);
    await git(["commit", "-q", "-am", "feature"]);
  }
  return { path, sandbox: box, git };
}
