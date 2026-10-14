// rule: rust_doctor::source::sql_built_from_format
// weakness: typed-number
// source: EP-003 review, the type written on a let was lost through a format capture
// verdict: quiet

pub struct Page {
    pub limit: i64,
}

pub async fn page(pool: &sqlx::PgPool, page: Page) {
    let limit: i64 = page.limit;
    let _ = sqlx::query(&format!("SELECT * FROM t LIMIT {limit}")).fetch_all(pool).await;
}
