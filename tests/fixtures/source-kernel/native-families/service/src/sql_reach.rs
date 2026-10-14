//! SQL text the rule reaches through diesel's prelude glob, the builders'
//! raw-SQL entry points and `write!`.

use std::fmt::Write;

pub fn under_diesels_prelude(name: &str) -> diesel::SqlQuery {
    use diesel::prelude::*;
    let _ = Connection;
    diesel::sql_query(format!("DELETE FROM users WHERE name = '{name}'"))
}

pub fn builder_push(name: &str) {
    let mut query = sqlx::QueryBuilder::<sqlx::Postgres>::new("SELECT * FROM users WHERE ");
    query.push(format!("name = '{name}'"));
}

pub fn raw_fragment(name: &str) -> diesel::dsl::SqlLiteral {
    diesel::dsl::sql::<diesel::sql_types::Bool>(&format!("name = '{name}'"))
}

pub fn raw_statement(name: &str) -> sea_orm::Statement {
    sea_orm::Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        format!("SELECT * FROM users WHERE name = '{name}'"),
    )
}

pub fn written(name: &str) -> sqlx::Query {
    let mut sql = String::new();
    let _ = write!(sql, "SELECT * FROM users WHERE name = '{name}'");
    sqlx::query(&sql)
}
