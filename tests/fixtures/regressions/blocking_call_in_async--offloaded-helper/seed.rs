// rule: rust_doctor::source::blocking_call_in_async
// weakness: helper-call
// source: US-017, work a helper hands off the executor
// verdict: quiet

fn offload() {
    let _ = tokio::task::spawn_blocking(|| std::fs::read("data"));
}

pub async fn handler() {
    offload();
}
