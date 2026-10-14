//! The surface of reqwest the fixture calls, with none of its behavior.

pub async fn get(_url: &str) -> Result<(), ()> {
    Ok(())
}

pub mod blocking {
    pub fn get(_url: &str) -> Result<(), ()> {
        Ok(())
    }
}
