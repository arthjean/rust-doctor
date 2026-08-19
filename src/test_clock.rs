//! What a machine is allowed on a wall clock, for every unit test of this
//! repository that asserts one.
//!
//! The bounds those tests carry were measured on a development machine. A
//! shared CI runner is slower and less predictable, and a benchmark asserting
//! a wall clock on someone else's hardware measures that hardware. Rather than
//! raise the constants and lose the bound where it means something, the slower
//! machine declares itself through `RUST_DOCTOR_BENCHMARK_ALLOWANCE`, the same
//! way the corpus harness makes its own observation independent of machine
//! load through `RUST_DOCTOR_STRUCTURE_TIME_BUDGET_SECS`.
//!
//! It lives here rather than inside the structural benchmark that first needed
//! it, because a second clock assertion appeared in the source kernel and could
//! not reach it: `ci.yml` set the allowance for both and only one of the two
//! read it, so the alias benchmark went red on the runner the declaration was
//! written for.
//!
//! It touches clocks only. A counter assertion holds on any machine and is
//! never relaxed by it.

/// Multiple of every clock bound this machine is allowed.
pub(crate) fn machine_allowance() -> u32 {
    std::env::var("RUST_DOCTOR_BENCHMARK_ALLOWANCE")
        .ok()
        .and_then(|factor| factor.parse::<u32>().ok())
        .filter(|factor| *factor >= 1)
        .unwrap_or(1)
}
