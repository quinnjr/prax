//! Integration tests for `PraxClient::transaction` against a live
//! PostgreSQL server.
//!
//! Gated by `PRAX_E2E=1` and `#[ignore]` so the default `cargo test`
//! run skips them. Opt in with:
//!
//! ```sh
//! docker compose up -d postgres
//! PRAX_E2E=1 cargo test --test tx_postgres -- --include-ignored --nocapture
//! ```
//!
//! Shares the `tx_pg_users` table across both tests (like
//! `raw_postgres.rs`) and namespaces rows by email prefix so parallel
//! runs stay hermetic on the unique-email constraint.

#![cfg(test)]

use std::time::Duration;

use prax_orm::{Model, PraxClient, client};
use prax_postgres::{PgEngine, PgPool, PgPoolBuilder};
use prax_query::error::{QueryError, QueryResult};
use prax_query::raw::Sql;

#[derive(Debug, Model)]
#[prax(table = "tx_pg_users")]
struct User {
    #[prax(id, auto)]
    id: i32,
    #[prax(unique)]
    email: String,
    name: Option<String>,
}

// The client! macro wires `user()` onto PraxClient<E> and validates
// the derive output, so every test in this file funnels through the
// same typed Client API.
client!(User);

fn postgres_url() -> Option<String> {
    if std::env::var("PRAX_E2E").ok().as_deref() != Some("1") {
        return None;
    }
    // Compose publishes Postgres on host 5432 via `network_mode: host`.
    // The plan default was stale; keep the env-var override for
    // deployments that remap the port.
    Some(
        std::env::var("POSTGRES_URL").unwrap_or_else(|_| {
            "postgres://prax:prax_test_password@localhost:5432/prax_test".into()
        }),
    )
}

async fn build_pool(url: String) -> PgPool {
    PgPoolBuilder::new()
        .url(url)
        .max_connections(4)
        .connection_timeout(Duration::from_secs(10))
        .build()
        .await
        .expect("connect to postgres")
}

/// Build a client and ensure the `tx_pg_users` table exists.
///
/// Uses the same advisory-lock + `CREATE TABLE IF NOT EXISTS` pattern
/// as `tests/raw_postgres.rs` so concurrent tests don't race on
/// `pg_class_relname_nsp_index` while creating the table.
async fn setup() -> Option<(PraxClient<PgEngine>, PgPool)> {
    let url = postgres_url()?;
    let pool = build_pool(url).await;

    let conn = pool.get().await.expect("acquire conn for setup");
    conn.batch_execute(
        "BEGIN;
         SELECT pg_advisory_xact_lock(0x74785f70675f75);
         CREATE TABLE IF NOT EXISTS tx_pg_users (
             id SERIAL PRIMARY KEY,
             email TEXT NOT NULL UNIQUE,
             name TEXT
         );
         COMMIT",
    )
    .await
    .expect("create tx_pg_users");
    drop(conn);

    Some((PraxClient::new(PgEngine::new(pool.clone())), pool))
}

#[tokio::test]
#[ignore = "requires docker-compose postgres (PRAX_E2E=1)"]
async fn transaction_rolls_back_on_error() {
    let Some((client, _pool)) = setup().await else {
        eprintln!("skipping: PRAX_E2E not set");
        return;
    };

    // Scoped email prefix so the commit-on-ok sibling test can't
    // fight this one on the shared table's UNIQUE constraint.
    let email = "tx_rollback@example.com";

    // Pre-clean: remove any leftover row from a previous run so the
    // INSERT inside the transaction is expected to succeed.
    client
        .execute_raw(Sql::new("DELETE FROM tx_pg_users WHERE email = ").bind(email))
        .await
        .expect("pre-clean");

    let result: QueryResult<()> = client
        .transaction(|tx| async move {
            // Insert inside the tx. If rollback works, this row should
            // not be visible after the outer transaction resolves.
            tx.user()
                .create()
                .set("email", "tx_rollback@example.com")
                .set("name", "Rolled Back")
                .exec()
                .await?;

            // Deliberately bail with a non-database error so the
            // engine's rollback arm kicks in. The exact error kind
            // doesn't matter — any `Err(_)` triggers ROLLBACK.
            Err(QueryError::internal("intentional rollback trigger"))
        })
        .await;

    assert!(
        result.is_err(),
        "closure returned Err, tx should surface it"
    );

    // After rollback the row must not exist.
    let rows: Vec<User> = client
        .query_raw(Sql::new("SELECT id, email, name FROM tx_pg_users WHERE email = ").bind(email))
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
#[ignore = "requires docker-compose postgres (PRAX_E2E=1)"]
async fn transaction_commits_on_ok() {
    let Some((client, _pool)) = setup().await else {
        eprintln!("skipping: PRAX_E2E not set");
        return;
    };

    let email = "tx_commit@example.com";

    // Pre-clean: ensure the UNIQUE insert can succeed on repeat runs.
    client
        .execute_raw(Sql::new("DELETE FROM tx_pg_users WHERE email = ").bind(email))
        .await
        .expect("pre-clean");

    let created_id: i32 = client
        .transaction(|tx| async move {
            let u = tx
                .user()
                .create()
                .set("email", "tx_commit@example.com")
                .set("name", "Committed")
                .exec()
                .await?;
            // Return the PK so the outer scope can assert on it after
            // commit — proves the Ok-branch round-trips a value.
            Ok(u.id)
        })
        .await
        .expect("commit-on-ok transaction");
    assert!(created_id > 0, "expected auto-assigned PK from INSERT");

    // After commit the row is visible via a fresh pool-backed query.
    let rows: Vec<User> = client
        .query_raw(Sql::new("SELECT id, email, name FROM tx_pg_users WHERE email = ").bind(email))
        .await
        .expect("post-commit read");
    assert_eq!(rows.len(), 1, "commit did not persist the row");
    assert_eq!(rows[0].id, created_id);
    assert_eq!(rows[0].name.as_deref(), Some("Committed"));

    // Tidy the table so the next run starts clean. This is outside
    // the transaction — uses a fresh pool connection.
    client
        .execute_raw(Sql::new("DELETE FROM tx_pg_users WHERE email = ").bind(email))
        .await
        .expect("post-test cleanup");
}
