// PATH profiles built from `fakes/`: each one is a directory of links to the
// recording stand-ins it allows, put in front of the toolchain directory, so a
// test decides which agents and clipboard commands exist and never reaches the
// machine's real ones.
import { existsSync, mkdirSync, readdirSync, readFileSync, symlinkSync } from "node:fs";
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

// The PATH of a profile, or of an explicit list of fake names.
export function pathProfile(profile: Profile | readonly string[]): string {
  if (profile === "none") return toolchainBin();
  const names = typeof profile === "string" ? MEMBERS[profile] : profile;
  const key = typeof profile === "string" ? profile : `custom-${[...names].sort().join("-")}`;
  const dir = join(RUN_ROOT, "profiles", key);
  if (!existsSync(dir)) {
    mkdirSync(dir, { recursive: true });
    for (const name of names) {
      const fake = join(import.meta.dir, "fakes", name);
      if (!existsSync(fake)) throw new Error(`no fake named ${name} under fakes/`);
      symlinkSync(fake, join(dir, name));
    }
  }
  return `${dir}:${toolchainBin()}`;
}

// The variables a spawn needs for the fakes to record under its sandbox.
export function fakeEnv(box: Sandbox, profile: Profile | readonly string[]): Record<string, string> {
  return { PATH: pathProfile(profile), RD_FAKE_LOG: join(box.dir, "fake-calls") };
}

export type FakeCall = { argv: string[]; stdin: string; cwd: string; tty: boolean };

// Every completed call of `tool` under the sandbox, in call order.
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
