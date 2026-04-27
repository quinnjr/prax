use prax_postgres::{PgEngine, PgPool, PgPoolBuilder};
use prax_query::filter::FilterValue;
use prax_query::row::{FromRow, RowError, RowRef};
use prax_query::traits::{Model, QueryEngine};

fn test_url() -> String {
    std::env::var("PRAX_POSTGRES_URL")
        .unwrap_or_else(|_| "postgres://prax:prax_test_password@localhost:5433/prax_test".into())
}

#[derive(Debug, PartialEq)]
struct Person {
    id: i32,
    email: String,
}

impl Model for Person {
    const MODEL_NAME: &'static str = "Person";
    const TABLE_NAME: &'static str = "crud_people";
    const PRIMARY_KEY: &'static [&'static str] = &["id"];
    const COLUMNS: &'static [&'static str] = &["id", "email"];
}

impl FromRow for Person {
    fn from_row(row: &impl RowRef) -> Result<Self, RowError> {
        Ok(Person {
            id: row.get_i32("id")?,
            email: row.get_string("email")?,
        })
    }
}

async fn setup(pool: &PgPool) {
    let conn = pool.get().await.unwrap();
    conn.batch_execute(
        "DROP TABLE IF EXISTS crud_people;
         CREATE TABLE crud_people (id SERIAL PRIMARY KEY, email TEXT NOT NULL);
         INSERT INTO crud_people (email) VALUES ('alice@example.com'), ('bob@example.com');",
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn query_many_returns_typed_rows() {
    let pool: PgPool = PgPoolBuilder::new().url(test_url()).build().await.unwrap();
    setup(&pool).await;
    let engine = PgEngine::new(pool);
    let rows = engine
        .query_many::<Person>(
            "SELECT id, email FROM crud_people ORDER BY id",
            Vec::<FilterValue>::new(),
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].email, "alice@example.com");
}
