use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=RUSTC");
    println!("cargo:rerun-if-env-changed=TARGET");

    if let Err(error) = embed_target_cfg() {
        eprintln!("rust-doctor build script failed: {error}");
        std::process::exit(1);
    }
}

fn embed_target_cfg() -> Result<(), String> {
    let rustc = env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let target = env::var("TARGET").map_err(|error| format!("TARGET is unavailable: {error}"))?;
    let output = Command::new(rustc)
        .args(["--print", "cfg", "--target", &target])
        .output()
        .map_err(|error| format!("rustc target cfg discovery cannot start: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "rustc target cfg discovery failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let cfg = String::from_utf8(output.stdout)
        .map_err(|error| format!("rustc target cfg is not UTF-8: {error}"))?;
    let output_dir = env::var_os("OUT_DIR").ok_or_else(|| "OUT_DIR is unavailable".to_string())?;
    let output_path = PathBuf::from(output_dir).join("rust-doctor-target.cfg");
    fs::write(output_path, format!("{target}\n{cfg}"))
        .map_err(|error| format!("embedded rustc target cfg cannot be written: {error}"))?;
    Ok(())
}
