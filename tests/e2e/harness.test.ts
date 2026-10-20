// The helpers every other file trusts, proven on the real binary: what a
// missing build does, what the watchdog and the preload kill, what the sealed
// environment lets through, and what the fingerprint and the copy see.
import { describe, expect, test } from "bun:test";
import { copyFileSync, existsSync, mkdirSync, utimesSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import {
  copyWorkspace,
  hermeticEnv,
  NON_INTERACTIVE_MARKERS,
  normalizeReport,
  parseNonInteractiveMarkers,
  REPO_ROOT,
  runCommand,
  runDoctor,
  sandbox,
  spawnTracked,
  treeFingerprint,
  UNATTENDED_VARIABLES,
} from "./e2e-helpers";
import { fakeCalls, fakeEnv } from "./fakes";
import { runPty } from "./pty";

// The suite's own bun, run on another file of the suite with a sealed
// environment of its own.
function bunTest(file: string, env: Record<string, string>) {
  const box = sandbox("bun");
  return runCommand([process.execPath, "test", file], {
    sandbox: box,
    cwd: import.meta.dir,
    env: { ...env, BUN_INSTALL_CACHE_DIR: join(box.tmp, "bun-cache") },
  });
}

describe("binary resolution", () => {
  test("a missing binary fails every file at load with the build instruction", async () => {
    const run = await bunTest("./cli-surface.test.ts", { RUST_DOCTOR_E2E_BIN: "/nonexistent/rust-doctor" });
    expect(run.exitCode).not.toBe(0);
    expect(run.stdout + run.stderr).toContain(
      "rust-doctor binary not found at /nonexistent/rust-doctor: run cargo build --release first",
    );
  });
});

describe("process lifetime", () => {
  test("the watchdog kills a process that outlives it, and the wait fails naming it", async () => {
    const box = sandbox("watchdog");
    const tracked = spawnTracked(["sleep", "30"], { cwd: box.dir, env: hermeticEnv(box), timeoutMs: 200 });
    await expect(tracked.exited).rejects.toThrow(`watchdog killed sleep 30 (pid ${tracked.pid}) after 200 ms`);
    await tracked.proc.exited;
    expect(tracked.proc.signalCode).toBe("SIGKILL");
  });

  test("a process a test leaves running is killed and named when the test ends", async () => {
    const run = await bunTest("./probes/leaves-a-process.probe.ts", {});
    expect(run.exitCode).not.toBe(0);
    expect(run.stdout + run.stderr).toMatch(/killed a process the test left running: sleep 30 \(pid \d+\)/);
  });
});

describe("the sealed environment", () => {
  test("a spawned process sees the allowlist and nothing the developer's shell carries", async () => {
    const saved = { ...process.env };
    try {
      for (const name of UNATTENDED_VARIABLES) process.env[name] = "1";
      process.env.GIT_DIR = "/elsewhere";
      process.env.NO_COLOR = "1";
      const box = sandbox("env");
      const run = await runCommand(["env"], { sandbox: box });
      const names = run.stdout.trim().split("\n").map((line) => line.slice(0, line.indexOf("=")));
      expect(names.sort()).toEqual([
        "CARGO_HOME", "CARGO_NET_OFFLINE", "GIT_CONFIG_GLOBAL", "GIT_CONFIG_NOSYSTEM", "HOME",
        "LC_ALL", "PATH", "RUSTUP_HOME", "TERM", "TMPDIR",
      ]);
      expect(run.stdout).toContain(`HOME=${box.home}\n`);
      expect(run.stdout).toContain(`TMPDIR=${box.tmp}\n`);
      expect(run.stdout).toContain("GIT_CONFIG_GLOBAL=/dev/null\n");
      expect(run.stdout).toContain("CARGO_NET_OFFLINE=true\n");
    } finally {
      for (const name of Object.keys(process.env)) if (!(name in saved)) delete process.env[name];
      Object.assign(process.env, saved);
    }
  });

  test("PATH holds the toolchain and no agent, clipboard or browser", async () => {
    const run = await runCommand(["sh", "-c", 'for t in cargo git rust-doctor claude codex cursor-agent wl-copy xclip xdg-open gh git-lfs; do command -v "$t" >/dev/null && echo "$t"; done']);
    expect(run.stdout.trim().split("\n")).toEqual(["cargo", "git", "rust-doctor"]);
  });

  test("a marker set in the test process does not reach the binary", async () => {
    const saved = process.env.CLAUDECODE;
    process.env.CLAUDECODE = "1";
    try {
      const run = await runDoctor([copyWorkspace("kernel-contract/todo"), "--yes"]);
      expect(run.exitCode).toBe(0);
      expect(run.stderr).not.toContain("skill");
    } finally {
      if (saved === undefined) delete process.env.CLAUDECODE;
      else process.env.CLAUDECODE = saved;
    }
  });

  test("a test's override sets or removes one variable", () => {
    const env = hermeticEnv(sandbox("override"), { CI: "true", TERM: undefined });
    expect(env.CI).toBe("true");
    expect("TERM" in env).toBe(false);
  });

  test("the marker list is parsed from the source, and a source without it fails the load", () => {
    expect(NON_INTERACTIVE_MARKERS).toContain("CLAUDECODE");
    expect(NON_INTERACTIVE_MARKERS.length).toBeGreaterThan(3);
    expect(UNATTENDED_VARIABLES.at(-1)).toBe("CI");
    expect(() => parseNonInteractiveMarkers("const OTHER: [&str; 1] = [\"X\"];")).toThrow(
      "NON_INTERACTIVE_MARKERS not found",
    );
    expect(() => parseNonInteractiveMarkers("NON_INTERACTIVE_MARKERS: [&str; 0] = [\n// none\n];")).toThrow(
      "parsed to no marker",
    );
  });
});

describe("reports and trees", () => {
  test("scans default to --advisory-db /dev/null, and a test can opt out", async () => {
    const workspace = copyWorkspace("kernel-contract/todo");
    const sealed = await runDoctor([workspace, "--yes", "--json"]);
    expect(sealed.args.slice(-2)).toEqual(["--advisory-db", "/dev/null"]);
    const open = await runDoctor([workspace, "--yes", "--json"], { advisoryDb: false });
    expect(open.args).not.toContain("--advisory-db");
  });

  test("normalizeReport removes timings, toolchain and scan.advisory_db, and two scans then agree", async () => {
    const workspace = copyWorkspace("kernel-contract/todo");
    const first = await runDoctor([workspace, "--yes", "--json", "--timings"]);
    const second = await runDoctor([workspace, "--yes", "--json", "--timings"]);
    const raw = first.json as Record<string, unknown>;
    expect(raw.timings).toBeDefined();
    expect(raw.toolchain).toBeDefined();
    const normalized = normalizeReport(raw);
    expect(normalized.timings).toBeUndefined();
    expect(normalized.toolchain).toBeUndefined();
    expect((normalized.scan as Record<string, unknown>).advisory_db).toBeUndefined();
    const withDb: Record<string, unknown> = { scan: { advisory_db: { age_days: 3 }, exit_code: 0 } };
    expect(normalizeReport(withDb)).toEqual({ scan: { exit_code: 0 } });
    expect(normalized).toEqual(normalizeReport(second.json as Record<string, unknown>));
  });

  test("the fingerprint sees file bodies and the index's time, and ignores target/", () => {
    const dir = join(sandbox("fingerprint").dir, "repo");
    mkdirSync(join(dir, ".git"), { recursive: true });
    writeFileSync(join(dir, ".git", "index"), "index");
    writeFileSync(join(dir, "a.rs"), "fn a() {}\n");
    const base = treeFingerprint(dir);
    mkdirSync(join(dir, "target"));
    writeFileSync(join(dir, "target", "out"), "build");
    expect(treeFingerprint(dir)).toBe(base);
    utimesSync(join(dir, ".git", "index"), new Date(0), new Date(0));
    const touched = treeFingerprint(dir);
    expect(touched).not.toBe(base);
    writeFileSync(join(dir, "a.rs"), "fn b() {}\n");
    expect(treeFingerprint(dir)).not.toBe(touched);
  });

  test("copyWorkspace drops target/ and an untracked Cargo.lock", () => {
    const copy = copyWorkspace("kernel-contract/todo");
    expect(existsSync(join(copy, "Cargo.toml"))).toBe(true);
    expect(existsSync(join(copy, "src", "lib.rs"))).toBe(true);
    expect(existsSync(join(copy, "target"))).toBe(false);
    expect(existsSync(join(copy, "Cargo.lock"))).toBe(false);
    expect(copy.startsWith(REPO_ROOT)).toBe(false);
  });
});

describe("fake tools and PATH profiles", () => {
  const probe = 'for t in claude codex cursor-agent wl-copy xclip xsel pbcopy xdg-open open gh git-lfs; do command -v "$t" >/dev/null && echo "$t"; done';

  test.each([
    ["none", []],
    ["clipboard", ["wl-copy", "xclip", "xsel", "pbcopy"]],
    ["agents", ["claude", "codex", "cursor-agent"]],
    ["all", ["claude", "codex", "cursor-agent", "wl-copy", "xclip", "xsel", "pbcopy", "xdg-open", "open", "gh", "git-lfs"]],
  ] as const)("profile %s resolves its fakes and nothing of the machine", async (profile, expected) => {
    const box = sandbox(`profile-${profile}`);
    const run = await runCommand(["sh", "-c", probe], { sandbox: box, env: fakeEnv(box, profile) });
    expect(run.stdout.trim().split("\n").filter(Boolean)).toEqual([...expected]);
    for (const name of expected) {
      const resolved = await runCommand(["sh", "-c", `readlink -f "$(command -v ${name})"`], { sandbox: box, env: fakeEnv(box, profile) });
      expect(resolved.stdout.trim()).toBe(join(import.meta.dir, "fakes", "record.sh"));
    }
  });

  test("a fake records argv, stdin, cwd and the terminal, and fails or hangs on demand", async () => {
    const box = sandbox("fake-calls");
    const env = fakeEnv(box, "all");
    await runCommand(["sh", "-c", "printf 'clip body' | wl-copy --type text/plain"], { sandbox: box, env });
    const failing = await runCommand(["xdg-open", "https://example.invalid/a b"], {
      sandbox: box,
      env: { ...env, RD_FAKE_XDG_OPEN_EXIT: "3" },
    });
    expect(failing.exitCode).toBe(3);
    const agent = await runPty(["claude", "--print", "prompt text"], { sandbox: box, env });
    expect(agent.exitCode).toBe(0);
    expect(fakeCalls(box, "wl-copy")).toEqual([{ argv: ["--type", "text/plain"], stdin: "clip body", cwd: box.dir, tty: false }]);
    expect(fakeCalls(box, "xdg-open")).toEqual([{ argv: ["https://example.invalid/a b"], stdin: "", cwd: box.dir, tty: false }]);
    expect(fakeCalls(box, "claude")).toEqual([{ argv: ["--print", "prompt text"], stdin: "", cwd: box.dir, tty: true }]);
    expect(fakeCalls(box, "codex")).toEqual([]);
  });
});

describe("documentation gate", () => {
  // Built at runtime: the gate reads this file too, and a literal citation of
  // a document that does not exist would fail it on this repository.
  const doc = (name: string) => ["docs", `${name}.md`].join("/");

  test("verify-doc-refs.sh fails a document that cites a path that does not exist", async () => {
    const box = sandbox("doc-refs");
    const root = join(box.dir, "repo");
    mkdirSync(join(root, "scripts"), { recursive: true });
    mkdirSync(join(root, "docs"));
    copyFileSync(join(REPO_ROOT, "scripts/verify-doc-refs.sh"), join(root, "scripts/verify-doc-refs.sh"));
    // The script cites the document that holds its reasoning.
    writeFileSync(join(root, doc("doc-gates")), "# Gates\n");
    writeFileSync(join(root, doc("guide")), `# Guide\n\nThe reasoning lives in ${doc("missing")}.\n`);
    expect((await runCommand(["git", "init", "-q"], { sandbox: box, cwd: root })).exitCode).toBe(0);
    const broken = await runCommand(["bash", "scripts/verify-doc-refs.sh"], { sandbox: box, cwd: root });
    expect(broken.exitCode).toBe(1);
    expect(broken.stderr).toContain(`FAIL ${doc("guide")}:3 names \`${doc("missing")}\`, which is not on disk`);
    writeFileSync(join(root, doc("missing")), "# Missing\n");
    const fixed = await runCommand(["bash", "scripts/verify-doc-refs.sh"], { sandbox: box, cwd: root });
    expect([fixed.exitCode, fixed.stderr]).toEqual([0, ""]);
  });
});
