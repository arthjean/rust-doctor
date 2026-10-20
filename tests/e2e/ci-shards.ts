// The CI shard planner, ported from pf (/home/arthur/dev/pf/tests/e2e/ci-shards.ts):
// every test file carries a checked-in weight in ci-shard-weights.json, the
// heaviest go first to the lightest shard, and a file without a weight, a
// weight without a file or a duplicate fails the plan naming it.
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

export type ShardPlan = { shards: string[][]; totals: number[] };

type WeightEntry = { file: string; weight: number };

const TEST_FILE = /^[^/\r\n]+\.test\.ts$/;

function byName(left: string, right: string): number {
  return left < right ? -1 : left > right ? 1 : 0;
}

function parseManifest(value: unknown): WeightEntry[] {
  if (!Array.isArray(value)) throw new Error("shard weight manifest must be an array");
  const seen = new Set<string>();
  return value.map((entry, index) => {
    if (!entry || typeof entry !== "object") throw new Error(`invalid manifest entry at index ${index}`);
    const { file, weight } = entry as Partial<WeightEntry>;
    if (typeof file !== "string" || !TEST_FILE.test(file)) throw new Error(`invalid manifest filename at index ${index}`);
    if (seen.has(file)) throw new Error(`duplicate manifest entry: ${file}`);
    seen.add(file);
    if (typeof weight !== "number" || !Number.isSafeInteger(weight) || weight <= 0) {
      throw new Error(`weight must be a positive integer: ${file}`);
    }
    return { file, weight };
  });
}

export function buildShardPlan(discoveredFiles: string[], manifestValue: unknown, shardCount: number): ShardPlan {
  if (!Number.isSafeInteger(shardCount) || shardCount <= 0) throw new Error("shard count must be a positive integer");
  const discovered = [...discoveredFiles].sort(byName);
  if (new Set(discovered).size !== discovered.length) throw new Error("discovered test filenames must be unique");
  for (const file of discovered) {
    if (!TEST_FILE.test(file)) throw new Error(`invalid discovered test filename: ${file}`);
  }
  const manifest = parseManifest(manifestValue);
  const weights = new Map(manifest.map((entry) => [entry.file, entry.weight]));
  const missing = discovered.filter((file) => !weights.has(file));
  if (missing.length > 0) throw new Error(`missing manifest entry: ${missing[0]}`);
  const discoveredSet = new Set(discovered);
  const stale = manifest.map((entry) => entry.file).filter((file) => !discoveredSet.has(file)).sort(byName);
  if (stale.length > 0) throw new Error(`stale manifest entry: ${stale[0]}`);

  const ordered = [...manifest].sort((left, right) => right.weight - left.weight || byName(left.file, right.file));
  const shards = Array.from({ length: shardCount }, () => [] as string[]);
  const totals = Array.from({ length: shardCount }, () => 0);
  for (const entry of ordered) {
    let target = 0;
    for (let index = 1; index < shardCount; index += 1) {
      if (totals[index]! < totals[target]!) target = index;
    }
    shards[target]!.push(entry.file);
    totals[target]! += entry.weight;
  }
  const empty = shards.findIndex((shard) => shard.length === 0);
  if (empty >= 0) throw new Error(`shard ${empty} is empty`);
  return { shards, totals };
}

export function selectShard(plan: ShardPlan, shardIndex: number): string[] {
  if (!Number.isSafeInteger(shardIndex) || shardIndex < 0 || shardIndex >= plan.shards.length) {
    throw new Error(`invalid shard index: ${shardIndex}`);
  }
  return plan.shards[shardIndex]!;
}

function integerArgument(args: string[], name: string): number {
  const index = args.indexOf(name);
  if (index < 0 || index + 1 >= args.length) throw new Error(`missing ${name}`);
  const value = Number(args[index + 1]);
  if (!Number.isSafeInteger(value)) throw new Error(`invalid ${name}: ${args[index + 1]}`);
  return value;
}

export function discoverTestFiles(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true })
    .filter((entry) => entry.isFile() && entry.name.endsWith(".test.ts"))
    .map((entry) => entry.name);
}

// `bun ci-shards.ts --shard-count 3 --shard-index 0` prints the shard's files,
// one per line, and every shard's weight on stderr.
function run(args: string[]): void {
  const shardCount = integerArgument(args, "--shard-count");
  const shardIndex = integerArgument(args, "--shard-index");
  const manifest: unknown = JSON.parse(readFileSync(join(import.meta.dir, "ci-shard-weights.json"), "utf8"));
  const plan = buildShardPlan(discoverTestFiles(import.meta.dir), manifest, shardCount);
  const selected = selectShard(plan, shardIndex);
  for (let index = 0; index < plan.shards.length; index += 1) {
    console.error(`E2E shard ${index}/${shardCount}: weight=${plan.totals[index]} files=${plan.shards[index]!.join(",")}`);
  }
  process.stdout.write(`${selected.join("\n")}\n`);
}

if (import.meta.main) {
  try {
    run(process.argv.slice(2));
  } catch (error) {
    console.error(error instanceof Error ? error.message : error);
    process.exitCode = 1;
  }
}
