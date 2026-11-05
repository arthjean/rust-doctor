






use std::path::Path;

fn main() {
    let probe = Path::new("build").join("probe.rs");
    let _ = probe.exists();
}
