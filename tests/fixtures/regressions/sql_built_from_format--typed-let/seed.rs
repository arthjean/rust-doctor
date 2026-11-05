pub struct Page {
    pub limit: i64,
}

pub async fn page(pool: &sqlx::PgPool, page: Page) {
    let limit: i64 = page.limit;
    let _ = sqlx::query(&format!("SELECT * FROM t LIMIT {limit}")).fetch_all(pool).await;
}
