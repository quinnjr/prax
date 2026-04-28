use mysql_async::prelude::*;
use prax_mysql::row_ref::MysqlRowRef;
use prax_query::row::RowRef;

fn test_url() -> String {
    std::env::var("PRAX_MYSQL_URL")
        .unwrap_or_else(|_| "mysql://prax:prax_test_password@localhost:3307/prax_test".into())
}

#[tokio::test]
async fn get_i32_and_string_from_row() {
    let pool = mysql_async::Pool::new(test_url().as_str());
    let mut conn = pool.get_conn().await.unwrap();
    let rows: Vec<mysql_async::Row> = conn.query("SELECT 42 AS n, 'hello' AS s").await.unwrap();
    let r = MysqlRowRef::from_row(rows.into_iter().next().unwrap()).unwrap();
    assert_eq!(r.get_i32("n").unwrap(), 42);
    assert_eq!(r.get_str("s").unwrap(), "hello");
}
