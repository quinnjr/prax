//! Integration tests for `PraxClient::transaction` against an
//! in-tempdir SQLite database.
//!
//! Gated by `PRAX_E2E=1` and `#[ignore]` so the default `cargo test`
//! run skips them. Opt in with:
//!
//! ```sh
//! PRAX_E2E=1 cargo test --test tx_sqlite -- --include-ignored --nocapture
//! ```
//!
//! Mirrors `tests/tx_postgres.rs`. SQLite uses a tempdir-backed file
//! rather than `:memory:` so the pool's multiple connections share
//! the same database — with `:memory:` each pool connection would
//! see a fresh empty DB and the test would fail nondeterministically.

#![cfg(test)]

use std::time::Duration;

use prax_orm::{Model, PraxClient, client};
use prax_query::error::{QueryError, QueryResult};
use prax_query::raw::Sql;
use prax_sqlite::{SqliteConfig, SqliteEngine, SqlitePool};
use tempfile::TempDir;

#[derive(Debug, Model)]
#[prax(table = "tx_sqlite_users")]
struct User {
    #[prax(id, auto)]
    id: i32,
    #[prax(unique)]
    email: String,
    name: Option<String>,
}

client!(User);

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
    let path = tempdir.path().join("tx.sqlite");
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
        "CREATE TABLE IF NOT EXISTS tx_sqlite_users (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             email TEXT UNIQUE NOT NULL,
             name TEXT
         )",
    )
    .await
    .expect("create tx_sqlite_users");
    drop(conn);

    Some(SqliteTest {
        client: PraxClient::new(SqliteEngine::new(pool)),
        _tempdir: tempdir,
    })
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 (no container needed)"]
async fn transaction_rolls_back_on_error() {
    let Some(t) = setup().await else {
        eprintln!("skipping: PRAX_E2E not set");
        return;
    };
    let client = &t.client;

    let email = "tx_rollback@sqlite.example.com";

    client
        .execute_raw(Sql::new("DELETE FROM tx_sqlite_users WHERE email = ").bind(email))
        .await
        .expect("pre-clean");

    let result: QueryResult<()> = client
        .transaction(|tx| async move {
            tx.user()
                .create()
                .set("email", "tx_rollback@sqlite.example.com")
                .set("name", "Rolled Back")
                .exec()
                .await?;

            Err(QueryError::internal("intentional rollback trigger"))
        })
        .await;

    assert!(
        result.is_err(),
        "closure returned Err, tx should surface it"
    );

    let rows: Vec<User> = client
        .query_raw(
            Sql::new("SELECT id, email, name FROM tx_sqlite_users WHERE email = ").bind(email),
        )
        .await
        .expect("post-rollback read");
    assert!(
        rows.is_empty(),
        "rollback did not happen: found {} row(s) with email {}",
        rows.len(),
        email
    );
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 (no container needed)"]
async fn transaction_commits_on_ok() {
    let Some(t) = setup().await else {
        eprintln!("skipping: PRAX_E2E not set");
        return;
    };
    let client = &t.client;

    let email = "tx_commit@sqlite.example.com";

    client
        .execute_raw(Sql::new("DELETE FROM tx_sqlite_users WHERE email = ").bind(email))
        .await
        .expect("pre-clean");

    let created_id: i32 = client
        .transaction(|tx| async move {
            let u = tx
                .user()
                .create()
                .set("email", "tx_commit@sqlite.example.com")
                .set("name", "Committed")
                .exec()
                .await?;
            Ok(u.id)
        })
        .await
        .expect("commit-on-ok transaction");
    assert!(created_id > 0, "expected auto-assigned PK from INSERT");

    let rows: Vec<User> = client
        .query_raw(
            Sql::new("SELECT id, email, name FROM tx_sqlite_users WHERE email = ").bind(email),
        )
        .await
        .expect("post-commit read");
    assert_eq!(rows.len(), 1, "commit did not persist the row");
    assert_eq!(rows[0].id, created_id);
    assert_eq!(rows[0].name.as_deref(), Some("Committed"));

    client
        .execute_raw(Sql::new("DELETE FROM tx_sqlite_users WHERE email = ").bind(email))
        .await
        .expect("post-test cleanup");
}
