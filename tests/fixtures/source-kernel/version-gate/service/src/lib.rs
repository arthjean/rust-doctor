


#[tokio::main(flavor = "current_thread")]
pub async fn serve() -> u8 {
    tokio::task::block_in_place(|| 1)
}
