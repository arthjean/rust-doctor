pub async fn page(pool: &sqlx::PgPool, count: usize) {
    let _ = sqlx::query(&format!("SELECT * FROM t LIMIT {}", count as i64)).fetch_all(pool).await;
}
