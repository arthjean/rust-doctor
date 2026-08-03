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
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import {
  PLATFORM_PACKAGES,
  launchNative,
  resolveNativeBinary,
  signalExitCode,
} from "../lib/launcher.js";
import { packageRoot } from "../scripts/pack-local.mjs";

const roots = [];

function fixture(name) {
  const root = mkdtempSync(join(tmpdir(), `rust-doctor-wrapper-${name}-`));
  roots.push(root);
  const wrapper = join(root, "node_modules/rust-doctor");
  mkdirSync(wrapper, { recursive: true });
  cpSync(join(packageRoot, "package.json"), join(wrapper, "package.json"));
  cpSync(join(packageRoot, "bin"), join(wrapper, "bin"), { recursive: true });
  cpSync(join(packageRoot, "lib"), join(wrapper, "lib"), { recursive: true });
  return {
    root,
    wrapper,
    command: join(wrapper, "bin/rust-doctor.js"),
    native: join(root, "node_modules/@rust-doctor/linux-x64"),
  };
}

function installNative(
  fixture_,
  source,
  { mode = 0o755, version = "0.1.0", os = "linux", cpu = "x64" } = {},
) {
  const bin = join(fixture_.native, "bin");
  mkdirSync(bin, { recursive: true });
  writeFileSync(
    join(fixture_.native, "package.json"),
    `${JSON.stringify({ name: "@rust-doctor/linux-x64", version, os: [os], cpu: [cpu] })}\n`,
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
    expect(absentOutput.stderr).toContain("@rust-doctor/linux-x64 is not installed");
    expect(Buffer.byteLength(absentOutput.stderr)).toBeLessThan(1024);

    const mismatch = fixture("mismatch");
    installNative(mismatch, "#!/bin/sh\nexit 0\n", { version: "9.9.9" });
    expect(run(mismatch).stderr).toContain("must match rust-doctor 0.1.0");

    const mode = fixture("mode");
    installNative(mode, "#!/bin/sh\nexit 0\n", { mode: 0o644 });
    expect(run(mode).stderr).toContain("does not contain an executable rust-doctor");

    const wrongPlatform = fixture("wrong-platform");
    installNative(wrongPlatform, "#!/bin/sh\nexit 0\n", { cpu: "arm64" });
    expect(run(wrongPlatform).stderr).toContain("does not match linux-x64");
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
