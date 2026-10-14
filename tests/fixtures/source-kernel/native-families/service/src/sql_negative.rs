const TABLE: &str = "users";

pub fn sqlx_macro(name: &str) -> sqlx::Query {
    sqlx::query!("SELECT * FROM users WHERE name = $1", name)
}

pub fn sqlx_literal() -> sqlx::Query {
    sqlx::query("SELECT 1")
}

pub fn sqlx_bound(name: &str) -> sqlx::Query {
    sqlx::query("SELECT * FROM users WHERE name = $1").bind(name)
}

pub fn literal_arguments() -> sqlx::Query {
    sqlx::query(&format!("SELECT * FROM {} WHERE id = {}", "users", 1))
}

pub fn const_arguments() -> sqlx::Query {
    sqlx::query(&format!("SELECT * FROM {TABLE}"))
}

pub fn literal_pushed() -> sqlx::Query {
    let mut sql = String::from("SELECT *");
    sql.push_str(" FROM users");
    sqlx::query(&sql)
}

pub struct Cache;

impl Cache {
    pub fn execute(&self, _key: &str) {}

    pub fn open() -> Self {
        Self
    }
}

pub fn unlisted_receiver(cache: &Cache, name: &str) {
    cache.execute(&format!("user:{name}"));
}

pub fn unlisted_constructor(name: &str) {
    let cache = Cache::open();
    cache.execute(&format!("user:{name}"));
}

pub async fn pattern_bound_client(name: &str) -> Result<Vec<()>, tokio_postgres::Error> {
    let (client, _connection) = tokio_postgres::connect("host=localhost").await?;
    client.query(&format!("SELECT * FROM users WHERE name = '{name}'")).await
}
