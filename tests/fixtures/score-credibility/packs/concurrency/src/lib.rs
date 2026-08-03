//! Fixture positive du pack concurrence et asynchrone.
//!
//! La fixture n'embarque aucun runtime asynchrone: les fonctions `async` sont
//! compilées, jamais exécutées. Le verdict de chaque lint est donc figé par le
//! seul toolchain normatif, sans dépendance à un exécuteur.
//!
//! `rc_mutex` ne vise que les items non exportés, comme les lints de signature
//! du pack performance: Clippy refuse de proposer un changement de signature sur
//! une API publique. La fixture le déclenche donc sur un item privé, exercé par
//! un point d'entrée public.

mod negatives;

pub use negatives::*;

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

/// Exerce le lint réservé aux items non exportés.
pub fn positive_private_signatures(guarded: Rc<Mutex<u8>>) -> u8 {
    positive_rc_mutex(guarded)
}
