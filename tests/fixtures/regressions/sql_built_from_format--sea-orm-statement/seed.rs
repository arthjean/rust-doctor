// rule: rust_doctor::source::sql_built_from_format
// weakness: api-variant
// source: audit Z1a-N04, fixed by US-018
// verdict: fires

pub fn statement(name: &str) {
    let _ = sea_orm::Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        format!("SELECT * FROM users WHERE name = '{name}'"),
    );
}
