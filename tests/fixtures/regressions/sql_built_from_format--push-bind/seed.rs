// rule: rust_doctor::source::sql_built_from_format
// weakness: api-variant
// source: US-018, the builder's bound parameter
// verdict: quiet

use sqlx::QueryBuilder;

pub fn find(name: &str) {
    let mut query = QueryBuilder::<sqlx::Postgres>::new("SELECT * FROM users WHERE name = ");
    query.push_bind(name);
}
