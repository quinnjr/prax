use prax_query::row::{FromRow, RowError, RowRef};
use prax_query::traits::{Model, QueryEngine};
use prax_sqlite::{SqliteEngine, SqlitePool, SqlitePoolBuilder};

struct Item {
    id: i32,
    name: String,
}

impl Model for Item {
    const MODEL_NAME: &'static str = "Item";
    const TABLE_NAME: &'static str = "items";
    const PRIMARY_KEY: &'static [&'static str] = &["id"];
    const COLUMNS: &'static [&'static str] = &["id", "name"];
}

impl FromRow for Item {
    fn from_row(r: &impl RowRef) -> Result<Self, RowError> {
        Ok(Item {
            id: r.get_i32("id")?,
            name: r.get_string("name")?,
        })
    }
}

#[tokio::test]
async fn sqlite_query_many() {
    // Use a shared in-memory database so all connections see the same data
    let pool: SqlitePool = SqlitePoolBuilder::new()
        .url("file::memory:?cache=shared")
        .build()
        .await
        .unwrap();
    let engine = SqliteEngine::new(pool);
    engine
        .execute_raw(
            "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
            vec![],
        )
        .await
        .unwrap();
    engine
        .execute_raw(
            "INSERT INTO items (id, name) VALUES (1, 'a'), (2, 'b')",
            vec![],
        )
        .await
        .unwrap();
    let rows = engine
        .query_many::<Item>("SELECT id, name FROM items ORDER BY id", vec![])
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].id, 1);
    assert_eq!(rows[0].name, "a");
    assert_eq!(rows[1].id, 2);
    assert_eq!(rows[1].name, "b");
}
