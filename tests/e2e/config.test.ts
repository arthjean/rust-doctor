// CF-01 to CF-44 of `tasks/e2e-catalog/subcommands.md` section 11:
// `rust-doctor.toml`. Every row scans its own copy of the catalog's `cfg`
// workspace (a `todo!()` in `src/lib.rs`, `src/generated/mod.rs` and
// `src/quiet/mod.rs`, and `x > u8::MAX` in `src/quiet/mod.rs`) and reads the
// policy, the gate and the diagnostics from `--json`.
import { describe, expect, test } from "bun:test";
import { mkdirSync, symlinkSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { copyWorkspace, runDoctor, scratchDir, treeFingerprint, type DoctorRun } from "./e2e-helpers";
import { gitScenario } from "./git-scenario";

type Level = "off" | "info" | "warn" | "error";
type Report = {
  status: string;
  policy: {
    config_file: string | null;
    blocking: { level: string; source: string };
    rules: Array<{ id: string; level: Level; source: string }>;
  } | null;
  gate: { blocking: string; status: string; blocking_diagnostics: number | null };
  scan: { ignored: number };
  errors: Array<{ stage: string; code: string; message: string }>;
  diagnostics: Array<{ code: string; path: string; severity: string; span: { line_start: number } }>;
};

const TODO = "clippy::todo";
const ABSURD = "clippy::absurd_extreme_comparisons";
const COMPLEX = "rust_doctor::structure::complex_function";

const CFG: Record<string, string> = {
  "Cargo.toml": '[package]\nname = "cfg"\nversion = "0.1.0"\nedition = "2024"\npublish = false\n\n[workspace]\n',
  "src/lib.rs": "pub mod generated;\npub mod quiet;\n\npub fn top() -> u8 {\n    todo!()\n}\n",
  "src/generated/mod.rs": "pub fn generated() -> u8 {\n    todo!()\n}\n",
  "src/quiet/mod.rs": "pub fn quiet() -> u8 {\n    todo!()\n}\n\npub fn absurd(x: u8) -> bool {\n    x > u8::MAX\n}\n",
};

function writeTree(dir: string, files: Record<string, string | Uint8Array>): void {
  for (const [path, body] of Object.entries(files)) {
    mkdirSync(dirname(join(dir, path)), { recursive: true });
    writeFileSync(join(dir, path), body);
  }
}

function cfg(config?: string | Uint8Array): string {
  const dir = join(scratchDir("cfg"), "cfg");
  writeTree(dir, CFG);
  if (config !== undefined) writeFileSync(join(dir, "rust-doctor.toml"), config);
  return dir;
}

// A scan that must leave the workspace as it found it.
async function scan(workspace: string, flags: string[] = [], json = true): Promise<DoctorRun & { report: Report }> {
  const before = treeFingerprint(workspace);
  const run = await runDoctor([workspace, "--yes", ...(json ? ["--json"] : []), ...flags]);
  expect(treeFingerprint(workspace)).toBe(before);
  return { ...run, report: run.json as Report };
}

const rule = (report: Report, id: string) => report.policy?.rules.find((entry) => entry.id === id);

// `code@path:line=severity` of every diagnostic, sorted.
const sites = (report: Report) =>
  report.diagnostics.map((d) => `${d.code.replace("clippy::", "")}@${d.path}:${d.span.line_start}=${d.severity}`).sort();

const ALL_WARN = [
  "absurd_extreme_comparisons@src/quiet/mod.rs:6=warning",
  "todo@src/generated/mod.rs:2=warning",
  "todo@src/lib.rs:5=warning",
  "todo@src/quiet/mod.rs:2=warning",
];

const override = (paths: string[], table: "rules" | "categories", key: string, level: string) =>
  `[[overrides]]\npaths = ${JSON.stringify(paths)}\n[overrides.${table}]\n"${key}" = "${level}"\n\n`;

type Applied = {
  config?: string;
  flags?: string[];
  exit: number;
  // A line the terminal report of the same scan prints.
  terminal?: string;
  check: (report: Report) => void;
};

const applied: Array<[string, string, Applied]> = [
  ["CF-01", "no file loads the defaults and passes the default gate", {
    exit: 0,
    terminal: "Configuration: none loaded; blocking error (default)",
    check: (report) => {
      expect(report.policy?.config_file).toBeNull();
      expect(report.policy?.blocking).toEqual({ level: "error", source: "default" });
      expect(sites(report)).toEqual(ALL_WARN);
      expect(report.gate.status).toBe("passed");
    },
  }],
  ["CF-03", "blocking = warning fails the gate on the four warnings", {
    config: 'blocking = "warning"\n',
    exit: 1,
    terminal: "Configuration: rust-doctor.toml loaded; blocking warning (config)",
    check: (report) => {
      expect(report.policy?.blocking).toEqual({ level: "warning", source: "config" });
      expect(report.gate).toEqual({ blocking: "warning", status: "failed", blocking_diagnostics: 4 });
    },
  }],
  ["CF-04", "--blocking none beats the file's blocking", {
    config: 'blocking = "warning"\n',
    flags: ["--blocking", "none"],
    exit: 0,
    check: (report) => {
      expect(report.policy?.blocking).toEqual({ level: "none", source: "request" });
      expect(report.gate.status).toBe("passed");
    },
  }],
  ["CF-05", "--blocking error beats blocking = none and the todo errors block", {
    config: `blocking = "none"\n[rules]\n"${TODO}" = "error"\n`,
    flags: ["--blocking", "error"],
    exit: 1,
    check: (report) => {
      expect(report.policy?.blocking).toEqual({ level: "error", source: "request" });
      expect(report.gate).toEqual({ blocking: "error", status: "failed", blocking_diagnostics: 3 });
    },
  }],
  ["CF-06", "an ignore path matches whole segments: src/gen leaves src/generated", {
    config: '[ignore]\npaths = ["src/gen"]\n',
    exit: 0,
    check: (report) => {
      expect(report.scan.ignored).toBe(0);
      expect(sites(report)).toEqual(ALL_WARN);
    },
  }],
  ["CF-07", "ignore paths drop their diagnostics and count them in scan.ignored", {
    config: '[ignore]\npaths = ["src/generated/", "src/q*"]\n',
    exit: 0,
    check: (report) => {
      expect(sites(report)).toEqual(["todo@src/lib.rs:5=warning"]);
      expect(report.scan.ignored).toBe(3);
    },
  }],
  ["CF-08", "an override switches todo off under its paths only", {
    config: override(["src/quiet/**"], "rules", TODO, "off"),
    exit: 0,
    check: (report) => {
      expect(sites(report)).toEqual(ALL_WARN.filter((site) => site !== "todo@src/quiet/mod.rs:2=warning"));
    },
  }],
  ["CF-09", "an override raises a category under its paths only and fails the gate", {
    config: override(["src/quiet"], "categories", "correctness", "error"),
    exit: 1,
    check: (report) => {
      expect(sites(report)).toEqual([
        "absurd_extreme_comparisons@src/quiet/mod.rs:6=error",
        "todo@src/generated/mod.rs:2=warning",
        "todo@src/lib.rs:5=warning",
        "todo@src/quiet/mod.rs:2=error",
      ]);
      expect(report.gate.status).toBe("failed");
    },
  }],
  ["CF-10", "the later override table wins", {
    config: override(["src/**"], "rules", TODO, "error") + override(["src/quiet/**"], "rules", TODO, "off"),
    exit: 1,
    check: (report) => {
      expect(sites(report)).toEqual([
        "absurd_extreme_comparisons@src/quiet/mod.rs:6=warning",
        "todo@src/generated/mod.rs:2=error",
        "todo@src/lib.rs:5=error",
      ]);
    },
  }],
  ["CF-11", "an override cannot switch on a rule the plan leaves off", {
    config: `[rules]\n"${TODO}" = "off"\n\n` + override(["src/quiet/**"], "rules", TODO, "error"),
    exit: 0,
    check: (report) => {
      expect(rule(report, TODO)).toMatchObject({ level: "off", source: "config-rule" });
      expect(sites(report)).toEqual(["absurd_extreme_comparisons@src/quiet/mod.rs:6=warning"]);
    },
  }],
  ["CF-12", "--rule beats an override", {
    config: override(["src/**"], "rules", TODO, "error"),
    flags: ["--rule", `${TODO}=warn`],
    exit: 0,
    check: (report) => {
      expect(rule(report, TODO)).toMatchObject({ level: "warn", source: "request-rule" });
      expect(sites(report)).toEqual(ALL_WARN);
    },
  }],
  ["CF-13", "a request category beats a config rule", {
    config: `[rules]\n"${TODO}" = "error"\n`,
    flags: ["--category", "correctness=off"],
    exit: 0,
    check: (report) => {
      expect(rule(report, TODO)).toMatchObject({ level: "off", source: "request-category" });
      expect(report.diagnostics).toEqual([]);
    },
  }],
  ["CF-14", "a request rule beats a config category", {
    config: "[categories]\ncorrectness = \"off\"\n",
    flags: ["--rule", `${TODO}=error`],
    exit: 1,
    check: (report) => {
      expect(rule(report, TODO)).toMatchObject({ level: "error", source: "request-rule" });
      expect(rule(report, ABSURD)).toMatchObject({ level: "off", source: "config-category" });
      expect(sites(report)).toEqual(["todo@src/generated/mod.rs:2=error", "todo@src/lib.rs:5=error", "todo@src/quiet/mod.rs:2=error"]);
    },
  }],
  ["CF-15", "a config rule beats a config category", {
    config: `[categories]\ncorrectness = "error"\n[rules]\n"${TODO}" = "off"\n`,
    exit: 1,
    check: (report) => {
      expect(rule(report, TODO)).toMatchObject({ level: "off", source: "config-rule" });
      expect(rule(report, ABSURD)).toMatchObject({ level: "error", source: "config-category" });
      expect(sites(report)).toEqual(["absurd_extreme_comparisons@src/quiet/mod.rs:6=error"]);
    },
  }],
  ["CF-16", "a request rule beats a request category", {
    flags: ["--rule", `${TODO}=off`, "--category", "correctness=error"],
    exit: 1,
    check: (report) => {
      expect(rule(report, TODO)).toMatchObject({ level: "off", source: "request-rule" });
      expect(rule(report, ABSURD)).toMatchObject({ level: "error", source: "request-category" });
      expect(sites(report)).toEqual(["absurd_extreme_comparisons@src/quiet/mod.rs:6=error"]);
    },
  }],
  ["CF-30", "a threshold of 10000 is accepted", {
    config: "[structure]\ncyclomatic-threshold = 10000\n",
    exit: 0,
    check: (report) => expect(report.status).toBe("complete"),
  }],
  ["CF-34", "a UTF-8 byte order mark is accepted", {
    config: '﻿blocking = "warning"\n',
    exit: 1,
    terminal: "Configuration: rust-doctor.toml loaded; blocking warning (config)",
    check: (report) => expect(report.policy?.blocking).toEqual({ level: "warning", source: "config" }),
  }],
  ["CF-35", "CRLF line endings are accepted", {
    config: `blocking = "warning"\r\n[rules]\r\n"${TODO}" = "error"\r\n`,
    exit: 1,
    check: (report) => {
      expect(report.policy?.blocking).toEqual({ level: "warning", source: "config" });
      expect(rule(report, TODO)).toMatchObject({ level: "error", source: "config-rule" });
    },
  }],
];

type Failure = {
  // One file per case; every one fails the same way.
  configs: Array<string | Uint8Array>;
  flags?: string[];
  stage: "configuration" | "policy";
  code: string;
  // The message of each case, in order, when the row pins it.
  messages?: string[];
};

// 65 536 bytes, the documented ceiling, holding a blocking key.
function sized(bytes: number): string {
  const head = 'blocking = "warning"\n';
  return head + "#".repeat(bytes - head.length - 1) + "\n";
}

const NOT_UTF8 = Buffer.concat([Buffer.from("# caf"), Buffer.from([0xff]), Buffer.from('\nblocking = "warning"\n')]);

const GLOBS = ["{a,b}", "[ab]", "!src", "/abs", "./src", "../x", "src/**x", "a\\b", "", ".", "src//gen"];

const failures: Array<[string, string, Failure]> = [
  ["CF-17", "a rule given twice on the command line fails the policy", {
    configs: [],
    flags: ["--rule", `${TODO}=warn`, "--rule", `${TODO}=off`],
    stage: "policy",
    code: "duplicate-rule-override",
  }],
  ["CF-20", "a selector out of case is invalid", {
    configs: ['[rules]\n"Clippy::Todo" = "off"\n'],
    stage: "configuration",
    code: "invalid-rule-selector",
  }],
  ["CF-21", "an unknown category is refused", {
    configs: ['[categories]\nstyle = "off"\n'],
    stage: "configuration",
    code: "unknown-category",
  }],
  ["CF-22", "a level outside off, info, warn, error is invalid at its line and column", {
    configs: [`[rules]\n"${TODO}" = "warning"\n`],
    stage: "configuration",
    code: "config-invalid",
    messages: ["Invalid rust-doctor.toml at line 2, column 18."],
  }],
  ["CF-23", "blocking = warn is invalid at its column", {
    configs: ['blocking = "warn"\n'],
    stage: "configuration",
    code: "config-invalid",
    messages: ["Invalid rust-doctor.toml at line 1, column 12."],
  }],
  ["CF-24", "unknown keys at the top, in ignore and in an override are invalid", {
    configs: [
      'blockng = "warning"\n',
      "[ignore]\npaths = []\nother = 1\n",
      '[[overrides]]\npaths = ["src"]\nlevel = "error"\n',
    ],
    stage: "configuration",
    code: "config-invalid",
    messages: [
      "Invalid rust-doctor.toml at line 1, column 1.",
      "Invalid rust-doctor.toml at line 3, column 1.",
      "Invalid rust-doctor.toml at line 3, column 1.",
    ],
  }],
  ["CF-26", "every malformed ignore glob is refused at its index, never echoed", {
    configs: GLOBS.map((glob) => `[ignore]\npaths = ["ok", ${JSON.stringify(glob)}]\n`),
    stage: "policy",
    code: "invalid-glob",
    messages: GLOBS.map(() => "Invalid glob in rust-doctor.toml at ignore.paths[1]."),
  }],
  ["CF-27", "a bad glob in the second override names overrides[1]", {
    configs: [override(["src"], "rules", TODO, "off") + override(["../x"], "rules", TODO, "off")],
    stage: "policy",
    code: "invalid-glob",
    messages: ["Invalid glob in rust-doctor.toml at overrides[1].paths[0]."],
  }],
  ["CF-28", "a snake_case threshold key is invalid", {
    configs: ["[structure]\ncyclomatic_threshold = 5\n"],
    stage: "configuration",
    code: "config-invalid",
    messages: ["Invalid rust-doctor.toml at line 2, column 1."],
  }],
  ["CF-29", "thresholds 0 and 10001 are out of range", {
    configs: ["[structure]\ncyclomatic-threshold = 0\n", "[structure]\ncognitive-threshold = 10001\n"],
    stage: "configuration",
    code: "config-invalid-threshold",
    messages: [
      "Structure thresholds in rust-doctor.toml must be between 1 and 10000.",
      "Structure thresholds in rust-doctor.toml must be between 1 and 10000.",
    ],
  }],
  ["CF-31", "a negative or quoted threshold is invalid at its value", {
    configs: ["[structure]\ncognitive-threshold = -1\n", '[structure]\ncognitive-threshold = "5"\n'],
    stage: "configuration",
    code: "config-invalid",
    messages: ["Invalid rust-doctor.toml at line 2, column 23.", "Invalid rust-doctor.toml at line 2, column 23."],
  }],
  ["CF-33", "a duplicate key is invalid at the second one", {
    configs: ['blocking = "warning"\nblocking = "none"\n'],
    stage: "configuration",
    code: "config-invalid",
    messages: ["Invalid rust-doctor.toml at line 2, column 1."],
  }],
  ["CF-36", "an invalid UTF-8 byte fails the scan, naming the file", {
    configs: [NOT_UTF8],
    stage: "configuration",
    code: "config-invalid-utf8",
    messages: ["rust-doctor.toml must be UTF-8."],
  }],
];

function expectFailedReport(run: DoctorRun & { report: Report }, stage: string, code: string): void {
  expect(run.exitCode).toBe(2);
  expect(run.report.status).toBe("failed");
  expect(run.report.policy).toBeNull();
  expect(run.report.gate.status).toBe("not-evaluated");
  expect(run.report.errors).toHaveLength(1);
  expect(run.report.errors[0]).toMatchObject({ stage, code });
}

describe("rust-doctor.toml", () => {
  test.each(applied)("%s %s", async (_id, _title, row) => {
    const workspace = cfg(row.config);
    const run = await scan(workspace, row.flags);
    expect(run.exitCode).toBe(row.exit);
    expect(run.report.status).toBe("complete");
    if (row.config !== undefined) expect(run.report.policy?.config_file).toBe("rust-doctor.toml");
    row.check(run.report);
    if (row.terminal) {
      const text = await scan(workspace, row.flags, false);
      expect(text.exitCode).toBe(row.exit);
      expect(text.stdout).toContain(`${row.terminal}\n`);
    }
  });

  test.each(failures)("%s %s", async (_id, _title, row) => {
    const configs = row.configs.length > 0 ? row.configs : [undefined];
    for (const [index, config] of configs.entries()) {
      const run = await scan(cfg(config), row.flags);
      expectFailedReport(run, row.stage, row.code);
      if (row.messages) expect(run.report.errors[0]!.message).toBe(row.messages[index]!);
      if (row.code === "invalid-glob") {
        const glob = GLOBS[index];
        if (glob && glob.length > 2) expect(run.stdout + run.stderr).not.toContain(glob);
      }
    }
  });

  test("CF-02 an empty file and a comment-only file load the defaults", async () => {
    for (const config of ["", "# nothing to configure\n"]) {
      const run = await scan(cfg(config));
      expect(run.exitCode).toBe(0);
      expect(run.report.policy?.config_file).toBe("rust-doctor.toml");
      expect(run.report.policy?.blocking).toEqual({ level: "error", source: "default" });
      expect(rule(run.report, TODO)).toMatchObject({ level: "warn", source: "default" });
      expect(sites(run.report)).toEqual(ALL_WARN);
    }
  });

  test.failing("CF-18 subcommands/F19 --rule takes the suffix rules explain takes", async () => {
    const workspace = copyWorkspace("kernel-contract/todo");
    expect((await runDoctor(["rules", "explain", "todo", workspace])).exitCode).toBe(0);
    const run = await runDoctor([workspace, "--yes", "--json", "--rule", "todo=off"]);
    expect(run.exitCode).toBe(0);
    const report = run.json as { diagnostics: Array<{ code: string }> };
    expect(report.diagnostics.map((diagnostic) => diagnostic.code)).not.toContain("clippy::todo");
  });

  test.failing("CF-19 subcommands/F19 [rules] takes the suffix rules explain takes", async () => {
    const run = await scan(cfg('[rules]\ntodo = "off"\n'));
    expect(run.exitCode).toBe(0);
    expect(rule(run.report, TODO)).toMatchObject({ level: "off", source: "config-rule" });
  });

  test("CF-25 an override without paths is invalid, one with an empty list is inert", async () => {
    const tables = `[overrides.rules]\n"${TODO}" = "off"\n`;
    const missing = await scan(cfg(`[[overrides]]\n${tables}`));
    expectFailedReport(missing, "configuration", "config-invalid");
    expect(missing.report.errors[0]!.message).toBe("Invalid rust-doctor.toml at line 1, column 1.");
    const empty = await scan(cfg(`[[overrides]]\npaths = []\n${tables}`));
    expect(empty.exitCode).toBe(0);
    expect(sites(empty.report)).toEqual(ALL_WARN);
  });

  test("CF-32 the complexity thresholds decide what complex_function reports", async () => {
    const workspace = copyWorkspace("structure/complex-function");
    const complex = (report: Report) => report.diagnostics.filter((d) => d.code === COMPLEX);
    writeFileSync(join(workspace, "rust-doctor.toml"), "[structure]\ncyclomatic-threshold = 12\ncognitive-threshold = 31\n");
    const raised = await scan(workspace);
    expect(raised.exitCode).toBe(0);
    expect(complex(raised.report)).toEqual([]);
    writeFileSync(join(workspace, "rust-doctor.toml"), "[structure]\ncognitive-threshold = 2\n");
    const lowered = await scan(workspace);
    expect(lowered.exitCode).toBe(0);
    expect(complex(lowered.report)).toHaveLength(2);
  });

  test("CF-37 65536 bytes are read, 65537 exit 2 naming the file", async () => {
    const atLimit = await scan(cfg(sized(65_536)));
    expect(atLimit.exitCode).toBe(1);
    expect(atLimit.report.policy?.blocking).toEqual({ level: "warning", source: "config" });
    const workspace = cfg(sized(65_537));
    const over = await scan(workspace);
    expectFailedReport(over, "configuration", "config-too-large");
    expect(over.report.errors[0]!.message).toBe("rust-doctor.toml exceeds 65536 bytes.");
    const text = await scan(workspace, [], false);
    expect(text.exitCode).toBe(2);
    expect(text.stdout.replace(/\s+/g, " ")).toContain("Scan failed: rust-doctor.toml exceeds 65536 bytes. (configuration/config-too-large)");
  });

  test("CF-38 a rust-doctor.toml symlink to a file outside the workspace is followed", async () => {
    const outside = join(scratchDir("cf-38-outside"), "shared.toml");
    writeFileSync(outside, 'blocking = "warning"\n');
    const workspace = cfg();
    symlinkSync(outside, join(workspace, "rust-doctor.toml"));
    const run = await scan(workspace);
    expect(run.exitCode).toBe(1);
    expect(run.report.policy?.config_file).toBe("rust-doctor.toml");
    expect(run.report.policy?.blocking).toEqual({ level: "warning", source: "config" });
  });

  test("CF-39 a directory and a dangling symlink are refused", async () => {
    const directory = cfg();
    mkdirSync(join(directory, "rust-doctor.toml"));
    const notFile = await scan(directory);
    expectFailedReport(notFile, "configuration", "config-not-file");
    expect(notFile.report.errors[0]!.message).toBe("rust-doctor.toml is not a regular file.");
    const dangling = cfg();
    symlinkSync("missing.toml", join(dangling, "rust-doctor.toml"));
    const unreadable = await scan(dangling);
    expectFailedReport(unreadable, "configuration", "config-unreadable");
    expect(unreadable.report.errors[0]!.message).toBe("Could not read rust-doctor.toml.");
  });

  // The acceptance criterion of US-026: oversized, invalid and non-UTF-8 files
  // exit 2 and name the file, in the JSON report and in the terminal one.
  const broken: Array<[string, string | Uint8Array, string, string]> = [
    ["invalid", "blockng = 1\n", "config-invalid", "Invalid rust-doctor.toml at line 1, column 1."],
    ["oversized", sized(65_537), "config-too-large", "rust-doctor.toml exceeds 65536 bytes."],
    ["non-UTF-8", NOT_UTF8, "config-invalid-utf8", "rust-doctor.toml must be UTF-8."],
  ];
  test("CF-40 an invalid, oversized or non-UTF-8 file fails the whole report and names the file", async () => {
    for (const [, config, code, message] of broken) {
      const workspace = cfg(config);
      const run = await scan(workspace);
      expectFailedReport(run, "configuration", code);
      expect(run.report.errors[0]!.message).toBe(message);
      expect(run.report.diagnostics).toEqual([]);
      const text = await scan(workspace, [], false);
      expect(text.exitCode).toBe(2);
      // The terminal report wraps at 80 columns, so the line break moves.
      expect(text.stdout.replace(/\s+/g, " ")).toContain(`Scan failed: ${message} (configuration/${code})`);
    }
  });

  test("CF-41 a member's own rust-doctor.toml is ignored by the scan", async () => {
    const dir = join(scratchDir("cf-41"), "ws");
    writeTree(dir, {
      "Cargo.toml": '[workspace]\nmembers = ["m"]\nresolver = "3"\n',
      "m/Cargo.toml": '[package]\nname = "m"\nversion = "0.1.0"\nedition = "2024"\npublish = false\n',
      "m/src/lib.rs": "pub fn m() -> u8 {\n    todo!()\n}\n",
      "m/rust-doctor.toml": `blocking = "warning"\n[rules]\n"${TODO}" = "error"\n`,
    });
    const run = await scan(dir);
    expect(run.exitCode).toBe(0);
    expect(run.report.policy?.config_file).toBeNull();
    expect(run.report.policy?.blocking).toEqual({ level: "error", source: "default" });
    expect(sites(run.report)).toEqual(["todo@m/src/lib.rs:2=warning"]);
  });

  test("CF-42 a git root's rust-doctor.toml does not reach a workspace in src-tauri", async () => {
    const files: Record<string, string> = { "rust-doctor.toml": 'blocking = "warning"\n', ".gitignore": "target/\n" };
    for (const [path, body] of Object.entries(CFG)) files[`src-tauri/${path}`] = body;
    const repo = await gitScenario("cf-42").write(files).commit("base").build();
    const before = treeFingerprint(repo.path);
    const run = await runDoctor([".", "--yes", "--json"], { cwd: repo.path, sandbox: repo.sandbox });
    expect(treeFingerprint(repo.path)).toBe(before);
    const report = run.json as Report;
    expect(run.exitCode).toBe(0);
    expect(report.policy?.config_file).toBeNull();
    expect(report.policy?.blocking).toEqual({ level: "error", source: "default" });
  });

  test("CF-43 a staged scan reads the staged rust-doctor.toml, never the working tree's", async () => {
    const untracked = await gitScenario("cf-43-untracked")
      .write({ ...CFG, ".gitignore": "/target\n" })
      .commit("base")
      .stage({ write: { "src/lib.rs": `${CFG["src/lib.rs"]}\npub fn more() -> u8 {\n    todo!()\n}\n` } })
      .untracked({ "rust-doctor.toml": 'blocking = "warning"\n' })
      .build();
    const staged = await gitScenario("cf-43-staged")
      .write({ ...CFG, ".gitignore": "/target\n" })
      .commit("base")
      .stage({
        write: {
          "src/lib.rs": `${CFG["src/lib.rs"]}\npub fn more() -> u8 {\n    todo!()\n}\n`,
          "rust-doctor.toml": 'blocking = "warning"\n',
        },
      })
      .unstaged({ "rust-doctor.toml": 'blocking = "none"\n' })
      .build();
    const stagedScan = async (path: string) => {
      const before = treeFingerprint(path);
      const run = await runDoctor([path, "--yes", "--json", "--staged", "--scope", "lines"]);
      expect(treeFingerprint(path)).toBe(before);
      return { run, report: run.json as Report };
    };
    const ignored = await stagedScan(untracked.path);
    expect(ignored.run.exitCode).toBe(0);
    expect(ignored.report.policy?.config_file).toBeNull();
    expect(ignored.report.gate).toEqual({ blocking: "error", status: "passed", blocking_diagnostics: 0 });
    const read = await stagedScan(staged.path);
    expect(read.run.exitCode).toBe(1);
    expect(read.report.policy?.config_file).toBe("rust-doctor.toml");
    expect(read.report.policy?.blocking).toEqual({ level: "warning", source: "config" });
    expect(read.report.gate).toEqual({ blocking: "warning", status: "failed", blocking_diagnostics: 1 });
  });

  test("CF-44 hostile selectors, paths and escapes never reach the JSON or terminal report", async () => {
    const hostile = "/home/private https://secret credential=token \u001b[31m";
    const sentinels = ["/home/private", "https://secret", "credential=token", "\u001b"];
    const configs = [
      `[rules]\n"${hostile}" = "warn"\n`,
      '[rules]\n"clippy::home_private_secret" = "warn"\n',
      '[ignore]\npaths = ["/home/private https://secret credential=token"]\n',
      override(["/home/private"], "rules", TODO, "off"),
      '[categories]\n"https://secret" = "off"\n',
    ];
    for (const config of configs) {
      const workspace = cfg(config);
      for (const json of [true, false]) {
        const run = await scan(workspace, [], json);
        expect(run.exitCode).toBe(2);
        for (const sentinel of sentinels) expect(run.stdout + run.stderr).not.toContain(sentinel);
      }
    }
  });
});
