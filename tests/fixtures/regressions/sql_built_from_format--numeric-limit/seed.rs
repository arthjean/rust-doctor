pub async fn page(pool: &sqlx::PgPool, limit: i64) {
    let _ = sqlx::query(&format!("SELECT * FROM t LIMIT {limit}")).fetch_all(pool).await;
}
