// rule: rust_doctor::source::sql_built_from_format
// weakness: api-variant
// source: US-014 seed, fixed by US-018
// verdict: fires

use std::fmt::Write;

pub async fn find(pool: &sqlx::PgPool, name: &str) {
    let mut sql = String::new();
    let _ = write!(sql, "SELECT * FROM users WHERE name = '{name}'");
    let _ = sqlx::query(&sql).fetch_all(pool).await;
}
