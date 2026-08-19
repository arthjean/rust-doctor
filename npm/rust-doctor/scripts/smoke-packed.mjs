import {
  chmodSync,
  existsSync,
  lstatSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  readlinkSync,
  renameSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, join, relative, resolve } from "node:path";

import { PLATFORM_PACKAGES } from "../lib/launcher.js";
import {
  archiveInventory,
  packLocal,
  repositoryRoot,
  sha256,
} from "./pack-local.mjs";

const artifactPath = join(repositoryRoot, "target/local-cli-dogfood.json");

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

function criterion(id, evidence) {
  return { id, verdict: "PASS", evidence };
}

function commandPath(name) {
  const path = Bun.which(name);
  if (!path) throw new Error(`packed smoke requires ${name} on PATH`);
  return path;
}

function run(command, options = {}) {
  return Bun.spawnSync(command, {
    cwd: options.cwd ?? repositoryRoot,
    env: options.env ?? process.env,
    stdin: options.stdin,
    stdout: "pipe",
    stderr: "pipe",
  });
}

function output(result) {
  return {
    code: result.exitCode,
    stdout: result.stdout.toString(),
    stderr: result.stderr.toString(),
  };
}

function checked(result, description) {
  const observed = output(result);
  if (observed.code !== 0) {
    throw new Error(
      `${description} failed (${observed.code})\nstdout:\n${observed.stdout}\nstderr:\n${observed.stderr}`,
    );
  }
  return observed;
}

function successful(result, description) {
  return checked(result, description).stdout.trim();
}

function hashText(value) {
  return new Bun.CryptoHasher("sha256").update(value).digest("hex");
}

function gitOutput(arguments_, description) {
  return checked(
    run(["git", "--no-optional-locks", ...arguments_]),
    description,
  ).stdout;
}

async function worktreeHash() {
  const listed = gitOutput(
    ["ls-files", "--cached", "--others", "--exclude-standard", "-z"],
    "git worktree inventory",
  );
  const paths = listed.split("\0").filter(Boolean).sort();
  const records = await Promise.all(paths.map(async (path) => {
    const fullPath = join(repositoryRoot, path);
    if (!existsSync(fullPath)) return `${path}\0missing`;
    const metadata = lstatSync(fullPath);
    const mode = (metadata.mode & 0o777).toString(8);
    if (metadata.isSymbolicLink()) {
      return `${path}\0link\0${mode}\0${readlinkSync(fullPath)}`;
    }
    if (!metadata.isFile()) return `${path}\0other\0${mode}`;
    return `${path}\0file\0${mode}\0${await sha256(fullPath)}`;
  }));
  return hashText(records.sort().join("\0"));
}

function forbiddenOutputState() {
  const targets = [];
  const visit = (directory) => {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      if (entry.name === ".git" || entry.name === "node_modules") continue;
      if (!entry.isDirectory()) continue;
      const path = join(directory, entry.name);
      if (entry.name === "target") {
        targets.push(relative(repositoryRoot, path));
      } else {
        visit(path);
      }
    }
  };
  visit(repositoryRoot);
  const paths = [
    ".rust-doctor",
    "rust-doctor.toml",
    "rust-doctor-handoff.md",
    ".github/workflows/rust-doctor.yml",
  ].filter((path) => existsSync(join(repositoryRoot, path)));
  return { paths, targets: targets.sort() };
}

async function repositoryState() {
  const [index, workingTree] = await Promise.all([
    sha256(join(repositoryRoot, ".git/index")),
    worktreeHash(),
  ]);
  return {
    head: successful(
      run(["git", "--no-optional-locks", "rev-parse", "HEAD"]),
      "git HEAD",
    ),
    tree: successful(
      run(["git", "--no-optional-locks", "rev-parse", "HEAD^{tree}"]),
      "git tree",
    ),
    index,
    refs: hashText(gitOutput(["show-ref", "--head"], "git refs")),
    index_entries: hashText(gitOutput(["ls-files", "--stage", "-z"], "git index entries")),
    status: hashText(gitOutput(
      ["status", "--porcelain=v2", "-z", "--untracked-files=all"],
      "git status",
    )),
    config: hashText(gitOutput(["config", "--local", "--null", "--list"], "git config")),
    objects: hashText(gitOutput(["count-objects", "-v"], "git objects")),
    working_tree: workingTree,
    forbidden_outputs: forbiddenOutputState(),
  };
}

function normalizeTerminal(value) {
  return value
    .replace(/\u001b\[[0-9;]*[A-Za-z]/gu, "")
    .replace(/Scanned (\d+) files in \d+\.\d+s/gu, "Scanned $1 files in <elapsed>");
}

// Read back from the source rather than pinned here. A hard-coded number turns
// every schema bump into a silent lie in this proof: it sat at 8 while the
// product shipped 14, and nothing said so because nothing ran this script.
function shippedSchemaVersion() {
  const source = readFileSync(join(repositoryRoot, "src/report.rs"), "utf8");
  const found = source.match(/pub const SCHEMA_VERSION: u8 = (\d+);/u);
  if (!found) throw new Error("SCHEMA_VERSION is no longer declared in src/report.rs");
  return Number(found[1]);
}

// The category tally is the one section with two spellings: the renderer names
// every category it counted ("Bugs: 0 errors, ...") and falls back to
// "Categories: none" only when there are none at all. Matching the literal
// "Categories:" asserted the empty scan, which is the shape this proof exists to
// rule out, so the section is matched by what both spellings have in common.
const SECTIONS = [
  { name: "Scope", match: "Scope: full codebase" },
  { name: "Scanned", match: "Scanned " },
  { name: "All", match: "All " },
  { name: "Categories", match: /^(?:Categories: none|\w[\w ]*: \d+ errors, )/mu },
  { name: "Score", match: "┌─────┐" },
  { name: "Share", match: "Share:" },
  { name: "Docs", match: "Docs:" },
  { name: "GitHub", match: "GitHub:" },
];

function sectionPosition(terminal, match) {
  if (typeof match === "string") return terminal.indexOf(match);
  const found = terminal.match(match);
  return found?.index ?? -1;
}

function sectionOrder(terminal) {
  let previous = -1;
  for (const section of SECTIONS) {
    const position = sectionPosition(terminal, section.match);
    if (position < previous || position === -1) {
      throw new Error(`packed terminal section ${JSON.stringify(section.name)} is missing or out of order`);
    }
    previous = position;
  }
  return SECTIONS.map((section) => section.name);
}

function createControlledBin(directory) {
  mkdirSync(directory, { recursive: true });
  for (const command of [
    "node",
    "cargo",
    "cargo-clippy",
    "rustc",
    "clippy-driver",
    "git",
  ]) {
    symlinkSync(commandPath(command), join(directory, command));
  }
}

function localOptionalOverrides(directory, version) {
  const overrides = {};
  for (const [key, packageName] of Object.entries(PLATFORM_PACKAGES)) {
    if (key === "linux-x64") continue;
    const [os, cpu] = key.split("-");
    const packageDirectory = join(directory, key);
    mkdirSync(packageDirectory, { recursive: true });
    writeFileSync(
      join(packageDirectory, "package.json"),
      `${JSON.stringify({ name: packageName, version, os: [os], cpu: [cpu], files: [] })}\n`,
    );
    overrides[packageName] = `file:${packageDirectory}`;
  }
  return overrides;
}

function installPacked(temporary, packed) {
  const wrapperInventory = archiveInventory(packed.wrapperArchive);
  const nativeInventory = archiveInventory(packed.nativeArchive);
  assert(
    JSON.stringify(wrapperInventory) === JSON.stringify([
      "package/LICENSE-APACHE",
      "package/LICENSE-MIT",
      "package/bin/rust-doctor.js",
      "package/lib/launcher.js",
      "package/package.json",
    ]),
    "wrapper tarball inventory differs",
  );
  assert(
    JSON.stringify(nativeInventory) === JSON.stringify([
      "package/LICENSE-APACHE",
      "package/LICENSE-MIT",
      "package/bin/rust-doctor",
      "package/package.json",
    ]),
    "native tarball inventory differs",
  );

  const install = join(temporary, "install");
  const optionalStubs = join(temporary, "optional-stubs");
  mkdirSync(install);
  writeFileSync(
    join(install, "package.json"),
    `${JSON.stringify({
      name: "rust-doctor-packed-proof",
      private: true,
      dependencies: {
        "rust-doctor": `file:${packed.wrapperArchive}`,
        "@rustdoctor/linux-x64": `file:${packed.nativeArchive}`,
      },
      overrides: localOptionalOverrides(optionalStubs, packed.version),
    }, null, 2)}\n`,
  );
  const installation = output(run(
    ["bun", "install", "--ignore-scripts", "--backend=copyfile"],
    {
      cwd: install,
      env: {
        ...process.env,
        NPM_CONFIG_REGISTRY: "http://127.0.0.1:9",
        HTTP_PROXY: "http://127.0.0.1:9",
        HTTPS_PROXY: "http://127.0.0.1:9",
        ALL_PROXY: "http://127.0.0.1:9",
        NO_PROXY: "",
      },
    },
  ));
  assert(installation.code === 0, `packed tarball installation failed: ${installation.stderr}`);
  assert(!/registry\.npmjs|GET https?:/u.test(installation.stderr), "packed install attempted registry access");

  const wrapper = join(install, "node_modules/.bin/rust-doctor");
  const nativeBinary = join(
    install,
    "node_modules/@rustdoctor/linux-x64/bin/rust-doctor",
  );
  assert(existsSync(wrapper), "installed wrapper bin is missing");
  assert(existsSync(nativeBinary), "installed native bin is missing");
  assert(relative(install, resolve(nativeBinary)).startsWith("node_modules/"), "native bin escaped install");

  const pathGuard = join(temporary, "path-guard");
  const pathGuardMarker = join(temporary, "path-fallback-used");
  mkdirSync(pathGuard);
  writeFileSync(
    join(pathGuard, "rust-doctor"),
    `#!/bin/sh\nprintf used > "${pathGuardMarker}"\nexit 97\n`,
  );
  chmodSync(join(pathGuard, "rust-doctor"), 0o755);
  const environment = {
    ...process.env,
    CARGO_NET_OFFLINE: "true",
    CARGO_TARGET_DIR: join(temporary, "scan-target"),
    CI: "1",
    NO_COLOR: "1",
    PATH: `${pathGuard}:${process.env.PATH}`,
  };
  const version = output(run([wrapper, "--version"], { cwd: install, env: environment }));
  assert(version.code === 0, "packed --version failed");
  assert(version.stdout.trim() === `rust-doctor ${packed.version}`, "packed version is inconsistent");
  assert(!existsSync(pathGuardMarker), "wrapper used a rust-doctor binary from PATH");

  return {
    install,
    wrapper,
    nativeBinary,
    environment,
    pathGuardMarker,
    wrapperInventory,
    nativeInventory,
    observation: { version_exit: version.code },
    criterion: criterion(
      "US-062-AC-1",
      "packed version, exact inventories and installed native path",
    ),
  };
}

function proveCli(installed) {
  const human = output(run(
    [installed.wrapper, repositoryRoot, "--yes"],
    { cwd: installed.install, env: installed.environment },
  ));
  const direct = output(run(
    [installed.nativeBinary, repositoryRoot, "--yes"],
    { cwd: installed.install, env: installed.environment },
  ));
  assert(human.code === direct.code, "packed and direct scan exit codes differ");
  assert(/^Scanning Rust files\.\.\.$/mu.test(human.stderr.trim()), "human progress is missing");
  const sections = sectionOrder(human.stdout);
  const scan = human.stdout.match(/Scanned (\d+) files in (\d+\.\d)s/u);
  const terminalScore = human.stdout.match(/(\d+) \/ 100 ([A-Za-z ]+)/u);
  assert(scan, "human scan count or one-decimal duration is missing");
  assert(terminalScore, "human score is missing");

  const jsonRun = output(run(
    [installed.wrapper, repositoryRoot, "--json"],
    { cwd: installed.install, env: installed.environment },
  ));
  assert(jsonRun.code === human.code, "JSON and human scan exit codes differ");
  assert(jsonRun.stderr === "", "JSON stderr must be empty for a successful scan");
  assert(jsonRun.stdout.endsWith("\n"), "JSON output must end with one newline");
  assert(!jsonRun.stdout.includes("\u001b["), "JSON output contains ANSI");
  assert(!jsonRun.stdout.includes("Choose what to scan"), "JSON output contains a prompt");
  assert(!jsonRun.stdout.includes(repositoryRoot), "JSON output contains the repository absolute path");
  const report = JSON.parse(jsonRun.stdout);
  const schemaVersion = shippedSchemaVersion();
  assert(
    report.schema_version === schemaVersion,
    `packed JSON schema is ${report.schema_version}, not the shipped v${schemaVersion}`,
  );
  assert(report.audit.source_files === Number(scan[1]), "terminal and JSON source counts differ");
  assert(report.audit.score?.value === Number(terminalScore[1]), "terminal and JSON scores differ");
  assert(report.audit.score?.label === terminalScore[2].trim(), "terminal and JSON labels differ");
  assert(!existsSync(installed.pathGuardMarker), "wrapper used a rust-doctor binary from PATH");

  return {
    observation: {
      human_exit: human.code,
      direct_exit: direct.code,
      json_exit: jsonRun.code,
      source_files: report.audit.source_files,
      score: report.audit.score?.value ?? null,
      score_label: report.audit.score?.label ?? null,
      sections,
    },
    criteria: [
      criterion(
        "US-062-AC-2",
        "ordered human sections, count, duration, score and direct exit parity",
      ),
      criterion(
        "US-062-AC-3",
        "single parseable JSON at the shipped schema version, without prompt, ANSI or absolute repository path",
      ),
    ],
  };
}

function proveDeterminism(installed) {
  const fixture = join(repositoryRoot, "tests/fixtures/kernel-contract/todo");
  const outputs = [];
  for (let index = 0; index < 5; index += 1) {
    const run_ = output(run([installed.wrapper, fixture, "--yes"], {
      cwd: installed.install,
      env: installed.environment,
    }));
    assert(run_.code === 0, `determinism run ${index + 1} failed`);
    outputs.push(normalizeTerminal(run_.stdout));
  }
  assert(new Set(outputs).size === 1, "five normalized terminal runs are not identical");
  return {
    observation: { deterministic_runs: outputs.length },
    criterion: criterion(
      "US-062-AC-5",
      "five normalized packed fixture runs matched",
    ),
  };
}

function proveHandoff(temporary, installed) {
  const controlledBin = join(temporary, "controlled-bin");
  const capture = join(temporary, "handoff-capture");
  mkdirSync(capture);
  createControlledBin(controlledBin);
  const tty = output(run([
    "cargo",
    "test",
    "--test",
    "local_cli_experience",
    "tty::packed_wrapper_tty_handoff_contract_when_requested",
    "--",
    "--exact",
  ], {
    env: {
      ...process.env,
      CARGO_NET_OFFLINE: "true",
      CARGO_TARGET_DIR: join(temporary, "tty-target"),
      RD_PACKED_WRAPPER: installed.wrapper,
      RD_PACKED_CONTROLLED_BIN: controlledBin,
      RD_PACKED_CAPTURE: capture,
      RD_PACKED_SCAN_TARGET: join(temporary, "tty-scan-target"),
    },
  }));
  assert(tty.code === 0, `packed TTY proof failed: ${tty.stderr}`);
  const payload = readFileSync(join(capture, "payload"));
  const arguments_ = readFileSync(join(capture, "argc"), "utf8");
  const streams = readFileSync(join(capture, "tty"), "utf8");
  const fixture = join(repositoryRoot, "tests/fixtures/kernel-contract/todo");
  assert(arguments_ === "1", "handoff argv count differs");
  assert(payload.length <= 12 * 1024, "handoff payload exceeds 12 KiB");
  assert(streams === "yyy", "handoff stdio is not inherited");
  assert(
    resolve(readFileSync(join(capture, "cwd"), "utf8").trim()) === resolve(fixture),
    "handoff cwd differs from the workspace",
  );
  return {
    observation: {
      tty_handoff_exit: tty.code,
      handoff_arguments: Number(arguments_),
      handoff_bytes: payload.length,
      handoff_tty: "stdin/stdout/stderr inherited",
    },
    criterion: criterion(
      "US-062-AC-4",
      "controlled Codex received one bounded argument with workspace cwd and inherited TTY streams",
    ),
  };
}

async function waitUntil(condition, message) {
  const deadline = Date.now() + 5000;
  while (!condition()) {
    if (Date.now() >= deadline) throw new Error(message);
    await Bun.sleep(10);
  }
}

function processExists(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

function compareRepositoryState(before, after, description) {
  const observation = {};
  for (const key of Object.keys(before)) {
    const unchanged = JSON.stringify(before[key]) === JSON.stringify(after[key]);
    observation[key] = unchanged;
    assert(unchanged, `${description} mutated repository state: ${key}`);
  }
  return observation;
}

async function proveInterruption(temporary, installed) {
  const interruptBin = join(temporary, "interrupt-bin");
  const ready = join(temporary, "interrupt-ready");
  mkdirSync(interruptBin);
  const cargo = join(interruptBin, "cargo");
  writeFileSync(
    cargo,
    "#!/bin/sh\nprintf '%s' \"$$\" > \"$RD_INTERRUPT_READY\"\nparent=$PPID\ntrap 'exit 130' INT TERM\nwhile kill -0 \"$parent\" 2>/dev/null; do sleep 0.05; done\n",
  );
  chmodSync(cargo, 0o755);
  const before = await repositoryState();
  const child = Bun.spawn(
    [installed.wrapper, repositoryRoot, "--yes"],
    {
      cwd: installed.install,
      env: {
        ...installed.environment,
        PATH: `${interruptBin}:${process.env.PATH}`,
        RD_INTERRUPT_READY: ready,
      },
      stdin: "ignore",
      stdout: "ignore",
      stderr: "ignore",
    },
  );
  let exited = false;
  let cargoPid;
  try {
    await waitUntil(
      () => existsSync(ready),
      "interrupted packed scan did not reach Cargo",
    );
    cargoPid = Number(readFileSync(ready, "utf8"));
    assert(Number.isSafeInteger(cargoPid) && cargoPid > 0, "interrupted Cargo pid is invalid");
    child.kill("SIGINT");
    const code = await Promise.race([
      child.exited,
      Bun.sleep(5000).then(() => {
        throw new Error("interrupted packed scan did not exit");
      }),
    ]);
    exited = true;
    assert(code === 130, `interrupted packed scan exited with ${code}`);
    await waitUntil(
      () => !processExists(cargoPid),
      "interrupted packed scan left its Cargo process alive",
    );
    const after = await repositoryState();
    compareRepositoryState(before, after, "interrupted dogfood");
    return {
      observation: {
        interrupted_exit: code,
        interrupted_repository_state: "unchanged",
        interrupted_orphans: 0,
      },
    };
  } finally {
    if (!exited) {
      child.kill("SIGKILL");
      await child.exited;
    }
    if (cargoPid && processExists(cargoPid)) process.kill(cargoPid, "SIGKILL");
  }
}

function proveNonMutation(before, after, interruption) {
  const observation = compareRepositoryState(before, after, "completed dogfood");
  assert(
    interruption.observation.interrupted_repository_state === "unchanged",
    "interrupted dogfood mutation proof is missing",
  );
  observation.cargo_targets = "temporary directories outside the repository";
  return {
    observation,
    criterion: criterion(
      "US-062-AC-6",
      "completed and interrupted runs preserved Git, worktree, modes and forbidden outputs",
    ),
  };
}

async function packageEvidence(packed, installed) {
  return {
    versions: {
      rust_doctor: packed.version,
      node: successful(run(["node", "--version"]), "Node version"),
      bun: successful(run(["bun", "--version"]), "Bun version"),
      rustc: successful(run(["rustc", "--version"]), "rustc version"),
    },
    tarballs: {
      wrapper: {
        file: basename(packed.wrapperArchive),
        sha256: await sha256(packed.wrapperArchive),
        inventory: installed.wrapperInventory,
      },
      native: {
        file: basename(packed.nativeArchive),
        sha256: await sha256(packed.nativeArchive),
        inventory: installed.nativeInventory,
      },
    },
  };
}

function buildArtifact({ evidence, installed, cli, deterministic, handoff, interruption, nonMutation }) {
  const measuredCriteria = [
    installed.criterion,
    ...cli.criteria,
    handoff.criterion,
    deterministic.criterion,
    nonMutation.criterion,
  ];
  assert(
    measuredCriteria.map(({ id }) => id).join(",")
      === [1, 2, 3, 4, 5, 6].map((id) => `US-062-AC-${id}`).join(","),
    "measured dogfood criteria are incomplete or out of order",
  );
  const criteria = [
    ...measuredCriteria,
    criterion(
      "US-062-AC-7",
      "validated artifact fields derive from measured proof results",
    ),
    criterion(
      "US-062-AC-8",
      "all required criteria were produced by successful proof functions",
    ),
  ];
  const artifact = {
    schema_version: 1,
    epic: "EP-021",
    platform: "linux-x64",
    versions: evidence.versions,
    tarballs: evidence.tarballs,
    commands: [
      "bun run scripts/pack-local.mjs",
      "bun install --ignore-scripts --backend=copyfile",
      "node_modules/.bin/rust-doctor --version",
      "node_modules/.bin/rust-doctor <repository> --yes",
      "node_modules/.bin/rust-doctor <repository> --json",
      "node_modules/.bin/rust-doctor <repository> --yes <SIGINT>",
      "cargo test --test local_cli_experience tty::packed_wrapper_tty_handoff_contract_when_requested -- --exact",
    ],
    observations: {
      ...installed.observation,
      ...cli.observation,
      ...handoff.observation,
      ...deterministic.observation,
      ...interruption.observation,
    },
    non_mutation: nonMutation.observation,
    criteria,
    verdict: "DONE",
  };
  const serialized = JSON.stringify(artifact);
  assert(!serialized.includes(tmpdir()), "dogfood artifact contains a temporary path");
  assert(
    Object.values(artifact.tarballs).every(({ sha256: digest }) => /^[0-9a-f]{64}$/u.test(digest)),
    "dogfood artifact contains an invalid tarball hash",
  );
  assert(
    artifact.criteria.length === 8
      && artifact.criteria.every(({ verdict }) => verdict === "PASS"),
    "dogfood artifact criteria are incomplete",
  );
  return artifact;
}

function writeArtifact(artifact) {
  // The build this proof drives goes to a temporary target directory, so
  // `target/` need not exist at all: it does on a machine that has built the
  // crate before, and does not on a fresh runner.
  mkdirSync(dirname(artifactPath), { recursive: true });
  const temporaryPath = `${artifactPath}.${process.pid}-${Date.now()}.tmp`;
  try {
    writeFileSync(temporaryPath, `${JSON.stringify(artifact, null, 2)}\n`, { flag: "wx" });
    renameSync(temporaryPath, artifactPath);
  } finally {
    rmSync(temporaryPath, { force: true });
  }
}

const temporary = mkdtempSync(join(tmpdir(), "rust-doctor-packed-smoke-"));
let packed;
try {
  const before = await repositoryState();
  packed = packLocal();
  const installed = installPacked(temporary, packed);
  const cli = proveCli(installed);
  const deterministic = proveDeterminism(installed);
  const handoff = proveHandoff(temporary, installed);
  const interruption = await proveInterruption(temporary, installed);
  const evidence = await packageEvidence(packed, installed);
  const after = await repositoryState();
  const nonMutation = proveNonMutation(before, after, interruption);
  const artifact = buildArtifact({
    evidence,
    installed,
    cli,
    deterministic,
    handoff,
    interruption,
    nonMutation,
  });
  writeArtifact(artifact);
  process.stdout.write(`Packed dogfood passed; wrote ${relative(repositoryRoot, artifactPath)}\n`);
} finally {
  if (packed) rmSync(packed.output, { recursive: true, force: true });
  rmSync(temporary, { recursive: true, force: true });
}
