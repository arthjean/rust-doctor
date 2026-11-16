#[allow(dead_code)]
pub fn announce() {
    println!("cargo::rerun-if-changed=build.rs");
}
