#[tokio::main(flavor = "current_thread")]
pub async fn serve() -> u8 {
    tokio::task::block_in_place(|| {
        std::thread::sleep(std::time::Duration::from_millis(1));
        1
    })
}

pub async fn nested_runtime() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {});
}

pub async fn nested_executor() -> u8 {
    futures::executor::block_on(async { 1 })
}

pub fn handle_in_async_block() {
    let _ = async { tokio::runtime::Handle::current().block_on(async {}) };
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn reads_in_place() {
        let _ = tokio::task::block_in_place(|| 1);
    }
}
