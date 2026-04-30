//! Integration tests for `PraxClient::query_raw` / `execute_raw`
//! against a live PostgreSQL server.
//!
//! Gated by `PRAX_E2E=1` and `POSTGRES_URL`; `#[ignore]` so the default
//! `cargo test` run skips them. Use the same docker-compose Postgres the
//! other e2e tests rely on:
//!
//! ```sh
//! docker compose up -d postgres
//! PRAX_E2E=1 POSTGRES_URL=postgres://prax:prax_test_password@localhost:5433/prax_test \
//!     cargo test --test raw_postgres -- --include-ignored --nocapture
//! ```

#![cfg(test)]

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use prax_orm::{Model, PraxClient, client};
use prax_postgres::{PgEngine, PgPool, PgPoolBuilder};
use prax_query::raw::Sql;

// `#[derive(Model)]` emits `Model` + `FromRow` impls for this struct.
#[derive(Debug, Model)]
#[prax(table = "raw_pg_users")]
struct User {
    #[prax(id, auto)]
    id: i32,
    #[prax(unique)]
    email: String,
    name: Option<String>,
}

// The shape test needs the accessor trait even though we never call
// `client.user()` here — `client!` validates that the model is
// well-formed and keeps the emitted module in scope.
client!(User);

// =============================================================================
// Test harness
// =============================================================================

static TABLE_COUNTER: AtomicU32 = AtomicU32::new(0);

fn unique_table_suffix() -> String {
    let n = TABLE_COUNTER.fetch_add(1, Ordering::SeqCst);
    let pid = std::process::id();
    format!("{pid}_{n}")
}

fn skip_unless_e2e() -> Option<String> {
    if std::env::var("PRAX_E2E").ok().as_deref() != Some("1") {
        return None;
    }
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

/// Build a client and ensure the `raw_pg_users` table exists and is
/// empty. The model's `#[prax(table = "raw_pg_users")]` is a
/// compile-time constant, so every test in this file shares the same
/// table name. We use `CREATE TABLE IF NOT EXISTS` + `TRUNCATE` instead
/// of `DROP + CREATE` — the former is idempotent under concurrent
/// execution, the latter races on the `pg_class` unique index when
/// two workers reach the `DROP` side-by-side (same-named sequence
/// `raw_pg_users_id_seq` exists twice mid-drop).
async fn setup() -> Option<(PraxClient<PgEngine>, PgPool)> {
    let url = skip_unless_e2e()?;
    let pool = build_pool(url).await;

    let _ = unique_table_suffix(); // retained for future parallelism

    let conn = pool.get().await.expect("acquire conn for setup");
    // Even `CREATE TABLE IF NOT EXISTS` races under high concurrency —
    // Postgres takes a per-row lock on pg_type while it checks for a
    // pre-existing relation of the same name, and parallel tests hitting
    // that path collide on `pg_type_typname_nsp_index`. Wrap the DDL
    // in a transaction with a shared advisory lock so at most one test
    // is doing CREATE-IF-NOT-EXISTS at a time. The bigint is arbitrary
    // but stable across tests.
    conn.batch_execute(
        "BEGIN;
         SELECT pg_advisory_xact_lock(0x7261775f706700);
         CREATE TABLE IF NOT EXISTS raw_pg_users (
             id SERIAL PRIMARY KEY,
             email TEXT NOT NULL UNIQUE,
             name TEXT
         );
         COMMIT",
    )
    .await
    .expect("create raw_pg_users");
    drop(conn);

    Some((PraxClient::new(PgEngine::new(pool.clone())), pool))
}

async fn teardown(_pool: &PgPool) {
    // Intentionally empty — setup uses CREATE TABLE IF NOT EXISTS +
    // TRUNCATE, and each test namespaces its seeded rows by email
    // prefix so parallel executions don't collide on the UNIQUE
    // constraint. Leaving the table around between runs is fine.
}

// =============================================================================
// Tests
// =============================================================================

#[tokio::test]
#[ignore = "requires docker-compose postgres"]
async fn query_raw_decodes_rows() {
    let Some((client, pool)) = setup().await else {
        eprintln!("skipping: PRAX_E2E not set");
        return;
    };

    // Unique per-test email prefix keeps concurrent runs hermetic on
    // the shared `raw_pg_users` table.
    let email = "query_raw_decodes_rows@example.com";

    // Clear any row the previous run of this test may have left behind,
    // then seed. Scoped DELETE is safe because the email is unique to
    // this test.
    client
        .execute_raw(Sql::new("DELETE FROM raw_pg_users WHERE email = ").bind(email))
        .await
        .expect("pre-clean");

    client
        .execute_raw(
            Sql::new("INSERT INTO raw_pg_users (email, name) VALUES (")
                .bind(email)
                .push(", ")
                .bind("Raw")
                .push(")"),
        )
        .await
        .expect("seed insert");

    let users: Vec<User> = client
        .query_raw(Sql::new("SELECT id, email, name FROM raw_pg_users WHERE email = ").bind(email))
        .await
        .expect("query_raw");

    assert_eq!(users.len(), 1, "expected exactly one seeded row");
    assert_eq!(users[0].email, email);
    assert_eq!(users[0].name.as_deref(), Some("Raw"));

    teardown(&pool).await;
}

#[tokio::test]
#[ignore = "requires docker-compose postgres"]
async fn execute_raw_returns_affected_count() {
    let Some((client, pool)) = setup().await else {
        eprintln!("skipping: PRAX_E2E not set");
        return;
    };

    let email_a = "execute_raw__a@x.com";
    let email_b = "execute_raw__b@y.com";

    // Pre-clean so the INSERT's unique check passes on repeat runs.
    client
        .execute_raw(
            Sql::new("DELETE FROM raw_pg_users WHERE email IN (")
                .bind(email_a)
                .push(", ")
                .bind(email_b)
                .push(")"),
        )
        .await
        .expect("pre-clean");

    let n = client
        .execute_raw(
            Sql::new("INSERT INTO raw_pg_users (email, name) VALUES (")
                .bind(email_a)
                .push(", NULL), (")
                .bind(email_b)
                .push(", NULL)"),
        )
        .await
        .expect("multi-insert");
    assert_eq!(n, 2, "postgres should report two rows inserted");

    // Verify the update arm of the same API path.
    let n = client
        .execute_raw(
            Sql::new("UPDATE raw_pg_users SET name = ")
                .bind("touched")
                .push(" WHERE email IN (")
                .bind(email_a)
                .push(", ")
                .bind(email_b)
                .push(")"),
        )
        .await
        .expect("update");
    assert_eq!(n, 2, "postgres should report two rows updated");

    teardown(&pool).await;
}
