// rule: rust_doctor::source::blocking_call_in_async
// weakness: glob-import
// source: audit Z2-N02, fixed by US-015
// verdict: fires

use futures::prelude::*;

pub async fn tick() {
    std::thread::sleep(std::time::Duration::from_secs(1));
}
