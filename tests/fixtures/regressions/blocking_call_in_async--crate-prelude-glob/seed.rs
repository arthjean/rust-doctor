// rule: rust_doctor::source::blocking_call_in_async
// weakness: glob-import
// source: US-015, the abstention a crate glob keeps
// verdict: quiet

use crate::prelude::*;

pub async fn tick() {
    std::thread::sleep(std::time::Duration::from_secs(1));
}
