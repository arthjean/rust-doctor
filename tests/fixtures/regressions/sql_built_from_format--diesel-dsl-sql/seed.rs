pub fn filter(name: &str) {
    let _ = diesel::dsl::sql::<diesel::sql_types::Bool>(&format!("name = '{name}'"));
}
