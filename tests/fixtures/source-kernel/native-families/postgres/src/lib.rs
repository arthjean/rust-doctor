//! The surface of postgres the fixture calls, with none of its behavior.

pub struct Client;

#[derive(Debug)]
pub struct Error;

impl Client {
    pub fn connect(_parameters: &str) -> Result<Self, Error> {
        Ok(Self)
    }

    pub fn query(&mut self, _sql: &str) -> Result<Vec<()>, Error> {
        Ok(Vec::new())
    }

    pub fn execute(&mut self, _sql: &str) -> Result<u64, Error> {
        Ok(0)
    }

    pub fn batch_execute(&mut self, _sql: &str) -> Result<(), Error> {
        Ok(())
    }
}
