type Id = i64;

pub async fn find(pool: &sqlx::PgPool, id: Id) {
    let _ = sqlx::query(&format!("SELECT * FROM t WHERE id = {id}")).fetch_all(pool).await;
}
