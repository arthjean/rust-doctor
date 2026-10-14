//! The surface of rusqlite the fixture calls, with none of its behavior.

pub struct Connection;
pub struct Statement;

#[derive(Debug)]
pub struct Error;

impl Connection {
    pub fn open(_path: &str) -> Result<Self, Error> {
        Ok(Self)
    }

    pub fn open_in_memory() -> Result<Self, Error> {
        Ok(Self)
    }

    pub fn execute<P>(&self, _sql: &str, _params: P) -> Result<usize, Error> {
        Ok(0)
    }

    pub fn prepare(&self, _sql: &str) -> Result<Statement, Error> {
        Ok(Statement)
    }

    pub fn query_row<P>(&self, _sql: &str, _params: P) -> Result<(), Error> {
        Ok(())
    }
}
