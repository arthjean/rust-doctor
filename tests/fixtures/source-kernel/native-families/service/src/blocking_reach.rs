//! Blocking calls the rule reaches through an external prelude glob and
//! through the synchronous helpers of the file.

use std::io::prelude::*;
use std::time::Duration;

pub async fn under_an_external_prelude_glob() {
    std::thread::sleep(Duration::from_millis(1));
}

fn synchronous_helper() -> std::io::Result<Vec<u8>> {
    std::fs::read("Cargo.toml")
}

pub async fn calls_a_synchronous_helper() -> std::io::Result<Vec<u8>> {
    synchronous_helper()
}

pub async fn nested_synchronous_function() {
    fn inner() {
        std::thread::sleep(Duration::from_millis(1));
    }
    inner();
}

pub fn flush(mut out: impl Write) -> std::io::Result<()> {
    out.flush()
}
