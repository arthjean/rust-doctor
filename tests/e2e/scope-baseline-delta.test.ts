// `tasks/e2e-catalog/scopes.md` section E with L02, L03, H08 and F03d: what a
// baseline comparison calls new, fixed and pre-existing, its base-side cache,
// and the base trees it refuses. Every `scopes/BUG-A` expectation is plain:
// US-002 gave the base side a git view, so a repository finding the branch
// did not touch is pre-existing and one it deleted is fixed.
import { describe, expect, test } from "bun:test";
import { chmodSync, statSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { pathWithout, runCommand, runDoctor, treeFingerprint } from "./e2e-helpers";
import { gitScenario, miniTemplate, NEW_FN } from "./git-scenario";
import { deltaSummary, errorCodes, human, introduced, scan, scopeLine, sha256Mini, type Report } from "./scope-scan-support";
import { slowRepo, snapshots } from "./scope-snapshot-support";

const LIB = miniTemplate()["src/lib.rs"]!;
const CREDENTIAL_SITE = "rust_doctor::repo::hardcoded_credential@src/creds.rs:2";

type Fixed = { code: string; path: string; span: { line_start: number } | null };
type DeltaReport = Report & {
  gate: { blocking: string; status: string; blocking_diagnostics: number };
  delta: (NonNullable<Report["delta"]> & { fixed: Fixed[]; pre_existing: Array<{ current_id: string }> }) | null;
};

function fixedSites(report: DeltaReport): string[] {
  return (report.delta?.fixed ?? []).map((finding) => `${finding.code}@${finding.path}:${finding.span?.line_start}`).sort();
}

function preExistingSites(report: DeltaReport): string[] {
  const ids = new Set((report.delta?.pre_existing ?? []).map((pair) => pair.current_id));
  return report.diagnostics
    .filter((diagnostic) => ids.has(diagnostic.id))
    .map((diagnostic) => `${diagnostic.code}@${diagnostic.path}:${diagnostic.span?.line_start}`)
    .sort();
}

async function baseline(path: string, args: string[], options: Parameters<typeof scan>[2] = {}) {
  const { run, report } = await scan(path, ["--scope", "baseline", ...args], options);
  return { run, report: report as DeltaReport };
}

// `mini` on `main` and a branch `feature` holding NEWFN.
function newFn(name: string) {
  return gitScenario(name).template("mini").commit("base").branch("feature").append("src/lib.rs", NEW_FN).commit("feature").build();
}

describe("what the branch introduced", () => {
  test("E01 a no-op branch introduces nothing and keeps the credential pre-existing", async () => {
    const repo = await gitScenario("e01").template("mini").commit("base").branch("feature").build();
    const { run, report } = await baseline(repo.path, [], { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(0);
    expect(deltaSummary(report)).toEqual([0, 2, 0, 0, 0]);
    expect(preExistingSites(report)).toContain(CREDENTIAL_SITE);
  });

  test("E01b a no-op branch of a repository holding a credential passes --blocking warning with zero introduced", async () => {
    const repo = await gitScenario("e01b").template("mini").commit("base").branch("feature").build();
    const { run, report } = await baseline(repo.path, ["--blocking", "warning"], { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(0);
    expect(report.delta?.summary.introduced).toBe(0);
    expect(report.gate).toEqual({ blocking: "warning", status: "passed", blocking_diagnostics: 0 });
  });

  test("E03 a removed todo is fixed and printed as such", async () => {
    const repo = await gitScenario("e03")
      .template("mini").commit("base")
      .branch("feature").write({ "src/lib.rs": LIB.replace("todo!()", "0") }).commit("fix")
      .build();
    const { run, report } = await baseline(repo.path, [], { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(0);
    expect(deltaSummary(report)).toEqual([0, 1, 1, 0, 0]);
    expect(fixedSites(report)).toEqual(["clippy::todo@src/lib.rs:4"]);
    const before = treeFingerprint(repo.path);
    const human = await runDoctor([repo.path, "--yes", "--scope", "baseline"], { sandbox: repo.sandbox });
    expect(treeFingerprint(repo.path)).toBe(before);
    expect(human.stdout).toContain("Fixed: src/lib.rs:4:5 warning [clippy::todo]");
  });

  test("E04 deleting the credential file fixes the repository finding", async () => {
    const repo = await gitScenario("e04")
      .template("mini").commit("base")
      .branch("feature").stage({ write: { "src/lib.rs": LIB.replace("pub mod creds;\n", "") }, remove: ["src/creds.rs"] }).commit("drop creds")
      .build();
    const { run, report } = await baseline(repo.path, [], { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(0);
    expect(deltaSummary(report)).toEqual([0, 1, 1, 0, 0]);
    expect(fixedSites(report)).toEqual([CREDENTIAL_SITE]);
  });

  test("E05 an introduced todo fails --blocking warning", async () => {
    const repo = await newFn("e05");
    const { run, report } = await baseline(repo.path, ["--blocking", "warning"], { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(1);
    expect(deltaSummary(report)).toEqual([1, 2, 0, 0, 0]);
    expect(introduced(report)).toEqual(["clippy::todo@src/lib.rs:12"]);
    expect(report.gate.blocking_diagnostics).toBe(1);
  });

  test("E05b an introduced todo passes the default error gate", async () => {
    const repo = await newFn("e05b");
    const { run, report } = await baseline(repo.path, [], { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(0);
    expect(deltaSummary(report)).toEqual([1, 2, 0, 0, 0]);
  });

  test("L02 an introduced warning in a SHA-256 repository exits with --warning-exit-code", async () => {
    const repo = await sha256Mini("l02", NEW_FN);
    const { run, report } = await baseline(repo.path, ["--warning-exit-code", "3"], { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(3);
    expect(deltaSummary(report)).toEqual([1, 2, 0, 0, 0]);
  });

  // `one.rs` becomes `two.rs` with most of its lines rewritten, so git sees
  // no rename: the todo pairs on evidence alone.
  test("H08 a todo moved to a rewritten file pairs across files", async () => {
    const repo = await gitScenario("h08")
      .template("mini")
      .write({ "src/one.rs": "pub fn one() -> u8 {\n    todo!()\n}\n" })
      .append("src/lib.rs", "pub mod one;\n")
      .commit("base")
      .stage({
        write: {
          "src/two.rs": "// Moved from one.rs and rewritten.\n\n/// The second function.\npub fn one() -> u8 {\n    todo!()\n}\n\npub fn extra() -> u16 {\n    7\n}\n",
          "src/lib.rs": `${LIB}pub mod two;\n`,
        },
        remove: ["src/one.rs"],
      })
      .build();
    const { run, report } = await baseline(repo.path, ["--base", "HEAD"], { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(0);
    expect(deltaSummary(report)).toEqual([0, 3, 0, 1, 0]);
  });

  // A patch past the line map's bound: the comparison still runs and says
  // it could not map lines.
  test("F03d a diff too large to map still compares and says so", async () => {
    const big = Array.from({ length: 40_000 }, (_, line) => `// filler line ${String(line).padStart(6, "0")} of a very large generated file\n`).join("");
    const repo = await gitScenario("f03d").template("mini").commit("base").stage({ write: { "src/big.rs": big } }).build();
    const { run, report } = await baseline(repo.path, ["--base", "HEAD"], { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(0);
    expect(report.status).toBe("complete");
    expect(errorCodes(report)).toEqual(["delta/line-map-unavailable"]);
  });

  test("E10 --package runs no repository pass, on either side", async () => {
    const repo = await newFn("e10");
    const { run, report } = await baseline(repo.path, ["--package", "mini"], { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(0);
    expect(deltaSummary(report)).toEqual([1, 1, 0, 0, 0]);
  });

  test("E10b a files scope accepts --package", async () => {
    const repo = await newFn("e10b");
    const { run, report } = await scan(repo.path, ["--scope", "files", "--package", "mini"], { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(0);
    expect(report.scope?.files).toEqual(["src/lib.rs"]);
  });

  // The catalog's scope line was `Scope: full codebase` (scopes/BUG-B, fixed
  // by US-008).
  test("E10c an unknown --package fails and names the scope it never resolved", async () => {
    const repo = await newFn("e10c");
    const args = ["--scope", "files", "--package", "nope"];
    const { run, report } = await scan(repo.path, args, { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(2);
    expect(errorCodes(report)).toEqual(["policy/unknown-package"]);
    expect(await scopeLine(repo.path, args, { sandbox: repo.sandbox })).toBe("Scope: files (not resolved)");
  });
});

describe("the base-side cache", () => {
  test("E02a a fresh target misses and writes the side cache", async () => {
    const repo = await newFn("e02a");
    const target = join(repo.sandbox.dir, "target");
    const before = treeFingerprint(repo.path);
    const human = await runDoctor([repo.path, "--yes", "--scope", "baseline", "--verbose"], { sandbox: repo.sandbox, target });
    expect(treeFingerprint(repo.path)).toBe(before);
    expect(human.exitCode).toBe(0);
    expect(human.stdout).toContain("Base side cache: miss");
    expect(statSync(join(target, "rust-doctor/cache/base-sides.json")).isFile()).toBe(true);
    expect(statSync(join(target, "rust-doctor/baseline")).isDirectory()).toBe(true);
  });

  // A TMPDIR nobody can write proves the hit materializes no snapshot.
  test("E02b an unchanged base hits the cache, gives the same delta and creates no snapshot", async () => {
    const repo = await newFn("e02b");
    const target = join(repo.sandbox.dir, "target");
    const first = await baseline(repo.path, [], { sandbox: repo.sandbox, target });
    chmodSync(repo.sandbox.tmp, 0o500);
    try {
      const second = await baseline(repo.path, [], { sandbox: repo.sandbox, target });
      expect(second.run.exitCode).toBe(0);
      expect(second.report.delta).toEqual(first.report.delta);
      const linear = await human(repo.path, ["--scope", "baseline", "--verbose"], { sandbox: repo.sandbox, target });
      expect(linear.stdout).toContain("Base side cache: hit");
    } finally {
      chmodSync(repo.sandbox.tmp, 0o700);
    }
  });

  test("E02c RUST_DOCTOR_NO_CACHE disables the cache, and no JSON report names it", async () => {
    const repo = await newFn("e02c");
    const target = join(repo.sandbox.dir, "target");
    const env = { RUST_DOCTOR_NO_CACHE: "1" };
    const reports = [
      await baseline(repo.path, [], { sandbox: repo.sandbox, target }),
      await baseline(repo.path, [], { sandbox: repo.sandbox, target }),
      await baseline(repo.path, [], { sandbox: repo.sandbox, target, env }),
    ];
    for (const { run } of reports) expect(run.stdout).not.toContain("base_side_cache");
    const linear = await human(repo.path, ["--scope", "baseline", "--verbose"], { sandbox: repo.sandbox, target, env });
    expect(linear.stdout).toContain("Base side cache: disabled");
  });
});

describe("base trees the baseline refuses or cannot read", () => {
  // `vendor/sub` is a submodule committed on `main`.
  async function e06() {
    const sub = await gitScenario("e06-sub").template("mini").commit("base").build();
    return gitScenario("e06")
      .template("mini").submodule("vendor/sub", sub).commit("base")
      .branch("feature").append("src/lib.rs", NEW_FN).commit("feature")
      .build();
  }

  test.failing("E06 scopes/BUG-E a submodule in the base tree does not refuse the baseline", async () => {
    const repo = await e06();
    const { report } = await baseline(repo.path, ["--base", "main"], { sandbox: repo.sandbox });
    expect(report.errors).toEqual([]);
    expect(report.delta?.summary.introduced).toBe(1);
  });

  test("E06b a files scope works beside a submodule", async () => {
    const repo = await e06();
    const { run, report } = await scan(repo.path, ["--scope", "files"], { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(0);
    expect(report.scope?.files).toEqual(["src/lib.rs"]);
  });

  test("E06c a staged scan skips the gitlink", async () => {
    const repo = await e06();
    const { run, report } = await scan(repo.path, ["--scope", "files", "--staged"], { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(0);
    expect(report.errors).toEqual([]);
  });

  // `src/s.rs -> ../shared/s.rs`: relative, through `..`, and inside the
  // repository.
  function e07() {
    return gitScenario("e07")
      .template("mini")
      .write({ "shared/s.rs": "pub fn s() -> u8 {\n    2\n}\n" })
      .symlink("src/s.rs", "../shared/s.rs")
      .append("src/lib.rs", "pub mod s;\n")
      .commit("base")
      .branch("feature").append("src/lib.rs", NEW_FN).commit("feature")
      .build();
  }

  test.failing("E07 scopes/BUG-F a relative symlink through .. that stays inside the repository is accepted", async () => {
    const repo = await e07();
    const { report } = await baseline(repo.path, ["--base", "main"], { sandbox: repo.sandbox });
    expect(report.errors).toEqual([]);
    expect(report.delta?.summary.introduced).toBe(1);
  });

  test("E07b a files scope follows the .. symlink", async () => {
    const repo = await e07();
    const { run } = await scan(repo.path, ["--scope", "files"], { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(0);
  });

  test.failing("E07c scopes/BUG-F a staged scan materializes a .. symlink that stays inside the repository", async () => {
    const repo = await e07();
    const { run, report } = await scan(repo.path, ["--scope", "files", "--staged"], { sandbox: repo.sandbox });
    expect(report.errors).toEqual([]);
    expect(run.exitCode).toBe(0);
  });

  test("E07d a same-directory symlink is accepted", async () => {
    const repo = await gitScenario("e07d")
      .template("mini").symlink("src/alias.rs", "lib.rs").commit("base")
      .branch("feature").append("src/lib.rs", NEW_FN).commit("feature")
      .build();
    const { run, report } = await baseline(repo.path, [], { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(0);
    expect(deltaSummary(report)).toEqual([1, 2, 0, 0, 0]);
  });

  test("E07e an absolute symlink in the base tree is refused", async () => {
    const repo = await gitScenario("e07e")
      .template("mini").symlink("abs-link", "/etc/hostname").commit("base")
      .branch("feature").append("src/lib.rs", NEW_FN).commit("feature")
      .build();
    const { run, report } = await baseline(repo.path, [], { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(2);
    expect(errorCodes(report)).toEqual(["baseline/baseline-entry-invalid"]);
  });

  // The base commit holds a README and no Cargo workspace.
  function e08() {
    return gitScenario("e08").write({ "README.md": "# e08\n" }).commit("base").branch("feature").template("mini").commit("workspace").build();
  }

  test("E08 a base without a workspace cannot be scanned", async () => {
    const repo = await e08();
    const { run, report } = await baseline(repo.path, [], { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(2);
    expect(errorCodes(report)).toEqual(["baseline/baseline-scan-incomplete"]);
  });

  test("E08b a files scope lists every file the branch added", async () => {
    const repo = await e08();
    const { run, report } = await scan(repo.path, ["--scope", "files"], { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(0);
    expect(report.scope?.files).toEqual([".gitignore", "Cargo.lock", "Cargo.toml", "src/creds.rs", "src/lib.rs"]);
  });

  // The base commit does not compile; the branch repairs it.
  function e09() {
    return gitScenario("e09")
      .template("mini").append("src/lib.rs", "pub fn broken( {\n").commit("base")
      .branch("feature").write({ "src/lib.rs": LIB }).commit("repair")
      .build();
  }

  test("E09 a base that does not compile fails the baseline", async () => {
    const repo = await e09();
    const { run, report } = await baseline(repo.path, [], { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(2);
    expect(errorCodes(report)).toEqual(["baseline/baseline-scan-incomplete"]);
  });

  test("E09b a files scope judges the repaired tree", async () => {
    const repo = await e09();
    const { run } = await scan(repo.path, ["--scope", "files"], { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(0);
  });

  test("E12 a partial clone missing the base blobs fails closed without fetching them", async () => {
    const origin = await gitScenario("e12").template("mini").commit("base").append("src/lib.rs", NEW_FN).commit("current").build();
    await origin.git(["config", "uploadpack.allowFilter", "true"]);
    const base = await origin.sha("HEAD~1");
    const clone = join(origin.sandbox.dir, "partial");
    const cloned = await runCommand(
      ["git", "-c", "protocol.file.allow=always", "clone", "-q", "--filter=blob:none", `file://${origin.path}`, clone],
      { sandbox: origin.sandbox },
    );
    expect(cloned.exitCode).toBe(0);
    const absent = () => runCommand(["git", "--no-lazy-fetch", "cat-file", "-e", `${base}:src/lib.rs`], { sandbox: origin.sandbox, cwd: clone });
    expect((await absent()).exitCode).not.toBe(0);
    const { run, report } = await baseline(clone, ["--base", base], { sandbox: origin.sandbox });
    expect(run.exitCode).toBe(2);
    expect(report.status).toBe("failed");
    expect(errorCodes(report)).toEqual(["baseline/baseline-inventory-failed"]);
    expect(report.delta).toBeNull();
    expect((await absent()).exitCode).not.toBe(0);
  });

  // No `rustup` on PATH: `rustup show active-toolchain` cannot answer, and
  // the base side would otherwise run under a toolchain nobody named.
  test("E13 an unresolvable active toolchain fails the baseline closed before any scan", async () => {
    const repo = await newFn("e13");
    const { run, report } = await baseline(repo.path, [], { sandbox: repo.sandbox, env: { PATH: pathWithout("rustup") } });
    expect(run.exitCode).toBe(2);
    expect(errorCodes(report)).toEqual(["baseline/baseline-scan-incomplete"]);
    expect(report.delta).toBeNull();
    expect(snapshots(repo.sandbox.tmp)).toEqual([]);
  });

  test("E14 a base blob past 64 MiB exceeds the snapshot's limit", async () => {
    const repo = await gitScenario("e14").template("mini").commit("base").build();
    writeFileSync(join(repo.path, "huge.bin"), Buffer.alloc(64 * 1024 * 1024 + 1));
    await repo.git(["add", "huge.bin"]);
    await repo.git(["commit", "-q", "-m", "huge"]);
    await repo.git(["switch", "-q", "-c", "feature"]);
    const { run, report } = await baseline(repo.path, [], { sandbox: repo.sandbox });
    expect(run.exitCode).toBe(2);
    expect(errorCodes(report)).toEqual(["baseline/baseline-limit-exceeded"]);
    expect(snapshots(repo.sandbox.tmp)).toEqual([]);
  });
});

describe("--max-duration", () => {
  // A warm target and side cache, so the measured run never nears the limit
  // on a loaded machine.
  test("F04 a fast baseline completes well inside --max-duration 1", async () => {
    const repo = await newFn("f04");
    const target = join(repo.sandbox.dir, "target");
    await baseline(repo.path, ["--base", "HEAD"], { sandbox: repo.sandbox, target });
    const { run, report } = await baseline(repo.path, ["--base", "HEAD", "--max-duration", "1"], { sandbox: repo.sandbox, target });
    expect(run.exitCode).toBe(0);
    expect(report.errors).toEqual([]);
  });

  // `docs/subsystems/execution.md`: a child still running at the deadline
  // is killed with `deadline-exceeded` at its own stage, as a full or staged
  // scan reports it.
  test.failing("L03 scopes/BUG-L a base side cut by --max-duration names the deadline", async () => {
    const repo = await slowRepo("l03");
    const { run, report } = await baseline(repo.path, ["--max-duration", "2"], { sandbox: repo.sandbox, env: repo.env });
    expect(run.exitCode).toBe(2);
    expect(snapshots(repo.sandbox.tmp)).toEqual([]);
    expect(report.errors.map((error) => error.code)).toContain("deadline-exceeded");
  });
});
