//! Positive fixture of the concurrency and asynchronous pack.
//!
//! The fixture embeds no asynchronous runtime: the `async` functions are
//! compiled, never executed. The verdict of every lint is therefore frozen by
//! the normative toolchain alone, with no dependency on an executor.
//!
//! `rc_mutex` only aims at non-exported items, like the signature lints of the
//! performance pack: Clippy refuses to propose a signature change on a public
//! API. The fixture therefore triggers it on a private item, exercised by a
//! public entry point.

mod negatives;

pub use negatives::*;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

/// Trivial future shared by the cases of the pack. It awaits nothing, so it
/// aims at `unused_async`: the lint is neutralized here so that only the
/// dedicated positive case triggers it.
#[allow(clippy::unused_async)]
async fn ready(value: u8) -> u8 {
    value
}

/// clippy::arc_with_non_send_sync
pub fn positive_arc_with_non_send_sync() -> Arc<RefCell<u8>> {
    Arc::new(RefCell::new(0))
}

/// clippy::await_holding_lock
#[allow(clippy::unwrap_used)]
pub async fn positive_await_holding_lock(guarded: &Mutex<u8>) -> u8 {
    let value = guarded.lock().unwrap();
    ready(*value).await
}

/// clippy::await_holding_refcell_ref
pub async fn positive_await_holding_refcell_ref(guarded: &Rc<RefCell<u8>>) -> u8 {
    let value = guarded.borrow();
    ready(*value).await
}

/// clippy::mut_mutex_lock
#[allow(clippy::unwrap_used)]
pub fn positive_mut_mutex_lock(guarded: &mut Mutex<u8>) -> u8 {
    *guarded.lock().unwrap()
}

/// clippy::unused_async
pub async fn positive_unused_async(value: u8) -> u8 {
    value
}

/// clippy::rc_mutex
#[allow(clippy::unwrap_used)]
fn positive_rc_mutex(guarded: Rc<Mutex<u8>>) -> u8 {
    *guarded.lock().unwrap()
}

/// Exercises the lint reserved for non-exported items.
pub fn positive_private_signatures(guarded: Rc<Mutex<u8>>) -> u8 {
    positive_rc_mutex(guarded)
}
