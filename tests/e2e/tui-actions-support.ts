// What the action screens of the interactive report need beyond tui-support.ts:
// a fixture whose findings include info, a read-only workflows directory,
// agents written for one test, a session whose stderr goes to a file, and the
// byte log read as text.
import { chmodSync, existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { BIN, defaultTarget, doctorArgs, scratchDir, toolchainBin, type EnvOverrides, type Sandbox } from "./e2e-helpers";
import { pathProfile, type Profile } from "./fakes";
import { TmuxSession } from "./tmux-helpers";
import { tuiWorkspace, type TuiWorkspace } from "./tui-support";

// A binary whose only findings are three `println!` sites, which the report
// grades as info, beside the two warnings a lockless binary gets.
export function infoBinary(): string {
  const workspace = join(scratchDir("info-binary"), "info-binary");
  mkdirSync(join(workspace, "src"), { recursive: true });
  writeFileSync(join(workspace, "Cargo.toml"), '[package]\nname = "info-binary"\nversion = "0.1.0"\nedition = "2024"\npublish = false\n\n[workspace]\n');
  writeFileSync(join(workspace, "src/main.rs"), 'fn main() {\n    println!("a");\n    println!("b");\n    println!("c");\n}\n');
  return workspace;
}

export const IS_ROOT = process.getuid?.() === 0;

// `todo-rogh` of the catalog: `todo-git` whose `.github/workflows` cannot be
// written. The caller restores the mode in `finally`, or the run's scratch
// root cannot be removed.
export async function readOnlyWorkflows(): Promise<TuiWorkspace & { restore(): void }> {
  const ws = await tuiWorkspace("todo-git");
  const dir = join(ws.path, ".github/workflows");
  mkdirSync(dir, { recursive: true });
  chmodSync(dir, 0o555);
  return { ...ws, restore: () => chmodSync(dir, 0o755) };
}

// A PATH whose agents are the shell scripts given here, in front of a fakes.ts
// profile, for a test that needs an agent to report its terminals, fail or
// run long. Each script records into `$RD_AGENT_LOG/<name>.*`.
export function agentPath(scripts: Record<string, string>, profile: Profile | readonly string[] = "none"): string {
  const dir = join(scratchDir("agents"), "bin");
  mkdirSync(dir, { recursive: true });
  for (const [name, body] of Object.entries(scripts)) {
    writeFileSync(join(dir, name), body, { mode: 0o755 });
  }
  return `${dir}:${profile === "none" ? toolchainBin() : pathProfile(profile)}`;
}

// An agent that records whether stdin, stdout and stderr are terminals and
// its pid, prints one line, optionally sleeps, and exits with `code`.
export function probeAgent(name: string, options: { code?: number; sleep?: number } = {}): string {
  return [
    "#!/bin/sh",
    'mkdir -p "$RD_AGENT_LOG"',
    `log="$RD_AGENT_LOG/${name}"`,
    'echo $$ > "$log.pid"',
    'pwd > "$log.cwd"',
    't=""; for fd in 0 1 2; do if [ -t $fd ]; then t="${t}y"; else t="${t}n"; fi; done',
    'printf %s "$t" > "$log.tty"',
    'printf %s "$1" > "$log.payload"',
    `echo "FAKE-AGENT ${name} RAN"`,
    options.sleep ? `sleep ${options.sleep}` : "",
    `exit ${options.code ?? 0}`,
    "",
  ].join("\n");
}

export function agentLog(box: Sandbox): string {
  return join(box.dir, "agent-log");
}

export function agentRecord(box: Sandbox, name: string, field: "pid" | "cwd" | "tty" | "payload"): string | undefined {
  const path = join(agentLog(box), `${name}.${field}`);
  return existsSync(path) ? readFileSync(path, "utf8") : undefined;
}

// The report on `workspace` with its stderr in a file, so the pane holds
// only what stdout wrote and the test reads stderr whole.
export function reportWithStderrFile(
  ws: { path: string; sandbox: Sandbox },
  options: { width?: number; height?: number; env?: EnvOverrides; args?: string[] } = {},
): { session: TmuxSession; stderr: () => string } {
  const path = join(ws.sandbox.dir, `stderr-${Date.now()}-${Math.random().toString(36).slice(2)}.txt`);
  const args = doctorArgs([ws.path, ...(options.args ?? [])]);
  const session = TmuxSession.create(["sh", "-c", 'f="$1"; shift; exec "$@" 2>"$f"', "sh", path, BIN, ...args], {
    width: options.width ?? 100,
    height: options.height ?? 30,
    sandbox: ws.sandbox,
    env: { NO_COLOR: "1", CARGO_TARGET_DIR: defaultTarget([ws.path], ws.sandbox.dir), ...options.env },
  });
  return { session, stderr: () => (existsSync(path) ? readFileSync(path, "utf8") : "") };
}

// The byte log as a terminal would show it line by line: the line discipline
// turns every newline into CR LF.
export function logText(session: TmuxSession): string {
  return session.byteLog().replace(/\r\n/g, "\n");
}

export async function porcelain(ws: TuiWorkspace): Promise<string> {
  if (!ws.git) throw new Error(`${ws.kind} is not a git repository`);
  return ws.git(["status", "--porcelain", "--untracked-files=all"]);
}

export async function waitForFile(path: string, timeoutMs = 15_000): Promise<string> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (existsSync(path)) {
      const body = readFileSync(path, "utf8").trim();
      if (body.length > 0) return body;
    }
    await Bun.sleep(25);
  }
  throw new Error(`${path} did not appear within ${timeoutMs} ms`);
}

export function alive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

// Every live process whose command line mentions `needle`.
export function processesMentioning(needle: string): number[] {
  const pids: number[] = [];
  for (const name of readdirSync("/proc")) {
    if (!/^\d+$/.test(name)) continue;
    try {
      if (readFileSync(`/proc/${name}/cmdline`, "latin1").includes(needle)) pids.push(Number(name));
    } catch {}
  }
  return pids;
}

// The byte log once tmux's `pipe-pane` has flushed it: a pane that just died
// can still have its last bytes on the way to the file.
export async function settledLog(session: TmuxSession, quietMs = 150, timeoutMs = 3_000): Promise<string> {
  const deadline = Date.now() + timeoutMs;
  let last = session.byteLog();
  let since = Date.now();
  while (Date.now() < deadline) {
    await Bun.sleep(25);
    const now = session.byteLog();
    if (now !== last) {
      last = now;
      since = Date.now();
    } else if (Date.now() - since >= quietMs) {
      break;
    }
  }
  return last;
}
