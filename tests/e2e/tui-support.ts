// The workspaces of `tasks/e2e-catalog/tui.md` section 1.3, each a fresh copy
// whose git state is set here rather than inherited: the same fixture scores
// differently inside and outside a repository, so every interactive-report
// test names the kind it opens.
import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { copyWorkspace, sandbox, scratchDir, type Sandbox } from "./e2e-helpers";
import { gitScenario, type Repo } from "./git-scenario";
import { cloneThisRepository, type SelfClone } from "./self-clone";

export type TuiKind =
  | "todo" | "todo-git" | "todo-nocommit" | "todo-otherwf"
  | "clean" | "denied" | "partial" | "broken-member" | "no-rust" | "tests-only"
  | "uncatalogued" | "sleeping" | "bad-manifest" | "parent1" | "many" | "repo";

export type TuiWorkspace = {
  kind: TuiKind;
  // The path the report is opened on.
  path: string;
  sandbox: Sandbox;
  // Set for the kinds that are git repositories.
  git?: (args: string[]) => Promise<string>;
};

const FIXTURES: Partial<Record<TuiKind, string>> = {
  todo: "kernel-contract/todo",
  clean: "projects/clean",
  denied: "kernel-contract/denied",
  partial: "kernel-contract/partial-failure",
  "broken-member": "scan-failures/broken-member",
  "no-rust": "local-cli-experience/no-rust",
  uncatalogued: "score-reasons/uncatalogued",
  sleeping: "scan-failures/sleeping-build",
  "bad-manifest": "scan-failures/bad-manifest",
};

export const OTHER_WORKFLOW = "on: push\njobs:\n  scan:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: arthjean/rust-doctor@v0\n";

function fromRepo(kind: TuiKind, repo: Repo): TuiWorkspace {
  return { kind, path: repo.path, sandbox: repo.sandbox, git: (args) => repo.git(args) };
}

function fromClone(clone: SelfClone): TuiWorkspace {
  return { kind: "repo", path: clone.path, sandbox: clone.sandbox, git: (args) => clone.git(args) };
}

// `todo-git` commits the fixture with a `.gitignore` for `target/`, so the
// only finding git adds is none: the report matches `todo` but offers the CI
// entry.
export async function tuiWorkspace(kind: TuiKind): Promise<TuiWorkspace> {
  switch (kind) {
    case "todo-git":
      return fromRepo(kind, await gitScenario(kind).fixture("kernel-contract/todo").write({ ".gitignore": "/target\n" }).commit("base").build());
    case "todo-nocommit":
      return fromRepo(kind, await gitScenario(kind).fixture("kernel-contract/todo").write({ ".gitignore": "/target\n" }).build());
    case "todo-otherwf":
      return fromRepo(kind, await gitScenario(kind)
        .fixture("kernel-contract/todo")
        .write({ ".gitignore": "/target\n", ".github/workflows/scan.yml": OTHER_WORKFLOW })
        .commit("base")
        .build());
    case "tests-only": {
      const path = join(scratchDir(kind), kind);
      mkdirSync(join(path, "src"), { recursive: true });
      mkdirSync(join(path, "tests"));
      writeFileSync(join(path, "Cargo.toml"), '[package]\nname = "tests-only"\nversion = "0.1.0"\nedition = "2024"\npublish = false\n\n[workspace]\n');
      writeFileSync(join(path, "src/lib.rs"), "");
      writeFileSync(join(path, "tests/pending.rs"), "#[test]\nfn pending() {\n    todo!()\n}\n");
      return { kind, path, sandbox: sandbox(kind) };
    }
    // Seven rules, one site each, all in production code: enough issues to
    // step, page and count through, a number this suite fixes rather than
    // this repository's (FR-12), and few enough to fit one page at 120x40.
    case "many": {
      const path = join(scratchDir(kind), kind);
      mkdirSync(join(path, "src"), { recursive: true });
      writeFileSync(join(path, "Cargo.toml"), '[package]\nname = "many"\nversion = "0.1.0"\nedition = "2024"\npublish = false\n\n[workspace]\n');
      writeFileSync(join(path, "src/lib.rs"), [
        "pub fn pending() -> u8 {\n    todo!()\n}\n",
        "pub fn missing() -> u8 {\n    unimplemented!()\n}\n",
        "pub fn first(values: &[u8]) -> u8 {\n    *values.first().unwrap()\n}\n",
        "pub fn last(values: &[u8]) -> u8 {\n    *values.last().expect(\"a value\")\n}\n",
        "pub fn refuse() {\n    panic!(\"refused\");\n}\n",
        "pub fn trace(value: u8) -> u8 {\n    dbg!(value)\n}\n",
        "pub fn shout() {\n    println!(\"loud\");\n}\n",
      ].join("\n"));
      return { kind, path, sandbox: sandbox(kind) };
    }
    case "parent1": {
      const parent = join(scratchDir(kind), kind);
      copyWorkspace("kernel-contract/todo", join(parent, "inner"));
      return { kind, path: parent, sandbox: sandbox(kind) };
    }
    case "repo":
      return fromClone(await cloneThisRepository("tui-repo"));
    default: {
      const fixture = FIXTURES[kind];
      if (!fixture) throw new Error(`no fixture for ${kind}`);
      return { kind, path: copyWorkspace(fixture), sandbox: sandbox(kind) };
    }
  }
}
