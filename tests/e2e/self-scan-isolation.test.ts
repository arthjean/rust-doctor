// `tasks/e2e-catalog/real-codebases.md` section 2, invariants 5, 11 and 12:
// a scan of this repository depends on nothing but its tracked tree. Two
// clones in two places agree, ignored files change nothing, a scan writes
// nothing into the repository, and a scan without a network agrees with one
// that has it.
import { describe, expect, test } from "bun:test";
import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { requireNetns, runDoctorOffline } from "./netns";
import { cloneThisRepository, sharedClone } from "./self-clone";
import {
  report,
  repositoryState,
  scanArgs,
  scanSelf,
  SCAN_TIMEOUT_MS,
  selfEnv,
  TEST_TIMEOUT_MS,
  withoutTimings,
  type Report,
} from "./self-scan-support";

const clone = await sharedClone();
const target = clone.target("scan");

// A Rust file under `dir`, which this repository's .gitignore excludes.
function plantIgnored(root: string, dir: string): void {
  mkdirSync(join(root, dir), { recursive: true });
  writeFileSync(join(root, dir, "junk.rs"), "pub fn junk() -> u8 {\n    todo!()\n}\n");
}

let first: Promise<Report> | undefined;
const fullScan = () => (first ??= scanSelf(clone, ["--json"], { target }).then(report));

describe("this repository: isolation", () => {
  test("INV-05 a clone elsewhere, holding an ignored build directory, scans as the first clone does", async () => {
    const reference = await fullScan();
    const elsewhere = await cloneThisRepository("elsewhere");
    plantIgnored(elsewhere.path, "target/debug");
    expect(await elsewhere.git(["status", "--porcelain"])).toBe("");
    const moved = report(await scanSelf(elsewhere, ["--json"], { target }));
    expect(withoutTimings(moved)).toEqual(withoutTimings(reference));
  }, TEST_TIMEOUT_MS);

  // The orphan walk lists the directories under a target's root without
  // asking git, so a Rust file .gitignore excludes, here under the suite's
  // own dependencies, is reported as a module nothing declares.
  test.failing("real/B4 an ignored Rust file beside the module tree changes nothing in the report (INV-05)", async () => {
    const reference = await fullScan();
    const elsewhere = await cloneThisRepository("ignored-rust");
    plantIgnored(elsewhere.path, "tests/e2e/node_modules/stray");
    expect(await elsewhere.git(["status", "--porcelain"])).toBe("");
    const moved = report(await scanSelf(elsewhere, ["--json"], { target }));
    expect(withoutTimings(moved)).toEqual(withoutTimings(reference));
  }, TEST_TIMEOUT_MS);

  test("INV-11 a full scan and a baseline scan leave the repository byte for byte as they found it", async () => {
    await fullScan();
    const before = await repositoryState(clone);
    expect((await scanSelf(clone, ["--json"], { target })).exitCode).toBe(0);
    expect(await repositoryState(clone)).toBe(before);
    expect((await scanSelf(clone, ["--json", "--scope", "baseline", "--base", await clone.sha()], { target })).exitCode).toBe(0);
    expect(await repositoryState(clone)).toBe(before);
  }, TEST_TIMEOUT_MS);

  test.skipIf(!requireNetns())("INV-12 a scan inside a network namespace publishes the report a networked scan does", async () => {
    const reference = await fullScan();
    const offline = await runDoctorOffline(scanArgs(clone, ["--json"]), {
      sandbox: clone.sandbox,
      target,
      env: selfEnv(),
      timeoutMs: SCAN_TIMEOUT_MS,
    });
    expect(offline.exitCode).toBe(0);
    expect(withoutTimings(report(offline))).toEqual(withoutTimings(reference));
  }, TEST_TIMEOUT_MS);
});
