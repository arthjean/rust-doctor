// rule: rust_doctor::source::sql_built_from_format
// weakness: typed-number
// source: US-016, a cast to a primitive
// verdict: quiet

pub async fn page(pool: &sqlx::PgPool, count: usize) {
    let _ = sqlx::query(&format!("SELECT * FROM t LIMIT {}", count as i64)).fetch_all(pool).await;
}
