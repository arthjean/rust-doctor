fn main() {
    // The only reference to this dependency lives in the build script, which
    // is enumerated as its own target: the unused rule stays silent.
    let _ = build_probe::value();
}
