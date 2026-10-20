// A person at a terminal, played by tmux. Ported from pf's `TmuxSession`
// (/home/arthur/dev/pf/tests/e2e/tmux-helpers.ts) and cut to what the
// interactive report needs: one server per session on a short relative
// socket, a pane of an explicit size whose program starts only once the byte
// log is attached, keys, captures, polling waits, resize and an idempotent
// kill. Every wait polls every 25 ms and never sleeps blind.
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import {
  BIN,
  defaultTarget,
  doctorArgs,
  hermeticEnv,
  RUN_ROOT,
  sandbox,
  track,
  untrack,
  type EnvOverrides,
  type Sandbox,
} from "./e2e-helpers";

export const TMUX = Bun.which("tmux");

let warned = false;

// Whether a TUI file can run. Without tmux it skips with one visible reason,
// unless E2E_REQUIRE_TMUX=1 (CI), where a missing tmux fails the file.
export function requireTmux(): boolean {
  if (TMUX) return true;
  if (process.env.E2E_REQUIRE_TMUX === "1") {
    throw new Error("tmux is not installed and E2E_REQUIRE_TMUX=1 requires it");
  }
  if (!warned) {
    console.warn("tmux is not installed: TUI files skipped");
    warned = true;
  }
  return false;
}

export function shellQuote(value: string): string {
  return `'${value.replace(/'/g, `'\\''`)}'`;
}

// The directory holding the sockets: ci-run-files.sh gives each file its own
// through TMUX_TMPDIR, so resetting a file's servers reaches no other file.
function socketDir(): string {
  const dir = process.env.TMUX_TMPDIR ?? join(RUN_ROOT, "tmux");
  mkdirSync(dir, { recursive: true });
  return dir;
}

let sessions = 0;

export type SessionOptions = {
  cwd?: string;
  env?: EnvOverrides;
  sandbox?: Sandbox;
  width?: number;
  height?: number;
};

// The stable text markers of `tasks/e2e-catalog/tui.md` section 1.4.
export const MENU_HINT = "↑/↓ move · enter select · q quit";
export const VIEWER_HINT = "esc back · q to quit";
export const CI_SETUP = "Add Rust Doctor to GitHub Actions?";

// The interactive report on `workspace`, without color unless asked: the
// count-up then draws no frame, which removes two seconds from every test
// whose subject is not the animation.
export function openReport(
  workspace: string,
  options: SessionOptions & { color?: boolean; args?: string[] } = {},
): TmuxSession {
  return TmuxSession.doctor([workspace, ...(options.args ?? [])], {
    width: 100,
    height: 30,
    ...options,
    env: { ...(options.color ? {} : { NO_COLOR: "1" }), ...options.env },
  });
}

export type PaneStatus = { dead: boolean; status: number | null };

// How a zombie ended, from field 52 of /proc/<pid>/stat (Linux 3.5 and
// later), in waitpid's encoding: an exit status, or null for a signal. Null
// when the process still runs, is gone, or the field is unreadable.
export function zombieExit(pid: number): { status: number | null } | null {
  let stat: string;
  try {
    stat = readFileSync(`/proc/${pid}/stat`, "utf8");
  } catch {
    return null;
  }
  // Field 2, the command, may hold spaces: the fields after it start at 3.
  const fields = stat.slice(stat.lastIndexOf(")") + 2).trim().split(" ");
  if (fields[0] !== "Z") return null;
  const raw = Number(fields[52 - 3]);
  if (!Number.isInteger(raw)) return null;
  return { status: (raw & 0x7f) === 0 ? (raw >> 8) & 0xff : null };
}

export class TmuxSession {
  private killed = false;

  private constructor(
    private readonly socket: string,
    private readonly dir: string,
    readonly logPath: string,
    readonly sandbox: Sandbox,
  ) {}

  // Starts `argv` in a fresh server, under the sealed environment of
  // `runDoctor` with TERM=xterm-256color, and returns once it runs.
  static create(argv: string[], options: SessionOptions = {}): TmuxSession {
    if (!TMUX) throw new Error("tmux is not installed");
    const box = options.sandbox ?? sandbox("tmux");
    sessions += 1;
    const dir = socketDir();
    const socket = `s${process.pid}-${sessions}`;
    const logPath = join(box.dir, `pane-${sessions}.log`);
    const gate = `${socket}-start`;
    const env = hermeticEnv(box, { TERM: "xterm-256color", LC_ALL: "C.UTF-8", ...options.env });
    const program = [
      "/usr/bin/env", "-i",
      ...Object.entries(env).map(([key, value]) => `${key}=${value}`),
      ...argv,
    ].map(shellQuote).join(" ");
    const pane = `${shellQuote(TMUX)} wait-for ${gate} && cd ${shellQuote(options.cwd ?? box.dir)} && exec ${program}`;
    const session = new TmuxSession(socket, dir, logPath, box);
    track(`tmux:${socket}`, `tmux session ${socket} running ${argv.join(" ")}`, () => session.killServer());
    session.tmux([
      "start-server", ";",
      "set", "-g", "escape-time", "0", ";",
      "set", "-g", "status", "off", ";",
      "set", "-g", "remain-on-exit", "on", ";",
      "set", "-g", "default-terminal", "xterm-256color", ";",
      "new-session", "-d", "-s", "main",
      "-x", String(options.width ?? 80), "-y", String(options.height ?? 24), pane, ";",
      // After the session: tmux 3.4, the one ubuntu-24.04 ships, kills a
      // server that sets `window-size` before it holds a session.
      "set", "-g", "window-size", "manual", ";",
      "pipe-pane", "-t", "main", "-o", `cat >> ${shellQuote(logPath)}`, ";",
      "wait-for", "-S", gate,
    ]);
    return session;
  }

  // The binary in a pane, with the defaults of `runDoctor`: the advisory
  // database off and a target directory per workspace.
  static doctor(args: string[], options: SessionOptions = {}): TmuxSession {
    const box = options.sandbox ?? sandbox("tmux-doctor");
    const cwd = options.cwd ?? box.dir;
    return TmuxSession.create([BIN, ...doctorArgs(args)], {
      ...options,
      sandbox: box,
      cwd,
      env: { CARGO_TARGET_DIR: defaultTarget(args, cwd), ...options.env },
    });
  }

  private tmux(args: string[]): string {
    return execFileSync(TMUX!, ["-u", "-S", this.socket, "-f", "/dev/null", ...args], {
      cwd: this.dir,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
      env: { PATH: "/usr/bin:/bin", HOME: this.sandbox.home, SHELL: "/bin/sh", TERM: "xterm-256color", LC_ALL: "C.UTF-8" },
    });
  }

  // tmux key names: `Down`, `Enter`, `Escape`, `C-c`, `q`.
  sendKeys(...keys: string[]): void {
    this.tmux(["send-keys", "-t", "main", ...keys]);
  }

  sendLiteral(text: string): void {
    this.tmux(["send-keys", "-t", "main", "-l", text]);
  }

  capturePane(options: { escapes?: boolean; history?: boolean } = {}): string {
    return this.tmux([
      "capture-pane", "-p", "-t", "main",
      ...(options.escapes ? ["-e"] : []),
      ...(options.history ? ["-S", "-"] : []),
    ]);
  }

  async waitForPane(match: string | RegExp | ((pane: string) => boolean), timeoutMs = 3_000): Promise<string> {
    const test = typeof match === "function" ? match
      : typeof match === "string" ? (pane: string) => pane.includes(match)
      : (pane: string) => match.test(pane);
    const deadline = Date.now() + timeoutMs;
    let last = "";
    while (Date.now() < deadline) {
      last = this.capturePane();
      if (test(last)) return last;
      await Bun.sleep(25);
    }
    throw new Error(`timed out after ${timeoutMs} ms waiting for ${String(match)} in tmux ${this.socket}; last pane:\n${last}`);
  }

  // Resolves once the pane has not changed for `quietMs`.
  async waitForStable(quietMs = 400, timeoutMs = 15_000): Promise<string> {
    const deadline = Date.now() + timeoutMs;
    let last = this.capturePane();
    let since = Date.now();
    while (Date.now() < deadline) {
      await Bun.sleep(25);
      const pane = this.capturePane();
      if (pane !== last) {
        last = pane;
        since = Date.now();
      } else if (Date.now() - since >= quietMs) {
        return pane;
      }
    }
    throw new Error(`tmux ${this.socket} never stayed still for ${quietMs} ms; last pane:\n${last}`);
  }

  resize(cols: number, rows: number): void {
    this.tmux(["resize-window", "-t", "main", "-x", String(cols), "-y", String(rows)]);
  }

  size(): { cols: number; rows: number } {
    const [cols, rows] = this.tmux(["display", "-p", "-t", "main", "#{window_width} #{window_height}"]).trim().split(" ");
    return { cols: Number(cols), rows: Number(rows) };
  }

  // Every byte the program wrote, escapes included: the place to assert
  // cursor restore, since a dead pane's cursor flag means nothing.
  byteLog(): string {
    return existsSync(this.logPath) ? readFileSync(this.logPath, "latin1") : "";
  }

  status(): PaneStatus {
    const [dead, status, signal, pid] = this.tmux(["display", "-p", "-t", "main", "#{pane_dead}:#{pane_dead_status}:#{pane_dead_signal}:#{pane_pid}"]).trim().split(":");
    if (dead !== "1") return { dead: false, status: null };
    if (status) return { dead: true, status: Number(status) };
    if (signal) return { dead: true, status: null };
    // tmux 3.4 marks the pane dead a moment before it records how the program
    // ended, and sometimes never reaps it at all: a program that exited is then
    // a zombie whose status the kernel still holds.
    const ended = zombieExit(Number(pid));
    return ended ? { dead: true, status: ended.status } : { dead: false, status: null };
  }

  async waitForExit(timeoutMs = 15_000): Promise<number | null> {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      const status = this.status();
      if (status.dead) return status.status;
      await Bun.sleep(25);
    }
    const raw = this.tmux(["display", "-p", "-t", "main", "#{pane_dead}:#{pane_dead_status}:#{pane_dead_signal}:#{pane_pid}"]).trim();
    throw new Error(`the program in tmux ${this.socket} did not exit within ${timeoutMs} ms (pane ${raw}); last pane:\n${this.capturePane()}`);
  }

  // The pid of the program in the pane, for a test that signals it.
  panePid(): number {
    return Number(this.tmux(["display", "-p", "-t", "main", "#{pane_pid}"]).trim());
  }

  private killServer(): void {
    if (this.killed) return;
    this.killed = true;
    try {
      this.tmux(["kill-server"]);
    } catch {}
  }

  kill(): void {
    untrack(`tmux:${this.socket}`);
    this.killServer();
  }
}
