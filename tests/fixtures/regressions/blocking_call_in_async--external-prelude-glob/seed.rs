use futures::prelude::*;

pub async fn tick() {
    std::thread::sleep(std::time::Duration::from_secs(1));
}
