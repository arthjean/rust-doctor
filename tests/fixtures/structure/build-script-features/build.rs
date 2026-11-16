mod probe;

fn main() {
    if std::env::var_os("CARGO_FEATURE_LINK").is_some() {
        println!("cargo::rustc-env=BUILD_LINKED=1");
    }
    probe::check();
}
