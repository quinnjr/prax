//! End-to-end tests for prax-postgres against a live PostgreSQL server.
//!
//! Gated by `PRAX_E2E=1` and requires `POSTGRES_URL` pointing at a
//! reachable Postgres instance. Tests are `#[ignore]`-marked so
//! `cargo test` in a dev workflow skips them; the docker-compose
//! `test-postgres` runner passes `--include-ignored` to opt in.
//!
//! ```sh
//! docker compose up -d postgres
//! docker compose run --rm test-postgres
//! ```
//!
//! Each test uses a uniquely named table to avoid stepping on other
//! tests' data when the suite is run in parallel.

#![cfg(test)]

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use prax_postgres::{PgPool, PgPoolBuilder};

// =============================================================================
// Test harness
// =============================================================================

/// Per-process counter used to mint unique table names per test so that
/// parallel runs don't interfere with each other.
static TABLE_COUNTER: AtomicU32 = AtomicU32::new(0);

fn unique_table(prefix: &str) -> String {
    let n = TABLE_COUNTER.fetch_add(1, Ordering::SeqCst);
    let pid = std::process::id();
    format!("e2e_{prefix}_{pid}_{n}")
}

fn skip_unless_e2e() -> Option<String> {
    if std::env::var("PRAX_E2E").ok().as_deref() != Some("1") {
        return None;
    }
    std::env::var("POSTGRES_URL").ok()
}

async fn pool() -> PgPool {
    let url = skip_unless_e2e().expect("PRAX_E2E=1 and POSTGRES_URL required");
    PgPoolBuilder::new()
        .url(url)
        .max_connections(4)
        .connection_timeout(Duration::from_secs(10))
        .build()
        .await
        .expect("connect to postgres")
}

async fn drop_table(pool: &PgPool, table: &str) {
    let conn = pool.get().await.expect("acquire conn for cleanup");
    let _ = conn
        .batch_execute(&format!("DROP TABLE IF EXISTS {table}"))
        .await;
}

// =============================================================================
// Core CRUD
// =============================================================================

#[tokio::test]
#[ignore = "requires running PostgreSQL via docker-compose"]
async fn e2e_crud_roundtrip() {
    if skip_unless_e2e().is_none() {
        eprintln!("skipping: PRAX_E2E not set");
        return;
    }
    let pool = pool().await;
    let table = unique_table("crud");
    drop_table(&pool, &table).await;

    let conn = pool.get().await.expect("conn");
    conn.batch_execute(&format!(
        "CREATE TABLE {table} (
            id SERIAL PRIMARY KEY,
            name TEXT NOT NULL,
            score INTEGER NOT NULL DEFAULT 0
        )"
    ))
    .await
    .expect("create table");

    // INSERT
    let n = conn
        .execute(
            &format!("INSERT INTO {table} (name, score) VALUES ($1, $2)"),
            &[&"alice", &42_i32],
        )
        .await
        .expect("insert");
    assert_eq!(n, 1, "expected one row inserted");

    // SELECT
    let rows = conn
        .query(&format!("SELECT name, score FROM {table}"), &[])
        .await
        .expect("select");
    assert_eq!(rows.len(), 1);
    let name: &str = rows[0].get(0);
    let score: i32 = rows[0].get(1);
    assert_eq!(name, "alice");
    assert_eq!(score, 42);

    // UPDATE
    let n = conn
        .execute(
            &format!("UPDATE {table} SET score = $1 WHERE name = $2"),
            &[&100_i32, &"alice"],
        )
        .await
        .expect("update");
    assert_eq!(n, 1);

    // verify
    let row = conn
        .query_one(
            &format!("SELECT score FROM {table} WHERE name = $1"),
            &[&"alice"],
        )
        .await
        .expect("query_one");
    let score: i32 = row.get(0);
    assert_eq!(score, 100);

    // DELETE
    let n = conn
        .execute(&format!("DELETE FROM {table}"), &[])
        .await
        .expect("delete");
    assert_eq!(n, 1);

    let rows = conn
        .query(&format!("SELECT * FROM {table}"), &[])
        .await
        .expect("select after delete");
    assert!(rows.is_empty());

    drop_table(&pool, &table).await;
}

// =============================================================================
// Transactions & savepoints
// =============================================================================

#[tokio::test]
#[ignore = "requires running PostgreSQL via docker-compose"]
async fn e2e_transaction_commit() {
    if skip_unless_e2e().is_none() {
        return;
    }
    let pool = pool().await;
    let table = unique_table("tx_commit");
    drop_table(&pool, &table).await;

    let mut conn = pool.get().await.expect("conn");
    conn.batch_execute(&format!(
        "CREATE TABLE {table} (id SERIAL PRIMARY KEY, v INTEGER NOT NULL)"
    ))
    .await
    .expect("create table");

    let tx = conn.transaction().await.expect("begin");
    tx.execute(&format!("INSERT INTO {table} (v) VALUES ($1)"), &[&1_i32])
        .await
        .expect("insert 1");
    tx.execute(&format!("INSERT INTO {table} (v) VALUES ($1)"), &[&2_i32])
        .await
        .expect("insert 2");
    tx.commit().await.expect("commit");

    let rows = conn
        .query(&format!("SELECT v FROM {table} ORDER BY v"), &[])
        .await
        .expect("select");
    let vs: Vec<i32> = rows.iter().map(|r| r.get::<_, i32>(0)).collect();
    assert_eq!(vs, vec![1, 2]);

    drop_table(&pool, &table).await;
}

#[tokio::test]
#[ignore = "requires running PostgreSQL via docker-compose"]
async fn e2e_transaction_rollback() {
    if skip_unless_e2e().is_none() {
        return;
    }
    let pool = pool().await;
    let table = unique_table("tx_rollback");
    drop_table(&pool, &table).await;

    let mut conn = pool.get().await.expect("conn");
    conn.batch_execute(&format!(
        "CREATE TABLE {table} (id SERIAL PRIMARY KEY, v INTEGER NOT NULL)"
    ))
    .await
    .expect("create");

    let tx = conn.transaction().await.expect("begin");
    tx.execute(&format!("INSERT INTO {table} (v) VALUES ($1)"), &[&999_i32])
        .await
        .expect("insert");
    tx.rollback().await.expect("rollback");

    let rows = conn
        .query(&format!("SELECT v FROM {table}"), &[])
        .await
        .expect("select");
    assert!(rows.is_empty(), "rollback should leave no rows");

    drop_table(&pool, &table).await;
}

#[tokio::test]
#[ignore = "requires running PostgreSQL via docker-compose"]
async fn e2e_savepoint_rollback() {
    if skip_unless_e2e().is_none() {
        return;
    }
    let pool = pool().await;
    let table = unique_table("sp");
    drop_table(&pool, &table).await;

    let mut conn = pool.get().await.expect("conn");
    conn.batch_execute(&format!(
        "CREATE TABLE {table} (id SERIAL PRIMARY KEY, v INTEGER NOT NULL)"
    ))
    .await
    .expect("create");

    let mut tx = conn.transaction().await.expect("begin");
    tx.execute(&format!("INSERT INTO {table} (v) VALUES ($1)"), &[&1_i32])
        .await
        .expect("first insert");
    tx.savepoint("sp1").await.expect("savepoint");
    tx.execute(&format!("INSERT INTO {table} (v) VALUES ($1)"), &[&2_i32])
        .await
        .expect("second insert");
    tx.rollback_to("sp1").await.expect("rollback_to");
    tx.commit().await.expect("commit");

    let rows = conn
        .query(&format!("SELECT v FROM {table}"), &[])
        .await
        .expect("select");
    let vs: Vec<i32> = rows.iter().map(|r| r.get::<_, i32>(0)).collect();
    assert_eq!(vs, vec![1], "second insert should have been rolled back");

    drop_table(&pool, &table).await;
}

// =============================================================================
// Concurrency
// =============================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires running PostgreSQL via docker-compose"]
async fn e2e_concurrent_writes_via_pool() {
    if skip_unless_e2e().is_none() {
        return;
    }
    let pool = pool().await;
    let table = unique_table("concurrent");
    drop_table(&pool, &table).await;

    {
        let conn = pool.get().await.expect("conn");
        conn.batch_execute(&format!(
            "CREATE TABLE {table} (id SERIAL PRIMARY KEY, worker INTEGER NOT NULL, seq INTEGER NOT NULL)"
        ))
        .await
        .expect("create");
    }

    let total_workers: i32 = 8;
    let rows_per_worker: i32 = 25;
    let mut tasks = Vec::new();
    for w in 0..total_workers {
        let pool = pool.clone();
        let table = table.clone();
        tasks.push(tokio::spawn(async move {
            let conn = pool.get().await.expect("acquire");
            for s in 0..rows_per_worker {
                conn.execute(
                    &format!("INSERT INTO {table} (worker, seq) VALUES ($1, $2)"),
                    &[&w, &s],
                )
                .await
                .expect("insert");
            }
        }));
    }
    for t in tasks {
        t.await.expect("worker joined");
    }

    let conn = pool.get().await.expect("conn");
    let row = conn
        .query_one(&format!("SELECT COUNT(*)::BIGINT FROM {table}"), &[])
        .await
        .expect("count");
    let count: i64 = row.get(0);
    assert_eq!(count, (total_workers * rows_per_worker) as i64);

    drop_table(&pool, &table).await;
}

// =============================================================================
// Type round-trips
// =============================================================================

#[tokio::test]
#[ignore = "requires running PostgreSQL via docker-compose"]
async fn e2e_type_roundtrip_common_types() {
    if skip_unless_e2e().is_none() {
        return;
    }
    let pool = pool().await;
    let table = unique_table("types");
    drop_table(&pool, &table).await;

    let conn = pool.get().await.expect("conn");
    conn.batch_execute(&format!(
        "CREATE TABLE {table} (
            i32_col INTEGER NOT NULL,
            i64_col BIGINT NOT NULL,
            f64_col DOUBLE PRECISION NOT NULL,
            text_col TEXT NOT NULL,
            bool_col BOOLEAN NOT NULL,
            uuid_col UUID NOT NULL,
            json_col JSONB NOT NULL,
            null_col TEXT
        )"
    ))
    .await
    .expect("create");

    let uuid = uuid::Uuid::new_v4();
    let json = serde_json::json!({"k": "v", "n": 42});
    conn.execute(
        &format!(
            "INSERT INTO {table} (i32_col, i64_col, f64_col, text_col, bool_col, uuid_col, json_col, null_col) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"
        ),
        &[
            &1_i32,
            &(i64::MAX / 2),
            &std::f64::consts::PI,
            &"hello \"world\"",
            &true,
            &uuid,
            &json,
            &Option::<&str>::None,
        ],
    )
    .await
    .expect("insert");

    let row = conn
        .query_one(
            &format!("SELECT i32_col, i64_col, f64_col, text_col, bool_col, uuid_col, json_col, null_col FROM {table}"),
            &[],
        )
        .await
        .expect("select");
    assert_eq!(row.get::<_, i32>(0), 1);
    assert_eq!(row.get::<_, i64>(1), i64::MAX / 2);
    assert!((row.get::<_, f64>(2) - std::f64::consts::PI).abs() < 1e-12);
    assert_eq!(row.get::<_, &str>(3), "hello \"world\"");
    assert!(row.get::<_, bool>(4));
    assert_eq!(row.get::<_, uuid::Uuid>(5), uuid);
    assert_eq!(row.get::<_, serde_json::Value>(6), json);
    assert!(row.get::<_, Option<String>>(7).is_none());

    drop_table(&pool, &table).await;
}

// =============================================================================
// Pool health
// =============================================================================

#[tokio::test]
#[ignore = "requires running PostgreSQL via docker-compose"]
async fn e2e_pool_is_healthy() {
    if skip_unless_e2e().is_none() {
        return;
    }
    let pool = pool().await;
    assert!(pool.is_healthy().await, "pool should report healthy");
}

// =============================================================================
// Query engine typed decoding
// =============================================================================

#[tokio::test]
#[ignore = "requires running PostgreSQL via docker-compose"]
async fn e2e_query_many_typed_decodes_rows() {
    use prax_postgres::PgEngine;
    use prax_query::filter::FilterValue;
    use prax_query::row::{FromRow, RowError, RowRef};
    use prax_query::traits::{Model, QueryEngine};

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

    if skip_unless_e2e().is_none() {
        eprintln!("skipping: PRAX_E2E not set");
        return;
    }
    let pool = pool().await;
    let table = unique_table("crud_people");
    drop_table(&pool, &table).await;

    let conn = pool.get().await.expect("conn");
    conn.batch_execute(&format!(
        "CREATE TABLE {table} (id SERIAL PRIMARY KEY, email TEXT NOT NULL)"
    ))
    .await
    .expect("create table");

    conn.batch_execute(&format!(
        "INSERT INTO {table} (email) VALUES ('alice@example.com'), ('bob@example.com')"
    ))
    .await
    .expect("insert");

    let engine = PgEngine::new(pool.clone());
    let rows = engine
        .query_many::<Person>(
            &format!("SELECT id, email FROM {table} ORDER BY id"),
            Vec::<FilterValue>::new(),
        )
        .await
        .expect("query_many");

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].email, "alice@example.com");
    assert_eq!(rows[1].email, "bob@example.com");

    drop_table(&pool, &table).await;
}

// =============================================================================
// Row reference primitive reads
// =============================================================================

#[tokio::test]
#[ignore = "requires running PostgreSQL via docker-compose"]
async fn e2e_row_ref_primitive_reads() {
    use prax_postgres::row_ref::PgRow;
    use prax_query::row::RowRef;

    if skip_unless_e2e().is_none() {
        eprintln!("skipping: PRAX_E2E not set");
        return;
    }
    let pool = pool().await;
    let conn = pool.get().await.expect("conn");

    let raw_row = conn
        .query_one("SELECT 42::int4 AS n, 'hello'::text AS s", &[])
        .await
        .expect("query_one");

    let row = PgRow::from(raw_row);
    assert_eq!(row.get_i32("n").unwrap(), 42);
    assert_eq!(row.get_str("s").unwrap(), "hello");
}

/// Regression test for the `query_one` tail-materialization contract.
///
/// `query_one` is supposed to materialize "exactly one row", but the
/// driver implementations disagree on what happens when the SQL returns
/// 2+:
///
/// - `tokio-postgres::query_one` errors on 0 or 2+ rows (what this
///   engine uses) — this is the strict contract.
/// - `mysql_async::exec_first` silently takes the first row.
/// - `rusqlite::rows.next()` silently takes the first row.
/// - `tiberius::query_one` silently takes the first row.
///
/// This test locks down Postgres's strict "row count mismatch" error
/// for multi-row queries. If tokio-postgres ever changes to match the
/// other drivers (silently taking first), callers that relied on the
/// error to detect bad queries will regress — the failure should show
/// up here and force a CHANGELOG / migration update.
#[tokio::test]
#[ignore = "requires running PostgreSQL via docker-compose"]
async fn e2e_query_one_with_multiple_rows_behavior() {
    use prax_postgres::PgEngine;
    use prax_query::filter::FilterValue;
    use prax_query::row::{FromRow, RowError, RowRef};
    use prax_query::traits::{Model, QueryEngine};

    #[derive(Debug)]
    #[allow(dead_code)]
    struct Person {
        id: i32,
        email: String,
    }
    impl Model for Person {
        const MODEL_NAME: &'static str = "Person";
        const TABLE_NAME: &'static str = "e2e_pg_one_multi";
        const PRIMARY_KEY: &'static [&'static str] = &["id"];
        const COLUMNS: &'static [&'static str] = &["id", "email"];
    }
    impl FromRow for Person {
        fn from_row(r: &impl RowRef) -> Result<Self, RowError> {
            Ok(Person {
                id: r.get_i32("id")?,
                email: r.get_string("email")?,
            })
        }
    }

    if skip_unless_e2e().is_none() {
        return;
    }
    let pool = pool().await;
    let table = unique_table("one_multi");
    drop_table(&pool, &table).await;

    let conn = pool.get().await.expect("conn");
    conn.batch_execute(&format!(
        "CREATE TABLE {table} (id SERIAL PRIMARY KEY, email TEXT NOT NULL)"
    ))
    .await
    .expect("create");
    conn.batch_execute(&format!(
        "INSERT INTO {table} (email) VALUES ('a@x.com'), ('b@x.com')"
    ))
    .await
    .expect("insert");
    drop(conn);

    let engine = PgEngine::new(pool.clone());
    let result = engine
        .query_one::<Person>(
            &format!("SELECT id, email FROM {table} ORDER BY id"),
            Vec::<FilterValue>::new(),
        )
        .await;

    // Observed: tokio-postgres' `query_one` errors with "query returned
    // an unexpected number of rows" when the result set has 2+. The
    // engine routes that through QueryError::database — it's not a
    // NotFound because that's reserved for the zero-row case.
    match result {
        Ok(p) => {
            panic!(
                "Postgres is expected to error on multi-row query_one \
                 (tokio-postgres semantics). If the driver has changed \
                 to silently take the first row, update this test, the \
                 Unreleased CHANGELOG migration guide, and warn callers \
                 that relied on the strict 'exactly one' guarantee. Got: \
                 {p:?}"
            );
        }
        Err(e) => {
            // Sanity-check the error is about row count, not a
            // connection/deserialization/type issue that would mask a
            // real regression.
            let msg = e.to_string().to_lowercase();
            assert!(
                msg.contains("row") || msg.contains("unexpected"),
                "expected a row-count-related error from tokio-postgres \
                 on multi-row query_one, got: {e:?}"
            );
        }
    }

    drop_table(&pool, &table).await;
}
