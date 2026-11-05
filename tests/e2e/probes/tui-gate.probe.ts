import { describe, expect, test } from "bun:test";
import { requireTmux } from "../tmux-helpers";

describe.skipIf(!requireTmux())("a TUI file", () => {
  test("runs only where tmux is installed", () => {
    expect(true).toBe(true);
  });
});
