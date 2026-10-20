// Holds the defect ledger exact: `known-bugs.json` mirrors the table of
// `tasks/e2e-catalog/README.md`, every open entry is referenced by at least one
// `test.failing` row, and no `test.failing` row names an entry the ledger
// lacks or one already fixed (its fixing commit turns the row into a test).
import { describe, expect, test } from "bun:test";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { REPO_ROOT } from "./e2e-helpers";

type Entry = { id: string; severity: string; status: "open" | "fixed"; fixed_by?: string; defect: string };

const IDENTIFIER = /\b(?:tui|scopes|subcommands|failures|real)\/[A-Za-z0-9-]+/g;

// `tui/BUG-01 to tui/BUG-05, tui/BUG-07` expands to six identifiers; a bare
// `BUG-12` inherits the catalog of the identifier before it.
export function expandIdentifiers(cell: string): string[] {
  const ids: string[] = [];
  let catalog = "";
  for (const part of cell.split(",").map((piece) => piece.trim().replace(/`/g, ""))) {
    const ends = part.split(" to ").map((end) => {
      const slash = end.indexOf("/");
      if (slash >= 0) catalog = end.slice(0, slash);
      return slash >= 0 ? end.slice(slash + 1) : end;
    });
    if (ends.length === 1) {
      ids.push(`${catalog}/${ends[0]}`);
      continue;
    }
    const first = ends[0]!.match(/^(.*?)(\d+)$/);
    const last = ends[1]!.match(/^(.*?)(\d+)$/);
    if (!first || !last) throw new Error(`unreadable range: ${part}`);
    for (let n = Number(first[2]); n <= Number(last[2]); n += 1) {
      ids.push(`${catalog}/${first[1]}${String(n).padStart(first[2]!.length, "0")}`);
    }
  }
  return ids;
}

export function parseLedger(readme: string): Entry[] {
  const section = readme.split("## Defect ledger")[1];
  if (!section) throw new Error("no Defect ledger section");
  return section
    .split("\n")
    .filter((line) => line.startsWith("| ") && line.split("|")[1]!.includes("/"))
    .flatMap((line) => {
      const [identifier, severity, defect, story] = line.slice(1, -1).split("|").map((cell) => cell.trim());
      const open = story === "`test.failing`";
      return expandIdentifiers(identifier!).map((id): Entry => ({
        id,
        severity: severity!,
        status: open ? "open" : "fixed",
        ...(open ? {} : { fixed_by: story! }),
        defect: defect!,
      }));
    });
}

export function failingTitles(source: string): string[] {
  const call = /\b(?:test|it)\.failing(?:\.each\([\s\S]*?\)\s*)?\(\s*(["'`])((?:\\.|(?!\1).)*)\1/g;
  return [...source.matchAll(call)].map((match) => match[2]!);
}

export function ledgerViolations(entries: Entry[], titles: string[]): string[] {
  const byId = new Map(entries.map((entry) => [entry.id, entry]));
  const referenced = new Set<string>();
  const violations: string[] = [];
  for (const title of titles) {
    const ids = title.match(IDENTIFIER) ?? [];
    if (ids.length === 0) violations.push(`test.failing "${title}" names no ledger identifier`);
    for (const id of ids) {
      const entry = byId.get(id);
      if (!entry) violations.push(`test.failing "${title}" names ${id}, which the ledger lacks`);
      else if (entry.status === "fixed") violations.push(`test.failing "${title}" names ${id}, fixed by ${entry.fixed_by}`);
      else referenced.add(id);
    }
  }
  for (const entry of entries) {
    if (entry.status === "open" && !referenced.has(entry.id)) {
      violations.push(`open entry ${entry.id} is referenced by no test.failing row`);
    }
  }
  return violations;
}

const known: Entry[] = JSON.parse(readFileSync(join(import.meta.dir, "known-bugs.json"), "utf8"));
const ledger = parseLedger(readFileSync(join(REPO_ROOT, "tasks/e2e-catalog/README.md"), "utf8"));

function suiteTitles(): string[] {
  return readdirSync(import.meta.dir)
    // This file's own fixtures are strings, not defect rows.
    .filter((name) => name.endsWith(".test.ts") && name !== "known-bugs.test.ts")
    .flatMap((name) => failingTitles(readFileSync(join(import.meta.dir, name), "utf8")));
}

describe("the defect ledger", () => {
  test("known-bugs.json is the ledger of the catalog README, entry for entry", () => {
    expect(known).toEqual(ledger);
  });

  test("every open entry has a test.failing row, and every row names an open entry", () => {
    expect(ledgerViolations(known, suiteTitles())).toEqual([]);
  });

  test("a row naming an identifier the ledger lacks is a violation", () => {
    const entries: Entry[] = [{ id: "tui/BUG-01", severity: "low", status: "open", defect: "d" }];
    expect(ledgerViolations(entries, ["tui/BUG-01 header", "tui/BUG-99 ghost"])).toEqual([
      'test.failing "tui/BUG-99 ghost" names tui/BUG-99, which the ledger lacks',
    ]);
  });

  test("an open entry no row references is a violation", () => {
    const entries: Entry[] = [
      { id: "tui/BUG-01", severity: "low", status: "open", defect: "d" },
      { id: "scopes/BUG-C", severity: "low", status: "open", defect: "d" },
    ];
    expect(ledgerViolations(entries, ["tui/BUG-01 header"])).toEqual([
      "open entry scopes/BUG-C is referenced by no test.failing row",
    ]);
  });

  test("a row naming a fixed entry is a violation", () => {
    const entries: Entry[] = [{ id: "failures/BUG-01", severity: "high", status: "fixed", fixed_by: "US-001", defect: "d" }];
    expect(ledgerViolations(entries, ["failures/BUG-01 leak"])).toEqual([
      'test.failing "failures/BUG-01 leak" names failures/BUG-01, fixed by US-001',
    ]);
  });

  test("ranges and inherited catalogs expand", () => {
    expect(expandIdentifiers("tui/BUG-01 to tui/BUG-03, tui/BUG-07")).toEqual([
      "tui/BUG-01", "tui/BUG-02", "tui/BUG-03", "tui/BUG-07",
    ]);
    expect(expandIdentifiers("subcommands/F7 to F9, F11")).toEqual([
      "subcommands/F7", "subcommands/F8", "subcommands/F9", "subcommands/F11",
    ]);
    expect(failingTitles('test.failing("a/b x", () => {}); it.failing(`c`, f)')).toEqual(["a/b x", "c"]);
    expect(failingTitles('test.failing.each(["files", "lines"])(\n  "N02 scopes/BUG-C a %s scan",')).toEqual([
      "N02 scopes/BUG-C a %s scan",
    ]);
  });
});
