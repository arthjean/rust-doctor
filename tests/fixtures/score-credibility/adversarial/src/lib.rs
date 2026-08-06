//! Reference adversarial fixture of the score model.
//!
//! It gathers the defects the score-credibility-kernel PRD measures as
//! coexisting with a score of 99 under `core-v1`. Under `core-v2`, the command
//! injection carries tier `P0`, so the overall score can no longer exceed its
//! cap whatever the average of the other dimensions.
//!
//! What the current catalog does not detect yet, a hard-coded secret, SQL
//! concatenation, unjustified `unsafe`, `unwrap`, `panic!` and unchecked
//! indexing, stays written here on purpose: the fixture must remain the
//! measurement point of the following slices.

/// Detected: `rust_doctor::source::dynamic_shell_command`, tier `P0`.
pub fn run_user_command(user: &str) -> std::process::Output {
    std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("echo {user}"))
        .output()
        .unwrap_or_else(|error| panic!("the shell should answer: {error}"))
}

/// Detected: `clippy::unimplemented`, tier `P1`.
pub fn rotate_credentials() -> &'static str {
    unimplemented!()
}

/// Detected: `clippy::todo`, tier `P2`.
pub fn revoke_session(_token: &str) -> bool {
    todo!()
}

/// Detected: `clippy::dbg_macro`, tier `P3`.
pub fn trace_request(path: &str) -> usize {
    dbg!(path.len())
}

/// Not detected by the current catalog: hard-coded payment identifier.
///
/// The literal deliberately takes the shape of no real provider: GitHub's push
/// protection would refuse the fixture, and raw-file secret scanning is
/// explicitly outside the scope of this slice. What it documents here is the
/// presence of a plaintext identifier inside the source code, nothing about
/// the format of any given issuer.
pub const PAYMENT_KEY: &str = "PLACEHOLDER-PAYMENT-CREDENTIAL-DO-NOT-USE";

/// Not detected by the current catalog: SQL concatenation.
pub fn user_query(name: &str) -> String {
    format!("SELECT * FROM users WHERE name = '{name}'")
}

/// Not detected by the current catalog: `unsafe` without justification.
pub fn first_byte(bytes: &[u8]) -> u8 {
    unsafe { *bytes.get_unchecked(0) }
}

/// Not detected by the current catalog: unchecked indexing and `unwrap`.
pub fn third_field(line: &str) -> String {
    let fields: Vec<&str> = line.split(',').collect();
    let parsed: u8 = fields[2].parse().unwrap();
    parsed.to_string()
}
