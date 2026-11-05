pub(crate) fn machine_allowance() -> u32 {
    std::env::var("RUST_DOCTOR_BENCHMARK_ALLOWANCE")
        .ok()
        .and_then(|factor| factor.parse::<u32>().ok())
        .filter(|factor| *factor >= 1)
        .unwrap_or(1)
}
