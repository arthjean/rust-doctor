#[tokio::main]
pub async fn multi_thread_main() -> u8 {
    tokio::task::block_in_place(|| 1)
}

pub fn outside_a_runtime() -> u8 {
    tokio::task::block_in_place(|| 1)
}

pub fn synchronous_entry() {
    let runtime = tokio::runtime::Builder::new_multi_thread().build().unwrap();
    runtime.block_on(async {});
}

pub struct Pool;

impl Pool {
    pub fn block_on(&self) {}
}

pub async fn unrelated_block_on(pool: Pool) {
    pool.block_on();
}

#[cfg(test)]
mod tests {
    #[tokio::test(flavor = "multi_thread")]
    async fn multi_thread_test() {
        let _ = tokio::task::block_in_place(|| 1);
    }
}
