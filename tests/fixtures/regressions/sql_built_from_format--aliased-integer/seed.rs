// rule: rust_doctor::source::sql_built_from_format
// weakness: typed-number
// source: US-016, an alias is not proof of a number
// verdict: fires

type Id = i64;

pub async fn find(pool: &sqlx::PgPool, id: Id) {
    let _ = sqlx::query(&format!("SELECT * FROM t WHERE id = {id}")).fetch_all(pool).await;
}
