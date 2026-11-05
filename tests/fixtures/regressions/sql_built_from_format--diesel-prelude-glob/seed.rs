use diesel::prelude::*;

pub fn remove(name: &str) {
    let _ = diesel::sql_query(format!("DELETE FROM users WHERE name = '{name}'"));
}
