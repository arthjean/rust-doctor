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
