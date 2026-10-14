//! The surface of diesel the fixture calls, with none of its behavior.

pub struct SqlQuery;

pub fn sql_query(_sql: impl Into<String>) -> SqlQuery {
    SqlQuery
}

pub struct Connection;

pub mod prelude {
    pub use crate::Connection;
}

pub mod sql_types {
    pub struct Bool;
}

pub mod dsl {
    pub struct SqlLiteral;

    pub fn sql<T>(_sql: &str) -> SqlLiteral {
        SqlLiteral
    }
}
