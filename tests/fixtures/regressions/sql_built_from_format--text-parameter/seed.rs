// rule: rust_doctor::source::sql_built_from_format
// weakness: typed-number
// source: US-016, text in the same position
// verdict: fires

pub async fn find(pool: &sqlx::PgPool, name: &str) {
    let _ = sqlx::query(&format!("SELECT * FROM t WHERE name = '{name}'")).fetch_all(pool).await;
}
