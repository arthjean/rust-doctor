//! The surface of Tokio the fixture calls, with none of its behavior.

pub mod fs {
    pub async fn read(_path: &str) -> std::io::Result<Vec<u8>> {
        Ok(Vec::new())
    }
}

pub mod time {
    pub async fn sleep(_duration: std::time::Duration) {}
}

pub mod task {
    pub fn spawn_blocking<F: FnOnce() -> R, R>(work: F) -> R {
        work()
    }

    pub fn block_in_place<F: FnOnce() -> R, R>(work: F) -> R {
        work()
    }
}

pub mod process {
    pub struct Command;

    impl Command {
        pub fn new(_program: &str) -> Self {
            Self
        }

        pub async fn output(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
}

pub use tokio_macros::{main, test};

pub mod runtime {
    pub struct Runtime;

    impl Runtime {
        pub fn new() -> std::io::Result<Self> {
            Ok(Self)
        }

        pub fn block_on<F: std::future::Future>(&self, _future: F) -> F::Output {
            panic!("stub")
        }
    }

    pub struct Builder;

    impl Builder {
        pub fn new_current_thread() -> Self {
            Self
        }

        pub fn new_multi_thread() -> Self {
            Self
        }

        pub fn build(&mut self) -> std::io::Result<Runtime> {
            Ok(Runtime)
        }
    }

    pub struct Handle;

    impl Handle {
        pub fn current() -> Self {
            Self
        }

        pub fn block_on<F: std::future::Future>(&self, _future: F) -> F::Output {
            panic!("stub")
        }
    }
}
