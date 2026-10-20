// `tasks/e2e-catalog/tui.md` section 3.13: how fast the interactive report
// shows, repaints and idles, against the timings of section 1.5 with 50
// percent headroom. Every workspace is warmed first, so a budget measures the
// report and not a compile, and the rows run one after another: bun runs a
// file's tests serially, and nothing here is concurrent. A runner slower than
// the machine the timings came from declares itself through
// RUST_DOCTOR_BENCHMARK_ALLOWANCE, as the Rust benchmark does; it stretches
// the clocks only, never a byte or frame count.
import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { runDoctor, type Sandbox } from "./e2e-helpers";
import { cloneThisRepository, type SelfClone } from "./self-clone";
import { settledLog } from "./tui-actions-support";
import { MENU_HINT, openReport, requireTmux, TmuxSession } from "./tmux-helpers";
import { tuiWorkspace, type TuiKind } from "./tui-support";

const ALLOWANCE = Number(process.env.RUST_DOCTOR_BENCHMARK_ALLOWANCE ?? "1");
const HEADROOM = 1.5 * ALLOWANCE;

type Opened = { path: string; sandbox: Sandbox; target?: string };

let clone: SelfClone | undefined;

async function repo(): Promise<Opened> {
  clone ??= await cloneThisRepository("performance");
  return { path: clone.path, sandbox: clone.sandbox, target: clone.target("tui") };
}

// A workspace scanned once, so the session that follows finds its build warm.
async function warmed(kind: TuiKind | "repo"): Promise<Opened> {
  const ws: Opened = kind === "repo" ? await repo() : await tuiWorkspace(kind);
  const run = await runDoctor([ws.path, "--yes", "--json"], { sandbox: ws.sandbox, target: ws.target, timeoutMs: 300_000 });
  expect(run.json).toBeDefined();
  return ws;
}

function session(ws: Opened, color: boolean, size = { width: 100, height: 40 }): TmuxSession {
  return openReport(ws.path, {
    ...size,
    color,
    sandbox: ws.sandbox,
    env: ws.target ? { CARGO_TARGET_DIR: ws.target } : {},
  });
}

// The landing's own marker: the menu hint, or `q quit` on a landing with no
// action.
function landed(pane: string): boolean {
  return pane.includes(MENU_HINT) || /^q quit$/m.test(pane);
}

// User and system ticks the process has used so far.
function cpuTicks(pid: number): number {
  const stat = readFileSync(`/proc/${pid}/stat`, "utf8");
  const fields = stat.slice(stat.lastIndexOf(")") + 2).split(" ");
  return Number(fields[11]) + Number(fields[12]);
}

// Section 1.5: the hint, then a pane quiet for 0.4 s, in seconds. Since
// US-009 `denied` and `partial` publish no score, so like `no-rust` they draw
// no count-up even in color.
const TIMINGS: Array<[string, TuiKind | "repo", boolean, number, number]> = [
  ["no-rust", "no-rust", false, 0.12, 0.53],
  ["todo without color", "todo", false, 0.11, 0.5],
  ["repo without color", "repo", false, 0.85, 1.25],
  ["clean", "clean", true, 2.13, 2.54],
  ["denied", "denied", true, 0.12, 0.53],
  ["partial", "partial", true, 0.12, 0.53],
  ["todo", "todo", true, 2.65, 3.07],
  ["uncatalogued", "uncatalogued", true, 2.65, 3.07],
  ["repo", "repo", true, 3.02, 3.43],
];

describe.skipIf(!requireTmux())("interactive report performance", () => {
  test.each(TIMINGS)("PERF-01 %s shows its landing and settles within its budget", async (label, kind, color, hint, stable) => {
    const ws = await warmed(kind);
    const started = performance.now();
    const report = session(ws, color);
    try {
      await report.waitForPane(landed, 30_000);
      const shown = (performance.now() - started) / 1000;
      await report.waitForStable(400);
      const settled = (performance.now() - started) / 1000;
      console.log(`PERF-01 ${label}: hint ${shown.toFixed(2)} s (budget ${(hint * HEADROOM).toFixed(2)}), stable ${settled.toFixed(2)} s (budget ${(stable * HEADROOM).toFixed(2)})`);
      expect(shown).toBeLessThanOrEqual(hint * HEADROOM);
      expect(settled).toBeLessThanOrEqual(stable * HEADROOM);
    } finally {
      report.kill();
    }
  }, 120_000);

  test("PERF-02 a key in the split viewer repaints in under 150 ms", async () => {
    const ws = await warmed("repo");
    const report = session(ws, false, { width: 120, height: 40 });
    try {
      await report.waitForPane(MENU_HINT, 30_000);
      report.sendKeys("Enter");
      await report.waitForPane("issue 1/");
      report.sendKeys("G");
      const last = Number((await report.waitForPane(/issue (\d+)\/\1\b/)).match(/issue (\d+)\//)![1]);
      expect(last).toBeGreaterThan(5);
      const latencies: number[] = [];
      for (let issue = last - 1; issue >= last - 5; issue -= 1) {
        const started = performance.now();
        report.sendKeys("k");
        while (!report.capturePane().includes(`issue ${issue}/${last}`)) {
          if (performance.now() - started > 2_000) throw new Error(`issue ${issue}/${last} never painted`);
        }
        latencies.push(performance.now() - started);
      }
      const median = [...latencies].sort((a, b) => a - b)[2]!;
      console.log(`PERF-02 repaint latencies ${latencies.map((ms) => ms.toFixed(1)).join(", ")} ms, median ${median.toFixed(1)}`);
      expect(median).toBeLessThan(100 * HEADROOM);
    } finally {
      report.kill();
    }
  }, 120_000);

  test("PERF-03 the count-up lasts its frames, not a compile", async () => {
    const ws = await warmed("uncatalogued");
    const report = session(ws, true);
    try {
      await report.waitForPane("/ 100", 30_000);
      const started = performance.now();
      await report.waitForPane(MENU_HINT, 15_000);
      const seconds = (performance.now() - started) / 1000;
      console.log(`PERF-03 count-up ${seconds.toFixed(2)} s`);
      expect(seconds).toBeGreaterThanOrEqual(2.0);
      expect(seconds).toBeLessThanOrEqual(3.2 * HEADROOM);
    } finally {
      report.kill();
    }
  });

  test("PERF-04 the landing draws 58 frames before the first key", async () => {
    const ws = await warmed("uncatalogued");
    const report = session(ws, true);
    try {
      await report.waitForPane(MENU_HINT, 30_000);
      await report.waitForStable(400);
      const frames = (await settledLog(report)).split("\x1b[J").length - 1;
      console.log(`PERF-04 ${frames} frames`);
      expect(frames).toBe(58);
    } finally {
      report.kill();
    }
  });

  test("PERF-05 waiting for a key costs no CPU", async () => {
    const ws = await warmed("repo");
    const report = session(ws, false);
    try {
      await report.waitForPane(MENU_HINT, 30_000);
      await report.waitForStable(400);
      const pid = report.panePid();
      const before = cpuTicks(pid);
      await Bun.sleep(3_000);
      const used = cpuTicks(pid) - before;
      console.log(`PERF-05 ${used} ticks over 3 s`);
      // A blocking read uses none; one tick absorbs a boundary of the clock.
      expect(used).toBeLessThanOrEqual(1);
    } finally {
      report.kill();
    }
  });

  test("PERF-06 the landing session of a real report writes under 54 KB", async () => {
    const ws = await warmed("repo");
    const report = session(ws, true);
    try {
      await report.waitForPane(MENU_HINT, 30_000);
      await report.waitForStable(400);
      const bytes = Buffer.byteLength(await settledLog(report), "latin1");
      console.log(`PERF-06 ${bytes} bytes`);
      expect(bytes).toBeLessThanOrEqual(36 * 1024 * 1.5);
    } finally {
      report.kill();
    }
  });
});
