// Run by tmux-helpers.test.ts in its own bun process with a PATH that holds
// no tmux: a TUI file's gate, which skips or fails depending on
// E2E_REQUIRE_TMUX.
import { describe, expect, test } from "bun:test";
import { requireTmux } from "../tmux-helpers";

describe.skipIf(!requireTmux())("a TUI file", () => {
  test("runs only where tmux is installed", () => {
    expect(true).toBe(true);
  });
});
