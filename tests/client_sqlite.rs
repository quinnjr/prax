//! End-to-end CRUD test routing every operation through `PraxClient`
//! against an in-tempdir SQLite database.
//!
//! SQLite is embedded and doesn't need a container, but the test still
//! holds a `TempDir` so the database file lives somewhere the pool can
//! share across connections. `:memory:` wouldn't work here — every
//! pooled connection would see a fresh empty DB.
//!
//! Gated by `PRAX_E2E=1` and `#[ignore]`-marked so `cargo test` stays
//! fast. Opt in with:
//!
//! ```sh
//! PRAX_E2E=1 cargo test --test client_sqlite -- --include-ignored
//! ```
//!
//! Mirrors `tests/client_postgres.rs` — create, find_many, update,
//! count, delete_many, final count — but against SQLite so the driver's
//! parameter binding and RowRef bridge get exercised end-to-end.

#![cfg(test)]

use std::time::Duration;

use prax_orm::{Model, PraxClient, client};
use prax_sqlite::{SqliteConfig, SqliteEngine, SqlitePool};
use tempfile::TempDir;

#[derive(Model, Debug)]
#[prax(table = "client_sqlite_users")]
struct User {
    #[prax(id, auto)]
    id: i32,
    #[prax(unique)]
    email: String,
    name: Option<String>,
}

client!(User);

/// Holds the client alongside the tempdir so the sqlite file outlives
/// the test body — if the tempdir dropped, rusqlite would error on the
/// first query with "unable to open database file".
struct SqliteTest {
    client: PraxClient<SqliteEngine>,
    _tempdir: TempDir,
}

fn e2e_enabled() -> bool {
    std::env::var("PRAX_E2E").ok().as_deref() == Some("1")
}

async fn setup() -> Option<SqliteTest> {
    if !e2e_enabled() {
        return None;
    }
    let tempdir = tempfile::tempdir().expect("tempdir");
    let path = tempdir.path().join("client.sqlite");
    let config = SqliteConfig::file(&path);

    let pool: SqlitePool = SqlitePool::builder()
        .config(config)
        // Writers serialize in SQLite; keep the pool small to avoid
        // busy-timeout churn during the test.
        .max_connections(2)
        .connection_timeout(Duration::from_secs(5))
        .build()
        .await
        .expect("build sqlite pool");

    let conn = pool.get().await.expect("acquire conn");
    conn.execute_batch(
        "CREATE TABLE client_sqlite_users (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             email TEXT UNIQUE NOT NULL,
             name TEXT
         )",
    )
    .await
    .expect("create client_sqlite_users");
    drop(conn);

    Some(SqliteTest {
        client: PraxClient::new(SqliteEngine::new(pool)),
        _tempdir: tempdir,
    })
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 (no container needed)"]
async fn client_crud_roundtrip() {
    let Some(t) = setup().await else {
        eprintln!("skipping: PRAX_E2E not set");
        return;
    };
    let c = &t.client;

    // CREATE
    let alice = c
        .user()
        .create()
        .set("email", "alice@sqlite.example.com")
        .set("name", "Alice")
        .exec()
        .await
        .expect("create alice");
    assert_eq!(alice.email, "alice@sqlite.example.com");
    assert_eq!(alice.name.as_deref(), Some("Alice"));
    assert!(alice.id > 0, "auto id should be assigned");

    // FIND_MANY
    let all = c
        .user()
        .find_many()
        .exec()
        .await
        .expect("find_many after create");
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].id, alice.id);

    // UPDATE by equality on id (WhereParam -> Filter conversion)
    let updated = c
        .user()
        .update()
        .r#where(user::id::equals(alice.id))
        .set("name", "Alicia")
        .exec()
        .await
        .expect("update alice");
    assert_eq!(updated.len(), 1);
    assert_eq!(updated[0].name.as_deref(), Some("Alicia"));
    assert_eq!(updated[0].id, alice.id);

    // COUNT
    let before_delete = c.user().count().exec().await.expect("count before delete");
    assert_eq!(before_delete, 1);

    // DELETE_MANY using a string-contains filter
    let deleted = c
        .user()
        .delete_many()
        .r#where(user::email::contains("sqlite.example.com"))
        .exec()
        .await
        .expect("delete_many");
    assert_eq!(deleted, 1);

    // Verify empty
    let remaining = c.user().count().exec().await.expect("count after delete");
    assert_eq!(remaining, 0);
}
