//! Tokio 0.2's surface for the version gate, with none of its behavior.

pub use tokio_macros::{main, test};

pub mod task {
    pub fn block_in_place<F: FnOnce() -> R, R>(work: F) -> R {
        work()
    }
}
