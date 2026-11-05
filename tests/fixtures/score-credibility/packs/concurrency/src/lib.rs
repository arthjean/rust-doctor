










mod negatives;

pub use negatives::*;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};




#[allow(clippy::unused_async, reason = "the admission contract requires a silent counterpart")]
async fn ready(value: u8) -> u8 {
    value
}


pub fn positive_arc_with_non_send_sync() -> Arc<RefCell<u8>> {
    Arc::new(RefCell::new(0))
}


#[allow(clippy::unwrap_used, reason = "the admission contract requires a silent counterpart")]
pub async fn positive_await_holding_lock(guarded: &Mutex<u8>) -> u8 {
    let value = guarded.lock().unwrap();
    ready(*value).await
}


pub async fn positive_await_holding_refcell_ref(guarded: &Rc<RefCell<u8>>) -> u8 {
    let value = guarded.borrow();
    ready(*value).await
}


#[allow(clippy::unwrap_used, reason = "the admission contract requires a silent counterpart")]
pub fn positive_mut_mutex_lock(guarded: &mut Mutex<u8>) -> u8 {
    *guarded.lock().unwrap()
}


pub async fn positive_unused_async(value: u8) -> u8 {
    value
}


#[allow(clippy::unwrap_used, reason = "the admission contract requires a silent counterpart")]
fn positive_rc_mutex(guarded: Rc<Mutex<u8>>) -> u8 {
    *guarded.lock().unwrap()
}


pub fn positive_private_signatures(guarded: Rc<Mutex<u8>>) -> u8 {
    positive_rc_mutex(guarded)
}
