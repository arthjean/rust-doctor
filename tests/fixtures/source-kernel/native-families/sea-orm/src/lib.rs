

pub enum DatabaseBackend {
    Postgres,
}

pub struct Statement;

impl Statement {
    pub fn from_string(_backend: DatabaseBackend, _sql: impl Into<String>) -> Self {
        Statement
    }
}
