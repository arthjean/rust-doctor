// `tasks/e2e-catalog/failures.md` section PV: privacy and secret redaction.
// One planted probe (privacy-support.ts) is scanned once per output mode, and
// every mode is searched for every planted value, environment value and
// private path. Its credentials are built at runtime.
import { beforeAll, describe, expect, test } from "bun:test";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { BIN, runDoctor, scratchDir, type DoctorRun } from "./e2e-helpers";
import { credential } from "./git-scenario";
import { requireNetns, runDoctorOffline } from "./netns";
import { buildProbe, leaks, type Probe } from "./privacy-support";
import { spawnPty } from "./pty";
import { MENU_HINT } from "./tmux-helpers";

type Diagnostic = {
  code: string | null;
  path: string | null;
  message: string;
  span?: { line_start: number };
  suggestion?: { replacement: string };
};
type Report = {
  status: string;
  errors: Array<{ code: string; message: string }>;
  diagnostics: Diagnostic[];
  toolchain: { removed_lint_flags: string[] };
};

let probe: Probe;
let json: DoctorRun;
let verbose: DoctorRun;
let linear: DoctorRun;
let markdown: DoctorRun;

function scanProbe(args: string[]): Promise<DoctorRun> {
  return runDoctor([probe.workspace, "--yes", ...args], { sandbox: probe.sandbox, target: probe.target, env: probe.env });
}

beforeAll(async () => {
  probe = await buildProbe();
  json = await scanProbe(["--json"]);
  verbose = await scanProbe(["--verbose"]);
  linear = await scanProbe([]);
  const saved = join(probe.sandbox.dir, "report.json");
  writeFileSync(saved, json.stdout);
  markdown = await runDoctor(["report", "markdown", saved], { sandbox: probe.sandbox, env: probe.env });
});

function report(): Report {
  return json.json as Report;
}

function at(code: string, line: number): Diagnostic | undefined {
  return report().diagnostics.find((entry) => entry.code === code && entry.path === "src/lib.rs" && entry.span?.line_start === line);
}

// A one-crate workspace of `files` outside any HOME the scan is told about.
function crate(name: string, files: Record<string, string>): string {
  const dir = join(scratchDir(name), name);
  const manifest = `[package]\nname = "${name}"\nversion = "0.1.0"\nedition = "2024"\npublish = false\n\n[workspace]\n`;
  for (const [path, body] of Object.entries({ "Cargo.toml": manifest, "src/lib.rs": "", ...files })) {
    mkdirSync(dirname(join(dir, path)), { recursive: true });
    writeFileSync(join(dir, path), body);
  }
  return dir;
}

describe("privacy and secret redaction", () => {
  test("PV-01 no output mode carries a private path, a username, an environment value or a planted secret", () => {
    for (const run of [json, verbose, linear, markdown]) {
      expect(run.exitCode).toBe(0);
      expect(leaks(run.stdout + run.stderr, probe.needles)).toEqual([]);
    }
    expect(report().status).toBe("complete");
  });

  test("PV-02 every code frame masks a credential behind its four-character prefix", () => {
    const planted = probe.planted;
    const out = verbose.stdout;
    for (const masked of [
      `"AKIA<redacted>"`,
      `"${planted.token!.slice(0, 4)}<redacted>"`,
      `expect("${planted.apiKey!.slice(0, 4)}<redacted>")`,
      `unwrap_or("${planted.fallback!.slice(0, 4)}<redacted>".to_owned())`,
      `&"${planted.slack!.slice(0, 4)}<redacted>".to_string()`,
      `token ${planted.workflowToken!.slice(0, 4)}<redacted>`,
    ]) {
      expect(out).toContain(masked);
    }
    const password = planted.databaseUrl!.split(":")[2]!.split("@")[0]!;
    expect(out).toContain(`${password.slice(0, 4)}<redacted>@db.prod.net`);
    expect(out).toMatch(/> {4}1 \| ----<redacted>\n[^\n]*\n {5}2 \| <redacted>\n {5}3 \| <redacted>\n/);
    // The access key's line is masked in the frame of the neighboring unwrap too.
    expect(out).toMatch(/\n {5}2 \| {5}"AKIA<redacted>"/);
  });

  test("PV-03 the probe's findings name each credential's pattern and never its value", () => {
    const counts: Record<string, number> = {};
    for (const entry of report().diagnostics) counts[entry.code ?? "null"] = (counts[entry.code ?? "null"] ?? 0) + 1;
    expect(counts).toEqual({
      "rust_doctor::repo::hardcoded_credential": 7,
      "rust_doctor::repo::tracked_secret_file": 2,
      "rust_doctor::source::secret_env_fallback": 1,
      "clippy::unwrap_used": 1,
      "clippy::expect_used": 1,
      "clippy::unnecessary_to_owned": 2,
    });
    const files = report().diagnostics.filter((entry) => entry.code === "rust_doctor::repo::tracked_secret_file").map((entry) => entry.path);
    expect(files.sort()).toEqual([".env", "keys/server.pem"]);
    for (const entry of report().diagnostics.filter((item) => item.code === "rust_doctor::repo::hardcoded_credential")) {
      expect(entry.message).toMatch(/^string literal matching the .+ credential pattern$/);
    }
  });

  test("PV-04 a lint flag of RUSTFLAGS is removed and named, and its other tokens are never published", () => {
    expect(report().toolchain.removed_lint_flags).toEqual(["-D warnings"]);
    expect(json.stdout).not.toContain("planted_rustflag");
  });

  test("PV-05 a Clippy suggestion that would carry a credential is withheld, and the finding kept", () => {
    const withheld = at("clippy::unnecessary_to_owned", 23);
    expect(withheld).toBeDefined();
    expect(withheld!.suggestion).toBeUndefined();
    expect(at("rust_doctor::repo::hardcoded_credential", 23)).toBeDefined();
    expect(at("clippy::unnecessary_to_owned", 27)?.suggestion?.replacement).toBe('"plain words"');
    const replace = verbose.stdout.split("\n").filter((line) => line.includes("Replace with:"));
    expect(replace).toEqual(['Replace with: "plain words"']);
  });

  test("PV-06 a token glued to a flag is outside the closed list: neither reported nor masked", async () => {
    const token = credential("token");
    const workspace = crate("glued", {
      "src/lib.rs": `pub fn run(command: &mut std::process::Command, value: Option<u8>) -> u8 {\n    command.arg("-H${token}");\n    value.unwrap()\n}\n`,
    });
    const scanned = await runDoctor([workspace, "--yes", "--json"]);
    expect((scanned.json as Report).diagnostics.map((entry) => entry.code)).not.toContain("rust_doctor::repo::hardcoded_credential");
    const frames = await runDoctor([workspace, "--yes", "--verbose"]);
    expect(frames.stdout).toContain(`command.arg("-H${token}")`);
  });

  test("PV-07 the markdown of a report holding a withheld suggestion carries neither the suggestion nor the secret", () => {
    expect(markdown.stdout.length).toBeGreaterThan(0);
    expect(markdown.stdout).not.toContain(probe.planted.slack!);
    expect(markdown.stdout).not.toContain("Replace with");
  });

  test("PV-08 a missing member outside the workspace is named without its absolute path", async () => {
    const root = scratchDir("pv-08");
    mkdirSync(join(root, "ws"));
    writeFileSync(join(root, "ws/Cargo.toml"), '[workspace]\nmembers = ["../gone"]\n');
    const run = await runDoctor([join(root, "ws"), "--yes", "--json"]);
    expect(run.exitCode).toBe(2);
    const message = (run.json as Report).errors[0]!.message;
    expect(message).toContain("<external>/gone/Cargo.toml");
    expect(run.stdout).not.toContain(root);
  });

  test.skipIf(!requireNetns())("PV-09 a rustup error under a RUSTUP_HOME outside HOME names $RUSTUP_HOME, never the path", async () => {
    // Inside a namespace with no network, so rustup's install attempt fails
    // at once instead of downloading a toolchain.
    const rustupHome = join(scratchDir("pv-09"), "rustup");
    mkdirSync(rustupHome);
    const workspace = crate("pinned", { "rust-toolchain.toml": '[toolchain]\nchannel = "1.80.0"\n' });
    const run = await runDoctorOffline([workspace, "--yes", "--json"], { env: { RUSTUP_HOME: rustupHome }, timeoutMs: 60_000 });
    expect(run.exitCode).toBe(2);
    const message = (run.json as Report).errors.map((error) => error.message).join("\n");
    expect(message).toContain("$RUSTUP_HOME/tmp/");
    expect(run.stdout).not.toContain(rustupHome);
  });

  test("PV-10 a failing build script is named under $CARGO_TARGET_DIR, never the target's path", async () => {
    const workspace = crate("failing-build", { "build.rs": "fn main() {\n    std::process::exit(3);\n}\n" });
    const target = join(scratchDir("pv-10-target"), "target");
    const run = await runDoctor([workspace, "--yes", "--json"], { target });
    expect(run.exitCode).toBe(1);
    const message = (run.json as Report).errors.map((error) => error.message).join("\n");
    expect(message).toMatch(/`\$CARGO_TARGET_DIR\/debug\/build\/failing-build-[0-9a-f]+\/build-script-build` \(exit status: 3\)/);
    expect(run.stdout).not.toContain(target);
  });

  test("PV-13 a private key a failing build script echoes is masked in the report error and its markdown", async () => {
    // The header is assembled in the build script and here, so neither this
    // file nor the scanned tree tracks a key; no three characters of the body
    // occur in a message that does not carry it.
    const body = "Q+/Z9".repeat(13);
    const script = [
      "fn main() {",
      '    eprintln!("-----BEGIN {}PRIVATE KEY-----", "RSA ");',
      `    for _ in 0..40 {\n        eprintln!("${body}");\n    }`,
      '    eprintln!("-----END {}PRIVATE KEY-----", "RSA ");',
      "    std::process::exit(1);",
      "}",
      "",
    ].join("\n");
    const workspace = crate("echoed-key", { "build.rs": script });
    const run = await runDoctor([workspace, "--yes", "--json"]);
    expect(run.exitCode).toBe(1);
    const saved = join(scratchDir("pv-13-report"), "report.json");
    writeFileSync(saved, run.stdout);
    const rendered = await runDoctor(["report", "markdown", saved]);
    expect(rendered.exitCode).toBe(0);
    const messages = (run.json as Report).errors.map((error) => error.message).join("\n");
    expect(messages).toContain("<redacted>");
    const windows = Array.from({ length: body.length - 2 }, (_, index) => body.slice(index, index + 3));
    for (const published of [messages, rendered.stdout]) {
      expect(windows.filter((window) => published.includes(window))).toEqual([]);
    }
  });

  test("PV-11 a --rule value carrying a secret is never echoed", async () => {
    const secret = credential("token");
    for (const args of [["--rule", `${secret}=bogus`], ["--json", "--rule", `${secret}=bogus`], ["--json", "--rule", `clippy::${secret}=warn`]]) {
      const run = await runDoctor([crate("selector", {}), "--yes", ...args]);
      expect(run.exitCode).toBe(2);
      expect(run.stdout + run.stderr).not.toContain(secret);
    }
  });

  test("PV-12 the handoff prompt of the interactive report carries no private path or secret", async () => {
    const session = spawnPty([BIN, probe.workspace, "--advisory-db", "/dev/null"], {
      cols: 120,
      rows: 40,
      sandbox: probe.sandbox,
      env: { ...probe.env, CARGO_TARGET_DIR: probe.target, NO_COLOR: "1" },
    });
    const until = async (text: string) => {
      const deadline = Date.now() + 15_000;
      while (!session.output().includes(text)) {
        if (Date.now() > deadline) throw new Error(`no "${text}" on the terminal:\n${session.output().slice(-2000)}`);
        await Bun.sleep(25);
      }
    };
    await until(MENU_HINT);
    session.write("j");
    await until("❯ Add to GitHub Actions");
    session.write("j");
    await until("❯ Hand off to an agent");
    session.write("\r");
    await until("❯ Add to GitHub Actions first");
    session.write("j");
    await until("❯ Continue without GitHub Actions");
    session.write("\r");
    await until("❯ Copy prompt");
    session.write("\r");
    const run = await session.exited;
    expect(run.exitCode).toBe(0);
    const prompt = run.output.slice(run.output.indexOf("---- Agent prompt ----"));
    expect(prompt).toContain("rust_doctor::repo::hardcoded_credential");
    expect(prompt).toContain("Validate with: rust-doctor . --yes");
    expect(leaks(run.output, probe.needles)).toEqual([]);
  });
});
