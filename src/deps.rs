//! External tool dependency checking and installation.
//!
//! Centralizes the registry of external tools that rust-doctor delegates to,
//! with availability detection and optional auto-installation via `--install-deps`.

use std::process::{Command, ExitStatus, Stdio};

/// How a tool should be installed.
enum InstallMethod {
    /// `cargo install <crate>`
    CargoInstall,
    /// `rustup component add <component>`
    RustupComponent(&'static str),
}

/// An external tool that rust-doctor can delegate analysis to.
struct ExternalTool {
    /// Human-readable name (e.g., "cargo-deny")
    name: &'static str,
    /// Cargo subcommand used to probe availability (e.g., "deny")
    subcommand: &'static str,
    /// What this tool does (shown during `--install-deps`)
    description: &'static str,
    /// How to install it
    method: InstallMethod,
}

/// All external tools that rust-doctor can use, in recommended install order.
const TOOLS: &[ExternalTool] = &[
    ExternalTool {
        name: "clippy",
        subcommand: "clippy",
        description: "Lint analysis",
        method: InstallMethod::RustupComponent("clippy"),
    },
    ExternalTool {
        name: "cargo-deny",
        subcommand: "deny",
        description: "Supply-chain checking (advisories, licenses, bans)",
        method: InstallMethod::CargoInstall,
    },
    ExternalTool {
        name: "cargo-audit",
        subcommand: "audit",
        description: "CVE vulnerability scanning",
        method: InstallMethod::CargoInstall,
    },
    ExternalTool {
        name: "cargo-geiger",
        subcommand: "geiger",
        description: "Unsafe code auditing across dependency tree",
        method: InstallMethod::CargoInstall,
    },
    ExternalTool {
        name: "cargo-machete",
        subcommand: "machete",
        description: "Unused dependency detection",
        method: InstallMethod::CargoInstall,
    },
    ExternalTool {
        name: "cargo-semver-checks",
        subcommand: "semver-checks",
        description: "Semver violation detection",
        method: InstallMethod::CargoInstall,
    },
];

impl ExternalTool {
    /// Check if this tool is reachable via `cargo <subcommand> --version`.
    fn is_available(&self) -> bool {
        Command::new("cargo")
            .args([self.subcommand, "--version"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }

    /// Attempt to install this tool. Returns the process exit status.
    fn install(&self) -> Result<ExitStatus, String> {
        match self.method {
            InstallMethod::RustupComponent(component) => Command::new("rustup")
                .args(["component", "add", component])
                .status()
                .map_err(|e| format!("failed to run rustup: {e}")),
            InstallMethod::CargoInstall => Command::new("cargo")
                .args(["install", self.name])
                .status()
                .map_err(|e| format!("failed to run cargo install: {e}")),
        }
    }

    /// Human-readable install command for display.
    fn install_command(&self) -> String {
        match self.method {
            InstallMethod::RustupComponent(c) => format!("rustup component add {c}"),
            InstallMethod::CargoInstall => format!("cargo install {}", self.name),
        }
    }
}

/// Check all external tools and install any that are missing.
///
/// Prints progress to stderr. Returns `true` if all tools are available
/// after installation (or were already present).
pub fn install_missing_tools() -> bool {
    let missing: Vec<&ExternalTool> = TOOLS.iter().filter(|t| !t.is_available()).collect();

    if missing.is_empty() {
        eprintln!("All external tools are already installed.");
        return true;
    }

    eprintln!("Found {} missing tool(s). Installing:\n", missing.len());

    let mut all_ok = true;
    for tool in &missing {
        eprintln!("  {} — {}", tool.name, tool.description);
        eprintln!("  $ {}", tool.install_command());

        match tool.install() {
            Ok(status) if status.success() => {
                eprintln!("  -> installed\n");
            }
            Ok(_) => {
                eprintln!("  -> FAILED (non-zero exit)\n");
                all_ok = false;
            }
            Err(e) => {
                eprintln!("  -> FAILED ({e})\n");
                all_ok = false;
            }
        }
    }

    if all_ok {
        eprintln!("All tools installed successfully.");
    } else {
        eprintln!("Some tools failed to install. Check the output above.");
    }

    all_ok
}

/// Print a status report of all external tools (installed / missing).
pub fn print_status() {
    eprintln!("External tools status:\n");
    for tool in TOOLS {
        let status = if tool.is_available() {
            "installed"
        } else {
            "MISSING"
        };
        eprintln!("  {:<25} {:<10} {}", tool.name, status, tool.description);
    }
    eprintln!();
}
