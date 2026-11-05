

pub mod executor {
    pub fn block_on<F: std::future::Future>(_future: F) -> F::Output {
        panic!("stub")
    }
}
