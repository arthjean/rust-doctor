import { BIN, defaultTarget, doctorArgs, type EnvOverrides, type Sandbox } from "./e2e-helpers";
import { spawnPty } from "./pty";
import type { TmuxSession } from "./tmux-helpers";

const ESCAPES = /\x1b\[[0-?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)/g;
export const SGR = /\x1b\[[0-9;]*m/;

export function stripAnsi(text: string): string {
  return text.replace(ESCAPES, "");
}

const LANDING = /(?:^|\s)q quit\s*$/m;

export type PtyReport = {
  exitCode: number | null;
  output: string;
  text: string;
  landing: boolean;
};

export type PtyReportOptions = {
  args?: string[];
  env?: EnvOverrides;
  sandbox?: Sandbox;
  cols?: number;
  rows?: number;
  shell?: string;
};

export async function ptyReport(workspace: string, options: PtyReportOptions = {}): Promise<PtyReport> {
  const args = doctorArgs([workspace, ...(options.args ?? [])]);
  const argv = options.shell ? ["sh", "-c", options.shell, BIN, ...args] : [BIN, ...args];
  const session = spawnPty(argv, {
    cols: options.cols ?? 100,
    rows: options.rows ?? 40,
    sandbox: options.sandbox,
    env: { NO_COLOR: "1", CARGO_TARGET_DIR: defaultTarget([workspace], workspace), ...options.env },
  });
  let done = false;
  const exited = session.exited.then((exit) => {
    done = true;
    return exit;
  });
  let landing = false;
  while (!done) {
    if (!landing && LANDING.test(stripAnsi(session.output()))) {
      landing = true;
      session.write("q");
    }
    await Bun.sleep(25);
  }
  const exit = await exited;
  return { exitCode: exit.exitCode, output: exit.output, text: stripAnsi(exit.output).replace(/\r/g, ""), landing };
}

export function frames(session: TmuxSession): string[] {
  const log = Buffer.from(session.byteLog(), "latin1").toString("utf8");
  return log.split("\x1b[J").slice(0, -1).map(stripAnsi);
}

export function frameScores(session: TmuxSession): number[] {
  return frames(session)
    .map((frame) => frame.match(/(\d+) \/ 100/))
    .filter((match): match is RegExpMatchArray => match !== null)
    .map((match) => Number(match[1]));
}

export function selected(pane: string): string {
  return pane.match(/❯ (.+)$/m)?.[1]?.trim() ?? "";
}

export async function quit(session: TmuxSession, key = "q"): Promise<number | null> {
  session.sendKeys(key);
  return session.waitForExit();
}
