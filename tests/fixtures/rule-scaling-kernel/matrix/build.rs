fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let _non_executable_context = "std::mem::forget(value); command.arg(\"two words\")";
}
