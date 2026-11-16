pub fn check() {
    if std::env::var("CARGO_FEATURE_FOO_BAR").is_ok() {
        println!("cargo::rustc-env=BUILD_FOO_BAR=1");
    }
}
