// rule: rust_doctor::source::sql_built_from_format
// weakness: api-variant
// source: audit Z1a-N04, fixed by US-018
// verdict: fires

use sqlx::QueryBuilder;

pub fn find(name: &str) {
    let mut query = QueryBuilder::<sqlx::Postgres>::new("SELECT * FROM users WHERE ");
    query.push(format!("name = '{name}'"));
}
