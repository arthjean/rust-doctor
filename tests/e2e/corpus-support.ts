// The pinned corpus of `tests/corpus.json`, scanned end to end through the
// shipped CLI. A subject is materialized from the clone cache into a scratch
// clone checked out at its pinned commit, as the Rust harness does
// (`materialise` in tests/support/corpus.rs): the repository pass enumerates
// through git, so a bare tree would score a subject as no clone of it scores.
// `--shared` borrows the cache's objects, so the cache is never scanned in
// place and never written. The files that
// use it run only where RUST_DOCTOR_E2E_CORPUS_DIR names that cache, which
// the corpus job restores; anywhere else they skip with one stated reason.
import { existsSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { REPO_ROOT, runCommand, sandbox, scratchDir, toolchainBin, type Sandbox } from "./e2e-helpers";

type ScoreRecord = {
  applied_ceiling: number | null;
  label: string;
  model: string;
  value: number;
  worst_tier: string | null;
};

type RuleRecord = { id: string; distinct: number; occurrences: number };

export type Observation = {
  name: string;
  commit: string;
  status: string;
  exit_code: number;
  authoritative: boolean;
  distinct: number;
  occurrences: number;
  production_lines: number;
  reasons: string[];
  rules: RuleRecord[];
  score: ScoreRecord | null;
};

export type Subject = {
  name: string;
  population: "healthy" | "agent";
  commit: string;
  // The arguments after the materialized path, exactly as the record's run.
  args: string[];
  record: Observation;
};

type Record_ = {
  toolchain: { rustc: string };
  harness: { scan_arguments: string[] };
  manifest: { repositories: Array<{ name: string; commit: string }> };
  observations: Observation[];
  agent_population: {
    repositories: Array<{ name: string; commit: string }>;
    scan_arguments: string[];
    observations: Observation[];
  };
};

const RECORD: Record_ = JSON.parse(readFileSync(join(REPO_ROOT, "tests/corpus.json"), "utf8"));

// The toolchain the record was measured under, `1.97.1` out of
// `rustc 1.97.1 (... )`: every scan pins it, so a machine whose default is
// newer replays the record rather than its own Clippy.
export const RECORD_TOOLCHAIN = RECORD.toolchain.rustc.split(" ")[1]!;

// The environment of the record's run beside the sealed one. A wall-clock
// cutoff would make a large subject's findings depend on machine load, and a
// third-party build script reaches for more of the system than the suite's
// tool directory links (jemalloc's configure under fd runs `expr` and
// `make`), so the system directories follow it, as they did for the record.
export function recordEnv(): Record<string, string> {
  return {
    PATH: `${toolchainBin()}:/usr/local/bin:/usr/bin:/bin`,
    RUSTUP_TOOLCHAIN: RECORD_TOOLCHAIN,
    RUST_DOCTOR_STRUCTURE_TIME_BUDGET_SECS: "600",
  };
}

function population(
  kind: Subject["population"],
  repositories: Array<{ name: string; commit: string }>,
  observations: Observation[],
  args: string[],
): Subject[] {
  return repositories.map((repository) => {
    const record = observations.find((entry) => entry.name === repository.name);
    if (!record) throw new Error(`tests/corpus.json has no observation for ${repository.name}`);
    return { name: repository.name, population: kind, commit: repository.commit, args, record };
  });
}

export const SUBJECTS: Subject[] = [
  ...population("healthy", RECORD.manifest.repositories, RECORD.observations, RECORD.harness.scan_arguments),
  ...population("agent", RECORD.agent_population.repositories, RECORD.agent_population.observations, RECORD.agent_population.scan_arguments),
];

export function subject(name: string): Subject {
  const found = SUBJECTS.find((entry) => entry.name === name);
  if (!found) throw new Error(`no pinned subject named ${name}`);
  return found;
}

export const CORPUS_DIR = process.env.RUST_DOCTOR_E2E_CORPUS_DIR
  ? resolve(process.env.RUST_DOCTOR_E2E_CORPUS_DIR)
  : undefined;

let warned = false;

// Whether the corpus files run. Unset, they skip with one reason; set to a
// directory that does not exist, every test fails naming it.
export function requireCorpus(): boolean {
  if (CORPUS_DIR) return true;
  if (!warned) {
    console.warn("RUST_DOCTOR_E2E_CORPUS_DIR is not set: corpus files skipped");
    warned = true;
  }
  return false;
}

export type Materialized = { path: string; sandbox: Sandbox };

// A fresh copy of `subject` at its pinned commit. A cache that lacks the
// subject or its commit fails naming both, rather than reaching for a clone.
export async function materialize(entry: Subject): Promise<Materialized> {
  if (!CORPUS_DIR) throw new Error("RUST_DOCTOR_E2E_CORPUS_DIR is not set");
  const repository = join(CORPUS_DIR, entry.name);
  if (!existsSync(join(repository, ".git"))) {
    throw new Error(`corpus subject ${entry.name} is missing from the clone cache at ${CORPUS_DIR}`);
  }
  const box = sandbox(`corpus-${entry.name}`);
  const path = join(scratchDir(`${entry.name}-work`), entry.name);
  const git = async (args: string[]) => {
    const run = await runCommand(["git", ...args], { sandbox: box });
    if (run.exitCode !== 0) {
      throw new Error(`corpus subject ${entry.name}: git ${args.join(" ")} failed (exit ${run.exitCode}): ${run.stderr.trim()}`);
    }
  };
  await git(["-C", repository, "cat-file", "-e", `${entry.commit}^{commit}`]).catch(() => {
    throw new Error(`corpus subject ${entry.name}: the clone cache at ${CORPUS_DIR} lacks the pinned commit ${entry.commit}`);
  });
  await git(["clone", "--quiet", "--shared", "--no-checkout", repository, path]);
  await git(["-C", path, "checkout", "--quiet", "--detach", entry.commit]);
  return { path, sandbox: box };
}

// What the record keeps of a report, computed as `observation` does in
// tests/support/corpus.rs: the categorized diagnostics are the findings,
// counted per rule, beside the score, the status and the exit code. The
// dimension densities and the findings digest stay with the Rust harness.
export function observe(exitCode: number | null, report: unknown): Omit<Observation, "name" | "commit"> {
  const value = report as {
    status?: string;
    diagnostics?: Array<{ category: unknown; code: string; occurrences: number }>;
    audit?: { production_lines?: number; score?: (ScoreRecord & { authoritative?: boolean; reasons?: string[] }) | null };
  };
  const findings = (value.diagnostics ?? []).filter((diagnostic) => diagnostic.category !== null && diagnostic.category !== undefined);
  const rules = new Map<string, RuleRecord>();
  for (const finding of findings) {
    const rule = rules.get(finding.code) ?? { id: finding.code, distinct: 0, occurrences: 0 };
    rule.distinct += 1;
    rule.occurrences += finding.occurrences;
    rules.set(finding.code, rule);
  }
  const score = value.audit?.score ?? null;
  return {
    status: value.status ?? "failed",
    exit_code: exitCode ?? -1,
    authoritative: score?.authoritative === true,
    distinct: findings.length,
    occurrences: findings.reduce((sum, finding) => sum + finding.occurrences, 0),
    production_lines: value.audit?.production_lines ?? 0,
    reasons: score?.reasons ?? [],
    rules: [...rules.values()].sort((left, right) => (left.id < right.id ? -1 : left.id > right.id ? 1 : 0)),
    score: score && {
      applied_ceiling: score.applied_ceiling ?? null,
      label: score.label,
      model: score.model,
      value: score.value,
      worst_tier: score.worst_tier ?? null,
    },
  };
}

// The record of `entry` in the shape `observe` returns.
export function recorded(entry: Subject): Omit<Observation, "name" | "commit"> {
  const { name: _name, commit: _commit, ...rest } = entry.record;
  const score = rest.score && {
    applied_ceiling: rest.score.applied_ceiling,
    label: rest.score.label,
    model: rest.score.model,
    value: rest.score.value,
    worst_tier: rest.score.worst_tier,
  };
  return {
    status: rest.status,
    exit_code: rest.exit_code,
    authoritative: rest.authoritative,
    distinct: rest.distinct,
    occurrences: rest.occurrences,
    production_lines: rest.production_lines,
    reasons: rest.reasons,
    rules: rest.rules.map((rule) => ({ id: rule.id, distinct: rule.distinct, occurrences: rule.occurrences })),
    score,
  };
}
