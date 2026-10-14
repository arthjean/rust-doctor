//! The surface of tokio-postgres the fixture calls, with none of its behavior.

pub struct Client;
pub struct Connection;

#[derive(Debug)]
pub struct Error;

impl Client {
    pub async fn connect(_parameters: &str) -> Result<Self, Error> {
        Ok(Self)
    }

    pub async fn query(&self, _sql: &str) -> Result<Vec<()>, Error> {
        Ok(Vec::new())
    }

    pub async fn execute(&self, _sql: &str) -> Result<u64, Error> {
        Ok(0)
    }

    pub async fn batch_execute(&self, _sql: &str) -> Result<(), Error> {
        Ok(())
    }
}

pub async fn connect(_parameters: &str) -> Result<(Client, Connection), Error> {
    Ok((Client, Connection))
}
