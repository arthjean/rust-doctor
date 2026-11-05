import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { RUN_ROOT, toolchainBin, type Sandbox } from "./e2e-helpers";

export const AGENTS = ["claude", "codex", "cursor-agent"] as const;
export const CLIPBOARDS = ["wl-copy", "xclip", "xsel", "pbcopy"] as const;
export const OTHERS = ["xdg-open", "open", "gh", "git-lfs"] as const;

export type Profile = "none" | "clipboard" | "agents" | "all";

const MEMBERS: Record<Exclude<Profile, "none">, readonly string[]> = {
  clipboard: CLIPBOARDS,
  agents: AGENTS,
  all: [...AGENTS, ...CLIPBOARDS, ...OTHERS],
};

const KNOWN: ReadonlySet<string> = new Set(MEMBERS.all);

export const FAKE_SCRIPT = `#!/bin/sh\n${readFileSync(join(import.meta.dir, "fakes", "record.sh"), "utf8")}`;

function profileDir(names: readonly string[], key: string): string {
  const dir = join(RUN_ROOT, "profiles", key);
  if (!existsSync(dir)) {
    mkdirSync(dir, { recursive: true });
    for (const name of names) {
      if (!KNOWN.has(name)) throw new Error(`no fake named ${name}`);
      writeFileSync(join(dir, name), FAKE_SCRIPT, { mode: 0o755 });
    }
  }
  return dir;
}

export function pathProfile(profile: Profile | readonly string[]): string {
  if (profile === "none") return toolchainBin();
  const names = typeof profile === "string" ? MEMBERS[profile] : profile;
  const key = typeof profile === "string" ? profile : `custom-${[...names].sort().join("-")}`;
  return `${profileDir(names, key)}:${toolchainBin()}`;
}

export function fakeProgram(name: string): string {
  return join(profileDir([name], `custom-${name}`), name);
}

export function fakeEnv(box: Sandbox, profile: Profile | readonly string[]): Record<string, string> {
  return { PATH: pathProfile(profile), RD_FAKE_LOG: join(box.dir, "fake-calls") };
}

export type FakeCall = { argv: string[]; stdin: string; cwd: string; tty: boolean };

export function fakeCalls(box: Sandbox, tool: string): FakeCall[] {
  const log = join(box.dir, "fake-calls");
  if (!existsSync(log)) return [];
  const numbers = readdirSync(log)
    .map((name) => name.match(new RegExp(`^${tool.replace(/[-]/g, "\\-")}\\.(\\d+)\\.argv$`)))
    .filter((match): match is RegExpMatchArray => match !== null)
    .map((match) => Number(match[1]))
    .sort((a, b) => a - b);
  return numbers.map((n) => {
    const base = join(log, `${tool}.${n}`);
    const argv = readFileSync(`${base}.argv`, "utf8");
    return {
      argv: argv.length === 0 ? [] : argv.slice(0, -1).split("\0"),
      stdin: readFileSync(`${base}.stdin`, "utf8"),
      cwd: readFileSync(`${base}.cwd`, "utf8").trim(),
      tty: existsSync(`${base}.tty`),
    };
  });
}
