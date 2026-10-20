// Holds the suite to its catalogs: every row of a catalog section the suite
// claims is automated exactly once, by a test title that starts with the row
// ID or by a `test.each` table row whose first cell is the ID. A section is
// claimed here when the story that automates it lands.
import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { REPO_ROOT } from "./e2e-helpers";

type Claim = { catalog: string; sections: RegExp; files: string[]; rows?: (markdown: string, sections: RegExp) => string[] };

const CLAIMS: Claim[] = [
  {
    catalog: "failures.md",
    sections: /^## (EX|AR|OW|OC):/,
    files: ["exit-codes.test.ts", "output-streams.test.ts"],
  },
  {
    catalog: "failures.md",
    sections: /^## (SG|TC|WS|AD|EN|PV|NW|FS|PG):/,
    files: [
      "signals-and-deadlines.test.ts", "toolchain-preflight.test.ts", "workspace-shapes.test.ts",
      "advisory-database.test.ts", "terminal-profile.test.ts", "privacy-redaction.test.ts",
      "offline-and-side-effects.test.ts", "poisoned-repository.test.ts",
    ],
  },
  {
    catalog: "tui.md",
    sections: /^### 3\.\d+ /,
    files: [
      "tui-gating.test.ts", "tui-startup.test.ts", "tui-keybindings.test.ts", "tui-exit-codes.test.ts",
      "tui-issues-viewer.test.ts", "tui-layout.test.ts", "tui-resize.test.ts", "tui-terminal-hygiene.test.ts",
      "tui-clipboard.test.ts", "tui-ci-workflow.test.ts", "tui-handoff.test.ts", "tui-interrupt-recovery.test.ts",
      "tui-performance.test.ts",
    ],
  },
  {
    catalog: "real-codebases.md",
    sections: /^## 2\. /,
    rows: invariantRows,
    files: [
      "self-scan.test.ts", "self-scan-parity.test.ts", "self-scan-isolation.test.ts", "scopes-real.test.ts",
      "deadline-and-interrupt.test.ts", "rules-catalog.test.ts",
    ],
  },
  {
    catalog: "scopes.md",
    sections: /^### R\. /,
    files: ["scopes-real.test.ts"],
  },
  {
    catalog: "scopes.md",
    sections: /^### (A|A2|C|D|S|E|G|K)\. /,
    files: [
      "scope-arguments.test.ts", "scope-base-resolution.test.ts", "scope-worktree-states.test.ts",
      "scope-staged.test.ts", "scope-baseline-delta.test.ts", "scope-workspace-placement.test.ts",
      "scope-repo-integrity.test.ts",
    ],
  },
  {
    catalog: "subcommands.md",
    sections: /^## ([1-9]|1[0-2])\. /,
    files: [
      "cli-surface.test.ts", "rules.test.ts", "why.test.ts", "report-markdown.test.ts", "hook-git.test.ts",
      "hook-agent-install.test.ts", "hook-run.test.ts", "skill-install.test.ts", "setup.test.ts",
      "ci-install.test.ts", "config.test.ts", "workflows.test.ts",
    ],
  },
];

// The row IDs of the claimed sections: the first cell of every table row
// under a matching heading, until the next heading of any level.
export function catalogRows(markdown: string, sections: RegExp): string[] {
  const rows: string[] = [];
  let claimed = false;
  for (const line of markdown.split("\n")) {
    if (line.startsWith("#")) claimed = sections.test(line);
    if (!claimed || !line.startsWith("| ")) continue;
    const id = line.split("|")[1]!.trim();
    if (/^[A-Z][A-Za-z0-9-]*\d[A-Za-z0-9-]*$/.test(id)) rows.push(id);
  }
  return rows;
}

// The self-scan invariants are a numbered list, not a table: item `n` of a
// claimed section is row `INV-0n`.
export function invariantRows(markdown: string, sections: RegExp): string[] {
  const rows: string[] = [];
  let claimed = false;
  for (const line of markdown.split("\n")) {
    if (line.startsWith("#")) claimed = sections.test(line);
    const item = claimed ? line.match(/^(\d+)\. /) : null;
    if (item) rows.push(`INV-${item[1]!.padStart(2, "0")}`);
  }
  return rows;
}

function escape(id: string): string {
  return id.replace(/[-]/g, "\\-");
}

// How many times `source` automates `id`: a title opening a test call, or a
// table row opening with the ID. A commented-out line, a skipped test and a
// todo automate nothing.
export function occurrences(source: string, id: string): number {
  source = source.replace(/^\s*\/\/.*$/gm, "");
  const title = new RegExp(`(?:\\b(?:test|it|failing|only)|\\))\\(\\s*(["'\`])${escape(id)}[\\s:]`, "g");
  const row = new RegExp(`\\[\\s*(["'\`])${escape(id)}\\1\\s*,`, "g");
  return (source.match(title)?.length ?? 0) + (source.match(row)?.length ?? 0);
}

export function coverageViolations(rows: string[], sources: string[]): string[] {
  const violations: string[] = [];
  for (const id of new Set(rows)) {
    const count = sources.reduce((sum, source) => sum + occurrences(source, id), 0);
    if (count !== 1) violations.push(`${id} is automated ${count} times`);
  }
  return violations;
}

describe("catalog coverage", () => {
  test.each(CLAIMS.map((claim) => [`${claim.catalog} ${claim.sections.source}`, claim] as const))("%s: every claimed row is automated once", (_name, claim) => {
    const rows = (claim.rows ?? catalogRows)(readFileSync(join(REPO_ROOT, "tasks/e2e-catalog", claim.catalog), "utf8"), claim.sections);
    expect(rows.length).toBeGreaterThan(0);
    const sources = claim.files.map((file) => readFileSync(join(import.meta.dir, file), "utf8"));
    expect(coverageViolations(rows, sources)).toEqual([]);
  });

  test("titles, failing titles and table rows count; a longer ID, a scenario name, a comment, a skip or a todo does not", () => {
    const source = [
      'test("C01 a title", f);',
      'test.failing.each(["files"])(\n  "C02 scopes/BUG-G a %s scan", f);',
      'const rows = [\n  ["C03", "x"],\n];',
      'gitScenario("C04"); test("C05b longer", f);',
      '// test("C06 commented out", f);',
      'test.skip("C07 skipped", f); test.todo("C08 later");',
    ].join("\n");
    expect(["C01", "C02", "C03", "C04", "C05", "C06", "C07", "C08"].map((id) => occurrences(source, id))).toEqual([1, 1, 1, 0, 0, 0, 0, 0]);
    expect(catalogRows("### A. args\n| ID | x |\n| A01 | y |\n### R. real\n| R01 | z |\n", /^### A\. /)).toEqual(["A01"]);
    expect(invariantRows("## 2. Self\n1. Exit 0\n12. Offline\n## 3. Next\n4. Not claimed\n", /^## 2\. /)).toEqual(["INV-01", "INV-12"]);
  });
});
