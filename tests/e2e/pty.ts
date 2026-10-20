// A pseudo-terminal from Bun itself (`Bun.spawn({ terminal })`, pinned bun
// 1.4.2), for the tests that only need the binary to see terminal streams:
// the terminal profile, colors, the gating of the interactive report. What a
// screen looks like is tmux's job (tmux-helpers.ts), not this one's.
import { DEFAULT_WATCHDOG_MS, hermeticEnv, sandbox, track, untrack, type EnvOverrides, type Sandbox } from "./e2e-helpers";

export type PtyOptions = {
  cols?: number;
  rows?: number;
  cwd?: string;
  env?: EnvOverrides;
  sandbox?: Sandbox;
  // Written to the terminal once the child has started.
  input?: string;
  timeoutMs?: number;
};

export type PtyExit = { exitCode: number | null; signal: string | null; output: string; durationMs: number };

export type PtySession = {
  pid: number;
  write(data: string): void;
  resize(cols: number, rows: number): void;
  output(): string;
  exited: Promise<PtyExit>;
};

export function spawnPty(argv: string[], options: PtyOptions = {}): PtySession {
  const box = options.sandbox ?? sandbox("pty");
  const decoder = new TextDecoder();
  let output = "";
  let closed: () => void = () => {};
  const eof = new Promise<void>((resolve) => (closed = resolve));
  const started = performance.now();
  const proc = Bun.spawn(argv, {
    cwd: options.cwd ?? box.dir,
    env: hermeticEnv(box, { TERM: "xterm-256color", ...options.env }),
    terminal: {
      cols: options.cols ?? 80,
      rows: options.rows ?? 24,
      data: (_terminal, data) => {
        output += decoder.decode(data, { stream: true });
      },
      exit: () => closed(),
    },
  });
  const terminal = proc.terminal;
  if (!terminal) throw new Error("Bun.spawn returned no terminal: a pseudo-terminal needs Linux or macOS");
  const command = argv.join(" ");
  track(proc.pid, command);
  if (options.input !== undefined) terminal.write(options.input);
  const watchdogMs = options.timeoutMs ?? DEFAULT_WATCHDOG_MS;
  let timedOut = false;
  const watchdog = setTimeout(() => {
    timedOut = true;
    proc.kill("SIGKILL");
  }, watchdogMs);
  const exited = (async (): Promise<PtyExit> => {
    await proc.exited;
    clearTimeout(watchdog);
    untrack(proc.pid);
    // The last bytes the child wrote can still be in the PTY when it exits.
    await Promise.race([eof, Bun.sleep(250)]);
    terminal.close();
    if (timedOut) throw new Error(`watchdog killed ${command} (pid ${proc.pid}) after ${watchdogMs} ms`);
    return {
      exitCode: proc.exitCode,
      signal: proc.signalCode,
      output,
      durationMs: performance.now() - started,
    };
  })();
  return {
    pid: proc.pid,
    write: (data) => void terminal.write(data),
    resize: (cols, rows) => terminal.resize(cols, rows),
    output: () => output,
    exited,
  };
}

export function runPty(argv: string[], options: PtyOptions = {}): Promise<PtyExit> {
  return spawnPty(argv, options).exited;
}
