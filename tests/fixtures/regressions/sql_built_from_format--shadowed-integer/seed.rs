// rule: rust_doctor::source::sql_built_from_format
// weakness: typed-number
// source: US-016, a primitive name a local item shadows
// verdict: fires

#[allow(non_camel_case_types)]
type i64 = String;

pub async fn find(pool: &sqlx::PgPool, id: i64) {
    let _ = sqlx::query(&format!("SELECT * FROM t WHERE id = '{id}'")).fetch_all(pool).await;
}
