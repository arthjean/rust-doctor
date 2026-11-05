pub async fn find(pool: &sqlx::PgPool, name: &str) {
    let _ = sqlx::query(&format!("SELECT * FROM t WHERE name = '{name}'")).fetch_all(pool).await;
}
