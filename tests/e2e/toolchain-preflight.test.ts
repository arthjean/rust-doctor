// `tasks/e2e-catalog/failures.md` section TC: toolchain problems. EP-006
// fills it; the rows here are the open toolchain-message defects of the
// ledger.
import { describe, expect, test } from "bun:test";
import { existsSync, mkdirSync, readdirSync, symlinkSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { copyWorkspace, pathWithout, runDoctor, RUSTUP_HOME, scratchDir } from "./e2e-helpers";

type Report = { status: string; errors: Array<{ code: string; message: string }> };

// A scratch RUSTUP_HOME whose default is a custom toolchain linking the real
// rustc, cargo and rustdoc of an installed one, and no Clippy.
function rustupWithoutClippy(): string {
  const installed = readdirSync(join(RUSTUP_HOME, "toolchains")).find((name) =>
    existsSync(join(RUSTUP_HOME, "toolchains", name, "bin", "cargo-clippy")),
  );
  if (!installed) throw new Error(`no toolchain with Clippy under ${RUSTUP_HOME}`);
  const root = scratchDir("noclippy");
  mkdirSync(join(root, "toolchain", "bin"), { recursive: true });
  for (const binary of ["rustc", "cargo", "rustdoc"]) {
    symlinkSync(join(RUSTUP_HOME, "toolchains", installed, "bin", binary), join(root, "toolchain", "bin", binary));
  }
  const home = join(root, "rustup");
  mkdirSync(join(home, "toolchains"), { recursive: true });
  symlinkSync(join(root, "toolchain"), join(home, "toolchains", "noclippy"));
  writeFileSync(join(home, "settings.toml"), 'version = "12"\ndefault_toolchain = "noclippy"\nprofile = "minimal"\n');
  return home;
}

describe("toolchain preflight", () => {
  test.failing("TC-01 failures/BUG-12 no cargo on PATH carries the rustup remedy", async () => {
    const run = await runDoctor([copyWorkspace("kernel-contract/todo"), "--yes", "--json"], { env: { PATH: pathWithout("cargo") } });
    expect(run.exitCode).toBe(2);
    const error = (run.json as Report).errors.find((entry) => entry.code === "cargo-unavailable");
    expect(error?.message).toContain("https://rustup.rs");
  });

  test.failing("TC-04 failures/BUG-11 a toolchain without Clippy is named in one sentence, one period", async () => {
    const run = await runDoctor([copyWorkspace("kernel-contract/todo"), "--yes", "--json"], { env: { RUSTUP_HOME: rustupWithoutClippy() } });
    expect(run.exitCode).toBe(2);
    const error = (run.json as Report).errors.find((entry) => entry.code === "clippy-unavailable");
    expect(error?.message).toContain("'noclippy'");
    expect(error?.message).not.toContain("..");
  });
});
