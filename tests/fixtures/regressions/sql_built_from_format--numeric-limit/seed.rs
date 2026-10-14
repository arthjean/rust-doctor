// rule: rust_doctor::source::sql_built_from_format
// weakness: typed-number
// source: audit Z2-N04, fixed by US-016
// verdict: quiet

pub async fn page(pool: &sqlx::PgPool, limit: i64) {
    let _ = sqlx::query(&format!("SELECT * FROM t LIMIT {limit}")).fetch_all(pool).await;
}
