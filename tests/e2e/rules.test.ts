// `tasks/e2e-catalog/subcommands.md` section 2: `rules list` and `rules
// explain`. EP-005 fills it; the row here is the open explainer defect of the
// ledger.
import { describe, expect, test } from "bun:test";
import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { runDoctor, scratchDir } from "./e2e-helpers";

describe("rules explain", () => {
  test.failing("RL-18 subcommands/F12 an override that cannot switch on a rule the plan leaves off says so", async () => {
    const workspace = join(scratchDir("rl-18"), "rl-18");
    mkdirSync(join(workspace, "src"), { recursive: true });
    writeFileSync(join(workspace, "Cargo.toml"), '[package]\nname = "rl-18"\nversion = "0.1.0"\nedition = "2024"\n\n[workspace]\n');
    writeFileSync(join(workspace, "src/lib.rs"), "pub fn f() {}\n");
    writeFileSync(
      join(workspace, "rust-doctor.toml"),
      '[categories]\ncorrectness = "off"\n\n[[overrides]]\npaths = ["src/*.rs"]\ncategories = { correctness = "error" }\n',
    );
    const run = await runDoctor(["rules", "explain", "absurd_extreme_comparisons", workspace]);
    expect(run.exitCode).toBe(0);
    expect(run.stdout).toContain("Effective level: off (config-category)");
    const override = run.stdout.split("\n").find((line) => line.startsWith("Override:")) ?? "";
    expect(override).toMatch(/no effect|cannot|inert/i);
  });
});
