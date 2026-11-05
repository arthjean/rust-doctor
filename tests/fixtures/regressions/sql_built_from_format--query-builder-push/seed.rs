use sqlx::QueryBuilder;

pub fn find(name: &str) {
    let mut query = QueryBuilder::<sqlx::Postgres>::new("SELECT * FROM users WHERE ");
    query.push(format!("name = '{name}'"));
}
