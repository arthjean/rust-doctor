// `tasks/e2e-catalog/real-codebases.md` section 2, invariants 8, 9 and 19:
// the human report, the saved report rendered as Markdown and the JSON of the
// same scan of this repository say the same thing, and the advisory database
// a scan reads is the one it was told to read. Every number compared here is
// read from the JSON, never written in the test.
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

// The audit category a diagnostic is tallied under, as `category_mapping` buckets it.
function bucket(category: string | null): string {
  switch (category) {
    case "security": return "Security";
    case "correctness": case "reliability": return "Bugs";
    case "performance": return "Performance";
    case "cargo": case "dependencies": return "Dependencies";
    case "maintainability": return "Maintainability";
    default: return "Other";
  }
}

// A category line counts what weighs and sets the rest apart (EP-002 US-010
// of the self-scan remediation), so it is rebuilt from the diagnostics.
function categoryLine(scan: Report, name: string): string {
  const counts = { error: 0, warning: 0, info: 0, unknown: 0 } as Record<string, number>;
  let outside = 0;
  let notScored = 0;
  for (const diagnostic of scan.diagnostics.filter((diagnostic) => bucket(diagnostic.category) === name)) {
    if (diagnostic.context) outside += diagnostic.occurrences;
    else if (diagnostic.unscored) notScored += diagnostic.occurrences;
    else counts[diagnostic.severity] = (counts[diagnostic.severity] ?? 0) + diagnostic.occurrences;
  }
  const apart = [[outside, "outside production code"], [notScored, "not scored"]]
    .filter(([count]) => (count as number) > 0)
    .map(([count, what]) => `+${count} ${what}`);
  const suffix = apart.length > 0 ? ` (${apart.join(", ")})` : "";
  return `${name}: ${counts.error} errors, ${counts.warning} warnings, ${counts.info} info, ${counts.unknown} unknown (occurrences)${suffix}`;
}

describe("this repository: human, Markdown and JSON parity", () => {
  test("INV-08 the human report states the counts, score and share link of the JSON, and --verbose lists every finding", async () => {
    const scan = await jsonScan();
    const human = await scanSelf(clone, [], { target });
    expect(human.exitCode).toBe(0);
    const out = human.stdout;
    // The report wraps at 80 columns, on spaces, so a long sentence is read
    // back with its rows joined.
    const flat = out.replace(/\n/g, " ");
    expect(flat).toContain(`Scored ${scan.audit.source_files} production files (${scan.audit.production_lines} lines) in `);
    expect(flat).toContain("Clippy linted library and binary targets, native rules also read tests");
    expect(out).toContain(`All ${scan.summary.occurrences.total} occurrences across ${scan.summary.total} findings`);
    for (const category of scan.audit.categories) {
      expect(flat).toContain(categoryLine(scan, category.name));
    }
    const score = scan.audit.score!;
    expect(out).toContain(`${score.value} / 100 ${score.label}`);
    // A ceiling the dimensions already score under is named, never as the value.
    const ceiling = score.applied_ceiling;
    expect(out.includes("Capped at")).toBe(ceiling !== null && score.value >= ceiling);
    expect(out.includes("caps the score at")).toBe(ceiling !== null && score.value < ceiling);
    const share = out.match(/https:\/\/rust-doctor\.com\/share\?\S+/);
    expect(share).not.toBeNull();
    const params = new URL(share![0]).searchParams;
    expect(Object.fromEntries(params)).toEqual({
      s: String(score.value),
      m: score.model,
      w: String(scan.summary.occurrences.warnings),
      i: String(scan.summary.occurrences.info),
      f: String(scan.audit.source_files),
      l: String(scan.audit.production_lines),
    });

    // A scored finding is listed at its location; a rule whose sites all lie
    // outside production code is summed on one `Unscored:` line instead.
    const text = (await scanSelf(clone, ["--verbose"], { target })).stdout;
    const verbose = lines(text);
    const unscored = new Set([...text.matchAll(/Unscored: [^(]*\(\d+ occurrences? outside production code,\s+([\w:]+)\)/g)].map((match) => match[1]!));
    const missing = scan.diagnostics
      .filter((diagnostic) => diagnostic.span !== null && !unscored.has(diagnostic.code))
      .map((diagnostic) => `${diagnostic.path}:${diagnostic.span!.line_start}:${diagnostic.span!.column_start}`)
      .filter((location) => !verbose.includes(location));
    expect(missing).toEqual([]);
    const rules = [...new Set(scan.diagnostics.map((diagnostic) => diagnostic.code))]
      .filter((code) => !verbose.includes(`Rule ID: ${code}`) && !unscored.has(code));
    expect(rules).toEqual([]);
  }, TEST_TIMEOUT_MS);

  // No cargo on PATH: the renderer reads the file and scans nothing.
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
        // The table lists the default 20 findings of a longer report.
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
