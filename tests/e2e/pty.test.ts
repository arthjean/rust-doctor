// The pseudo-terminal helper, proven on a shell and then on the binary: the
// child sees a terminal of the requested size on both streams, reads the
// input, and its exit code comes back.
import { describe, expect, test } from "bun:test";
import { join } from "node:path";
import { BIN, copyWorkspace, runDoctor, scratchDir } from "./e2e-helpers";
import { runPty, spawnPty } from "./pty";

describe("pseudo-terminal", () => {
  test("the child sees a terminal of the requested size on stdin and stdout", async () => {
    const run = await runPty(
      ["sh", "-c", 'stty size; [ -t 0 ] && echo stdin-tty; [ -t 1 ] && echo stdout-tty; exit 5'],
      { cols: 97, rows: 31 },
    );
    expect(run.exitCode).toBe(5);
    expect(run.output).toContain("31 97");
    expect(run.output).toContain("stdin-tty");
    expect(run.output).toContain("stdout-tty");
  });

  test("input reaches the child, and a resize reaches its next read", async () => {
    const session = spawnPty(["sh", "-c", 'read first; echo "got $first"; read second; stty size'], {
      cols: 40,
      rows: 10,
      input: "abc\n",
    });
    while (!session.output().includes("got abc")) await Bun.sleep(25);
    session.resize(120, 30);
    session.write("\n");
    const run = await session.exited;
    expect(run.exitCode).toBe(0);
    expect(run.output).toContain("30 120");
  });

  test("the binary colors its report on a terminal and not into a pipe", async () => {
    const workspace = copyWorkspace("kernel-contract/todo");
    const terminal = await runPty([BIN, workspace, "--yes", "--advisory-db", "/dev/null"], {
      cols: 100,
      rows: 40,
      env: { CARGO_TARGET_DIR: join(scratchDir("pty-target"), "target") },
    });
    const piped = await runDoctor([workspace, "--yes"]);
    expect(terminal.exitCode).toBe(0);
    expect(piped.exitCode).toBe(0);
    expect(terminal.output).toContain("\x1b[");
    expect(piped.stdout).not.toContain("\x1b[");
  });
});
