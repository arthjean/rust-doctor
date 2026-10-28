// CL-01 to CL-13 of `tasks/e2e-catalog/subcommands.md` section 1: the parser
// surface every other file builds on. Nothing here compiles a workspace but
// the one scan that saves the report CL-11 and CL-13 read.
import { beforeAll, describe, expect, test } from "bun:test";
import { writeFileSync } from "node:fs";
import { join } from "node:path";
import {
  BIN,
  cargoVersion,
  copyWorkspace,
  isEmptyDir,
  runCommand,
  runDoctor,
  sandbox,
  type Sandbox,
} from "./e2e-helpers";

const ESC = "\x1b";
let box: Sandbox;
let report: string;

beforeAll(async () => {
  box = sandbox("cli-surface");
  const workspace = copyWorkspace("kernel-contract/todo");
  const scan = await runDoctor([workspace, "--yes", "--json"], { sandbox: box });
  expect(scan.exitCode).toBe(0);
  report = join(box.dir, "report.json");
  writeFileSync(report, scan.stdout);
});

describe("cli surface", () => {
  test("CL-01 --version prints the Cargo.toml version", async () => {
    const run = await runDoctor(["--version"], { sandbox: box });
    expect(run.exitCode).toBe(0);
    expect(run.stdout).toBe(`rust-doctor ${cargoVersion()}\n`);
  });

  const documented: Array<[string, string[]]> = [
    ["rules", []],
    ["rules list", ["--json"]],
    ["rules explain", ["--json"]],
    ["why", ["--report", "--json"]],
    ["report", []],
    ["report markdown", ["--limit", "[default: 20]"]],
    ["setup", ["--dry-run"]],
    ["hook", []],
    ["hook install", []],
    ["hook install git", ["--update", "--dry-run"]],
    ["hook install claude", ["--shared", "--update", "--dry-run"]],
    ["hook install cursor", ["--update", "--dry-run"]],
    ["hook run", ["[possible values: claude, cursor]"]],
    ["skill", []],
    ["skill install", ["--update", "--dry-run", "[possible values: claude, codex, cursor, all, detected]"]],
    ["ci", []],
    ["ci install", ["--update", "--dry-run", "--comment", "[possible values: none, error, warning]", "[default: 1.99.1]"]],
  ];
  test.each(documented)("CL-02 %s --help documents its flags", async (command, flags) => {
    const run = await runDoctor([...command.split(" "), "--help"], { sandbox: box });
    expect(run.exitCode).toBe(0);
    expect(run.stderr).toBe("");
    expect(run.stdout).toContain(`Usage: rust-doctor ${command}`);
    for (const flag of flags) expect(run.stdout).toContain(flag);
  });

  test.each(["rules", "hook", "hook install", "skill", "ci", "report"])(
    "CL-03 bare %s prints its usage on stderr and exits 2",
    async (command) => {
      const run = await runDoctor(command.split(" "), { sandbox: box });
      expect(run.exitCode).toBe(2);
      expect(run.stdout).toBe("");
      expect(run.stderr).toContain(`Usage: rust-doctor ${command} <COMMAND>`);
    },
  );

  test("CL-04 skill -h and skill --help carry two different abouts", async () => {
    const short = await runDoctor(["skill", "-h"], { sandbox: box });
    const long = await runDoctor(["skill", "--help"], { sandbox: box });
    expect(short.stdout.split("\n")[0]).toBe("Install the agent skill");
    expect(long.stdout.split("\n")[0]).toBe(
      "The skill an agent reads to drive the tool, written into the workspace.",
    );
  });

  test("CL-05 one argv holding a subcommand and a flag is a PATH", async () => {
    const run = await runDoctor(["rules --help"], { sandbox: box });
    expect(run.exitCode).toBe(2);
    expect(run.stdout + run.stderr).toContain("Scan failed: path is not a directory (discovery/no-manifest)");
  });

  test("CL-06 setup is the subcommand, ./setup is the directory", async () => {
    const parent = join(box.dir, "cl-06");
    copyWorkspace("kernel-contract/todo", join(parent, "setup"));
    const subcommand = await runDoctor(["setup", "--dry-run"], { sandbox: box, cwd: parent });
    expect(subcommand.exitCode).toBe(2);
    expect(subcommand.stderr).toContain("no agent detected");
    const scan = await runDoctor(["./setup", "--yes", "--json"], { sandbox: box, cwd: parent });
    expect(scan.exitCode).toBe(0);
    expect((scan.json as { schema_version: number }).schema_version).toBe(23);
  });

  test("CL-07 hook install codex is an unknown subcommand", async () => {
    const run = await runDoctor(["hook", "install", "codex"], { sandbox: box });
    expect(run.exitCode).toBe(2);
    expect(run.stderr).toContain("unrecognized subcommand 'codex'");
    expect(run.stderr).toContain("a similar subcommand exists: 'claude'");
  });

  test("CL-08 hook run codex is an invalid agent", async () => {
    const run = await runDoctor(["hook", "run", "codex"], { sandbox: box });
    expect(run.exitCode).toBe(2);
    expect(run.stderr).toContain("invalid value 'codex' for '<AGENT>'");
    expect(run.stderr).toContain("[possible values: claude, cursor]");
  });

  test("CL-09 skill install --agent bogus lists the five agents", async () => {
    const run = await runDoctor(["skill", "install", "--agent", "bogus"], { sandbox: box });
    expect(run.exitCode).toBe(2);
    expect(run.stderr).toContain("invalid value 'bogus'");
    expect(run.stderr).toContain("[possible values: claude, codex, cursor, all, detected]");
  });

  test.failing("CL-10 subcommands/F9 rules explain and why never echo an ESC byte of argv", async () => {
    const explain = await runDoctor(["rules", "explain", `${ESC}[31mtodo`], { sandbox: box });
    const why = await runDoctor(["why", `src/${ESC}[2Jlib.rs`], { sandbox: box });
    expect(explain.exitCode).toBe(2);
    expect(why.exitCode).toBe(2);
    expect(explain.stderr).not.toContain(ESC);
    expect(why.stderr).not.toContain(ESC);
  });

  test.each([
    ["rules list"],
    ["rules list --json"],
    ["rules explain todo"],
    ["why src/lib.rs:1 --report REPORT"],
    ["report markdown REPORT"],
  ])("CL-11 %s exits 0 into a pipe closed after one line", async (command) => {
    const args = command.split(" ").map((arg) => (arg === "REPORT" ? report : arg));
    const run = await runCommand(
      ["bash", "-c", '"$0" "$@" 2>"$STDERR_FILE" | head -1 >/dev/null; exit "${PIPESTATUS[0]}"', BIN, ...args],
      { sandbox: box, env: { STDERR_FILE: join(box.dir, "cl-11.stderr") } },
    );
    expect(run.exitCode).toBe(0);
    expect(await Bun.file(join(box.dir, "cl-11.stderr")).text()).toBe("");
  });

  test.each([
    ["rules list"],
    ["rules explain todo"],
    ["report markdown REPORT"],
    ["why src/lib.rs:1 --report REPORT"],
  ])("CL-13 %s succeeds with an empty PATH: it starts no process", async (command) => {
    const args = command.split(" ").map((arg) => (arg === "REPORT" ? report : arg));
    const run = await runDoctor(args, { sandbox: box, env: { PATH: "" } });
    expect(run.exitCode).toBe(0);
    expect(run.stdout.length).toBeGreaterThan(0);
  });

  // Last in the file: bun runs a file's tests in order, so this sees every
  // command above.
  test("CL-12 no command above wrote the fake HOME", () => {
    expect(isEmptyDir(box.home)).toBe(true);
  });
});
