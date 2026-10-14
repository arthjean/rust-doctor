// rule: rust_doctor::source::sql_built_from_format
// weakness: api-variant
// source: audit Z1a-N04, fixed by US-018
// verdict: fires

pub fn filter(name: &str) {
    let _ = diesel::dsl::sql::<diesel::sql_types::Bool>(&format!("name = '{name}'"));
}
