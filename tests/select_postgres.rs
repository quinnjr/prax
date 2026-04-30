//! Integration test for `FindManyOperation::select(...)` against a live
//! PostgreSQL server.
//!
//! Task 28 narrows the emitted SQL column list when a caller provides a
//! `Select::fields(...)` projection. Partial hydration is still a
//! follow-up — rows decode as whole `T` structs — so this test checks
//! two things:
//!
//! 1. `build_sql` emits an explicit column list instead of `SELECT *`.
//! 2. An `.exec()` through the live engine still round-trips when the
//!    projection covers every field on the model.
//!
//! Gated by `PRAX_E2E=1` and `#[ignore]`-marked. Opt in with:
//!
//! ```sh
//! docker compose up -d postgres
//! PRAX_E2E=1 POSTGRES_URL=postgres://prax:prax_test_password@localhost:5432/prax_test \
//!     cargo test --test select_postgres -- --include-ignored --nocapture
//! ```

#![cfg(test)]

use std::time::Duration;

use prax_orm::{Model, PraxClient, client};
use prax_postgres::{PgEngine, PgPool, PgPoolBuilder};
use prax_query::raw::Sql;
use prax_query::types::Select;

#[derive(Debug, Model)]
#[prax(table = "select_pg_users")]
struct User {
    #[prax(id, auto)]
    id: i32,
    #[prax(unique)]
    email: String,
    name: Option<String>,
}

client!(User);

fn postgres_url() -> Option<String> {
    if std::env::var("PRAX_E2E").ok().as_deref() != Some("1") {
        return None;
    }
    Some(
        std::env::var("POSTGRES_URL").unwrap_or_else(|_| {
            "postgres://prax:prax_test_password@localhost:5432/prax_test".into()
        }),
    )
}

// Build the client, ensure the shared `select_pg_users` table exists,
// and truncate so every run starts from a clean slate. Uses the same
// advisory-lock + CREATE IF NOT EXISTS + TRUNCATE pattern as
// raw_postgres.rs to stay hermetic under concurrent test execution.
async fn setup() -> Option<PraxClient<PgEngine>> {
    let url = postgres_url()?;
    let pool: PgPool = PgPoolBuilder::new()
        .url(url)
        .max_connections(4)
        .connection_timeout(Duration::from_secs(10))
        .build()
        .await
        .expect("connect to postgres");

    let conn = pool.get().await.expect("acquire conn");
    conn.batch_execute(
        "BEGIN;
         SELECT pg_advisory_xact_lock(0x73656c5f706700);
         CREATE TABLE IF NOT EXISTS select_pg_users (
             id SERIAL PRIMARY KEY,
             email TEXT NOT NULL UNIQUE,
             name TEXT
         );
         TRUNCATE TABLE select_pg_users RESTART IDENTITY;
         COMMIT",
    )
    .await
    .expect("create select_pg_users");
    drop(conn);

    Some(PraxClient::new(PgEngine::new(pool)))
}

#[tokio::test]
#[ignore = "requires docker-compose postgres (PRAX_E2E=1)"]
async fn select_narrows_sql_column_list() {
    let Some(c) = setup().await else {
        eprintln!("skipping: PRAX_E2E not set");
        return;
    };

    // Seed two rows.
    c.execute_raw(
        Sql::new("INSERT INTO select_pg_users (email, name) VALUES (")
            .bind("a@example.com")
            .push(", ")
            .bind("A")
            .push("), (")
            .bind("b@example.com")
            .push(", ")
            .bind("B")
            .push(")"),
    )
    .await
    .expect("seed");

    // Inspect the operation's emitted SQL directly — this is the
    // load-bearing assertion for task 28. `SELECT id, email FROM ...`
    // is the narrowing proof; we never send this one over the wire
    // because the missing `name` column would trip `from_row` (see the
    // migration note in CHANGELOG).
    let narrow_op = c.user().find_many().select(Select::fields(["id", "email"]));
    let (narrow_sql, _) = narrow_op.build_sql(&prax_query::dialect::Postgres);
    assert!(
        narrow_sql.contains("SELECT id, email FROM select_pg_users")
            && !narrow_sql.contains("SELECT *"),
        "expected narrow SELECT list, got: {narrow_sql}"
    );

    // Full-projection path: every non-Option field on User must appear
    // in the column list so FromRow succeeds. Proves the narrowing
    // wiring doesn't regress round-trip decoding when used correctly.
    let users = c
        .user()
        .find_many()
        .select(Select::fields(["id", "email", "name"]))
        .exec()
        .await
        .expect("find_many with explicit projection");
    assert_eq!(users.len(), 2, "expected both seeded rows");

    // Spot-check the emitted SQL on the full projection for symmetry
    // with the narrow case — catches a future regression where
    // .select([...]) silently reverts to `*`.
    let full_op = c
        .user()
        .find_many()
        .select(Select::fields(["id", "email", "name"]));
    let (full_sql, _) = full_op.build_sql(&prax_query::dialect::Postgres);
    assert!(
        full_sql.contains("SELECT id, email, name FROM select_pg_users"),
        "expected explicit three-column SELECT, got: {full_sql}"
    );

    // And the default (no .select()) still emits SELECT *.
    let default_op = c.user().find_many();
    let (default_sql, _) = default_op.build_sql(&prax_query::dialect::Postgres);
    assert!(
        default_sql.contains("SELECT * FROM select_pg_users"),
        "expected default SELECT *, got: {default_sql}"
    );
}
