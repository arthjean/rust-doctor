












pub fn run_user_command(user: &str) -> std::process::Output {
    std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("echo {user}"))
        .output()
        .unwrap_or_else(|error| panic!("the shell should answer: {error}"))
}


pub fn rotate_credentials() -> &'static str {
    unimplemented!()
}


pub fn revoke_session(_token: &str) -> bool {
    todo!()
}


pub fn trace_request(path: &str) -> usize {
    dbg!(path.len())
}








pub const PAYMENT_KEY: &str = "PLACEHOLDER-PAYMENT-CREDENTIAL-DO-NOT-USE";


pub fn user_query(name: &str) -> String {
    format!("SELECT * FROM users WHERE name = '{name}'")
}


pub fn first_byte(bytes: &[u8]) -> u8 {
    unsafe { *bytes.get_unchecked(0) }
}


pub fn third_field(line: &str) -> String {
    let fields: Vec<&str> = line.split(',').collect();
    let parsed: u8 = fields[2].parse().unwrap();
    parsed.to_string()
}
