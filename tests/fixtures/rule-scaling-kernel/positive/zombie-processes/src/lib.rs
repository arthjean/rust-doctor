pub fn positive() {
    std::process::Command::new("true")
        .spawn()
        .expect("fixture child should spawn");
}
