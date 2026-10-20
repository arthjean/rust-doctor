// RL-01 to RL-33 of `tasks/e2e-catalog/subcommands.md` section 2: `rules
// list` and `rules explain`. Nothing here compiles: the catalog numbers are
// read from `rules list --json`, never restated, so a rule added to the
// catalog moves every assertion with it.
import { beforeAll, describe, expect, test } from "bun:test";
import { mkdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { REPO_ROOT, runDoctor, sandbox, scratchDir, treeFingerprint, type DoctorOptions, type DoctorRun, type Sandbox } from "./e2e-helpers";
import { TWO_MEMBER_CONFIG, twoMember, writeTree } from "./explain-support";

type Rule = {
  id: string;
  category: string;
  producer: string;
  default_level: string;
  tier: string;
  help: string;
  diff_scope: string;
  title: string;
  not_a_defect_when: string;
  requires: unknown[];
};

type Explained = Rule & {
  url: string;
  corpus_noise_basis_points: number | null;
  corpus_reviewed_sites: number | null;
  effective?: unknown;
};

const PRODUCERS: Record<string, string> = {
  "clippy::": "clippy",
  "rust_doctor::cargo::": "cargo-health",
  "rust_doctor::source::": "source-kernel",
  "rust_doctor::structure::": "structure",
  "rust_doctor::repo::": "repo",
};

let box: Sandbox;
let catalog: Rule[];
let workspace: string;

beforeAll(async () => {
  box = sandbox("rules");
  const run = await runDoctor(["rules", "list", "--json"], { sandbox: box });
  expect(run.exitCode).toBe(0);
  catalog = run.json as Rule[];
  workspace = twoMember("rules-two-member");
});

function rule(id: string): Rule {
  const found = catalog.find((entry) => entry.id === id);
  if (!found) throw new Error(`no ${id} in rules list --json`);
  return found;
}

function prefixOf(id: string): string {
  const prefix = Object.keys(PRODUCERS).find((candidate) => id.startsWith(candidate));
  if (!prefix) throw new Error(`${id} carries no catalogued prefix`);
  return prefix;
}

// `rules explain` against a directory, asserting it wrote nothing there.
async function explain(args: string[], dir: string, options: DoctorOptions = {}): Promise<DoctorRun> {
  const before = treeFingerprint(dir);
  const run = await runDoctor(["rules", "explain", ...args], { sandbox: box, cwd: dir, ...options });
  expect(treeFingerprint(dir)).toBe(before);
  return run;
}

function effectiveLines(stdout: string): string[] {
  return stdout.split("\n").filter((line) => line.startsWith("Effective level:") || line.startsWith("Override:"));
}

// A one-crate workspace whose root holds `config`, for the refusals.
function crateWith(label: string, config?: string | Uint8Array): string {
  const dir = writeTree(join(scratchDir(label), "crate"), {
    "Cargo.toml": '[package]\nname = "c"\nversion = "0.1.0"\nedition = "2024"\n\n[workspace]\n',
    "src/lib.rs": "pub fn f() {}\n",
  });
  if (config !== undefined) writeFileSync(join(dir, "rust-doctor.toml"), config);
  return dir;
}

describe("rules list", () => {
  test("RL-01 the text list is one TAB-separated line per rule, in the JSON order", async () => {
    const dir = scratchDir("rl-01");
    const run = await runDoctor(["rules", "list"], { sandbox: box, cwd: dir });
    expect(run.exitCode).toBe(0);
    expect(run.stderr).toBe("");
    const lines = run.stdout.split("\n");
    expect(lines.pop()).toBe("");
    expect(lines).toHaveLength(catalog.length);
    for (const line of lines) expect(line.split("\t")).toHaveLength(4);
    expect(lines).toEqual(catalog.map((entry) => [entry.id, entry.category, entry.tier, entry.help].join("\t")));
    expect(lines[0]).toStartWith("clippy::absurd_extreme_comparisons\tcorrectness\tP1\t");
  });

  test("RL-02 every id prefix names its producer, and the text list counts them as the JSON does", async () => {
    const run = await runDoctor(["rules", "list"], { sandbox: box });
    const ids = run.stdout.trimEnd().split("\n").map((line) => line.split("\t")[0]!);
    const textCounts: Record<string, number> = {};
    for (const id of ids) textCounts[prefixOf(id)] = (textCounts[prefixOf(id)] ?? 0) + 1;
    const jsonCounts: Record<string, number> = {};
    for (const entry of catalog) {
      expect(entry.producer).toBe(PRODUCERS[prefixOf(entry.id)]!);
      jsonCounts[prefixOf(entry.id)] = (jsonCounts[prefixOf(entry.id)] ?? 0) + 1;
    }
    expect(textCounts).toEqual(jsonCounts);
    expect(Object.keys(jsonCounts).sort()).toEqual(Object.keys(PRODUCERS).sort());
  });

  test("RL-03 the JSON list carries exactly the documented keys and closed value sets", () => {
    expect(catalog.length).toBeGreaterThan(0);
    for (const entry of catalog) {
      expect(Object.keys(entry).sort()).toEqual([
        "category", "default_level", "diff_scope", "help", "id", "not_a_defect_when", "producer", "requires", "tier", "title",
      ]);
      expect(["P0", "P1", "P2", "P3"]).toContain(entry.tier);
      expect(entry.default_level).toBe("warn");
      expect(["line", "item", "file"]).toContain(entry.diff_scope);
      expect(Object.values(PRODUCERS)).toContain(entry.producer);
    }
    for (const tier of ["P0", "P1", "P2", "P3"]) {
      expect(catalog.some((entry) => entry.tier === tier)).toBe(true);
    }
  });

  test("RL-04 the README and the shipped skill state the count the binary publishes", () => {
    const count = String(catalog.length);
    const readme = readFileSync(join(REPO_ROOT, "README.md"), "utf8");
    const skill = readFileSync(join(REPO_ROOT, "skills/rust-doctor/SKILL.md"), "utf8");
    const stated = (text: string) => [...text.matchAll(/(\d+) curated rules/g)].map((match) => match[1]);
    expect(stated(readme).length).toBeGreaterThan(0);
    expect(stated(skill).length).toBeGreaterThan(0);
    expect(new Set([...stated(readme), ...stated(skill)])).toEqual(new Set([count]));
  });

  test("RL-05 a stray positional is refused by the parser", async () => {
    const run = await runDoctor(["rules", "list", "extra"], { sandbox: box });
    expect(run.exitCode).toBe(2);
    expect(run.stdout).toBe("");
    expect(run.stderr).toContain("unexpected argument 'extra' found");
  });

  test("RL-06 a repeated --json is refused by the parser", async () => {
    const run = await runDoctor(["rules", "list", "--json", "--json"], { sandbox: box });
    expect(run.exitCode).toBe(2);
    expect(run.stdout).toBe("");
    expect(run.stderr).toContain("cannot be used multiple times");
  });

  test("RL-07 the last segment of every id is unique, so a suffix names one rule", () => {
    const suffixes = catalog.map((entry) => entry.id.split("::").pop());
    expect(new Set(suffixes).size).toBe(catalog.length);
  });
});

describe("rules explain without a workspace", () => {
  test("RL-08 a suffix prints the catalog entry and no effective level", async () => {
    const todo = rule("clippy::todo");
    const run = await runDoctor(["rules", "explain", "todo"], { sandbox: box });
    expect(run.exitCode).toBe(0);
    expect(run.stderr).toBe("");
    expect(run.stdout).toBe([
      "clippy::todo",
      `Producer: ${todo.producer}`,
      `Category: ${todo.category}`,
      `Tier: ${todo.tier}`,
      `Default level: ${todo.default_level}`,
      `Help: ${todo.help}`,
      "Rule: https://rust-doctor.com/rules/clippy%3A%3Atodo",
      "Corpus noise: unmeasured",
      "",
    ].join("\n"));
    expect(run.stdout).not.toContain("Effective level");
  });

  test("RL-09 --json is the catalog entry with its URL and corpus noise, and no effective key", async () => {
    const run = await runDoctor(["rules", "explain", "clippy::todo", "--json"], { sandbox: box });
    expect(run.exitCode).toBe(0);
    const { url, corpus_noise_basis_points, corpus_reviewed_sites, ...entry } = run.json as Explained;
    expect(entry).toEqual(rule("clippy::todo"));
    expect(url).toBe("https://rust-doctor.com/rules/clippy%3A%3Atodo");
    expect(corpus_noise_basis_points).toBeNull();
    expect(corpus_reviewed_sites).toBeNull();
    expect(run.json as object).not.toHaveProperty("effective");
  });

  test("RL-10 a measured rule states its corpus noise in text as in JSON", async () => {
    const json = (await runDoctor(["rules", "explain", "clippy::exit", "--json"], { sandbox: box })).json as Explained;
    expect(json.corpus_noise_basis_points).not.toBeNull();
    expect(json.corpus_reviewed_sites).not.toBeNull();
    const text = await runDoctor(["rules", "explain", "clippy::exit"], { sandbox: box });
    expect(text.exitCode).toBe(0);
    const percent = Math.round(json.corpus_noise_basis_points! / 100);
    expect(text.stdout).toContain(`Corpus noise: ${percent}% on ${json.corpus_reviewed_sites} adjudicated sites\n`);
  });

  test("RL-11 a native rule names its producer and percent-encodes every separator of its URL", async () => {
    const id = "rust_doctor::source::dynamic_shell_command";
    const run = await runDoctor(["rules", "explain", id], { sandbox: box });
    expect(run.exitCode).toBe(0);
    expect(run.stdout).toContain("Producer: source-kernel\n");
    expect(run.stdout).toContain(`Tier: ${rule(id).tier}\n`);
    const url = run.stdout.split("\n").find((line) => line.startsWith("Rule: "))!.slice("Rule: ".length);
    expect(url).toBe(`https://rust-doctor.com/rules/${encodeURIComponent(id)}`);
    expect(url).not.toContain("::");
  });

  const unknown: Array<[string, string, string]> = [
    ["RL-12", "tood", "rust-doctor: no catalogued rule is named `tood`; did you mean `clippy::todo`?\n"],
    ["RL-13", "zzzzzzzzzzzz", "rust-doctor: no catalogued rule is named `zzzzzzzzzzzz`; `rust-doctor rules list` prints them all.\n"],
    ["RL-14", "", "rust-doctor: no catalogued rule is named ``; `rust-doctor rules list` prints them all.\n"],
  ];
  test.each(unknown)("%s an unknown rule is refused with one line", async (_id, name, stderr) => {
    const run = await runDoctor(["rules", "explain", name], { sandbox: box });
    expect(run.exitCode).toBe(2);
    expect(run.stdout).toBe("");
    expect(run.stderr).toBe(stderr);
  });

  test("RL-15 a missing RULE is refused by the parser", async () => {
    const run = await runDoctor(["rules", "explain"], { sandbox: box });
    expect(run.exitCode).toBe(2);
    expect(run.stdout).toBe("");
    expect(run.stderr).toContain("the following required arguments were not provided");
    expect(run.stderr).toContain("<RULE>");
  });
});

describe("rules explain against a workspace", () => {
  test("RL-16 the effective level and every override touching the rule, in table order", async () => {
    const run = await explain(["todo", "."], workspace);
    expect(run.exitCode).toBe(0);
    expect(run.stderr).toBe("");
    expect(effectiveLines(run.stdout)).toEqual([
      "Effective level: error (config-rule)",
      "Override: off under b/**",
      "Override: error under a/src/*.rs",
    ]);
  });

  test("RL-17 --json carries the effective level and the overrides as configured", async () => {
    const run = await explain(["todo", ".", "--json"], workspace);
    expect(run.exitCode).toBe(0);
    expect((run.json as Explained).effective).toEqual({
      level: "error",
      source: "config-rule",
      overrides: [
        { paths: ["b/**"], rules: { "clippy::todo": "off" }, categories: {} },
        { paths: ["a/src/*.rs"], rules: {}, categories: { correctness: "error" } },
      ],
    });
  });

  test.failing("RL-18 subcommands/F12 an override that cannot switch on a rule the plan leaves off says so", async () => {
    const run = await explain(["absurd_extreme_comparisons", "."], workspace);
    expect(run.exitCode).toBe(0);
    expect(run.stdout).toContain("Effective level: off (config-category)");
    const override = run.stdout.split("\n").find((line) => line.startsWith("Override:")) ?? "";
    expect(override).toMatch(/no effect|cannot|inert/i);
  });

  test("RL-19 a rule no configuration touches is at its default with no override", async () => {
    const run = await explain(["clippy::exit", "."], workspace);
    expect(run.exitCode).toBe(0);
    expect(effectiveLines(run.stdout)).toEqual(["Effective level: warn (default)"]);
  });

  // US-008 fixed subcommands/F4: a member path reads its workspace root's
  // configuration, as the scan does.
  test("RL-20 a member path answers with the workspace root's configuration", async () => {
    const root = await explain(["todo", "."], workspace);
    const member = await explain(["todo", "a"], workspace);
    expect(member.exitCode).toBe(0);
    expect(effectiveLines(member.stdout)[0]).toBe("Effective level: error (config-rule)");
    expect(member.stdout).toBe(root.stdout);
  });

  test("RL-21 a member's own rust-doctor.toml, which scans ignore, is not read", async () => {
    const dir = twoMember("rl-21");
    writeFileSync(join(dir, "b/rust-doctor.toml"), '[rules]\n"clippy::todo" = "off"\n');
    const run = await explain(["todo", "b"], dir);
    expect(run.exitCode).toBe(0);
    expect(effectiveLines(run.stdout)[0]).toBe("Effective level: error (config-rule)");
  });

  // US-007 fixed subcommands/F5: a PATH that does not exist is refused.
  test("RL-22 a PATH that does not exist is refused with one line naming it", async () => {
    const run = await explain(["todo", "/nonexistent/path"], workspace);
    expect(run.exitCode).toBe(2);
    expect(run.stdout).toBe("");
    expect(run.stderr.split("\n")).toHaveLength(2);
    expect(run.stderr).toContain("/nonexistent/path");
    expect(run.stderr).toContain("does not exist");
  });

  // US-008 resolves PATH through discovery, so a manifest names its workspace
  // and no longer fails to read `Cargo.toml/rust-doctor.toml`.
  test("RL-23 a manifest as PATH answers for its workspace", async () => {
    const run = await explain(["todo", "Cargo.toml"], workspace);
    expect(run.exitCode).toBe(0);
    expect(run.stderr).toBe("");
    expect(effectiveLines(run.stdout)[0]).toBe("Effective level: error (config-rule)");
  });

  const refusals: Array<[string, string, string | Uint8Array | "dir", string]> = [
    ["RL-24", "a level outside the vocabulary", '# levels\n[rules]\n"clippy::todo" = "warning"\n', "Invalid rust-doctor.toml at line 3, column 18."],
    ["RL-25", "an unknown key", "nope = 1\n", "Invalid rust-doctor.toml at line 1, column 1."],
    ["RL-26", "an unknown rule", '[rules]\n"clippy::nope" = "off"\n', "Unknown rule selector in rust-doctor.toml."],
    ["RL-27", "an absolute ignore glob", '[ignore]\npaths = ["/abs"]\n', "Invalid glob in rust-doctor.toml at ignore.paths[0]."],
    ["RL-28", "a zero threshold", "[structure]\ncyclomatic-threshold = 0\n", "Structure thresholds in rust-doctor.toml must be between 1 and 10000."],
    ["RL-29", "a file over 64 KiB", "#".repeat(65537), "rust-doctor.toml exceeds 65536 bytes."],
    ["RL-30", "bytes that are not UTF-8", new Uint8Array([0xff, 0xfe]), "rust-doctor.toml must be UTF-8."],
    ["RL-31", "a directory", "dir", "rust-doctor.toml is not a regular file."],
  ];
  test.each(refusals)("%s a configuration holding %s is refused", async (id, _what, config, message) => {
    const dir = crateWith(id, config === "dir" ? undefined : config);
    if (config === "dir") mkdirSync(join(dir, "rust-doctor.toml"));
    const run = await explain(["todo", dir], dir);
    expect(run.exitCode).toBe(2);
    expect(run.stdout).toBe("");
    expect(run.stderr).toBe(`rust-doctor: ${message}\n`);
  });

  test("RL-32 a directory holding no configuration is at the default", async () => {
    const dir = scratchDir("rl-32");
    const run = await explain(["todo", dir], dir);
    expect(run.exitCode).toBe(0);
    expect(effectiveLines(run.stdout)).toEqual(["Effective level: warn (default)"]);
  });

  test("RL-33 explaining writes nothing and leaves the configuration untouched", async () => {
    const config = join(workspace, "rust-doctor.toml");
    const before = statSync(config).mtimeMs;
    const run = await explain(["todo", "."], workspace);
    expect(run.exitCode).toBe(0);
    expect(statSync(config).mtimeMs).toBe(before);
    expect(readFileSync(config, "utf8")).toBe(TWO_MEMBER_CONFIG);
  });
});
