pub fn statement(name: &str) {
    let _ = sea_orm::Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        format!("SELECT * FROM users WHERE name = '{name}'"),
    );
}
