// rule: rust_doctor::source::sql_built_from_format
// weakness: glob-import
// source: audit Z2-N02, fixed by US-015
// verdict: fires

use diesel::prelude::*;

pub fn remove(name: &str) {
    let _ = diesel::sql_query(format!("DELETE FROM users WHERE name = '{name}'"));
}
