//! A current-thread `block_in_place` written against tokio 0.2, which the
//! rule was never validated on: the gate closes it, and nothing is reported.

#[tokio::main(flavor = "current_thread")]
pub async fn serve() -> u8 {
    tokio::task::block_in_place(|| 1)
}
