//! The surface of sqlx the fixture calls, with none of its behavior.

pub struct Query;

impl Query {
    pub fn bind<T>(self, _value: T) -> Self {
        self
    }
}

pub fn query(_sql: &str) -> Query {
    Query
}

pub fn query_as<T>(_sql: &str) -> Query {
    Query
}

pub fn query_scalar(_sql: &str) -> Query {
    Query
}

pub fn raw_sql(_sql: &str) -> Query {
    Query
}

#[macro_export]
macro_rules! query {
    ($sql:literal $(, $argument:expr)*) => {{
        $(let _ = $argument;)*
        $crate::Query
    }};
}
