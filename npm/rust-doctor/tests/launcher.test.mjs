import { afterEach, describe, expect, test } from "bun:test";
import { spawn, spawnSync } from "node:child_process";
import { EventEmitter } from "node:events";
import {
  chmodSync,
  cpSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { constants, tmpdir } from "node:os";
import { join } from "node:path";

import {
  PLATFORM_PACKAGES,
  launchNative,
  resolveNativeBinary,
  signalExitCode,
} from "../lib/launcher.js";
import { packageRoot } from "../scripts/pack-local.mjs";

const roots = [];

// The launcher refuses a native package whose version differs from the
// wrapper's, so the fixtures state the shipped version rather than a literal
// that a release bump would silently invalidate.
const wrapperVersion = JSON.parse(
  readFileSync(join(packageRoot, "package.json"), "utf8"),
).version;

function fixture(name, {
  wrapperPath = "node_modules/rust-doctor",
  nativePath = "node_modules/@rustdoctor/linux-x64",
} = {}) {
  const root = mkdtempSync(join(tmpdir(), `rust-doctor-wrapper-${name}-`));
  roots.push(root);
  const wrapper = join(root, wrapperPath);
  mkdirSync(wrapper, { recursive: true });
  cpSync(join(packageRoot, "package.json"), join(wrapper, "package.json"));
  cpSync(join(packageRoot, "bin"), join(wrapper, "bin"), { recursive: true });
  cpSync(join(packageRoot, "lib"), join(wrapper, "lib"), { recursive: true });
  return {
    root,
    wrapper,
    command: join(wrapper, "bin/rust-doctor.js"),
    native: join(root, nativePath),
  };
}

function installNative(
  fixture_,
  source,
  { mode = 0o755, version = wrapperVersion, os = "linux", cpu = "x64" } = {},
) {
  const bin = join(fixture_.native, "bin");
  mkdirSync(bin, { recursive: true });
  writeFileSync(
    join(fixture_.native, "package.json"),
    `${JSON.stringify({ name: "@rustdoctor/linux-x64", version, os: [os], cpu: [cpu] })}\n`,
  );
  const binary = join(bin, "rust-doctor");
  writeFileSync(binary, source);
  chmodSync(binary, mode);
  return binary;
}

function run(fixture_, arguments_ = [], options = {}) {
  return spawnSync("node", [fixture_.command, ...arguments_], {
    cwd: options.cwd ?? fixture_.root,
    env: { ...process.env, ...options.env },
    input: options.input,
    encoding: "utf8",
  });
}

async function waitFor(path) {
  const deadline = Date.now() + 5000;
  while (!existsSync(path)) {
    if (Date.now() >= deadline) throw new Error(`timed out waiting for ${path}`);
    await Bun.sleep(10);
  }
}

afterEach(() => {
  for (const root of roots.splice(0)) {
    rmSync(root, { recursive: true, force: true });
  }
});

describe("native launcher", () => {
  test("forwards argv, cwd, environment, stdin, stdout and stderr without a shell", () => {
    const fixture_ = fixture("fidelity");
    installNative(
      fixture_,
      `#!/usr/bin/env node
let input = "";
process.stdin.setEncoding("utf8");
process.stdin.on("data", (chunk) => { input += chunk; });
process.stdin.on("end", () => {
  process.stdout.write(JSON.stringify({ argv: process.argv.slice(2), cwd: process.cwd(), marker: process.env.RD_SAFE_MARKER, input }));
  process.stderr.write("child-stderr");
});
`,
    );
    const cwd = join(fixture_.root, "working directory");
    mkdirSync(cwd);
    const arguments_ = ["with space", "Unicode-été", "--dash", "'single'", '"double"', "./inspect"];
    const output = run(fixture_, arguments_, {
      cwd,
      env: { RD_SAFE_MARKER: "inherited" },
      input: "inherited stdin",
    });
    expect(output.status).toBe(0);
    expect(output.stderr).toBe("child-stderr");
    expect(JSON.parse(output.stdout)).toEqual({
      argv: arguments_,
      cwd,
      marker: "inherited",
      input: "inherited stdin",
    });
  });

  test("preserves child exit codes", () => {
    for (const code of [0, 1, 2, 127]) {
      const fixture_ = fixture(`exit-${code}`);
      installNative(fixture_, `#!/bin/sh\nexit ${code}\n`);
      expect(run(fixture_).status).toBe(code);
    }
  });

  for (const signal of ["SIGINT", "SIGTERM"]) {
    test(`forwards ${signal}, waits for the child and terminates by the same signal`, async () => {
      const fixture_ = fixture(signal.toLowerCase());
      const ready = join(fixture_.root, "ready");
      const observed = join(fixture_.root, "observed");
      installNative(
        fixture_,
        `#!/usr/bin/env node
import { writeFileSync } from "node:fs";
writeFileSync(process.env.RD_READY, "ready");
const signal = process.env.RD_SIGNAL;
process.on(signal, () => {
  writeFileSync(process.env.RD_OBSERVED, signal);
  process.removeAllListeners(signal);
  process.kill(process.pid, signal);
});
setInterval(() => {}, 1000);
`,
      );
      const child = spawn("node", [fixture_.command], {
        cwd: fixture_.root,
        env: { ...process.env, RD_READY: ready, RD_OBSERVED: observed, RD_SIGNAL: signal },
        stdio: "ignore",
      });
      await waitFor(ready);
      child.kill(signal);
      const outcome = await new Promise((resolve) => {
        child.once("close", (code, closedBy) => resolve({ code, signal: closedBy }));
      });
      expect(readFileSync(observed, "utf8")).toBe(signal);
      expect(outcome).toEqual({ code: null, signal });
    }, 10000);
  }

  test("uses a bounded non-POSIX fallback and waits for close", async () => {
    expect(signalExitCode("SIGINT")).toBe(130);
    expect(signalExitCode("SIGTERM")).toBe(143);
    expect(signalExitCode("NOT_A_SIGNAL")).toBe(1);

    for (const signal of ["SIGINT", "SIGTERM"]) {
      const signals = new EventEmitter();
      const child = new EventEmitter();
      const forwarded = [];
      child.kill = (forwardedSignal) => {
        forwarded.push(forwardedSignal);
        return true;
      };
      let settled = false;
      const outcome = launchNative("unused", [], {
        platform: "win32",
        signals,
        spawnChild: () => child,
      });
      outcome.then(() => {
        settled = true;
      });

      signals.emit(signal);
      await Promise.resolve();
      expect(forwarded).toEqual([undefined]);
      expect(settled).toBeFalse();
      child.emit("close", null, null);
      expect(await outcome).toEqual({
        code: signalExitCode(signal),
        error: undefined,
        signal,
      });
      expect(signals.listenerCount("SIGINT")).toBe(0);
      expect(signals.listenerCount("SIGTERM")).toBe(0);
    }
  });

  test("fails closed for unsupported, absent, mismatched and non-executable packages", () => {
    expect(() => resolveNativeBinary({ platform: "freebsd", arch: "riscv64" }))
      .toThrow("does not yet ship a binary for freebsd-riscv64");

    const absent = fixture("absent");
    const absentOutput = run(absent);
    expect(absentOutput.status).toBe(1);
    expect(absentOutput.stdout).toBe("");
    expect(absentOutput.stderr).toContain("@rustdoctor/linux-x64 is not installed");
    expect(Buffer.byteLength(absentOutput.stderr)).toBeLessThan(1024);

    const mismatch = fixture("mismatch");
    installNative(mismatch, "#!/bin/sh\nexit 0\n", { version: "9.9.9" });
    expect(run(mismatch).stderr).toContain(`must match rust-doctor ${wrapperVersion}`);

    const mode = fixture("mode");
    installNative(mode, "#!/bin/sh\nexit 0\n", { mode: 0o644 });
    expect(run(mode).stderr).toContain("does not contain an executable rust-doctor");

    const wrongPlatform = fixture("wrong-platform");
    installNative(wrongPlatform, "#!/bin/sh\nexit 0\n", { cpu: "arm64" });
    expect(run(wrongPlatform).stderr).toContain("does not match linux-x64");
  });

  // EP-004 US-015: the native package is looked up beside the wrapper only,
  // never in an ancestor `node_modules` the install did not write.
  test("resolves the native package in the nested, flat and pnpm layouts", () => {
    const layouts = {
      flat: {},
      nested: { nativePath: "node_modules/rust-doctor/node_modules/@rustdoctor/linux-x64" },
      pnpm: {
        wrapperPath: "node_modules/.pnpm/rust-doctor@0.0.0/node_modules/rust-doctor",
        nativePath: "node_modules/.pnpm/rust-doctor@0.0.0/node_modules/@rustdoctor/linux-x64",
      },
    };
    for (const [name, layout] of Object.entries(layouts)) {
      const fixture_ = fixture(`layout-${name}`, layout);
      installNative(fixture_, "#!/bin/sh\nprintf resolved\n");
      const output = run(fixture_);
      expect(output.stderr).toBe("");
      expect(output.status).toBe(0);
      expect(output.stdout).toBe("resolved");
    }
  });

  test("never executes a same-named native package from an ancestor node_modules", () => {
    const ancestor = fixture("ancestor-only", {
      wrapperPath: "project/node_modules/rust-doctor",
      nativePath: "node_modules/@rustdoctor/linux-x64",
    });
    const marker = join(ancestor.root, "ancestor-ran");
    installNative(ancestor, `#!/bin/sh\ntouch "${marker}"\n`);
    const output = run(ancestor);
    expect(output.status).toBe(1);
    expect(output.stderr).toContain("@rustdoctor/linux-x64 is not installed");
    expect(existsSync(marker)).toBeFalse();

    // A sibling manifest selects the sibling root: its missing binary is
    // never completed from the ancestor's.
    const incomplete = fixture("sibling-incomplete", {
      wrapperPath: "project/node_modules/rust-doctor",
      nativePath: "node_modules/@rustdoctor/linux-x64",
    });
    const incompleteMarker = join(incomplete.root, "ancestor-ran");
    installNative(incomplete, `#!/bin/sh\ntouch "${incompleteMarker}"\n`);
    const sibling = join(incomplete.root, "project/node_modules/@rustdoctor/linux-x64");
    mkdirSync(sibling, { recursive: true });
    writeFileSync(
      join(sibling, "package.json"),
      `${JSON.stringify({ name: "@rustdoctor/linux-x64", version: wrapperVersion, os: ["linux"], cpu: ["x64"] })}\n`,
    );
    const incompleteOutput = run(incomplete);
    expect(incompleteOutput.status).toBe(1);
    expect(incompleteOutput.stderr).toContain("@rustdoctor/linux-x64 is incomplete");
    expect(existsSync(incompleteMarker)).toBeFalse();
  });

  test("spawn failure is bounded and action-oriented", () => {
    const fixture_ = fixture("spawn-failure");
    installNative(fixture_, "#!/path/that/does/not/exist\n");
    const output = run(fixture_);
    expect(output.status).toBe(1);
    expect(output.stdout).toBe("");
    expect(output.stderr).toContain("Native binary could not start (ENOENT)");
    expect(Buffer.byteLength(output.stderr)).toBeLessThan(1024);
  });

  test("source contains no command interpreter or dynamic execution path", () => {
    const launcher = readFileSync(join(packageRoot, "lib/launcher.js"), "utf8");
    const bin = readFileSync(join(packageRoot, "bin/rust-doctor.js"), "utf8");
    expect(launcher).toContain('stdio: "inherit"');
    expect(launcher).toContain("shell: false");
    expect(launcher).not.toMatch(/\b(?:exec|execFile|spawnSync|eval)\s*\(/u);
    expect(bin).not.toMatch(/\b(?:exec|execFile|spawnSync|eval)\s*\(/u);
    expect(Object.keys(PLATFORM_PACKAGES)).toEqual([
      "darwin-x64",
      "darwin-arm64",
      "linux-x64",
      "linux-arm64",
      "win32-x64",
    ]);
  });
});

// EP-002 US-010: the built binary behind the real launcher. A terminal's
// Ctrl-C reaches the whole foreground process group: the launcher forwards it
// and the binary receives it from the terminal too, twice within a few
// milliseconds. The binary has to read that as one delivery and still publish
// its report. `RUST_DOCTOR_BINARY` names the binary, else the newest of
// `target/release` and `target/debug`; the launcher job builds neither, so
// there the test says it was skipped rather than passing on nothing.
const repositoryRoot = join(packageRoot, "../..");
const builtBinary = [
  process.env.RUST_DOCTOR_BINARY,
  join(repositoryRoot, "target/release/rust-doctor"),
  join(repositoryRoot, "target/debug/rust-doctor"),
]
  .filter((path) => path && existsSync(path))
  .sort((left, right) => statSync(right).mtimeMs - statSync(left).mtimeMs)[0];
const unixWithBinary = process.platform !== "win32" && builtBinary !== undefined;

function git(cwd, ...arguments_) {
  const result = spawnSync("git", arguments_, { cwd, encoding: "utf8" });
  if (result.status !== 0) throw new Error(`git ${arguments_.join(" ")}: ${result.stderr}`);
}

describe.skipIf(!unixWithBinary)("built binary behind the launcher", () => {
  for (const scope of [[], ["--scope", "baseline", "--base", "HEAD"]]) {
    const name = scope.length === 0 ? "full" : "baseline";
    test(`a group SIGINT during Clippy ends a ${name} scan with its interrupted report`, async () => {
      const fixture_ = fixture(`group-sigint-${name}`);
      const bin = join(fixture_.native, "bin");
      mkdirSync(bin, { recursive: true });
      writeFileSync(
        join(fixture_.native, "package.json"),
        `${JSON.stringify({ name: "@rustdoctor/linux-x64", version: wrapperVersion, os: ["linux"], cpu: ["x64"] })}\n`,
      );
      symlinkSync(builtBinary, join(bin, "rust-doctor"));
      const workspace = join(fixture_.root, "workspace");
      cpSync(join(repositoryRoot, "tests/fixtures/scan-failures/sleeping-build"), workspace, { recursive: true });
      git(workspace, "init", "-q");
      git(workspace, "add", "-A");
      git(workspace, "-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false", "commit", "-q", "-m", "base");
      const temporary = join(fixture_.root, "tmp");
      mkdirSync(temporary);
      const ready = join(fixture_.root, "sleeper.pid");
      const child = spawn("node", [fixture_.command, workspace, "--yes", "--json", "--advisory-db", "/dev/null", ...scope], {
        cwd: fixture_.root,
        detached: true,
        env: {
          ...process.env,
          CARGO_NET_OFFLINE: "true",
          CARGO_TARGET_DIR: join(fixture_.root, "target"),
          RUST_DOCTOR_SLEEPER_PID: ready,
          TMPDIR: temporary,
        },
        stdio: ["ignore", "pipe", "pipe"],
      });
      let stdout = "";
      child.stdout.setEncoding("utf8");
      child.stdout.on("data", (chunk) => { stdout += chunk; });
      const closed = new Promise((resolve) => {
        child.once("close", (code, signal) => resolve({ code, signal }));
      });
      const deadline = Date.now() + 120000;
      while (!existsSync(ready)) {
        if (Date.now() >= deadline) throw new Error("the build script never ran");
        await Bun.sleep(50);
      }
      process.kill(-child.pid, "SIGINT");
      const outcome = await closed;
      // What a shell reports: the code, or 128 plus the signal the launcher
      // re-raised on itself after the binary exited.
      const status = outcome.code ?? 128 + constants.signals[outcome.signal];
      expect(status).toBe(130);
      const report = JSON.parse(stdout);
      expect(report.status).toBe("failed");
      expect(report.errors.map((error) => error.code)).toEqual(["interrupted"]);
      expect(report.audit.score).toBeNull();
      expect(readdirSync(temporary).filter((entry) => entry.startsWith("rust-doctor-baseline-"))).toEqual([]);
    }, 180000);
  }
});
