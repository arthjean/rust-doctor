

pub use tokio_macros::{main, test};

pub mod task {
    pub fn block_in_place<F: FnOnce() -> R, R>(work: F) -> R {
        work()
    }
}
