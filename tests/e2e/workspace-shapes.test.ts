// `tasks/e2e-catalog/failures.md` section WS: workspace shapes and
// filesystem conditions. A shape Cargo refuses fails with Cargo's own
// sentence and no absolute path; a shape it accepts scans as the plain copy
// does; and a scan leaves the tree as it found it, a lockfile it made
// included. Permission rows need a user that permissions bind, so they skip
// under root.
import { describe, expect, test } from "bun:test";
import { chmodSync, existsSync, mkdirSync, readdirSync, readFileSync, symlinkSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import {
  BIN,
  copyWorkspace,
  E2E_BASE,
  normalizeReport,
  pathWithout,
  runCommand,
  runDoctor,
  sandbox,
  scratchDir,
  treeFingerprint,
  type DoctorOptions,
  type DoctorRun,
} from "./e2e-helpers";
import { gitScenario } from "./git-scenario";
import { requireNetns, runDoctorOffline } from "./netns";

type ScanError = { stage: string; code: string; message: string };
type Diagnostic = { code: string | null; path: string | null; severity: string; related?: unknown[] };
type Report = {
  status: string;
  audit: { score: { value: number } | null; packages: Array<{ name: string; unlinted?: boolean; score: unknown }> };
  policy: { rules: Array<{ id: string; not_evaluated?: string }> } | null;
  scan: { excluded_generated: number; advisory_db?: unknown };
  diagnostics: Diagnostic[];
  errors: ScanError[];
  timings?: Record<string, unknown>;
};

const ROOT = process.getuid?.() === 0;
if (ROOT) console.warn("running as root, which file permissions do not bind: permission rows skipped");

async function scan(path: string, args: string[] = [], options: DoctorOptions = {}): Promise<DoctorRun & { report: Report }> {
  const run = await runDoctor([path, "--yes", "--json", ...args], options);
  return { ...run, report: run.json as Report };
}

function codes(report: Report): string[] {
  return report.errors.map((error) => `${error.stage}/${error.code}`);
}

function message(report: Report, code: string): string {
  const found = report.errors.find((entry) => entry.code === code);
  if (!found) throw new Error(`no ${code} error in ${JSON.stringify(report.errors)}`);
  return found.message;
}

function rules(report: Report): Array<string | null> {
  return report.diagnostics.map((diagnostic) => diagnostic.code);
}

function same(left: unknown, right: unknown): void {
  expect(normalizeReport(left)).toEqual(normalizeReport(right));
}

// A crate written for the row, under a fresh scratch directory unless `at`
// names its place.
function crate(name: string, files: Record<string, string>, at?: string): string {
  const dir = at ?? join(scratchDir(name), name);
  for (const [path, body] of Object.entries(files)) {
    mkdirSync(dirname(join(dir, path)), { recursive: true });
    writeFileSync(join(dir, path), body);
  }
  return dir;
}

function manifest(name: string, extra = "", workspace = true): string {
  return `[package]\nname = "${name}"\nversion = "0.1.0"\nedition = "2024"\npublish = false\n${extra}${workspace ? "\n[workspace]\n" : ""}`;
}

// `chmod` for the length of `body`, undone whatever happens, so the run root
// stays removable.
async function withMode<T>(path: string, mode: number, restore: number, body: () => Promise<T>): Promise<T> {
  chmodSync(path, mode);
  try {
    return await body();
  } finally {
    chmodSync(path, restore);
  }
}

async function chmodTree(path: string, mode: string): Promise<void> {
  const run = await runCommand(["chmod", "-R", mode, path]);
  if (run.exitCode !== 0) throw new Error(`chmod -R ${mode} ${path}: ${run.stderr}`);
}

// 3000 modules in 60 files of 50 declarations, each holding the same body:
// one file of 3000 `mod` lines would trip the source kernel's per-file
// alias-bindings limit instead.
async function bigNative(): Promise<string> {
  const listed = await runDoctor(["rules", "list", "--json"]);
  const off = (listed.json as Array<{ id: string }>)
    .filter((rule) => rule.id.startsWith("clippy::"))
    .map((rule) => `"${rule.id}" = "off"`)
    .join("\n");
  const body = "pub fn same(values: &[u32]) -> u32 {\n    let mut total = 0;\n    for value in values {\n        total += value * 2 + 1;\n    }\n    total\n}\n";
  const files: Record<string, string> = { "Cargo.toml": manifest("bignative"), "rust-doctor.toml": `[rules]\n${off}\n` };
  const groups: string[] = [];
  for (let group = 0; group < 60; group += 1) {
    groups.push(`pub mod g${group};`);
    const members: string[] = [];
    for (let member = 0; member < 50; member += 1) {
      members.push(`pub mod m${member};`);
      files[`src/g${group}/m${member}.rs`] = body;
    }
    files[`src/g${group}.rs`] = `${members.join("\n")}\n`;
  }
  files["src/lib.rs"] = `${groups.join("\n")}\n`;
  return crate("bignative", files);
}

describe("workspace shapes", () => {
  test("WS-01 a manifest Cargo cannot parse is quoted with its relative position", async () => {
    const { exitCode, report, stdout } = await scan(copyWorkspace("scan-failures/bad-manifest"));
    expect(exitCode).toBe(2);
    expect(codes(report)).toEqual(["metadata/cargo-metadata"]);
    expect(message(report, "cargo-metadata")).toContain("--> Cargo.toml:8:20");
    expect(stdout).not.toContain(E2E_BASE);
  });

  test("WS-02 a lockfile Cargo cannot parse is named by both passes", async () => {
    const { exitCode, report } = await scan(copyWorkspace("scan-failures/bad-lockfile"));
    expect(exitCode).toBe(1);
    expect(report.status).toBe("incomplete");
    expect(codes(report)).toContain("dependencies/lockfile-invalid");
    expect(message(report, "clippy-exit")).toContain("failed to parse lock file at: ./Cargo.lock");
  });

  test("WS-03 a broken member leaves the others linted and is named", async () => {
    const { exitCode, report } = await scan(copyWorkspace("scan-failures/broken-member"));
    expect(exitCode).toBe(1);
    expect(message(report, "packages-unlinted")).toBe("Not linted because it did not compile: broken-a, dependent-c");
    expect(report.diagnostics).toContainEqual(expect.objectContaining({ code: "clippy::todo", path: "b/src/lib.rs" }));
    expect(report.audit.score?.value).toBe(94);
  });

  test("WS-04 a build script runs, and its failure names the target directory by its variable", async () => {
    const workspace = crate("build-fails", {
      "Cargo.toml": manifest("build-fails"),
      "src/lib.rs": "pub fn f() {}\n",
      "build.rs": 'fn main() {\n    let _ = std::fs::write(concat!(env!("CARGO_MANIFEST_DIR"), "/marker"), "ran");\n    std::process::exit(3);\n}\n',
    });
    const { exitCode, report, stdout } = await scan(workspace);
    expect(exitCode).toBe(1);
    expect(existsSync(join(workspace, "marker"))).toBe(true);
    expect(message(report, "clippy-exit")).toMatch(/`\$CARGO_TARGET_DIR\/debug\/build\/build-fails-[0-9a-f]+\/build-script-build` \(exit status: 3\)/);
    expect(stdout).not.toContain(E2E_BASE);
  });

  test("WS-05 a virtual workspace with no member is Cargo's sentence, incomplete", async () => {
    const { exitCode, report } = await scan(crate("no-members", { "Cargo.toml": "[workspace]\nmembers = []\n" }));
    expect(exitCode).toBe(1);
    expect(report.status).toBe("incomplete");
    expect(message(report, "clippy-exit")).toContain("manifest path `.` contains no package: The manifest is virtual, and the workspace has no members.");
  });

  test("WS-06 a declared lib path with no file publishes no score", async () => {
    const { exitCode, report } = await scan(crate("no-lib", { "Cargo.toml": manifest("no-lib", '\n[lib]\npath = "src/lib.rs"\n') }));
    expect(exitCode).toBe(1);
    expect(report.audit.score).toBeNull();
    expect(message(report, "read-failed")).toBe('Source path "src/lib.rs" could not be resolved.');
  });

  test("WS-07 a member outside the workspace is named by its relative source path only", async () => {
    const parent = scratchDir("outside-member");
    copyWorkspace("projects/virtual-workspace", join(parent, "virtual-workspace"));
    copyWorkspace("projects/shared", join(parent, "shared"));
    const { exitCode, report, stdout } = await scan(join(parent, "virtual-workspace"));
    expect(exitCode).toBe(1);
    expect(message(report, "path-outside-workspace")).toBe('Source path "lib.rs" resolves outside the workspace.');
    expect(stdout).not.toContain(parent);
  });

  test("WS-08 a missing member outside the workspace publishes no absolute path, in JSON or linear", async () => {
    const parent = scratchDir("missing-member");
    const workspace = crate("ws", { "Cargo.toml": '[workspace]\nmembers = ["../gone"]\n' }, join(parent, "ws"));
    const { exitCode, report, stdout } = await scan(workspace);
    expect(exitCode).toBe(2);
    expect(codes(report)).toEqual(["metadata/cargo-metadata"]);
    expect(message(report, "cargo-metadata")).toContain("gone/Cargo.toml");
    expect(stdout).not.toContain(parent);
    const linear = await runDoctor([workspace, "--yes"]);
    expect(linear.exitCode).toBe(2);
    expect(linear.stdout).toContain("(metadata/cargo-metadata)");
    expect(linear.stdout + linear.stderr).not.toContain(parent);
  });

  test("failures/BUG-16 a path that merely starts with the workspace root's text is not rewritten into it (WS-08)", async () => {
    const parent = scratchDir("prefix-member");
    const workspace = crate("gone", { "Cargo.toml": '[workspace]\nmembers = ["../gone-member"]\n' }, join(parent, "gone"));
    const { report } = await scan(workspace);
    const text = message(report, "cargo-metadata");
    expect(text).toContain("gone-member/Cargo.toml");
    expect(text).not.toContain(".-member");
  });

  test("WS-09 two workspaces below the path are refused by name", async () => {
    const parent = scratchDir("ambiguous");
    for (const name of ["a", "b"]) crate(name, { "Cargo.toml": manifest(name), "src/lib.rs": "" }, join(parent, name));
    const { exitCode, report } = await scan(parent);
    expect(exitCode).toBe(2);
    expect(codes(report)).toEqual(["discovery/ambiguous-workspace"]);
    expect(message(report, "ambiguous-workspace")).toBe("Several Cargo workspaces under this path: a, b. Pass one of them.");
  });

  test("WS-10 the one workspace below the path scans as itself, with one stderr notice", async () => {
    const parent = scratchDir("descend");
    const inner = copyWorkspace("projects/clippy-warning", join(parent, "other place ü"));
    const descended = await scan(parent);
    const direct = await scan(inner);
    expect(descended.exitCode).toBe(0);
    expect(descended.stdout).toBe(direct.stdout);
    expect(descended.stderr).toBe("No Cargo workspace at this path: scanning the one in other place ü/.\n");
    expect(direct.stderr).toBe("");
  });

  test("WS-11 a subdirectory or the manifest itself scans as the root does", async () => {
    const workspace = copyWorkspace("projects/clippy-warning");
    const root = await scan(workspace);
    for (const path of [join(workspace, "src"), join(workspace, "Cargo.toml")]) {
      const run = await scan(path);
      expect(run.exitCode).toBe(0);
      expect(run.stdout).toBe(root.stdout);
      expect(run.stderr).toBe("");
    }
  });

  test("WS-12 a symlink to the workspace scans as the workspace", async () => {
    const workspace = copyWorkspace("projects/clippy-warning");
    const link = join(scratchDir("links"), "warn-link");
    symlinkSync(workspace, link);
    expect((await scan(link)).stdout).toBe((await scan(workspace)).stdout);
  });

  test("WS-13 a directory with a space and non-ASCII scans as the plain copy, absolute or relative", async () => {
    const plain = await scan(copyWorkspace("projects/clippy-warning"));
    const parent = scratchDir("odd-name");
    copyWorkspace("projects/clippy-warning", join(parent, "other place ü"));
    const absolute = await scan(join(parent, "other place ü"));
    const relative = await scan("other place ü", [], { cwd: parent });
    expect([absolute.exitCode, relative.exitCode]).toEqual([0, 0]);
    expect(absolute.stdout).toBe(plain.stdout);
    expect(relative.stdout).toBe(plain.stdout);
  });

  // A name that is not UTF-8 cannot be a JavaScript string, so the shell
  // builds it and passes it on.
  test("WS-14 a directory name that is not UTF-8 is Cargo's limit, reported", async () => {
    const parent = scratchDir("not-utf8");
    const box = sandbox("not-utf8");
    const source = copyWorkspace("projects/clippy-warning");
    const name = '"$(printf "ws\\377")"';
    try {
      const made = await runCommand(["sh", "-c", `mkdir ${name} && cp -R "$1"/. ${name}/`, "sh", source], { cwd: parent, sandbox: box });
      expect(made.exitCode).toBe(0);
      const run = await runCommand(["sh", "-c", `exec "$1" ${name} --yes --json --advisory-db /dev/null`, "sh", BIN], {
        cwd: parent,
        sandbox: box,
        env: { CARGO_TARGET_DIR: scratchDir("not-utf8-target") },
      });
      expect(run.exitCode).toBe(2);
      const report = JSON.parse(run.stdout) as Report;
      expect(codes(report)).toEqual(["metadata/cargo-metadata"]);
      expect(message(report, "cargo-metadata")).toContain("invalid UTF-8 was detected in one or more arguments");
    } finally {
      await runCommand(["sh", "-c", `rm -rf ${name}`], { cwd: parent, sandbox: box });
    }
  });

  test("WS-15 a source file over the byte limit is named, and Clippy's findings stay", async () => {
    const workspace = copyWorkspace("projects/clippy-warning");
    writeFileSync(join(workspace, "src/big.rs"), `// ${"x".repeat(9_000_042)}\n`);
    writeFileSync(join(workspace, "src/lib.rs"), `${readFileSync(join(workspace, "src/lib.rs"), "utf8")}\nmod big;\n`);
    const { exitCode, report } = await scan(workspace);
    expect(exitCode).toBe(1);
    expect(message(report, "limit-exceeded")).toBe('Source limit "file-bytes" exceeded (maximum 8388608).');
    expect(rules(report)).toContain("clippy::unwrap_used");
  });

  test("WS-16 a file declared generated is excluded and counted", async () => {
    const workspace = copyWorkspace("projects/clippy-warning");
    writeFileSync(join(workspace, "src/generated.rs"), "// @generated by a tool. DO NOT EDIT.\npub fn g() -> u8 {\n    Some(1).unwrap()\n}\n");
    writeFileSync(join(workspace, "src/lib.rs"), `${readFileSync(join(workspace, "src/lib.rs"), "utf8")}\npub mod generated;\n`);
    const { exitCode, report } = await scan(workspace);
    expect(exitCode).toBe(0);
    expect(report.scan.excluded_generated).toBe(1);
    expect(report.diagnostics.map((diagnostic) => diagnostic.path)).not.toContain("src/generated.rs");
    expect(report.diagnostics.filter((diagnostic) => diagnostic.code === "clippy::unwrap_used").map((diagnostic) => diagnostic.path)).toEqual(["src/lib.rs"]);
  });

  test("WS-17 a 3000-module crate scanned by the native passes alone completes as one JSON document", async () => {
    const { exitCode, report, stdout } = await scan(await bigNative(), ["--timings"], { timeoutMs: 90_000 });
    expect(exitCode).toBe(0);
    expect(report.status).toBe("complete");
    expect(stdout.trimEnd().split("\n")).toHaveLength(1);
    const duplicates = report.diagnostics.filter((diagnostic) => diagnostic.code === "rust_doctor::structure::duplicate_function_body");
    expect(duplicates).toHaveLength(1);
    expect(duplicates[0]!.related?.length).toBeGreaterThan(0);
    expect(report.timings).toBeDefined();
  });

  test("WS-18 a path dependency outside the workspace that does not compile names the app unlinted, with no path", async () => {
    const parent = scratchDir("outside-dep");
    crate("dep", { "Cargo.toml": manifest("dep"), "src/lib.rs": 'pub fn d() -> u8 {\n    "x"\n}\n' }, join(parent, "dep"));
    const app = crate("app", { "Cargo.toml": manifest("app", '\n[dependencies]\ndep = { path = "../dep" }\n'), "src/lib.rs": "pub fn a() -> u8 {\n    dep::d()\n}\n" }, join(parent, "app"));
    const { exitCode, report, stdout } = await scan(app);
    expect(exitCode).toBe(1);
    expect(rules(report)).toContain("rust_doctor::cargo::path_dependency_outside_workspace");
    expect(report.diagnostics).toContainEqual(expect.objectContaining({ code: "E0308", path: null }));
    expect(message(report, "packages-unlinted")).toBe("Not linted because it did not compile: app");
    expect(stdout).not.toContain(parent);
  });

  // The harness exports CARGO_NET_OFFLINE=true, so an uncached dependency
  // fails at once with Cargo's offline sentence instead of retrying.
  test("WS-19 an uncached dependency offline is incomplete, bounded, and carries no score", async () => {
    const workspace = crate("uncached", { "Cargo.toml": manifest("uncached", '\n[dependencies]\nitoa = "1"\n'), "src/lib.rs": "pub fn f() {}\n" });
    const before = treeFingerprint(workspace);
    const { exitCode, report, durationMs } = await scan(workspace, [], { env: { CARGO_HOME: scratchDir("empty-cargo-home") } });
    expect(exitCode).toBe(1);
    expect(report.status).toBe("incomplete");
    expect(message(report, "clippy-exit")).toContain("no matching package named `itoa` found");
    expect(message(report, "packages-unlinted")).toBe("Not linted because it did not compile: uncached");
    expect(report.audit.score).toBeNull();
    expect(report.audit.packages).toEqual([expect.objectContaining({ unlinted: true, score: null })]);
    expect(durationMs).toBeLessThan(20_000);
    expect(treeFingerprint(workspace)).toBe(before);
  });

  test.skipIf(!requireNetns())("WS-20 a vendored registry scans the same without a network", async () => {
    const workspace = copyWorkspace("cargo-health/offline-registry");
    const online = await scan(workspace);
    const offline = await runDoctorOffline([workspace, "--yes", "--json"]);
    expect(offline.exitCode).toBe(0);
    expect((offline.json as Report).status).toBe("complete");
    same(offline.json, online.json);
  });

  test("WS-21 a library without a lockfile is left without one", async () => {
    const workspace = crate("lockless-lib", { "Cargo.toml": manifest("lockless-lib"), "src/lib.rs": "pub fn f() {}\n" });
    const before = treeFingerprint(workspace);
    const { exitCode, report } = await scan(workspace);
    expect(exitCode).toBe(0);
    expect(report.status).toBe("complete");
    expect(existsSync(join(workspace, "Cargo.lock"))).toBe(false);
    expect(treeFingerprint(workspace)).toBe(before);
  });

  test("WS-22 a binary without a lockfile gets the same verdict on every scan", async () => {
    const workspace = crate("lockless-bin", { "Cargo.toml": manifest("lockless-bin"), "src/main.rs": "fn main() {}\n" });
    const before = treeFingerprint(workspace);
    const first = await scan(workspace);
    expect(treeFingerprint(workspace)).toBe(before);
    const second = await scan(workspace);
    expect(treeFingerprint(workspace)).toBe(before);
    expect(rules(first.report)).toContain("rust_doctor::cargo::missing_lockfile");
    same(second.json, first.json);
  });

  test.skipIf(ROOT)("WS-23 a read-only workspace with its lockfile scans complete", async () => {
    const workspace = copyWorkspace("scan-failures/lint-flags");
    const before = treeFingerprint(workspace);
    await chmodTree(workspace, "a-w");
    try {
      const { exitCode, report } = await scan(workspace);
      expect(exitCode).toBe(0);
      expect(report.status).toBe("complete");
    } finally {
      await chmodTree(workspace, "u+w");
    }
    expect(treeFingerprint(workspace)).toBe(before);
  });

  // Fully read-only, Cargo cannot write the lockfile at all; writable until
  // its build script runs, the lockfile exists and cannot be removed, which
  // the scan says once on stderr.
  test.skipIf(ROOT)("WS-24 a read-only lockless workspace is Cargo's refusal, and an unremovable lockfile one stderr line", async () => {
    const workspace = crate("read-only-lockless", { "Cargo.toml": manifest("read-only-lockless"), "src/lib.rs": "pub fn f() {}\n" });
    await chmodTree(workspace, "a-w");
    try {
      const { exitCode, report } = await scan(workspace);
      expect(exitCode).toBe(1);
      expect(message(report, "clippy-exit")).toContain("failed to write ./Cargo.lock");
      expect(readdirSync(workspace).sort()).toEqual(["Cargo.toml", "src"]);
    } finally {
      await chmodTree(workspace, "u+w");
    }

    const locking = crate("locks-itself", {
      "Cargo.toml": manifest("locks-itself"),
      "src/main.rs": "fn main() {}\n",
      "build.rs": 'use std::os::unix::fs::PermissionsExt;\nfn main() {\n    let root = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();\n    let _ = std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o555));\n}\n',
    });
    try {
      const { exitCode, report, stderr } = await scan(locking);
      expect(exitCode).toBe(0);
      expect(rules(report)).toContain("rust_doctor::cargo::missing_lockfile");
      expect(stderr.split("\n").filter((line) => line.includes("Cargo.lock"))).toHaveLength(1);
    } finally {
      chmodSync(locking, 0o755);
    }
  });

  test.skipIf(ROOT)("WS-25 a read-only target directory fails the build with the path scrubbed", async () => {
    const parent = scratchDir("read-only-target");
    const { exitCode, report, stdout } = await withMode(parent, 0o555, 0o755, () => scan(copyWorkspace("scan-failures/lint-flags"), [], { target: join(parent, "target") }));
    expect(exitCode).toBe(1);
    expect(message(report, "clippy-exit")).toContain("Permission denied (os error 13)");
    // The path Cargo is refused is the sibling it creates beside the target
    // directory, so it is named as a path outside the workspace, not as the
    // target directory (failures/BUG-16).
    expect(message(report, "clippy-exit")).toContain("<external>/");
    expect(message(report, "clippy-exit")).not.toContain(parent);
    expect(stdout).not.toContain(parent);
  });

  test("failures/BUG-16 a sibling of the target directory is not rewritten as the target directory (WS-25)", async () => {
    const parent = scratchDir("read-only-target-sibling");
    const { report } = await withMode(parent, 0o555, 0o755, () => scan(copyWorkspace("scan-failures/lint-flags"), [], { target: join(parent, "target") }));
    expect(message(report, "clippy-exit")).not.toMatch(/\$CARGO_TARGET_DIR[A-Za-z0-9]/);
  });

  test("WS-26 without CARGO_TARGET_DIR the build lands in the workspace's target/", async () => {
    const workspace = copyWorkspace("scan-failures/lint-flags");
    const { exitCode } = await scan(workspace, [], { env: { CARGO_TARGET_DIR: undefined } });
    expect(exitCode).toBe(0);
    expect(existsSync(join(workspace, "target/debug"))).toBe(true);
  });

  test("WS-27 a relative CARGO_TARGET_DIR resolves against the workspace, not the caller's directory", async () => {
    const workspace = copyWorkspace("scan-failures/lint-flags");
    const caller = scratchDir("caller");
    const { exitCode } = await scan(workspace, [], { cwd: caller, env: { CARGO_TARGET_DIR: "reltarget" } });
    expect(exitCode).toBe(0);
    expect(existsSync(join(workspace, "reltarget"))).toBe(true);
    expect(readdirSync(caller)).toEqual([]);
  });

  test("WS-28 CARGO_BUILD_TARGET_DIR places the build", async () => {
    const workspace = copyWorkspace("scan-failures/lint-flags");
    const target = scratchDir("build-target-dir");
    const { exitCode } = await scan(workspace, [], { env: { CARGO_TARGET_DIR: undefined, CARGO_BUILD_TARGET_DIR: target } });
    expect(exitCode).toBe(0);
    expect(existsSync(join(target, "debug"))).toBe(true);
    expect(existsSync(join(workspace, "target"))).toBe(false);
  });

  test("WS-29 an invalid rust-doctor.toml names its position", async () => {
    const workspace = copyWorkspace("scan-failures/lint-flags");
    writeFileSync(join(workspace, "rust-doctor.toml"), "[rules");
    const { exitCode, report } = await scan(workspace);
    expect(exitCode).toBe(2);
    expect(codes(report)).toEqual(["configuration/config-invalid"]);
    expect(message(report, "config-invalid")).toBe("Invalid rust-doctor.toml at line 1, column 7.");
  });

  test.skipIf(ROOT)("WS-30 an unreadable rust-doctor.toml is refused", async () => {
    const workspace = copyWorkspace("scan-failures/lint-flags");
    const config = join(workspace, "rust-doctor.toml");
    writeFileSync(config, "[rules]\n");
    const { exitCode, report } = await withMode(config, 0, 0o644, () => scan(workspace));
    expect(exitCode).toBe(2);
    expect(codes(report)).toEqual(["configuration/config-unreadable"]);
    expect(message(report, "config-unreadable")).toBe("Could not read rust-doctor.toml.");
  });

  test.skipIf(ROOT)("WS-31 an unreadable module is named, beside the compile failure it causes", async () => {
    const workspace = copyWorkspace("scan-failures/lint-flags");
    writeFileSync(join(workspace, "src/hidden.rs"), "pub fn h() {}\n");
    writeFileSync(join(workspace, "src/lib.rs"), `pub mod hidden;\n${readFileSync(join(workspace, "src/lib.rs"), "utf8")}`);
    const { exitCode, report } = await withMode(join(workspace, "src/hidden.rs"), 0, 0o644, () => scan(workspace));
    expect(exitCode).toBe(1);
    expect(message(report, "read-failed")).toBe('Source path "src/hidden.rs" could not be opened.');
    expect(codes(report)).toContain("execution/build-failed");
  });

  test.skipIf(ROOT)("WS-32 an unreadable .cargo/config.toml stops cargo metadata", async () => {
    const workspace = copyWorkspace("scan-failures/lint-flags-config");
    const { exitCode, report } = await withMode(join(workspace, ".cargo/config.toml"), 0, 0o644, () => scan(workspace));
    expect(exitCode).toBe(2);
    expect(codes(report)).toEqual(["metadata/cargo-metadata"]);
    expect(message(report, "cargo-metadata")).toContain("failed to read configuration file `./.cargo/config.toml`");
    expect(message(report, "cargo-metadata")).toContain("Permission denied");
  });

  test("WS-33 a repository scanned without git records the skipped pass", async () => {
    const repo = await gitScenario("ws-33").fixture("scan-failures/lint-flags").write({ ".gitignore": "/target\n" }).commit("base").build();
    const { exitCode, report } = await scan(repo.path, [], { sandbox: repo.sandbox, env: { PATH: pathWithout("git") } });
    expect(exitCode).toBe(1);
    expect(report.status).toBe("incomplete");
    expect(codes(report)).toEqual(["repo/git-unavailable"]);
    expect(message(report, "git-unavailable")).toBe("Repository hygiene skipped: git was not available.");
  });

  test.failing("WS-34 failures/BUG-08 a workspace outside git scans complete without git on PATH", async () => {
    const workspace = copyWorkspace("kernel-contract/todo");
    expect(((await runDoctor([workspace, "--yes", "--json"])).json as Report).status).toBe("complete");
    const run = await runDoctor([workspace, "--yes", "--json"], { env: { PATH: pathWithout("git") } });
    expect((run.json as Report).status).toBe("complete");
    expect(run.exitCode).toBe(0);
  });

  test("WS-35 a workspace outside git, git present, is complete and the repository rules stay silent", async () => {
    const { exitCode, report } = await scan(copyWorkspace("scan-failures/lint-flags"));
    expect(exitCode).toBe(0);
    expect(report.status).toBe("complete");
    expect(report.errors).toEqual([]);
    expect(rules(report).filter((code) => code?.startsWith("rust_doctor::repo::"))).toEqual([]);
  });

  test("WS-36 a baseline without temporary storage is refused, whether TMPDIR is missing or read-only", async () => {
    const repo = await gitScenario("ws-36").fixture("scan-failures/lint-flags").write({ ".gitignore": "/target\n" }).commit("base").build();
    const readOnly = scratchDir("read-only-tmp");
    const tmpdirs: Array<[string, () => Promise<DoctorRun & { report: Report }>]> = [
      // A missing TMPDIR is skipped for the cache home, so HOME is missing too.
      ["missing", () => scan(repo.path, ["--scope", "baseline", "--base", "HEAD"], { sandbox: repo.sandbox, env: { TMPDIR: join(repo.sandbox.dir, "missing-tmp"), HOME: join(repo.sandbox.dir, "missing-home"), RUST_DOCTOR_NO_CACHE: "1" } })],
    ];
    if (!ROOT) {
      tmpdirs.push(["read-only", () => withMode(readOnly, 0o555, 0o755, () => scan(repo.path, ["--scope", "baseline", "--base", "HEAD"], { sandbox: repo.sandbox, env: { TMPDIR: readOnly, RUST_DOCTOR_NO_CACHE: "1" } }))]);
    }
    for (const [kind, run] of tmpdirs) {
      const { exitCode, report } = await run();
      expect(exitCode).toBe(2);
      expect(codes(report)).toEqual(["baseline/baseline-temp-unavailable"]);
      // A missing TMPDIR is refused as a candidate before any write: WS-36b.
      if (kind === "read-only") {
        expect(message(report, "baseline-temp-unavailable")).toBe("Git baseline temporary storage is unavailable.");
      }
    }
  });

  test("WS-36b a missing TMPDIR with no private fallback names the refused candidate", async () => {
    const repo = await gitScenario("ws-36b").fixture("scan-failures/lint-flags").write({ ".gitignore": "/target\n" }).commit("base").build();
    const env = { TMPDIR: join(repo.sandbox.dir, "missing-tmp"), HOME: join(repo.sandbox.dir, "missing-home"), RUST_DOCTOR_NO_CACHE: "1" };
    const { exitCode, report } = await scan(repo.path, ["--scope", "baseline", "--base", "HEAD"], { sandbox: repo.sandbox, env });
    expect(exitCode).toBe(2);
    expect(codes(report)).toEqual(["baseline/baseline-temp-unavailable"]);
    expect(message(report, "baseline-temp-unavailable")).toMatch(/^no private directory for the snapshot: \S*\/missing-tmp is unavailable$/);
  });

  test.skipIf(ROOT)("WS-37 a full scan never needs TMPDIR", async () => {
    const tmp = scratchDir("read-only-tmp-full");
    const { exitCode, report } = await withMode(tmp, 0o555, 0o755, () => scan(copyWorkspace("scan-failures/lint-flags"), [], { env: { TMPDIR: tmp } }));
    expect(exitCode).toBe(0);
    expect(report.status).toBe("complete");
  });

  test("WS-38 without HOME or CARGO_HOME the scan completes and finds no advisory database", async () => {
    const { exitCode, report } = await scan(copyWorkspace("scan-failures/lint-flags"), [], { advisoryDb: false, env: { HOME: undefined, CARGO_HOME: undefined } });
    expect(exitCode).toBe(0);
    expect(report.status).toBe("complete");
    expect(report.scan.advisory_db).toBeUndefined();
    expect(report.policy?.rules.find((rule) => rule.id === "rust_doctor::cargo::vulnerable_dependency")?.not_evaluated).toBe("advisory-db-missing");
  });
});
