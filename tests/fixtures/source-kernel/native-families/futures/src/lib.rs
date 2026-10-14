//! The surface of futures the fixture calls, with none of its behavior.

pub mod executor {
    pub fn block_on<F: std::future::Future>(_future: F) -> F::Output {
        panic!("stub")
    }
}
