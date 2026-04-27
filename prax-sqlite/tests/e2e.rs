//! End-to-end tests for prax-sqlite.
//!
//! SQLite is embedded and doesn't need a Docker container, but the tests
//! remain `#[ignore]` by default so the main `cargo test` stays fast.
//! Opt in via `PRAX_E2E=1` (`docker compose run --rm test-sqlite` sets
//! this and passes `--include-ignored`).
//!
//! Every test uses an in-memory database (`:memory:`) or a tempfile so
//! suite runs are hermetic and don't leave artifacts behind.

#![cfg(test)]

use std::time::Duration;

use prax_sqlite::{SqliteConfig, SqlitePool};
use tempfile::TempDir;

fn skip_unless_e2e() -> bool {
    std::env::var("PRAX_E2E").ok().as_deref() == Some("1")
}

/// Test fixture that owns a tempdir for the duration of the test. A
/// file-backed SQLite DB is necessary for the pool to share state across
/// connections; `:memory:` is per-connection.
struct TestDb {
    pool: SqlitePool,
    _tempdir: TempDir,
}

async fn test_db() -> TestDb {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let path = tempdir.path().join("e2e.sqlite");
    let config = SqliteConfig::file(&path);
    let pool = SqlitePool::builder()
        .config(config)
        // Writers serialize; keep the pool small to avoid busy-timeout churn.
        .max_connections(2)
        .connection_timeout(Duration::from_secs(5))
        .build()
        .await
        .expect("build sqlite pool");
    TestDb {
        pool,
        _tempdir: tempdir,
    }
}

#[tokio::test]
#[ignore = "SQLite E2E — run via `docker compose run --rm test-sqlite`"]
async fn e2e_crud_roundtrip() {
    if !skip_unless_e2e() {
        return;
    }
    let db = test_db().await;
    let pool = &db.pool;
    let conn = pool.get().await.expect("acquire");

    conn.execute_batch(
        "CREATE TABLE e2e_crud (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            score INTEGER NOT NULL
        )",
    )
    .await
    .expect("create table");

    // INSERT
    let rowid = conn
        .execute_insert_params(
            "INSERT INTO e2e_crud (name, score) VALUES (?, ?)",
            vec![
                rusqlite::types::Value::Text("alice".into()),
                rusqlite::types::Value::Integer(42),
            ],
        )
        .await
        .expect("insert");
    assert!(rowid > 0);

    // SELECT
    let rows = conn
        .query("SELECT name, score FROM e2e_crud")
        .await
        .expect("select");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["name"], serde_json::json!("alice"));
    assert_eq!(rows[0]["score"], serde_json::json!(42));

    // UPDATE
    let n = conn
        .execute_params(
            "UPDATE e2e_crud SET score = ? WHERE name = ?",
            vec![
                rusqlite::types::Value::Integer(100),
                rusqlite::types::Value::Text("alice".into()),
            ],
        )
        .await
        .expect("update");
    assert_eq!(n, 1);

    // verify
    let row = conn
        .query_one("SELECT score FROM e2e_crud WHERE name = 'alice'")
        .await
        .expect("query_one");
    assert_eq!(row["score"], serde_json::json!(100));

    // DELETE
    let n = conn.execute("DELETE FROM e2e_crud").await.expect("delete");
    assert_eq!(n, 1);
}

#[tokio::test]
#[ignore = "SQLite E2E — run via `docker compose run --rm test-sqlite`"]
async fn e2e_null_and_default_roundtrip() {
    if !skip_unless_e2e() {
        return;
    }
    let db = test_db().await;
    let pool = &db.pool;
    let conn = pool.get().await.expect("conn");
    conn.execute_batch(
        "CREATE TABLE e2e_nulls (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            opt TEXT,
            blob_col BLOB,
            real_col REAL NOT NULL DEFAULT 0.0
        )",
    )
    .await
    .expect("create");

    conn.execute_params(
        "INSERT INTO e2e_nulls (opt, blob_col) VALUES (?, ?)",
        vec![
            rusqlite::types::Value::Null,
            rusqlite::types::Value::Blob(vec![0xDE, 0xAD, 0xBE, 0xEF]),
        ],
    )
    .await
    .expect("insert");

    let row = conn
        .query_one("SELECT opt, real_col FROM e2e_nulls")
        .await
        .expect("query");
    assert_eq!(row["opt"], serde_json::Value::Null);
    assert_eq!(row["real_col"], serde_json::json!(0.0));
}

#[tokio::test]
#[ignore = "SQLite E2E — run via `docker compose run --rm test-sqlite`"]
async fn e2e_missing_row_returns_none() {
    if !skip_unless_e2e() {
        return;
    }
    let db = test_db().await;
    let pool = &db.pool;
    let conn = pool.get().await.expect("conn");
    conn.execute_batch("CREATE TABLE e2e_empty (id INTEGER PRIMARY KEY)")
        .await
        .expect("create");

    let row = conn
        .query_optional("SELECT id FROM e2e_empty WHERE id = 999")
        .await
        .expect("query_optional");
    assert!(row.is_none());
}

#[tokio::test]
#[ignore = "SQLite E2E — run via `docker compose run --rm test-sqlite`"]
async fn e2e_batch_of_ddl_then_dml() {
    if !skip_unless_e2e() {
        return;
    }
    let db = test_db().await;
    let pool = &db.pool;
    let conn = pool.get().await.expect("conn");

    // execute_batch handles multi-statement scripts
    conn.execute_batch(
        "CREATE TABLE e2e_batch_a (id INTEGER PRIMARY KEY, v INT);
         CREATE TABLE e2e_batch_b (id INTEGER PRIMARY KEY, v INT);
         INSERT INTO e2e_batch_a (v) VALUES (1), (2), (3);
         INSERT INTO e2e_batch_b (v) VALUES (10);",
    )
    .await
    .expect("batch");

    let row = conn
        .query_one("SELECT COUNT(*) AS n FROM e2e_batch_a")
        .await
        .expect("count a");
    assert_eq!(row["n"], serde_json::json!(3));

    let row = conn
        .query_one("SELECT COUNT(*) AS n FROM e2e_batch_b")
        .await
        .expect("count b");
    assert_eq!(row["n"], serde_json::json!(1));
}

#[tokio::test]
#[ignore = "SQLite E2E — run via `docker compose run --rm test-sqlite`"]
async fn e2e_pool_reuses_connections() {
    if !skip_unless_e2e() {
        return;
    }
    let db = test_db().await;
    let pool = &db.pool;

    // A sequence of short-lived borrows exercises the release path of
    // the pooled connection wrapper; a leak would show up as a hang when
    // the semaphore exhausts.
    for i in 0..32_i32 {
        let conn = pool.get().await.expect("acquire");
        let row = conn
            .query_one(&format!("SELECT {i} AS v"))
            .await
            .expect("query");
        assert_eq!(row["v"], serde_json::json!(i));
    }
}
