




use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};




#[allow(clippy::unused_async, reason = "the admission contract requires a silent counterpart")]
async fn ready(value: u8) -> u8 {
    value
}


pub fn negative_arc_with_non_send_sync() -> Arc<Mutex<u8>> {
    Arc::new(Mutex::new(0))
}


#[allow(clippy::unwrap_used, reason = "the admission contract requires a silent counterpart")]
pub async fn negative_await_holding_lock(guarded: &Mutex<u8>) -> u8 {
    let value = *guarded.lock().unwrap();
    ready(value).await
}


pub async fn negative_await_holding_refcell_ref(guarded: &Rc<RefCell<u8>>) -> u8 {
    let value = *guarded.borrow();
    ready(value).await
}


pub fn negative_mut_mutex_lock(guarded: &mut Mutex<u8>) -> u8 {
    *guarded.get_mut().unwrap_or(&mut 0)
}


pub async fn negative_unused_async(value: u8) -> u8 {
    ready(value).await
}


#[allow(clippy::unwrap_used, reason = "the admission contract requires a silent counterpart")]
fn negative_rc_mutex(guarded: Rc<RefCell<u8>>) -> u8 {
    *guarded.borrow()
}


pub fn negative_private_signatures(guarded: Rc<RefCell<u8>>) -> u8 {
    negative_rc_mutex(guarded)
}


#[allow(clippy::arc_with_non_send_sync, reason = "the admission contract requires a silent counterpart")]
pub fn negative_allowed_arc_with_non_send_sync() -> Arc<RefCell<u8>> {
    Arc::new(RefCell::new(0))
}


#[allow(clippy::await_holding_lock, clippy::unwrap_used, reason = "the admission contract requires a silent counterpart")]
pub async fn negative_allowed_await_holding_lock(guarded: &Mutex<u8>) -> u8 {
    let value = guarded.lock().unwrap();
    ready(*value).await
}


#[allow(clippy::await_holding_refcell_ref, reason = "the admission contract requires a silent counterpart")]
pub async fn negative_allowed_await_holding_refcell_ref(guarded: &Rc<RefCell<u8>>) -> u8 {
    let value = guarded.borrow();
    ready(*value).await
}


#[allow(clippy::mut_mutex_lock, clippy::unwrap_used, reason = "the admission contract requires a silent counterpart")]
pub fn negative_allowed_mut_mutex_lock(guarded: &mut Mutex<u8>) -> u8 {
    *guarded.lock().unwrap()
}


#[allow(clippy::unused_async, reason = "the admission contract requires a silent counterpart")]
pub async fn negative_allowed_unused_async(value: u8) -> u8 {
    value
}


#[allow(clippy::rc_mutex, clippy::unwrap_used, reason = "the admission contract requires a silent counterpart")]
fn negative_allowed_rc_mutex(guarded: Rc<Mutex<u8>>) -> u8 {
    *guarded.lock().unwrap()
}


pub fn negative_allowed_private_signatures(guarded: Rc<Mutex<u8>>) -> u8 {
    negative_allowed_rc_mutex(guarded)
}
