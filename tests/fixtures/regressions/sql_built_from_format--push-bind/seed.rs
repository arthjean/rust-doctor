use sqlx::QueryBuilder;

pub fn find(name: &str) {
    let mut query = QueryBuilder::<sqlx::Postgres>::new("SELECT * FROM users WHERE name = ");
    query.push_bind(name);
}
