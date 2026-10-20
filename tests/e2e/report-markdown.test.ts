// RM-01 to RM-22 of `tasks/e2e-catalog/subcommands.md` section 4: `report
// markdown`. One scan of `twoMember` saves the report every row renders; the
// failed, baseline, hostile and foreign documents are derived from it or
// written by hand, so nothing else compiles.
import { beforeAll, describe, expect, test } from "bun:test";
import { chmodSync, mkdirSync, readFileSync, truncateSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import {
  isEmptyDir,
  runCommand,
  runDoctor,
  scratchDir,
  treeFingerprint,
  type DoctorOptions,
  type DoctorRun,
} from "./e2e-helpers";
import { twoMemberFiles } from "./explain-support";
import { gitScenario, type Repo } from "./git-scenario";

type Diagnostic = { id: string; code: string; message: string; path: string; severity: string; span: { line_start: number } | null };
type Report = {
  schema_version: number;
  status: string;
  complete: boolean;
  audit: { score: { value: number; label: string; authoritative: boolean } | null };
  scope: Record<string, unknown> | null;
  diagnostics: Diagnostic[];
  delta: unknown;
  errors: Array<{ stage: string; code: string; message: string }>;
};

let repo: Repo;
let docs: string;
let saved: Report;
let report: string;

beforeAll(async () => {
  repo = await gitScenario("report-markdown").write(twoMemberFiles()).commit("base").build();
  const scan = await runDoctor([repo.path, "--yes", "--json"], { sandbox: repo.sandbox });
  expect(scan.exitCode).toBe(1);
  saved = scan.json as Report;
  docs = join(repo.sandbox.dir, "docs");
  mkdirSync(docs);
  report = writeDoc("report.json", scan.stdout);
});

function writeDoc(name: string, body: string | Report): string {
  const path = join(docs, name);
  writeFileSync(path, typeof body === "string" ? body : JSON.stringify(body));
  return path;
}

function derived(change: (report: Report) => void): Report {
  const copy = structuredClone(saved);
  change(copy);
  return copy;
}

// `report markdown` reads, and never writes, the documents or the workspace.
async function render(args: string[], options: DoctorOptions = {}): Promise<DoctorRun> {
  const before = [treeFingerprint(docs), treeFingerprint(repo.path)];
  const run = await runDoctor(["report", "markdown", ...args], { sandbox: repo.sandbox, cwd: docs, ...options });
  expect([treeFingerprint(docs), treeFingerprint(repo.path)]).toEqual(before);
  return run;
}

function refused(run: DoctorRun, message: string): void {
  expect(run.exitCode).toBe(2);
  expect(run.stdout).toBe("");
  expect(run.stderr).toBe(`rust-doctor: ${message}\n`);
}

// The refusals RM-22 replays, each built when it runs.
const REFUSALS: Record<string, () => string[]> = {
  "RM-04": () => [report, "--limit", "1001"],
  "RM-05": () => [report, "--limit", "-1"],
  "RM-06": () => [report, "--limit", "abc"],
  "RM-07": () => ["/nonexistent.json"],
  "RM-08": () => [docs],
  "RM-09": () => [writeDoc("rm-22-09.json", '{"a":1}')],
  "RM-10": () => [writeDoc("rm-22-10.json", `{"schema_version":${saved.schema_version}}`)],
  "RM-11": () => [writeDoc("rm-22-11.json", derived((r) => void (r.schema_version += 1)))],
  "RM-12": () => [writeDoc("rm-22-12.json", derived((r) => void (r.schema_version = 19)))],
  "RM-13": () => [writeDoc("rm-22-13.json", "")],
  "RM-14": () => ["-"],
};

describe("report markdown", () => {
  test("RM-01 the saved report renders as a summary, a linked table and a footer", async () => {
    expect(saved.diagnostics.map((diagnostic) => diagnostic.code)).toEqual(["clippy::todo"]);
    const score = saved.audit.score!;
    const run = await render([report]);
    expect(run.exitCode).toBe(0);
    expect(run.stderr).toBe("");
    const lines = run.stdout.split("\n");
    expect(lines[0]).toBe("## rust-doctor");
    expect(lines).toContain(`**Score: ${score.value}/100 (${score.label})**, authoritative.`);
    expect(lines).toContain("| Rule | Location | Severity | Message |");
    expect(lines).toContain(
      "| [`clippy::todo`](https://rust-doctor.com/rules/clippy%3A%3Atodo) | a/src/lib.rs:2 | error | \\`todo\\` should not be present in production code |",
    );
    expect(run.stdout).toEndWith("\n1 finding.\n");
  });

  test("RM-02 --limit 0 drops the table and says how many were left out", async () => {
    const run = await render([report, "--limit", "0"]);
    expect(run.exitCode).toBe(0);
    expect(run.stdout).not.toContain("| Rule |");
    expect(run.stdout).toEndWith("\nShowing 0 of 1 findings.\n");
  });

  test("RM-03 --limit 1000 is accepted", async () => {
    const run = await render([report, "--limit", "1000"]);
    expect(run.exitCode).toBe(0);
    expect(run.stdout).toBe((await render([report])).stdout);
  });

  test("RM-04 --limit 1001 is out of range", async () => {
    const run = await render([report, "--limit", "1001"]);
    expect(run.exitCode).toBe(2);
    expect(run.stdout).toBe("");
    expect(run.stderr).toContain("invalid value '1001' for '--limit <N>': 1001 is not in 0..=1000");
  });

  test("RM-05 --limit -1 reads as an unknown flag, with the -- tip", async () => {
    const run = await render([report, "--limit", "-1"]);
    expect(run.exitCode).toBe(2);
    expect(run.stderr).toContain("unexpected argument '-1' found");
    expect(run.stderr).toContain("-- -1");
  });

  test("RM-06 --limit abc is not a number", async () => {
    const run = await render([report, "--limit", "abc"]);
    expect(run.exitCode).toBe(2);
    expect(run.stderr).toContain("invalid digit found in string");
  });

  test("RM-07 a missing file is refused naming it", async () => {
    refused(await render(["/nonexistent.json"]), "/nonexistent.json: the report could not be read (entity not found).");
  });

  test("RM-08 a directory is refused", async () => {
    refused(await render([docs]), `${docs}: the report could not be read (is a directory).`);
  });

  test("RM-09 a document without schema_version is refused", async () => {
    const path = writeDoc("rm-09.json", '{"a":1}');
    refused(await render([path]), `${path}: the report carries no schema_version, and this binary reads ${saved.schema_version}.`);
  });

  test("RM-10 a schema_version alone is not a report", async () => {
    const path = writeDoc("rm-10.json", `{"schema_version":${saved.schema_version}}`);
    refused(await render([path]), `${path}: the file is not a rust-doctor report.`);
  });

  test("RM-11 a newer schema is refused with the remedy", async () => {
    const next = saved.schema_version + 1;
    const path = writeDoc("rm-11.json", derived((r) => void (r.schema_version = next)));
    refused(
      await render([path]),
      `${path}: the report has schema_version ${next}, and this binary reads ${saved.schema_version}: render it with the rust-doctor that wrote it.`,
    );
  });

  test("RM-12 schema 20 is the oldest one read, 19 is refused", async () => {
    const old = writeDoc("rm-12-19.json", derived((r) => void (r.schema_version = 19)));
    const oldest = writeDoc("rm-12-20.json", derived((r) => void (r.schema_version = 20)));
    const refusedRun = await render([old]);
    expect(refusedRun.exitCode).toBe(2);
    expect(refusedRun.stderr).toContain("the report has schema_version 19");
    const accepted = await render([oldest]);
    expect(accepted.exitCode).toBe(0);
    expect(accepted.stdout).toStartWith("## rust-doctor\n");
  });

  test("RM-13 an empty file is not valid JSON", async () => {
    const path = writeDoc("rm-13.json", "");
    refused(await render([path]), `${path}: the report is not valid JSON (line 1, column 0).`);
  });

  test("RM-14 - is a file name, not stdin", async () => {
    refused(await render(["-"], { stdin: readFileSync(report, "utf8") }), "-: the report could not be read (entity not found).");
  });

  // Through a shell pipe: Bun hands a child's stdin over a socket, which
  // `/dev/stdin` cannot be opened on.
  test("RM-15 /dev/stdin renders a piped report", async () => {
    const before = treeFingerprint(docs);
    const run = await runCommand(["bash", "-c", 'cat "$0" | rust-doctor report markdown /dev/stdin', report], { sandbox: repo.sandbox });
    expect(treeFingerprint(docs)).toBe(before);
    expect(run.exitCode).toBe(0);
    expect(run.stdout).toBe((await render([report])).stdout);
  });

  test("RM-16 a file over 64 MiB is refused unread", async () => {
    const path = writeDoc("rm-16.json", "");
    truncateSync(path, 64 * 1024 * 1024 + 1);
    const run = await render([path]);
    expect(run.exitCode).toBe(2);
    expect(run.stdout).toBe("");
    expect(run.stderr).toContain("larger than 64 MiB");
  });

  test("RM-17 hostile text in a message and a path is escaped for markdown and HTML", async () => {
    const path = writeDoc("rm-17.json", derived((r) => {
      r.diagnostics[0]!.message = "<img src=x onerror=alert(1)> | pipe | `code` [link](http://evil) \x1b[31mred\n@everyone";
      r.diagnostics[0]!.path = "a/src/<b>x</b>|.rs";
    }));
    const run = await render([path]);
    expect(run.exitCode).toBe(0);
    expect(run.stdout).not.toContain("\x1b");
    const row = run.stdout.split("\n").find((line) => line.startsWith("| [`clippy::todo`]")) ?? "";
    expect(row).toContain("&lt;img src=x onerror=alert(1)&gt;");
    expect(row).toContain("\\| pipe \\|");
    expect(row).toContain("\\`code\\`");
    expect(row).toContain("\\[link\\](http:&#8203;//evil)");
    expect(row).toContain("red @&#8203;everyone");
    expect(row).toContain("a/src/&lt;b&gt;x&lt;/b&gt;\\|.rs:2");
    expect(row).not.toContain("<img");
  });

  test("RM-18 a failed scan renders no score, its error and no table", async () => {
    const path = writeDoc("rm-18.json", derived((r) => {
      r.status = "failed";
      r.complete = false;
      r.audit.score = null;
      r.scope = null;
      r.diagnostics = [];
      r.errors = [{ stage: "configuration", code: "config-invalid", message: "Invalid rust-doctor.toml at line 1, column 1." }];
    }));
    const run = await render([path]);
    expect(run.exitCode).toBe(0);
    const lines = run.stdout.split("\n");
    expect(lines).toContain("**Score: none**, not authoritative.");
    expect(lines).toContain("**The scan reported 1 error:**");
    expect(lines).toContain("- configuration config-invalid: Invalid rust-doctor.toml at line 1, column 1.");
    expect(run.stdout).toEndWith("\nNo finding was collected.\n");
    expect(run.stdout).not.toContain("| Rule |");
  });

  test("RM-19 a baseline report counts introduced and fixed and lists only what it introduced", async () => {
    const todo = saved.diagnostics[0]!;
    const path = writeDoc("rm-19.json", derived((r) => {
      r.scope = { ...r.scope, mode: "baseline", comparison_base: "0".repeat(40), base_ref: "main" };
      r.delta = {
        fingerprint_version: 1,
        base_diagnostics: 0,
        current_diagnostics: 1,
        introduced: [todo.id],
        pre_existing: [],
        fixed: [],
        summary: { introduced: 1, pre_existing: 0, fixed: 0, cross_file_matches: 0 },
      };
    }));
    const run = await render([path]);
    expect(run.exitCode).toBe(0);
    expect(run.stdout.split("\n")).toContain("**Against the base:** 1 introduced, 0 fixed.");
    expect(run.stdout.split("\n").filter((line) => line.startsWith("| [`"))).toHaveLength(1);
    expect(run.stdout).toEndWith("\n1 introduced finding.\n");
  });

  test("RM-20 a reader that closes early leaves the renderer at exit 0", async () => {
    const run = await runCommand(["bash", "-c", 'rust-doctor report markdown "$0" | head -1; echo "status ${PIPESTATUS[0]}"', report], {
      sandbox: repo.sandbox,
    });
    expect(run.stdout).toBe("## rust-doctor\nstatus 0\n");
    expect(run.stderr).toBe("");
  });

  test("RM-21 rendering starts no program", async () => {
    const dir = scratchDir("rm-21");
    const bin = join(dir, "bin");
    const marks = join(dir, "marks");
    mkdirSync(bin);
    mkdirSync(marks);
    for (const tool of ["cargo", "git", "rustc", "clippy-driver", "rustup", "npm", "gh"]) {
      writeFileSync(join(bin, tool), `#!/bin/sh\n: > "${marks}/${tool}"\n`);
      chmodSync(join(bin, tool), 0o755);
    }
    const run = await render([report], { env: { PATH: bin } });
    expect(run.exitCode).toBe(0);
    expect(run.stdout).toStartWith("## rust-doctor\n");
    expect(isEmptyDir(marks)).toBe(true);
  });

  test("RM-22 every refusal leaves stdout empty", async () => {
    for (const [id, args] of Object.entries(REFUSALS)) {
      const run = await render(args(), { stdin: "" });
      expect({ id, exitCode: run.exitCode, stdout: run.stdout }).toEqual({ id, exitCode: 2, stdout: "" });
    }
  });
});
