//! Stands for a build script that never returns. It records its process id
//! where the test asks, so the test can prove the deadline killed it.
use std::time::Duration;

fn main() {
    println!("cargo::rerun-if-env-changed=RUST_DOCTOR_SLEEPER_PID");
    if let Some(path) = std::env::var_os("RUST_DOCTOR_SLEEPER_PID") {
        let _ = std::fs::write(path, std::process::id().to_string());
    }
    std::thread::sleep(Duration::from_secs(120));
}
