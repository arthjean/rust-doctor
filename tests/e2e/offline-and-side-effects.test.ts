// `tasks/e2e-catalog/failures.md` sections NW and FS: the scan needs no
// network, and writes nothing outside the workspace's target directory and
// its own temporary root. The network rows run the binary inside a namespace
// with no interface (netns.ts) or under strace; the filesystem rows list
// HOME, TMPDIR and a private CARGO_HOME before and after.
import { describe, expect, test } from "bun:test";
import { existsSync, mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { join, relative } from "node:path";
import {
  BIN,
  copyWorkspace,
  doctorArgs,
  hermeticEnv,
  normalizeReport,
  runCommand,
  runDoctor,
  sandbox,
  scratchDir,
  spawnDoctor,
  spawnTracked,
  targetDir,
  treeFingerprint,
} from "./e2e-helpers";
import { fakeCalls, fakeEnv } from "./fakes";
import { gitScenario, miniTemplate, NEW_FN, waitForSnapshot } from "./git-scenario";
import { requireNetns, runDoctorOffline } from "./netns";
import { gone, slowRepo, snapshots, waitForBuildScript } from "./scope-snapshot-support";

const STRACE = Bun.which("strace", { PATH: "/usr/bin:/bin:/usr/sbin:/sbin" });
if (!STRACE) console.warn("strace is not installed: the socket trace skipped");

type Report = {
  status: string;
  errors: Array<{ stage: string; code: string; message: string }>;
  delta?: { summary: { introduced: number; fixed: number } };
};

// Every path under `dir`, relative, directories marked with a trailing slash.
function listing(dir: string): string[] {
  if (!existsSync(dir)) return [];
  const out: string[] = [];
  const walk = (current: string) => {
    for (const name of readdirSync(current).sort()) {
      const path = join(current, name);
      const directory = statSync(path).isDirectory();
      out.push(relative(dir, path) + (directory ? "/" : ""));
      if (directory) walk(path);
    }
  };
  walk(dir);
  return out;
}

// A build script that holds Cargo until the test creates `$E2E_GATE`, so a
// baseline run's snapshot can be looked at before the run ends normally.
const GATED_BUILD = `fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    if let Ok(gate) = std::env::var("E2E_GATE") {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !std::path::Path::new(&gate).exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}
`;

describe("network", () => {
  test.skipIf(!requireNetns()).each(["projects/clean", "projects/clippy-warning"])(
    "NW-01 %s scans inside a network namespace exactly as outside it",
    async (fixture) => {
      const workspace = copyWorkspace(fixture);
      const target = targetDir(fixture.replace("/", "-"));
      const plain = await runDoctor([workspace, "--yes", "--json"], { target });
      const offline = await runDoctorOffline([workspace, "--yes", "--json"], { target });
      expect(offline.exitCode).toBe(plain.exitCode);
      expect((offline.json as Report).status).toBe("complete");
      expect(normalizeReport(offline.json)).toEqual(normalizeReport(plain.json));
    },
  );

  // E2E_REQUIRE_NETNS=1 requires the socket trace with the namespaces.
  test.skipIf(!STRACE && process.env.E2E_REQUIRE_NETNS !== "1")("NW-02 no process of the scan opens a socket", async () => {
    if (!STRACE) throw new Error("strace is not installed and E2E_REQUIRE_NETNS=1 requires the socket trace");
    const workspace = copyWorkspace("projects/clippy-warning");
    const box = sandbox("nw-02");
    const log = join(box.dir, "strace.log");
    const target = targetDir("projects-clippy-warning");
    const args = doctorArgs([workspace, "--yes", "--json"]);
    const run = await spawnTracked([STRACE!, "-f", "-qq", "-e", "trace=socket,connect,sendto", "-o", log, BIN, ...args], {
      cwd: box.dir,
      env: hermeticEnv(box, { CARGO_TARGET_DIR: target }),
    }).exited;
    expect(run.exitCode).toBe(0);
    expect((JSON.parse(run.stdout) as Report).status).toBe("complete");
    const calls = readFileSync(log, "utf8").split("\n").filter((line) => /\b(socket|connect|sendto)\(/.test(line));
    expect(calls).toEqual([]);
  });

  test("NW-03 with CARGO_NET_OFFLINE=true an uncached dependency leaves the package unlinted at once, quoting Cargo's offline refusal", async () => {
    const dir = join(scratchDir("nw-03"), "needs-itoa");
    mkdirSync(join(dir, "src"), { recursive: true });
    writeFileSync(join(dir, "Cargo.toml"), '[package]\nname = "needs-itoa"\nversion = "0.1.0"\nedition = "2024"\npublish = false\n\n[dependencies]\nitoa = "1"\n\n[workspace]\n');
    writeFileSync(join(dir, "src/lib.rs"), "pub fn n() -> u8 {\n    1\n}\n");
    const cargoHome = join(scratchDir("nw-03-cargo-home"), "cargo");
    mkdirSync(cargoHome);
    const runs = [await runDoctor([dir, "--yes", "--json"], { env: { CARGO_HOME: cargoHome }, timeoutMs: 30_000 })];
    if (requireNetns()) runs.push(await runDoctorOffline([dir, "--yes", "--json"], { env: { CARGO_HOME: cargoHome }, timeoutMs: 30_000 }));
    for (const run of runs) {
      expect(run.exitCode).toBe(1);
      expect(run.durationMs).toBeLessThan(15_000);
      const report = run.json as Report & { audit: { score: unknown } };
      expect(report.status).toBe("incomplete");
      expect(report.audit.score).toBeNull();
      expect(report.errors.map((error) => error.code)).toContain("packages-unlinted");
      const message = report.errors.map((error) => error.message).join("\n");
      expect(message).toContain("offline");
      expect(message).toContain("itoa");
      expect(message).not.toContain("spurious network error");
    }
    expect(existsSync(join(dir, "Cargo.lock"))).toBe(false);
  });

  test.skipIf(!requireNetns())("NW-04 RUSTUP_AUTO_INSTALL=0 refuses an uninstalled pinned toolchain without a download", async () => {
    const dir = join(scratchDir("nw-04"), "pinned");
    mkdirSync(join(dir, "src"), { recursive: true });
    writeFileSync(join(dir, "Cargo.toml"), '[package]\nname = "pinned"\nversion = "0.1.0"\nedition = "2024"\npublish = false\n\n[workspace]\n');
    writeFileSync(join(dir, "src/lib.rs"), "");
    writeFileSync(join(dir, "rust-toolchain.toml"), '[toolchain]\nchannel = "1.80.0"\n');
    const rustupHome = join(scratchDir("nw-04-rustup"), "rustup");
    mkdirSync(rustupHome);
    const run = await runDoctorOffline([dir, "--yes", "--json"], { env: { RUSTUP_HOME: rustupHome, RUSTUP_AUTO_INSTALL: "0" }, timeoutMs: 30_000 });
    expect(run.exitCode).toBe(2);
    const message = (run.json as Report).errors.map((error) => error.message).join("\n");
    expect(message).toContain("'1.80.0-");
    expect(message).toContain("is not installed");
    expect(message).not.toContain("syncing channel updates");
  });

  test("NW-05 no scope runs git-lfs, the filter that would fetch over the network", async () => {
    const repo = await gitScenario("nw-05")
      .template("mini")
      .write({ ".gitattributes": "*.rs filter=lfs diff=lfs merge=lfs\n" })
      .commit("base")
      .build();
    await repo.git(["config", "filter.lfs.clean", "git-lfs clean -- %f"]);
    await repo.git(["config", "filter.lfs.smudge", "git-lfs smudge -- %f"]);
    await repo.git(["config", "filter.lfs.required", "true"]);
    writeFileSync(join(repo.path, "src/lib.rs"), miniTemplate()["src/lib.rs"]! + NEW_FN);
    const env = fakeEnv(repo.sandbox, ["git-lfs"]);
    const control = await runCommand(["git", "cat-file", "--filters", "HEAD:src/lib.rs"], { cwd: repo.path, sandbox: repo.sandbox, env });
    expect(control.exitCode).toBe(0);
    expect(fakeCalls(repo.sandbox, "git-lfs")).toHaveLength(1);
    for (const args of [
      [],
      ["--scope", "files", "--base", "HEAD"],
      ["--scope", "lines", "--base", "HEAD"],
      ["--scope", "baseline", "--base", "HEAD"],
      ["--scope", "baseline", "--base", "HEAD", "--staged"],
    ]) {
      const run = await runDoctor([repo.path, "--yes", "--json", ...args], { sandbox: repo.sandbox, env: { ...env, RUST_DOCTOR_NO_CACHE: "1" } });
      expect([0, 1]).toContain(run.exitCode!);
      expect((run.json as Report).errors.filter((error) => error.stage === "baseline" || error.stage === "scope")).toEqual([]);
    }
    expect(fakeCalls(repo.sandbox, "git-lfs")).toHaveLength(1);
  });
});

describe("filesystem side effects", () => {
  // A private CARGO_HOME the test can list, beside the machine's toolchain.
  function privateCargoHome(): string {
    const dir = join(scratchDir("cargo-home"), "cargo");
    mkdirSync(dir);
    return dir;
  }

  test("FS-01 a full scan leaves HOME, TMPDIR and the workspace as it found them, and CARGO_HOME holds only Cargo's caches", async () => {
    const repo = await gitScenario("fs-01").template("mini").commit("base").build();
    const cargoHome = privateCargoHome();
    const home = treeFingerprint(repo.sandbox.home);
    const workspace = treeFingerprint(repo.path);
    const run = await runDoctor([repo.path, "--yes", "--json"], { sandbox: repo.sandbox, env: { CARGO_HOME: cargoHome } });
    expect(run.exitCode).toBe(0);
    expect(treeFingerprint(repo.sandbox.home)).toBe(home);
    expect(treeFingerprint(repo.path)).toBe(workspace);
    expect(existsSync(join(repo.path, "target"))).toBe(false);
    expect(listing(repo.sandbox.tmp)).toEqual([]);
    for (const entry of listing(cargoHome)) {
      expect([".global-cache", ".package-cache", ".package-cache-mutate"]).toContain(entry);
    }
  });

  test("FS-02 a baseline run's snapshot lives in TMPDIR, private, while it runs, and its caches under the target directory", async () => {
    const repo = await gitScenario("fs-02")
      .template("mini")
      .write({ "build.rs": GATED_BUILD })
      .commit("base")
      .append("src/lib.rs", NEW_FN)
      .build();
    const gate = join(repo.sandbox.dir, "gate");
    const target = repo.target("fs-02");
    const before = treeFingerprint(repo.path);
    const spawned = spawnDoctor([repo.path, "--yes", "--json", "--scope", "baseline", "--base", "HEAD"], {
      sandbox: repo.sandbox,
      target,
      env: { E2E_GATE: gate, RUST_DOCTOR_NO_CACHE: "1" },
    });
    const snapshot = await waitForSnapshot(repo.sandbox.tmp);
    expect(snapshot).toMatch(new RegExp(`/rust-doctor-baseline-${spawned.pid}-\\d+$`));
    expect(statSync(snapshot).mode & 0o777).toBe(0o700);
    const deadline = Date.now() + 10_000;
    while (!existsSync(join(snapshot, "tree", "src", "lib.rs")) && Date.now() < deadline) await Bun.sleep(25);
    expect(readdirSync(snapshot)).toContain("tree");
    writeFileSync(gate, "");
    const run = await spawned.result;
    expect(run.exitCode).toBe(0);
    expect((run.json as Report).delta?.summary.introduced).toBe(1);
    expect(listing(repo.sandbox.tmp)).toEqual([]);
    expect(existsSync(join(target, "rust-doctor", "baseline"))).toBe(true);
    expect(treeFingerprint(repo.path)).toBe(before);
  });

  test("FS-03 a baseline answered from the side cache equals one rescanned without it", async () => {
    const repo = await gitScenario("fs-03").template("mini").commit("base").append("src/lib.rs", NEW_FN).build();
    const target = repo.target("fs-03");
    const args = [repo.path, "--yes", "--json", "--scope", "baseline", "--base", "HEAD"];
    const first = await runDoctor(args, { sandbox: repo.sandbox, target });
    expect(existsSync(join(target, "rust-doctor", "cache", "base-sides.json"))).toBe(true);
    const cached = await runDoctor(args, { sandbox: repo.sandbox, target });
    const rescanned = await runDoctor(args, { sandbox: repo.sandbox, target, env: { RUST_DOCTOR_NO_CACHE: "1" } });
    expect(first.exitCode).toBe(0);
    expect(normalizeReport(cached.json)).toEqual(normalizeReport(first.json));
    expect(normalizeReport(rescanned.json)).toEqual(normalizeReport(first.json));
  });

  test("FS-04 a workspace without a lockfile has none after the scan, and two scans agree", async () => {
    const workspace = join(scratchDir("fs-04"), "lockless");
    mkdirSync(join(workspace, "src"), { recursive: true });
    writeFileSync(join(workspace, "Cargo.toml"), '[package]\nname = "lockless"\nversion = "0.1.0"\nedition = "2024"\npublish = false\n\n[workspace]\n');
    writeFileSync(join(workspace, "src/main.rs"), "fn main() {}\n");
    const before = treeFingerprint(workspace);
    const target = targetDir("fs-04");
    const first = await runDoctor([workspace, "--yes", "--json"], { target });
    expect(existsSync(join(workspace, "Cargo.lock"))).toBe(false);
    expect(treeFingerprint(workspace)).toBe(before);
    const second = await runDoctor([workspace, "--yes", "--json"], { target });
    expect(normalizeReport(second.json)).toEqual(normalizeReport(first.json));
  });

  test("FS-05 without CARGO_TARGET_DIR the build lands in the workspace's target/, and nothing else changes", async () => {
    const workspace = copyWorkspace("projects/clean");
    const before = treeFingerprint(workspace);
    const run = await runDoctor([workspace, "--yes", "--json"], { env: { CARGO_TARGET_DIR: undefined } });
    expect(run.exitCode).toBe(0);
    expect(existsSync(join(workspace, "target"))).toBe(true);
    expect(treeFingerprint(workspace)).toBe(before);
  });

  test("FS-06 a SIGHUP during a baseline removes the snapshot and exits 130", async () => {
    const repo = await slowRepo("fs-06");
    const spawned = spawnDoctor([repo.path, "--yes", "--json", "--scope", "baseline"], { sandbox: repo.sandbox, env: repo.env });
    await waitForSnapshot(repo.sandbox.tmp);
    const builds = await waitForBuildScript(repo);
    spawned.proc.kill("SIGHUP");
    const run = await spawned.result;
    expect(run.exitCode).toBe(130);
    expect(snapshots(repo.sandbox.tmp)).toEqual([]);
    expect(await gone(builds)).toEqual([]);
  });
});
