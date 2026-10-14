//! The surface of diesel the fixture calls, with none of its behavior.

pub struct SqlQuery;

pub fn sql_query(_sql: impl Into<String>) -> SqlQuery {
    SqlQuery
}
