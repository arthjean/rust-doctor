fn main() {
    if std::env::var_os("CARGO_FEATURE_LINK").is_some() {
        println!("cargo::rustc-link-lib=dylib=z");
    }
}
