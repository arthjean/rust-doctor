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

pub struct Postgres;

pub struct QueryBuilder<DB> {
    database: std::marker::PhantomData<DB>,
}

impl<DB> QueryBuilder<DB> {
    pub fn new(_sql: impl Into<String>) -> Self {
        Self {
            database: std::marker::PhantomData,
        }
    }

    pub fn push(&mut self, _sql: impl std::fmt::Display) -> &mut Self {
        self
    }

    pub fn push_bind<T>(&mut self, _value: T) -> &mut Self {
        self
    }
}
