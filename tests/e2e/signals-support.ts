// What the signal and deadline files share: the sleeping build script's pid
// file as the moment to signal, process groups read from /proc and probed
// with `kill -0 -<pgid>`, and a spawn whose stderr is read while it runs, for
// a scan with no build script to synchronize on.
import { existsSync, readFileSync } from "node:fs";
import {
  BIN,
  defaultTarget,
  doctorArgs,
  hermeticEnv,
  sandbox,
  track,
  untrack,
  type DoctorOptions,
  type Sandbox,
} from "./e2e-helpers";

export async function waitForFile(path: string, timeoutMs = 30_000): Promise<string> {
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

// The process group of `pid`, field 5 of /proc/<pid>/stat.
export function groupOf(pid: number): number {
  const stat = readFileSync(`/proc/${pid}/stat`, "utf8");
  return Number(stat.slice(stat.lastIndexOf(")") + 2).split(" ")[2]);
}

// `kill -0 -<pgid>`: whether any process of the group still exists.
export function groupAlive(pgid: number): boolean {
  try {
    process.kill(-pgid, 0);
    return true;
  } catch {
    return false;
  }
}

// Polls until the group is empty, for children a signal reaches a moment
// after their parent exits; true when it emptied in time.
export async function groupGone(pgid: number, timeoutMs = 5_000): Promise<boolean> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (!groupAlive(pgid)) return true;
    await Bun.sleep(25);
  }
  return !groupAlive(pgid);
}

export function killGroup(pgid: number): void {
  try {
    process.kill(-pgid, "SIGKILL");
  } catch {}
}

export type Streaming = {
  pid: number;
  proc: Bun.Subprocess<"ignore", "pipe", "pipe">;
  sandbox: Sandbox;
  stderr(): string;
  waitForStderr(text: string, timeoutMs?: number): Promise<void>;
  exited: Promise<{ exitCode: number | null; signal: string | null; stdout: string; stderr: string }>;
};

// The binary with the defaults of `spawnDoctor`, its stderr readable while it
// runs. The preload kills it if a test leaves it running.
export function spawnStreaming(args: string[], options: DoctorOptions = {}): Streaming {
  const box = options.sandbox ?? sandbox("streaming");
  const cwd = options.cwd ?? box.dir;
  const env = hermeticEnv(box, { CARGO_TARGET_DIR: options.target ?? defaultTarget(args, cwd), ...options.env });
  const proc = Bun.spawn([BIN, ...doctorArgs(args, options)], { cwd, env, stdin: "ignore", stdout: "pipe", stderr: "pipe" });
  track(proc.pid, `rust-doctor ${args.join(" ")}`);
  let err = "";
  const decoder = new TextDecoder();
  const drained = (async () => {
    for await (const chunk of proc.stderr) err += decoder.decode(chunk, { stream: true });
  })();
  const exited = (async () => {
    const stdout = await new Response(proc.stdout).text();
    await drained;
    await proc.exited;
    untrack(proc.pid);
    return { exitCode: proc.exitCode, signal: proc.signalCode, stdout, stderr: err };
  })();
  return {
    pid: proc.pid,
    proc,
    sandbox: box,
    stderr: () => err,
    async waitForStderr(text, timeoutMs = 30_000) {
      const deadline = Date.now() + timeoutMs;
      while (Date.now() < deadline) {
        if (err.includes(text)) return;
        await Bun.sleep(10);
      }
      throw new Error(`stderr never showed ${JSON.stringify(text)} within ${timeoutMs} ms; stderr:\n${err}`);
    },
    exited,
  };
}
