// `tasks/e2e-catalog/failures.md` section TC: toolchain problems. Each row
// builds the toolchain it needs out of what is installed: a PATH directory of
// links into one toolchain's own binaries, or a scratch RUSTUP_HOME, so a
// missing cargo, rustc or Clippy is a fact of the test and not of the machine.
import { describe, expect, test } from "bun:test";
import { chmodSync, existsSync, mkdirSync, readdirSync, symlinkSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { CARGO_HOME, copyWorkspace, E2E_BASE, pathWithout, runDoctor, RUSTUP_HOME, scratchDir, toolchainBin } from "./e2e-helpers";
import { requireNetns, runDoctorOffline } from "./netns";

type ScanError = { stage: string; code: string; message: string };
type Report = {
  status: string;
  audit: { score: { value: number } | null; packages: Array<{ name: string; unlinted?: boolean; score: unknown }> };
  policy: { rules: Array<{ id: string; not_evaluated?: string }> } | null;
  toolchain: { rustc: string | null; cargo: string | null; clippy: string | null; removed_lint_flags: string[] };
  scan: { command: string[] | null };
  diagnostics: Array<{ code: string | null; severity: string }>;
  errors: ScanError[];
};

const TOOLCHAIN_TOOLS = new Set(["cargo", "rustc", "rustdoc", "cargo-clippy", "clippy-driver", "rustup"]);

function codes(report: Report): string[] {
  return report.errors.map((error) => `${error.stage}/${error.code}`);
}

function error(report: Report, code: string): ScanError {
  const found = report.errors.find((entry) => entry.code === code);
  if (!found) throw new Error(`no ${code} error in ${JSON.stringify(report.errors)}`);
  return found;
}

// The installed toolchains that ship Clippy, by directory name.
function toolchainsWithClippy(): string[] {
  return readdirSync(join(RUSTUP_HOME, "toolchains"))
    .filter((name) => existsSync(join(RUSTUP_HOME, "toolchains", name, "bin", "cargo-clippy")))
    .sort();
}

function toolchainBinary(name: string, binary: string): string {
  return join(RUSTUP_HOME, "toolchains", name, "bin", binary);
}

function version(rustc: string): string {
  const run = Bun.spawnSync([rustc, "--version"], { env: { PATH: "/usr/bin:/bin", RUSTUP_HOME, CARGO_HOME } });
  return run.stdout.toString().trim();
}

const INSTALLED = toolchainsWithClippy()[0];
if (!INSTALLED) throw new Error(`no toolchain with Clippy under ${RUSTUP_HOME}`);
const DEFAULT_RUSTC = version(join(CARGO_HOME, "bin", "rustc"));
const HOST = Bun.spawnSync([join(CARGO_HOME, "bin", "rustc"), "-vV"], { env: { PATH: "/usr/bin:/bin", RUSTUP_HOME, CARGO_HOME } })
  .stdout.toString().match(/^host: (\S+)$/m)?.[1] ?? "";

// An installed toolchain other than the default, preferring a numbered
// release, for TC-06's directory pin.
const OTHER = toolchainsWithClippy()
  .map((name) => ({ name, channel: name.endsWith(`-${HOST}`) ? name.slice(0, -HOST.length - 1) : name, rustc: version(toolchainBinary(name, "rustc")) }))
  .filter((toolchain) => toolchain.rustc !== "" && toolchain.rustc !== DEFAULT_RUSTC)
  .sort((left, right) => Number(/^\d/.test(right.channel)) - Number(/^\d/.test(left.channel)))[0];
if (!OTHER) console.warn("only one toolchain with Clippy is installed: TC-06 skipped");

// A PATH holding the suite's system tools and exactly `links`, none of the
// rustup proxies.
function pathWith(label: string, links: Record<string, string>): string {
  const dir = join(scratchDir(label), "bin");
  mkdirSync(dir);
  for (const tool of readdirSync(toolchainBin())) {
    if (!TOOLCHAIN_TOOLS.has(tool)) symlinkSync(join(toolchainBin(), tool), join(dir, tool));
  }
  for (const [name, target] of Object.entries(links)) symlinkSync(target, join(dir, name));
  return dir;
}

// A scratch RUSTUP_HOME whose default is a custom toolchain linking the real
// rustc, cargo and rustdoc of an installed one, and no Clippy.
function rustupWithoutClippy(): string {
  const root = scratchDir("noclippy");
  mkdirSync(join(root, "toolchain", "bin"), { recursive: true });
  for (const binary of ["rustc", "cargo", "rustdoc"]) {
    symlinkSync(toolchainBinary(INSTALLED!, binary), join(root, "toolchain", "bin", binary));
  }
  const home = join(root, "rustup");
  mkdirSync(join(home, "toolchains"), { recursive: true });
  symlinkSync(join(root, "toolchain"), join(home, "toolchains", "noclippy"));
  writeFileSync(join(home, "settings.toml"), 'version = "12"\ndefault_toolchain = "noclippy"\nprofile = "minimal"\n');
  return home;
}

// A crate pinned to `channel` by a `rust-toolchain.toml`, holding one
// `todo!()`.
function pinned(channel: string, manifestExtra = ""): string {
  const dir = join(scratchDir("pinned"), "pinned");
  mkdirSync(join(dir, "src"), { recursive: true });
  writeFileSync(join(dir, "Cargo.toml"), `[package]\nname = "pinned"\nversion = "0.1.0"\nedition = "2021"\npublish = false\n${manifestExtra}\n[workspace]\n`);
  writeFileSync(join(dir, "src/lib.rs"), "pub fn unfinished() -> u8 {\n    todo!()\n}\n");
  if (channel) writeFileSync(join(dir, "rust-toolchain.toml"), `[toolchain]\nchannel = "${channel}"\n`);
  return dir;
}

// A `clippy-driver` in front of the real one, answering `-W help` with `help`.
function fakeDriver(label: string, help: string): string {
  const dir = join(scratchDir(label), "bin");
  mkdirSync(dir);
  const real = join(toolchainBin(), "clippy-driver");
  writeFileSync(join(dir, "clippy-driver"), `#!/bin/sh\nif [ "$1" = "-W" ] && [ "$2" = "help" ]; then\n${help}\nfi\nexec "${real}" "$@"\n`);
  chmodSync(join(dir, "clippy-driver"), 0o755);
  return `${dir}:${toolchainBin()}`;
}

async function scan(workspace: string, env: Record<string, string | undefined> = {}): Promise<{ exitCode: number | null; report: Report; stderr: string }> {
  const run = await runDoctor([workspace, "--yes", "--json"], { env });
  return { exitCode: run.exitCode, report: run.json as Report, stderr: run.stderr };
}

describe("toolchain preflight", () => {
  test.failing("TC-01 failures/BUG-12 no cargo on PATH carries the rustup remedy", async () => {
    const run = await runDoctor([copyWorkspace("kernel-contract/todo"), "--yes", "--json"], { env: { PATH: pathWithout("cargo") } });
    expect(run.exitCode).toBe(2);
    const error = (run.json as Report).errors.find((entry) => entry.code === "cargo-unavailable");
    expect(error?.message).toContain("https://rustup.rs");
  });

  // Cargo looks for `cargo-clippy` under CARGO_HOME/bin as well as on PATH,
  // so the rows that withhold a tool also withhold the real CARGO_HOME.
  test("TC-02 cargo without rustc names rustc, the remedy, and publishes no half toolchain", async () => {
    const { exitCode, report } = await scan(copyWorkspace("projects/clippy-warning"), {
      PATH: pathWith("tc02", { cargo: toolchainBinary(INSTALLED, "cargo") }),
      CARGO_HOME: scratchDir("tc02-cargo-home"),
    });
    expect(exitCode).toBe(2);
    expect(report.status).toBe("failed");
    expect(error(report, "rustc-unavailable").message).toMatch(/^rustc could not be started: .*Install a Rust toolchain from https:\/\/rustup\.rs and put `rustc` on PATH\.$/);
    expect(report.toolchain).toMatchObject({ rustc: null, cargo: null, clippy: null });
  });

  test("TC-03 a toolchain on PATH without cargo-clippy names the component to install", async () => {
    const { exitCode, report } = await scan(copyWorkspace("projects/clippy-warning"), {
      PATH: pathWith("tc03", Object.fromEntries(["cargo", "rustc", "rustdoc"].map((tool) => [tool, toolchainBinary(INSTALLED, tool)]))),
      CARGO_HOME: scratchDir("tc03-cargo-home"),
    });
    expect(exitCode).toBe(2);
    expect(codes(report)).toEqual(["execution/clippy-unavailable"]);
    const message = error(report, "clippy-unavailable").message;
    expect(message).toContain("no such command: `clippy`");
    expect(message).toContain("rustup component add clippy");
    expect(message).toContain("components: clippy");
    expect(report.toolchain).toMatchObject({ rustc: null, cargo: null, clippy: null });
  });

  test.failing("TC-04 failures/BUG-11 a toolchain without Clippy is named in one sentence, one period", async () => {
    const run = await runDoctor([copyWorkspace("kernel-contract/todo"), "--yes", "--json"], { env: { RUSTUP_HOME: rustupWithoutClippy() } });
    expect(run.exitCode).toBe(2);
    const error = (run.json as Report).errors.find((entry) => entry.code === "clippy-unavailable");
    expect(error?.message).toContain("'noclippy'");
    expect(error?.message).not.toContain("..");
  });

  test("TC-05 a crate the toolchain is too old for is named unlinted and carries no score", async () => {
    const { exitCode, report } = await scan(pinned("", 'rust-version = "1.999"\n'));
    expect(exitCode).toBe(1);
    expect(report.status).toBe("incomplete");
    expect(codes(report)).toEqual(["clippy/packages-unlinted", "execution/build-finished-missing", "execution/clippy-exit"]);
    expect(error(report, "clippy-exit").message).toContain("requires rustc 1.999");
    expect(report.audit.score).toBeNull();
    expect(report.audit.packages).toEqual([expect.objectContaining({ name: "pinned", unlinted: true, score: null })]);
  });

  test.skipIf(!OTHER)("TC-06 the probe honors the workspace's toolchain pin", async () => {
    const { exitCode, report } = await scan(pinned(OTHER!.channel));
    expect(exitCode).toBe(0);
    expect(report.status).toBe("complete");
    expect(report.toolchain.rustc).toBe(OTHER!.rustc);
    expect(report.diagnostics.map((diagnostic) => diagnostic.code)).toContain("clippy::todo");
  });

  // A scratch RUSTUP_HOME keeps whatever rustup would write out of the
  // machine's own, and the namespace keeps a rustup that ignores
  // RUSTUP_AUTO_INSTALL off the network.
  test("TC-07 a pinned toolchain that is not installed fails before Clippy with rustup's remedy", async () => {
    const workspace = pinned("1.80.0");
    const env = { RUSTUP_HOME: scratchDir("tc07-rustup"), RUSTUP_AUTO_INSTALL: "0" };
    const run = requireNetns()
      ? await runDoctorOffline([workspace, "--yes", "--json"], { env })
      : await runDoctor([workspace, "--yes", "--json"], { env });
    const report = run.json as Report;
    expect(run.exitCode).toBe(2);
    expect(codes(report)).toEqual(["metadata/cargo-metadata"]);
    expect(error(report, "cargo-metadata").message).toContain(`toolchain '1.80.0-${HOST}' is not installed`);
    expect(error(report, "cargo-metadata").message).toContain("rustup toolchain install");
  });

  test.skipIf(!requireNetns())("TC-08 rustup's failed auto-install ends the scan with no absolute path, and RUSTUP_AUTO_INSTALL=0 stops it trying", async () => {
    const workspace = pinned("1.80.0");
    const rustupHome = scratchDir("tc08-rustup");
    const tried = await runDoctorOffline([workspace, "--yes", "--json"], { env: { RUSTUP_HOME: rustupHome }, timeoutMs: 90_000 });
    const report = tried.json as Report;
    expect(tried.exitCode).toBe(2);
    expect(codes(report)).toEqual(["metadata/cargo-metadata"]);
    expect(error(report, "cargo-metadata").message).toContain("1.80.0");
    expect(tried.stdout).not.toContain(rustupHome);
    expect(tried.stdout).not.toContain(E2E_BASE);

    const refused = await runDoctorOffline([workspace, "--yes", "--json"], { env: { RUSTUP_HOME: scratchDir("tc08-rustup-off"), RUSTUP_AUTO_INSTALL: "0" } });
    const message = error(refused.json as Report, "cargo-metadata").message;
    expect(message).toContain("is not installed");
    expect(message).not.toMatch(/download|syncing channel/);
  });

  test("TC-09 a catalogued lint the installed Clippy does not list is not evaluated, and an unlistable Clippy allows unknown lints", async () => {
    const omitted = await scan(copyWorkspace("scan-failures/lint-flags"), {
      PATH: fakeDriver("tc09-omit", `"${join(toolchainBin(), "clippy-driver")}" -W help | grep -v ' clippy::todo '\nexit 0`),
    });
    expect(omitted.exitCode).toBe(0);
    expect(omitted.report.status).toBe("complete");
    expect(omitted.report.policy?.rules.find((rule) => rule.id === "clippy::todo")?.not_evaluated).toBe("unknown-to-toolchain");
    expect(omitted.report.scan.command).not.toContain("clippy::todo");
    expect(omitted.report.diagnostics.map((diagnostic) => diagnostic.code)).not.toContain("clippy::todo");
    expect(omitted.report.errors).toEqual([]);

    const unlistable = await scan(copyWorkspace("scan-failures/lint-flags"), { PATH: fakeDriver("tc09-fail", "exit 1") });
    expect(unlistable.exitCode).toBe(0);
    expect(codes(unlistable.report)).toEqual(["toolchain/lint-list-unavailable"]);
    expect(unlistable.report.scan.command).toContain("clippy::todo");
    expect(unlistable.report.scan.command?.join(" ")).toContain("unknown_lints");
    expect(unlistable.report.diagnostics.map((diagnostic) => diagnostic.code)).toContain("clippy::todo");
  });

  type FlagRow = [id: string, variables: Record<string, string>, removed: string[]];
  const flagRows: FlagRow[] = [
    ["TC-10", { RUSTFLAGS: "-Dwarnings" }, ["-D warnings"]],
    ["TC-11", { CARGO_ENCODED_RUSTFLAGS: "-Dwarnings" }, ["-D warnings"]],
    ["TC-12", { RUSTFLAGS: "-Fclippy::todo --cap-lints=allow" }, ["-F clippy::todo", "--cap-lints allow"]],
    ["TC-13", { RUSTFLAGS: "-Ctarget-cpu=native" }, []],
    ["TC-14", { CARGO_BUILD_RUSTFLAGS: "-Dwarnings" }, []],
  ];

  // Whatever the caller exports, the `todo!()` of the fixture stays one
  // warning: a lint flag that would raise or silence it is removed where the
  // scan can see it, and outweighed where it cannot.
  test.each(flagRows)("%s caller rustflags never change a verdict", async (_id, variables, removed) => {
    const { exitCode, report } = await scan(copyWorkspace("scan-failures/lint-flags"), variables);
    expect(exitCode).toBe(0);
    expect(report.status).toBe("complete");
    expect(report.toolchain.removed_lint_flags).toEqual(removed);
    expect(report.diagnostics.filter((diagnostic) => diagnostic.code === "clippy::todo").map((diagnostic) => diagnostic.severity)).toEqual(["warning"]);
  });

  test("TC-15 the workspace's .cargo/config.toml cannot deny warnings either", async () => {
    const { exitCode, report } = await scan(copyWorkspace("scan-failures/lint-flags-config"));
    expect(exitCode).toBe(0);
    expect(report.status).toBe("complete");
    expect(report.toolchain.removed_lint_flags).toEqual(["-D warnings"]);
    expect(report.diagnostics.filter((diagnostic) => diagnostic.code === "clippy::todo").map((diagnostic) => diagnostic.severity)).toEqual(["warning"]);
  });
});
