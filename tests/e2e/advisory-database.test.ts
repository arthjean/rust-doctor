import { describe, expect, test } from "bun:test";
import { cpSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import {
  copyWorkspace,
  FIXTURES,
  isEmptyDir,
  normalizeReport,
  runDoctor,
  sandbox,
  scratchDir,
  treeFingerprint,
  type DoctorOptions,
  type DoctorRun,
} from "./e2e-helpers";
import { requireNetns, runDoctorOffline } from "./netns";

type Database = { source: string; date: string | null; advisories: number; skipped: number };
type Report = {
  status: string;
  scan: { advisory_db?: Database };
  audit: { score: { authoritative: boolean } | null };
  policy: { rules: Array<{ id: string; not_evaluated?: string | null }> };
  diagnostics: Array<{ code: string | null }>;
  errors: unknown[];
};

const DATABASE = join(FIXTURES, "advisory-db");
const ADVISORY_RULES = ["rust_doctor::cargo::unmaintained_dependency", "rust_doctor::cargo::vulnerable_dependency"];
const MISSING =
  "Known-vulnerability check skipped: no RustSec database under ~/.cargo. Run `cargo deny fetch` or `cargo audit` once, or pass --advisory-db <path>.";
const UNCHECKED = "Security and Dependencies were scored without the known-vulnerability check";
const UNREADABLE =
  "could not be read: 3 of 4 files were skipped. The known-vulnerability check did not run.";
const FIXTURE_DATABASE: Database = { source: "explicit", date: "2020-01-20", advisories: 38, skipped: 1 };
const CRLF_DATABASE = join(DATABASE, "crlf");

let shared: string | undefined;

function workspace(): string {
  shared ??= copyWorkspace("cargo-health/offline-registry");
  return shared;
}

async function scan(args: string[], options: DoctorOptions = {}): Promise<DoctorRun & { report: Report }> {
  const before = treeFingerprint(workspace());
  const run = await runDoctor([workspace(), "--yes", ...args], options);
  expect(treeFingerprint(workspace())).toBe(before);
  expect(run.exitCode).toBe(0);
  const report = (args.includes("--json") ? run.json : undefined) as Report;
  if (report) {
    expect(report.status).toBe("complete");
    expect(report.errors).toEqual([]);
  }
  return { ...run, report };
}

function flat(text: string): string {
  return text.split(/\s+/).join(" ");
}

function unreadableDatabase(at: string): void {
  const crates = join(at, "crates", "fixture-registry-dep");
  mkdirSync(crates, { recursive: true });
  cpSync(join(DATABASE, "crates", "fixture-registry-dep", "RUSTSEC-2020-9001.md"), join(crates, "RUSTSEC-2020-9001.md"));
  for (const number of [1, 2, 3]) {
    writeFileSync(join(crates, `RUSTSEC-2020-950${number}.md`), "```toml\n[advisory]\nid = = broken\n```\n");
  }
}

function rustsecClone(at: string): void {
  cpSync(DATABASE, at, { recursive: true });
  markRustsec(at);
}

function markRustsec(at: string): void {
  mkdirSync(join(at, ".git"));
  writeFileSync(join(at, ".git", "HEAD"), "ref: refs/heads/main\n");
  writeFileSync(join(at, ".git", "config"), '[remote "origin"]\n\turl = https://github.com/rustsec/advisory-db\n');
}

function cargoHome(at?: string): string {
  const home = join(scratchDir("cargo-home"), "cargo");
  mkdirSync(home);
  if (at) rustsecClone(join(home, at));
  return home;
}

function notEvaluated(report: Report): Array<string | null | undefined> {
  return ADVISORY_RULES.map((id) => report.policy.rules.find((rule) => rule.id === id)?.not_evaluated);
}

function advisoryFindings(report: Report): string[] {
  return report.diagnostics.map((diagnostic) => diagnostic.code ?? "").filter((code) => ADVISORY_RULES.includes(code)).sort();
}

const DISCOVERED: Array<[id: string, at: string, source: string]> = [
  ["AD-05", "advisory-db", "cargo-audit"],
  ["AD-06", "advisory-dbs/github.com-abc", "cargo-deny"],
];

describe("advisory database", () => {
  test("AD-01 without a database both advisory rules are not evaluated and the score keeps its authority", async () => {
    const env = { CARGO_HOME: cargoHome() };
    const { report } = await scan(["--json"], { advisoryDb: false, env });
    expect(report.scan.advisory_db).toBeUndefined();
    expect(notEvaluated(report)).toEqual(["advisory-db-missing", "advisory-db-missing"]);
    expect(advisoryFindings(report)).toEqual([]);
    expect(report.audit.score?.authoritative).toBe(true);
    expect(flat((await scan([], { advisoryDb: false, env })).stdout)).toContain(MISSING);
  });

  test("AD-02 a named database reports its advisories, its date and its counts, never its path", async () => {
    const { report, stdout } = await scan(["--json", "--advisory-db", DATABASE], { env: { CARGO_HOME: cargoHome() } });
    expect(report.scan.advisory_db).toEqual(FIXTURE_DATABASE);
    expect(notEvaluated(report)).toEqual([undefined, undefined]);
    expect(advisoryFindings(report)).toEqual([
      "rust_doctor::cargo::unmaintained_dependency",
      "rust_doctor::cargo::vulnerable_dependency",
      "rust_doctor::cargo::vulnerable_dependency",
    ]);
    expect(stdout).not.toContain(DATABASE);
  });

  test("AD-03 /dev/null is the off switch: the report equals a scan with no database", async () => {
    const env = { CARGO_HOME: cargoHome() };
    const off = await scan(["--json", "--advisory-db", "/dev/null"], { env });
    const none = await scan(["--json"], { advisoryDb: false, env });
    expect(normalizeReport(off.report)).toEqual(normalizeReport(none.report));
    expect(off.report.scan.advisory_db).toBeUndefined();
  });

  test("AD-04 a database path that does not exist is refused with exit 2 before any work", async () => {
    const box = sandbox("typo");
    const typo = join(box.dir, "advisory-dbb");
    for (const args of [["--json"], []]) {
      const target = scratchDir("typo-target");
      const before = treeFingerprint(workspace());
      const run = await runDoctor([workspace(), "--yes", ...args, "--advisory-db", typo], { sandbox: box, target });
      expect(run.exitCode).toBe(2);
      expect(run.stdout).toBe("");
      expect(run.stderr).toBe(`rust-doctor: --advisory-db: ${typo} does not exist.\n`);
      expect(isEmptyDir(target)).toBe(true);
      expect(treeFingerprint(workspace())).toBe(before);
    }
  });

  test.each(DISCOVERED)("%s a database under CARGO_HOME/%s is discovered as %s", async (_id, at, source) => {
    const { report } = await scan(["--json"], { advisoryDb: false, env: { CARGO_HOME: cargoHome(at) } });
    expect(report.scan.advisory_db).toEqual({ ...FIXTURE_DATABASE, source });
    expect(advisoryFindings(report)).toHaveLength(3);
  });

  test("AD-07 an empty CARGO_HOME falls back to HOME/.cargo", async () => {
    const box = sandbox("home-fallback");
    rustsecClone(join(box.home, ".cargo", "advisory-db"));
    const { report } = await scan(["--json"], { advisoryDb: false, sandbox: box, env: { CARGO_HOME: "" } });
    expect(report.scan.advisory_db).toEqual({ ...FIXTURE_DATABASE, source: "cargo-audit" });
  });

  test("AD-08 a stale database is read and the linear report states its age", async () => {
    const { stdout } = await scan(["--advisory-db", DATABASE], { env: { CARGO_HOME: cargoHome() } });
    expect(stdout).toMatch(/^The advisory database is \d+ days old\.$/m);
    expect(stdout).toContain("Vulnerable dependency");
  });

  test("AD-09 a malformed advisory file is counted as skipped, never an error", async () => {
    const database = join(scratchDir("malformed-db"), "db");
    cpSync(DATABASE, database, { recursive: true });
    mkdirSync(join(database, "crates", "x"));
    writeFileSync(join(database, "crates", "x", "RUSTSEC-2020-9999.md"), "```toml\nthis is = = not toml\n```\n# broken\n");
    const { report } = await scan(["--json", "--advisory-db", database], { env: { CARGO_HOME: cargoHome() } });
    expect(report.scan.advisory_db).toEqual({ ...FIXTURE_DATABASE, skipped: FIXTURE_DATABASE.skipped + 1 });
    expect(advisoryFindings(report)).toHaveLength(3);
  });

  test.skipIf(!requireNetns())("AD-10 a scan with a database inside a network namespace equals the networked one", async () => {
    const env = { CARGO_HOME: cargoHome() };
    const { report } = await scan(["--json", "--advisory-db", DATABASE], { env });
    const before = treeFingerprint(workspace());
    const offline = await runDoctorOffline([workspace(), "--yes", "--json", "--advisory-db", DATABASE], { env });
    expect(treeFingerprint(workspace())).toBe(before);
    expect(offline.exitCode).toBe(0);
    const isolated = offline.json as Report;
    expect(isolated.scan.advisory_db).toEqual(report.scan.advisory_db);
    expect(normalizeReport(isolated)).toEqual(normalizeReport(report));
  });

  test("AD-11 a CRLF checkout of the database reads every advisory", async () => {
    const sample = readFileSync(join(CRLF_DATABASE, "crates", "fixture-registry-dep", "RUSTSEC-2021-9201.md"), "utf8");
    expect(sample).toContain("\r\n");
    const { report } = await scan(["--json", "--advisory-db", CRLF_DATABASE], { env: { CARGO_HOME: cargoHome() } });
    expect(report.scan.advisory_db).toEqual({ source: "explicit", date: "2021-03-06", advisories: 6, skipped: 0 });
    expect(notEvaluated(report)).toEqual([undefined, undefined]);
    expect(advisoryFindings(report)).toEqual([
      "rust_doctor::cargo::unmaintained_dependency",
      "rust_doctor::cargo::vulnerable_dependency",
      "rust_doctor::cargo::vulnerable_dependency",
    ]);
  });

  test("AD-12 a database that cannot be read fails the dependency stage and the score loses its authority", async () => {
    const database = join(scratchDir("unreadable-db"), "db");
    unreadableDatabase(database);
    const before = treeFingerprint(workspace());
    const run = await runDoctor([workspace(), "--yes", "--json", "--advisory-db", database], { env: { CARGO_HOME: cargoHome() } });
    expect(treeFingerprint(workspace())).toBe(before);
    expect(run.exitCode).toBe(1);
    const report = run.json as Report & { complete: boolean };
    expect(report.status).toBe("incomplete");
    expect(report.complete).toBe(false);
    expect(report.errors).toEqual([
      { stage: "dependencies", code: "advisory-db-unreadable", message: `The RustSec database ${UNREADABLE}` },
    ]);
    expect(report.audit.score?.authoritative).toBe(false);
    expect(notEvaluated(report)).toEqual(["advisory-db-missing", "advisory-db-missing"]);
    expect(advisoryFindings(report)).toEqual([]);
    expect(report.scan.advisory_db).toBeUndefined();
    expect(run.stdout).not.toContain(database);
  });

  test("AD-13 the linear report names an unreadable database by its path from the home directory", async () => {
    const box = sandbox("unreadable-home");
    const clone = join(box.home, ".cargo", "advisory-db");
    unreadableDatabase(clone);
    markRustsec(clone);
    const run = await runDoctor([workspace(), "--yes"], { advisoryDb: false, sandbox: box, env: { CARGO_HOME: "" } });
    expect(run.exitCode).toBe(1);
    const said = flat(run.stdout);
    expect(said).toContain(
      `Scan incomplete: The RustSec database at ~/.cargo/advisory-db ${UNREADABLE} (dependencies/advisory-db-unreadable)`,
    );
    expect(said).not.toContain("Known-vulnerability check skipped");
    expect(said).not.toContain(UNCHECKED);
    expect(run.stdout).not.toContain(box.home);
  });

  test("AD-14 the score block says when no advisory was checked, and names the flag when the flag is why", async () => {
    const env = { CARGO_HOME: cargoHome() };
    const missing = flat((await scan([], { advisoryDb: false, env })).stdout);
    expect(missing).toMatch(/ · Dependencies \d+ Security and Dependencies were scored without the known-vulnerability check\. /);
    const off = flat((await scan(["--advisory-db", "/dev/null"], { env })).stdout);
    expect(off).toContain(`${UNCHECKED} (--advisory-db /dev/null).`);
    expect(off).toContain("Known-vulnerability check skipped: --advisory-db /dev/null turns it off.");
    expect(off).not.toContain("no RustSec database");
    const consulted = flat((await scan(["--advisory-db", DATABASE], { env })).stdout);
    expect(consulted).not.toContain("known-vulnerability");
    expect(consulted).toContain("1 advisory file could not be read.");
  });
});
