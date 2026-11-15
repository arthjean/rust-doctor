import { describe, expect, test } from "bun:test";
import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { FIXTURES, pathWithout, runCommand } from "./e2e-helpers";
import { sharedClone } from "./self-clone";
import { report, scanSelf, TEST_TIMEOUT_MS, withoutTimings, type Report } from "./self-scan-support";

const clone = await sharedClone();
const target = clone.target("scan");

let json: Promise<Report> | undefined;
const jsonScan = () => (json ??= scanSelf(clone, ["--json"], { target }).then(report));

let baseline: Promise<Report> | undefined;
const baselineScan = async () => (baseline ??= scanSelf(clone, ["--json", "--scope", "baseline", "--base", await clone.sha()], { target }).then(report));

function lines(text: string): string[] {
  return text.split("\n");
}

function bucket(category: string | null): string {
  switch (category) {
    case "security": return "Security";
    case "correctness": case "reliability": return "Reliability";
    case "performance": return "Performance";
    case "cargo": case "dependencies": return "Dependencies";
    case "maintainability": return "Maintainability";
    default: return "Other";
  }
}

const CONTEXTS: Array<[string, string]> = [
  ["tests", "in tests"], ["benchmark", "in benches"], ["example", "in examples"], ["build-script", "in build scripts"],
];

function ruleTitle(id: string): string {
  const words = id.split("::").pop()!.replace(/[_-]/g, " ");
  return words.charAt(0).toUpperCase() + words.slice(1);
}

function label(name: string): string {
  return name === "Bugs" ? "Reliability" : name;
}

function categoryLine(scan: Report, name: string): string {
  const counts = { error: 0, warning: 0, info: 0, unknown: 0 } as Record<string, number>;
  const outside = new Map<string, number>();
  const byRule = new Map<string, number>();
  let notScored = 0;
  for (const diagnostic of scan.diagnostics.filter((diagnostic) => bucket(diagnostic.category) === label(name))) {
    if (diagnostic.context) outside.set(diagnostic.context, (outside.get(diagnostic.context) ?? 0) + diagnostic.occurrences);
    else if (diagnostic.unscored) notScored += diagnostic.occurrences;
    else {
      counts[diagnostic.severity] = (counts[diagnostic.severity] ?? 0) + diagnostic.occurrences;
      if (diagnostic.code) byRule.set(diagnostic.code, (byRule.get(diagnostic.code) ?? 0) + diagnostic.occurrences);
    }
  }
  const named = ([["error", "error", "errors"], ["warning", "warning", "warnings"], ["info", "info", "info"], ["unknown", "unknown", "unknown"]] as const)
    .filter(([key]) => counts[key]! > 0)
    .map(([key, one, many]) => `${counts[key]} ${counts[key] === 1 ? one : many}`);
  const total = Object.values(counts).reduce((sum, count) => sum + count, 0);
  const [largest] = [...byRule].sort(([left, a], [right, b]) => b - a || (left < right ? -1 : 1));
  const notes = [
    largest && largest[1] * 2 > total ? `${largest[1]} from ${ruleTitle(largest[0])}` : null,
    [...CONTEXTS.map(([context, kind]) => [outside.get(context) ?? 0, kind] as const), [notScored, "not scored"] as const]
      .filter(([count]) => count > 0)
      .map(([count, kind]) => `+${count} ${kind}`)
      .join(", ") || null,
  ].filter((note) => note !== null);
  const suffix = notes.length > 0 ? ` (${notes.join("; ")})` : "";
  return `${label(name)}: ${named.length > 0 ? named.join(", ") : "0 findings"}${suffix}`;
}

describe("this repository: human, Markdown and JSON parity", () => {
  test("INV-08 the human report states the counts, score and share link of the JSON, and --verbose lists every finding", async () => {
    const scan = await jsonScan();
    const human = await scanSelf(clone, [], { target });
    expect(human.exitCode).toBe(0);
    const out = human.stdout;
    const flat = out.replace(/\n/g, " ");
    expect(flat).toContain(`Scored ${scan.audit.source_files} production files (${scan.audit.production_lines} lines) in `);
    expect(flat).toMatch(/Clippy linted library and binary targets for [\w.-]+; code gated to other platforms and test code were not linted\. Native rules also read tests\./);
    expect(out).toContain(`All ${scan.summary.occurrences.total} occurrences across ${scan.summary.total} findings`);
    for (const category of scan.audit.categories) {
      expect(flat).toContain(categoryLine(scan, category.name));
    }
    const score = scan.audit.score!;
    expect(out).toContain(`${score.value} / 100 ${score.label}`);
    const ceiling = score.applied_ceiling;
    expect(out.includes("The score is capped at")).toBe(ceiling !== null && score.value >= ceiling);
    expect(out.includes("already hold it lower")).toBe(false);
    const share = out.match(/https:\/\/rust-doctor\.com\/share\?\S+/);
    expect(share).not.toBeNull();
    const params = new URL(share![0]).searchParams;
    const production = (key: "errors" | "warnings" | "info") =>
      scan.audit.categories.reduce((sum, category) => sum + category.production[key], 0);
    const scored = (count: number, noun: string) => `${count} scored ${noun}${count === 1 ? "" : "s"}`;
    const errors = production("errors");
    expect(flat).toContain(`${errors > 0 ? `${scored(errors, "error")}, ` : ""}${scored(production("warnings"), "warning")}, ${production("info")} info`);
    expect(Object.fromEntries(params)).toEqual({
      s: String(score.value),
      m: score.model,
      w: String(production("warnings")),
      i: String(production("info")),
      f: String(scan.audit.source_files),
      l: String(scan.audit.production_lines),
    });

    const text = (await scanSelf(clone, ["--verbose"], { target })).stdout;
    const verbose = lines(text);
    const unscored = new Set([...text.matchAll(/ {2}x0 \(\+[^)]*\)\s+([\w:]+)$/gm)].map((match) => match[1]!));
    const missing = scan.diagnostics
      .filter((diagnostic) => diagnostic.span !== null && !unscored.has(diagnostic.code))
      .map((diagnostic) => `${diagnostic.path}:${diagnostic.span!.line_start}:${diagnostic.span!.column_start}`)
      .filter((location) => !verbose.includes(location));
    expect(missing).toEqual([]);
    const rules = [...new Set(scan.diagnostics.map((diagnostic) => diagnostic.code))]
      .filter((code) => !verbose.includes(`Rule ID: ${code}`) && !unscored.has(code));
    expect(rules).toEqual([]);
  }, TEST_TIMEOUT_MS);

  test("INV-09 report markdown renders the saved report's score, counts and delta without a toolchain", async () => {
    const markdown = async (value: Report) => {
      const file = join(clone.sandbox.dir, `report-${value.scope?.mode}.json`);
      writeFileSync(file, JSON.stringify(value));
      const run = await runCommand(["rust-doctor", "report", "markdown", file], { sandbox: clone.sandbox, env: { PATH: pathWithout("cargo") } });
      expect(run.exitCode).toBe(0);
      return run.stdout;
    };
    for (const value of [await jsonScan(), await baselineScan()]) {
      const out = await markdown(value);
      const score = value.audit.score!;
      expect(out).toContain(`**Score: ${score.value}/100 (${score.label})**`);
      if (value.delta) {
        expect(out).toContain(`**Against the base:** ${value.delta.summary.introduced} introduced, ${value.delta.summary.fixed} fixed.`);
      } else {
        expect(out).not.toContain("Against the base");
        if (value.summary.total > 20) expect(out).toContain(`Showing 20 of ${value.summary.total} findings.`);
      }
    }
    const live = await scanSelf(clone, ["--scope", "baseline", "--base", await clone.sha()], { target });
    const delta = (await baselineScan()).delta!.summary;
    expect(live.stdout).toContain(`Delta: +${delta.introduced} introduced; =${delta.pre_existing} pre-existing; -${delta.fixed} fixed`);
  }, TEST_TIMEOUT_MS);

  test("INV-19 the advisory database is the one named: off with /dev/null, read and dated from a fixture", async () => {
    const off = await scanSelf(clone, [], { target });
    expect(off.stdout).not.toMatch(/days old/);
    const database = join(FIXTURES, "advisory-db");
    const named = await scanSelf(clone, ["--advisory-db", database], { target });
    expect(named.stdout).toMatch(/The advisory database is \d+ days old\./);
    const first = report(await scanSelf(clone, ["--json", "--advisory-db", database], { target }));
    const second = report(await scanSelf(clone, ["--json", "--advisory-db", database], { target }));
    expect(withoutTimings(second)).toEqual(withoutTimings(first));
    expect(first.scan.advisory_db).toMatchObject({ source: "explicit" });
    expect(JSON.stringify(first)).not.toContain(database);
    expect((await jsonScan()).scan.advisory_db).toBeUndefined();
  }, TEST_TIMEOUT_MS);
});
