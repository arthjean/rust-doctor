// Named by the build script, compiled by nothing else. A build script that
// writes the name of a Rust file is doing something with it, so the file is
// not an orphan.

pub fn probe() -> u8 {
    5
}
