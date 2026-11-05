#[allow(non_camel_case_types)]
type i64 = String;

pub async fn find(pool: &sqlx::PgPool, id: i64) {
    let _ = sqlx::query(&format!("SELECT * FROM t WHERE id = '{id}'")).fetch_all(pool).await;
}
