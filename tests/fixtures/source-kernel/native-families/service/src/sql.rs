pub fn sqlx_format(table: &str) -> sqlx::Query {
    sqlx::query(&format!("SELECT * FROM {table}"))
}

pub fn sqlx_query_as(name: &str) -> sqlx::Query {
    sqlx::query_as::<u8>(&format!("SELECT id FROM users WHERE name = '{}'", name))
}

pub fn sqlx_concatenated(name: &str) -> sqlx::Query {
    sqlx::query_scalar(&("SELECT count(*) FROM users WHERE name = '".to_owned() + name + "'"))
}

pub fn sqlx_through_a_binding(name: &str) -> sqlx::Query {
    let sql = format!("DELETE FROM users WHERE name = '{name}'");
    sqlx::raw_sql(&sql)
}

pub fn sqlx_pushed(name: &str) -> sqlx::Query {
    let mut sql = String::from("SELECT * FROM users WHERE name = '");
    sql.push_str(name);
    sql.push('\'');
    sqlx::query(sql.as_str())
}

pub fn sqlx_imported(name: &str) -> sqlx::Query {
    use sqlx::query;
    query(&format!("SELECT * FROM users WHERE name = '{name}'"))
}

pub fn diesel_format(id: &str) -> diesel::SqlQuery {
    diesel::sql_query(format!("SELECT * FROM users WHERE id = {id}"))
}

pub fn rusqlite_method(name: &str) -> Result<usize, rusqlite::Error> {
    let conn = rusqlite::Connection::open("db.sqlite")?;
    conn.execute(&format!("DELETE FROM users WHERE name = '{name}'"), ())
}

pub fn rusqlite_prepare(name: &str) -> Result<rusqlite::Statement, rusqlite::Error> {
    let conn = rusqlite::Connection::open_in_memory()?;
    conn.prepare(&format!("SELECT * FROM users WHERE name = '{name}'"))
}

pub fn rusqlite_path_form(conn: &rusqlite::Connection, name: &str) -> Result<(), rusqlite::Error> {
    rusqlite::Connection::query_row(conn, &format!("SELECT 1 FROM users WHERE name = '{name}'"), ())
}

pub fn postgres_method(name: &str) -> Result<(), postgres::Error> {
    let mut client = postgres::Client::connect("host=localhost")?;
    client.batch_execute(&format!("DELETE FROM users WHERE name = '{name}'"))
}

pub async fn tokio_postgres_method(name: &str) -> Result<Vec<()>, tokio_postgres::Error> {
    let client = tokio_postgres::Client::connect("host=localhost").await?;
    client.query(&format!("SELECT * FROM users WHERE name = '{name}'")).await
}
