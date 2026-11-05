import { runDoctor, targetDir } from "./e2e-helpers";
import { MENU_HINT, openReport, type TmuxSession } from "./tmux-helpers";
import { tuiWorkspace, type TuiWorkspace } from "./tui-support";

export function rows(pane: string): string[] {
  const lines = pane.split("\n");
  if (lines.at(-1) === "") lines.pop();
  return lines;
}

export function cells(line: string): number {
  return [...line.replace(/\s+$/, "")].length;
}

export function widest(pane: string): number {
  return Math.max(0, ...rows(pane).map(cells));
}

export function rowsUsed(pane: string): number {
  const lines = rows(pane);
  let last = -1;
  lines.forEach((line, index) => {
    if (line.trim() !== "") last = index;
  });
  return last + 1;
}

export function faces(pane: string): number {
  return pane.split("┌─────┐").length - 1;
}

export function barCells(pane: string): number {
  const bar = rows(pane).find((line) => /[█░]/.test(line)) ?? "";
  return (bar.match(/[█▓░]/g) ?? []).length;
}

export function position(pane: string): { index: number; total: number } {
  const match = pane.match(/issue (\d+)\/(\d+)/);
  if (!match) throw new Error(`no issue position in the pane:\n${pane}`);
  return { index: Number(match[1]), total: Number(match[2]) };
}

export function unread(pane: string): number {
  const match = pane.match(/(\d+) issues? unread/);
  if (!match) throw new Error(`no unread counter in the pane:\n${pane}`);
  return Number(match[1]);
}

export function dividerColumns(pane: string): number[] {
  return [...new Set(rows(pane).map((line) => [...line].indexOf("│", 12)).filter((column) => column >= 0))];
}

export function utf8(log: string): string {
  return Buffer.from(log, "latin1").toString("utf8");
}

export function count(haystack: string, needle: string): number {
  return haystack.split(needle).length - 1;
}

type Report = {
  diagnostics: Array<{ code: string; severity: string; category: string | null }>;
  audit: { categories: Array<{ name: string }> };
};

export function issueCount(report: Report): number {
  return new Set(report.diagnostics.map((diagnostic) => diagnostic.code)).size;
}

export function listEntries(report: Report): number {
  const categories = (report.audit.categories as Array<{ name: string; distinct: { total: number } }>)
    .filter((category) => category.name !== "Other" && category.distinct.total > 0).length;
  const notes = report.diagnostics.some((diagnostic) => diagnostic.category === null) ? 1 : 0;
  return categories + notes + issueCount(report);
}

export type Warm = { ws: TuiWorkspace; target: string; report: Report };

const warmed = new Map<string, Promise<Warm>>();

export function warm(kind: Parameters<typeof tuiWorkspace>[0]): Promise<Warm> {
  let entry = warmed.get(kind);
  if (!entry) {
    entry = (async () => {
      const ws = await tuiWorkspace(kind);
      const target = targetDir(`warm-${kind}`);
      const run = await runDoctor([ws.path, "--yes", "--json"], { sandbox: ws.sandbox, target, timeoutMs: 180_000 });
      if (!run.json) throw new Error(`warming ${kind} produced no report: ${run.stderr}`);
      return { ws, target, report: run.json as Report };
    })();
    warmed.set(kind, entry);
  }
  return entry;
}

export function open(
  warmed: Warm,
  options: { width: number; height: number; color?: boolean; env?: Record<string, string | undefined>; args?: string[] },
): TmuxSession {
  return openReport(warmed.ws.path, {
    sandbox: warmed.ws.sandbox,
    width: options.width,
    height: options.height,
    color: options.color,
    args: options.args,
    env: { CARGO_TARGET_DIR: warmed.target, ...options.env },
  });
}

export async function openViewer(session: TmuxSession, landing: string | RegExp = MENU_HINT): Promise<string> {
  await session.waitForPane(landing, 30_000);
  session.sendKeys("Enter");
  await session.waitForPane(/issue 1\//);
  return session.waitForStable(300);
}

export async function press(session: TmuxSession, ...keys: string[]): Promise<string> {
  session.sendKeys(...keys);
  return session.waitForStable(250);
}
