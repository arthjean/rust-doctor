//! Fixture négative du pack concurrence et asynchrone.
//!
//! Le négatif de `rc_mutex` est privé comme son positif, sans quoi son silence
//! ne prouverait rien.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

/// Futur trivial partagé par les cas du pack. Il n'attend rien, donc il vise
/// `unused_async`: le lint est neutralisé ici pour que seul le cas positif
/// dédié le déclenche.
#[allow(clippy::unused_async)]
async fn ready(value: u8) -> u8 {
    value
}

/// negative_arc_with_non_send_sync
pub fn negative_arc_with_non_send_sync() -> Arc<Mutex<u8>> {
    Arc::new(Mutex::new(0))
}

/// negative_await_holding_lock
#[allow(clippy::unwrap_used)]
pub async fn negative_await_holding_lock(guarded: &Mutex<u8>) -> u8 {
    let value = *guarded.lock().unwrap();
    ready(value).await
}

/// negative_await_holding_refcell_ref
pub async fn negative_await_holding_refcell_ref(guarded: &Rc<RefCell<u8>>) -> u8 {
    let value = *guarded.borrow();
    ready(value).await
}

/// negative_mut_mutex_lock
pub fn negative_mut_mutex_lock(guarded: &mut Mutex<u8>) -> u8 {
    *guarded.get_mut().unwrap_or(&mut 0)
}

/// negative_unused_async
pub async fn negative_unused_async(value: u8) -> u8 {
    ready(value).await
}

/// negative_rc_mutex
#[allow(clippy::unwrap_used)]
fn negative_rc_mutex(guarded: Rc<RefCell<u8>>) -> u8 {
    *guarded.borrow()
}

/// Exerce le négatif privé.
pub fn negative_private_signatures(guarded: Rc<RefCell<u8>>) -> u8 {
    negative_rc_mutex(guarded)
}

/// negative_allowed_arc_with_non_send_sync
#[allow(clippy::arc_with_non_send_sync)]
pub fn negative_allowed_arc_with_non_send_sync() -> Arc<RefCell<u8>> {
    Arc::new(RefCell::new(0))
}

/// negative_allowed_await_holding_lock
#[allow(clippy::await_holding_lock, clippy::unwrap_used)]
pub async fn negative_allowed_await_holding_lock(guarded: &Mutex<u8>) -> u8 {
    let value = guarded.lock().unwrap();
    ready(*value).await
}

/// negative_allowed_await_holding_refcell_ref
#[allow(clippy::await_holding_refcell_ref)]
pub async fn negative_allowed_await_holding_refcell_ref(guarded: &Rc<RefCell<u8>>) -> u8 {
    let value = guarded.borrow();
    ready(*value).await
}

/// negative_allowed_mut_mutex_lock
#[allow(clippy::mut_mutex_lock, clippy::unwrap_used)]
pub fn negative_allowed_mut_mutex_lock(guarded: &mut Mutex<u8>) -> u8 {
    *guarded.lock().unwrap()
}

/// negative_allowed_unused_async
#[allow(clippy::unused_async)]
pub async fn negative_allowed_unused_async(value: u8) -> u8 {
    value
}

/// negative_allowed_rc_mutex
#[allow(clippy::rc_mutex, clippy::unwrap_used)]
fn negative_allowed_rc_mutex(guarded: Rc<Mutex<u8>>) -> u8 {
    *guarded.lock().unwrap()
}

/// Exerce le négatif privé neutralisé localement.
pub fn negative_allowed_private_signatures(guarded: Rc<Mutex<u8>>) -> u8 {
    negative_allowed_rc_mutex(guarded)
}
