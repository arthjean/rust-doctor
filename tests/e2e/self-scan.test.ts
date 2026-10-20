// `tasks/e2e-catalog/real-codebases.md` section 2: this repository scanned
// through the CLI as a real codebase, invariants 1 to 4, 6, 10, 14, 17 and 18.
// The subject is a clone of HEAD, never the working tree, and no test reads
// the score, the label, a count or a finding this repository happens to have:
// the self-scan is a gate, and every assertion here is a contract it must keep.
import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { CARGO_HOME, cargoVersion, E2E_BASE, REPO_ROOT, runCommand, RUSTUP_HOME, scratchDir } from "./e2e-helpers";
import { sharedClone } from "./self-clone";
import {
  PIN_INSTALLED,
  report,
  scanSelf,
  TEST_TIMEOUT_MS,
  TOOLCHAIN_PIN,
  withoutTimings,
  type Report,
} from "./self-scan-support";

const clone = await sharedClone();
const target = clone.target("scan");

let main: Promise<{ report: Report; stderr: string; exitCode: number | null }> | undefined;

// The scan most invariants read: JSON with timings, on this file's warm target.
function mainScan() {
  main ??= scanSelf(clone, ["--json", "--timings"], { target }).then((run) => ({ report: report(run), stderr: run.stderr, exitCode: run.exitCode }));
  return main;
}

const tiers = (value: Report) => new Map(value.policy.rules.map((rule) => [rule.id, rule.tier]));

describe("this repository as a real codebase", () => {
  test("INV-01 the scan is complete, authoritative, passes its default gate and carries no P0 or P1 finding", async () => {
    const { report: scan, exitCode } = await mainScan();
    expect(exitCode).toBe(0);
    expect(scan.status).toBe("complete");
    expect(scan.complete).toBe(true);
    expect(scan.errors).toEqual([]);
    expect(scan.audit.score?.authoritative).toBe(true);
    expect(scan.audit.score?.applied_ceiling).toBeNull();
    expect(scan.gate).toMatchObject({ blocking: "error", status: "passed", blocking_diagnostics: 0 });
    expect(["P0", "P1"]).not.toContain(scan.audit.score?.worst_tier);
    // A site under tests/ is test context, which the score never ranks.
    const tier = tiers(scan);
    const severe = scan.diagnostics.filter((diagnostic) => ["P0", "P1"].includes(tier.get(diagnostic.code) ?? "")
      && !/^(tests|benches|examples)\//.test(diagnostic.path ?? ""));
    expect(severe.map((diagnostic) => `${diagnostic.code} ${diagnostic.path}`)).toEqual([]);
  }, TEST_TIMEOUT_MS);

  test("INV-02 the schema and every published version agree with the sources", async () => {
    const { report: scan } = await mainScan();
    const schema = readFileSync(join(REPO_ROOT, "src/report.rs"), "utf8").match(/pub const SCHEMA_VERSION: u8 = (\d+);/);
    expect(scan.schema_version).toBe(Number(schema?.[1]));
    const version = cargoVersion();
    const printed = await runCommand(["rust-doctor", "--version"]);
    expect(printed.stdout.trim()).toBe(`rust-doctor ${version}`);
    expect(scan.toolchain.rust_doctor).toBe(version);
    expect(JSON.parse(readFileSync(join(REPO_ROOT, "npm/rust-doctor/package.json"), "utf8")).version).toBe(version);
    const action = Bun.YAML.parse(readFileSync(join(REPO_ROOT, "action.yml"), "utf8")) as { inputs: { version: { default: string } } };
    expect(action.inputs.version.default).toBe(version);
  }, TEST_TIMEOUT_MS);

  // CI installs the pin, so there a missing one fails rather than skips.
  test.skipIf(!PIN_INSTALLED && process.env.CI !== "true")(`INV-03 under the pinned toolchain ${TOOLCHAIN_PIN} every catalogued lint is evaluated and no flag is removed`, async () => {
    const { report: scan } = await mainScan();
    const [major, minor] = TOOLCHAIN_PIN.split(".");
    expect(scan.toolchain.rustc).toStartWith(`rustc ${TOOLCHAIN_PIN} `);
    expect(scan.toolchain.cargo).toStartWith(`cargo ${TOOLCHAIN_PIN} `);
    expect(scan.toolchain.clippy).toStartWith(`clippy 0.${major}.${minor} `);
    expect(scan.toolchain.removed_lint_flags).toEqual([]);
    const unevaluated = (scan.policy.rules as Array<{ id: string; not_evaluated?: string }>)
      .filter((rule) => rule.id.startsWith("clippy::") && rule.not_evaluated !== undefined);
    expect(unevaluated).toEqual([]);
  }, TEST_TIMEOUT_MS);

  // Valid because this repository commits its lockfile: a lockless workspace
  // would have the first scan's lockfile removed, as US-005 requires, and
  // still be stable, but this one never writes it at all.
  test("INV-04 a cold scan, a warm one and a third agree once timings are removed", async () => {
    const { report: warm } = await mainScan();
    const cold = report(await scanSelf(clone, ["--json"], { target: join(scratchDir("cold-target"), "target") }));
    const third = report(await scanSelf(clone, ["--json", "--timings"], { target }));
    expect(withoutTimings(cold)).toEqual(withoutTimings(warm));
    expect(withoutTimings(third)).toEqual(withoutTimings(warm));
  }, TEST_TIMEOUT_MS);

  test("INV-06 the report names no machine path, no environment value and only tracked files", async () => {
    const { report: scan } = await mainScan();
    const text = JSON.stringify(scan);
    const forbidden = [REPO_ROOT, clone.path, clone.sandbox.home, clone.sandbox.tmp, homedir(), CARGO_HOME, RUSTUP_HOME, target, E2E_BASE];
    expect(forbidden.filter((path) => text.includes(path))).toEqual([]);
    const tracked = new Set((await clone.git(["ls-files", "-z"])).split("\0").filter(Boolean));
    const strays = scan.diagnostics
      .map((diagnostic) => diagnostic.path)
      .filter((path): path is string => path !== null)
      .filter((path) => path.startsWith("/") || path.split("/").includes("..") || !(tracked.has(path) || path === "Cargo.lock"));
    expect(strays).toEqual([]);
  }, TEST_TIMEOUT_MS);

  test("INV-10 the crate passes its own production rules through the CLI", async () => {
    const { report: scan } = await mainScan();
    const forbidden = ["unwrap_used", "expect_used", "panic", "dbg_macro", "todo", "unimplemented"].map((name) => `clippy::${name}`);
    const structural = ["rust_doctor::structure::oversized_unit", "rust_doctor::structure::complex_function"];
    const hits = scan.diagnostics.filter((diagnostic) => forbidden.includes(diagnostic.code)
      || (structural.includes(diagnostic.code) && (diagnostic.path ?? "").startsWith("src/")));
    expect(hits.map((diagnostic) => `${diagnostic.code} ${diagnostic.path}`)).toEqual([]);
  }, TEST_TIMEOUT_MS);

  // A single-member workspace scanned as its one package judges the same
  // code. The repository pass belongs to the whole repository, so a package
  // scan reports its five rules as `package-scoped` and its score loses
  // authority for it: those are the only differences allowed.
  test("INV-14 --package on the single member judges what the full scan judges", async () => {
    const { report: full } = await mainScan();
    const member = report(await scanSelf(clone, ["--json", "--package", "rust-doctor"], { target }));
    expect(member.diagnostics).toEqual(full.diagnostics);
    expect(member.summary).toEqual(full.summary);
    expect(member.audit.categories).toEqual(full.audit.categories);
    expect(member.gate).toEqual(full.gate);
    expect(member.status).toBe(full.status);
    expect(member.audit.score?.value).toBe(full.audit.score!.value);
    const notEvaluated = (value: Report) => new Map((value.policy.rules as Array<{ id: string; not_evaluated?: string }>).map((rule) => [rule.id, rule.not_evaluated]));
    const before = notEvaluated(full);
    const changed = [...notEvaluated(member)].filter(([id, reason]) => before.get(id) !== reason);
    expect(changed.length).toBeGreaterThan(0);
    expect(changed.every(([id, reason]) => id.startsWith("rust_doctor::repo::") && reason === "package-scoped")).toBe(true);
  }, TEST_TIMEOUT_MS);

  test("INV-17 a JSON run in a sandboxed HOME without agent markers prints nothing on stderr", async () => {
    const { stderr } = await mainScan();
    expect(stderr).toBe("");
  }, TEST_TIMEOUT_MS);

  test("INV-18 --timings publishes the documented phases, none longer than the total", async () => {
    const { report: scan } = await mainScan();
    const source = readFileSync(join(REPO_ROOT, "src/timings.rs"), "utf8");
    const phases = [...source.matchAll(/pub (\w+): Option<u64>/g)].map((match) => match[1]!);
    expect(phases.length).toBeGreaterThan(0);
    const timings = scan.timings ?? {};
    expect(Object.keys(timings).sort()).toEqual([...phases, "total"].sort());
    for (const value of Object.values(timings)) expect(Number.isSafeInteger(value) && value >= 0).toBe(true);
    expect(Math.max(...phases.map((phase) => timings[phase]!))).toBeLessThanOrEqual(timings.total!);
  }, TEST_TIMEOUT_MS);
});
