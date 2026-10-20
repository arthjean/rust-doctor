// The binary inside a network namespace of its own (`unshare -rn`), the
// strongest proof that a scan needs no network. Hosted Ubuntu 24.04 refuses
// unprivileged user namespaces until the job lifts the AppArmor restriction,
// so a machine without them skips these scenarios with one stated reason,
// unless E2E_REQUIRE_NETNS=1 (CI), where their absence fails the file.
import {
  BIN,
  defaultTarget,
  doctorArgs,
  hermeticEnv,
  sandbox,
  spawnTracked,
  type DoctorOptions,
  type DoctorRun,
} from "./e2e-helpers";

export const UNSHARE = Bun.which("unshare", { PATH: "/usr/bin:/bin:/usr/sbin:/sbin" });

let probed: boolean | undefined;
let warned = false;

function probe(): boolean {
  if (!UNSHARE) return false;
  const run = Bun.spawnSync([UNSHARE, "-rn", "true"], { env: { PATH: "/usr/bin:/bin" }, stdout: "ignore", stderr: "ignore" });
  return run.exitCode === 0;
}

// Whether namespace scenarios can run here.
export function requireNetns(): boolean {
  probed ??= probe();
  if (probed) return true;
  if (process.env.E2E_REQUIRE_NETNS === "1") {
    throw new Error("network namespaces are unavailable and E2E_REQUIRE_NETNS=1 requires them");
  }
  if (!warned) {
    console.warn("network namespaces unavailable: skipped");
    warned = true;
  }
  return false;
}

// `runDoctor` with the binary started under `unshare -rn`: the same sealed
// environment, target directory and advisory default, and no network
// interface but a loopback that is down.
export function runDoctorOffline(args: string[], options: DoctorOptions = {}): Promise<DoctorRun> {
  if (!UNSHARE) throw new Error("unshare is not installed");
  const box = options.sandbox ?? sandbox("netns");
  const cwd = options.cwd ?? box.dir;
  const finalArgs = doctorArgs(args, options);
  const env = hermeticEnv(box, { CARGO_TARGET_DIR: options.target ?? defaultTarget(args, cwd), ...options.env });
  return spawnTracked([UNSHARE, "-rn", BIN, ...finalArgs], { cwd, env, stdin: options.stdin, timeoutMs: options.timeoutMs })
    .exited.then((exit): DoctorRun => {
      let json: unknown = undefined;
      if (finalArgs.includes("--json") && exit.stdout.trim().length > 0) {
        try {
          json = JSON.parse(exit.stdout);
        } catch {}
      }
      return { ...exit, args: finalArgs, json, sandbox: box };
    });
}
