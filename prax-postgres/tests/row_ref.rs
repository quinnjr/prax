use prax_postgres::row_ref::PgRow;
use prax_postgres::{PgPool, PgPoolBuilder};
use prax_query::row::RowRef;

fn test_url() -> String {
    std::env::var("PRAX_POSTGRES_URL")
        .unwrap_or_else(|_| "postgres://prax:prax_test_password@localhost:5433/prax_test".into())
}

#[tokio::test]
async fn get_i32_and_string_from_row() {
    let pool: PgPool = PgPoolBuilder::new().url(test_url()).build().await.unwrap();
    let conn = pool.get().await.unwrap();
    let raw_row = conn
        .query_one("SELECT 42::int4 AS n, 'hello'::text AS s", &[])
        .await
        .unwrap();
    let row = PgRow::from(raw_row);
    assert_eq!(row.get_i32("n").unwrap(), 42);
    assert_eq!(row.get_str("s").unwrap(), "hello");
}
