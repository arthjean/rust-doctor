//! A build script is a Cargo target: it roots the package directory, and it is
//! never an orphan.
//!
//! It also names a Rust file no module declaration reaches. `thiserror` feeds
//! `build/probe.rs` to rustc exactly this way, so a file a build script names
//! is reached even though nothing declares it.

use std::path::Path;

fn main() {
    let probe = Path::new("build").join("probe.rs");
    let _ = probe.exists();
}
