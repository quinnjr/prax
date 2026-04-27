//! End-to-end tests for prax-scylladb against a live ScyllaDB node.
//!
//! Gated by `PRAX_E2E=1` and requires `SCYLLA_URL`.
//!
//! ```sh
//! docker compose up -d scylladb
//! docker compose run --rm test-scylladb
//! ```
//!
//! Each test creates a uniquely named table inside the `prax_test`
//! keyspace so parallel runs don't collide. Scylla doesn't have
//! transactions, so we rely on LWT for atomicity assertions.

#![cfg(test)]

use std::sync::atomic::{AtomicU32, Ordering};

use prax_scylladb::ScyllaPool;
use scylla::frame::response::result::CqlValue;

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
    std::env::var("SCYLLA_URL").ok()
}

async fn pool() -> ScyllaPool {
    let url = skip_unless_e2e().expect("PRAX_E2E=1 and SCYLLA_URL required");
    let pool = ScyllaPool::from_url(&url)
        .await
        .expect("connect to scylladb");
    // Ensure the keyspace exists; idempotent and cheap.
    pool.session()
        .query_unpaged(
            "CREATE KEYSPACE IF NOT EXISTS prax_test
             WITH REPLICATION = { 'class': 'SimpleStrategy', 'replication_factor': 1 }",
            &[],
        )
        .await
        .expect("create keyspace");
    pool
}

async fn drop_table(pool: &ScyllaPool, table: &str) {
    let _ = pool
        .session()
        .query_unpaged(format!("DROP TABLE IF EXISTS prax_test.{table}"), &[])
        .await;
}

#[tokio::test]
#[ignore = "requires running ScyllaDB via docker-compose"]
async fn e2e_crud_roundtrip() {
    if skip_unless_e2e().is_none() {
        return;
    }
    let pool = pool().await;
    let table = unique_table("crud");
    drop_table(&pool, &table).await;

    pool.session()
        .query_unpaged(
            format!(
                "CREATE TABLE prax_test.{table} (
                    id UUID PRIMARY KEY,
                    name TEXT,
                    score INT
                )"
            ),
            &[],
        )
        .await
        .expect("create");

    let id = uuid::Uuid::new_v4();
    pool.query(
        &format!("INSERT INTO prax_test.{table} (id, name, score) VALUES (?, ?, ?)"),
        (id, "alice", 42_i32),
    )
    .await
    .expect("insert");

    let result = pool
        .query(
            &format!("SELECT name, score FROM prax_test.{table} WHERE id = ?"),
            (id,),
        )
        .await
        .expect("select");
    let rows = result.rows.expect("rows");
    assert_eq!(rows.len(), 1);
    let r = &rows[0];
    let name = r.columns[0].as_ref().and_then(|v| v.as_text()).cloned();
    let score = r.columns[1].as_ref().and_then(|v| v.as_int());
    assert_eq!(name.as_deref(), Some("alice"));
    assert_eq!(score, Some(42));

    pool.query(
        &format!("UPDATE prax_test.{table} SET score = ? WHERE id = ?"),
        (100_i32, id),
    )
    .await
    .expect("update");

    let result = pool
        .query(
            &format!("SELECT score FROM prax_test.{table} WHERE id = ?"),
            (id,),
        )
        .await
        .expect("select 2");
    let rows = result.rows.expect("rows");
    let score = rows[0].columns[0].as_ref().and_then(|v| v.as_int());
    assert_eq!(score, Some(100));

    pool.query(
        &format!("DELETE FROM prax_test.{table} WHERE id = ?"),
        (id,),
    )
    .await
    .expect("delete");

    let result = pool
        .query(
            &format!("SELECT id FROM prax_test.{table} WHERE id = ?"),
            (id,),
        )
        .await
        .expect("select 3");
    assert!(result.rows.unwrap_or_default().is_empty());

    drop_table(&pool, &table).await;
}

#[tokio::test]
#[ignore = "requires running ScyllaDB via docker-compose"]
async fn e2e_lightweight_transaction_insert_if_not_exists() {
    if skip_unless_e2e().is_none() {
        return;
    }
    let pool = pool().await;
    let table = unique_table("lwt");
    drop_table(&pool, &table).await;

    pool.session()
        .query_unpaged(
            format!(
                "CREATE TABLE prax_test.{table} (
                    id INT PRIMARY KEY,
                    v TEXT
                )"
            ),
            &[],
        )
        .await
        .expect("create");

    // First insert should apply.
    let result = pool
        .query(
            &format!("INSERT INTO prax_test.{table} (id, v) VALUES (?, ?) IF NOT EXISTS"),
            (1_i32, "first"),
        )
        .await
        .expect("lwt1");
    let rows = result.rows.expect("rows");
    let applied = rows[0].columns[0].as_ref().and_then(|v| match v {
        CqlValue::Boolean(b) => Some(*b),
        _ => None,
    });
    assert_eq!(applied, Some(true), "first LWT should apply");

    // Second insert with the same PK should NOT apply.
    let result = pool
        .query(
            &format!("INSERT INTO prax_test.{table} (id, v) VALUES (?, ?) IF NOT EXISTS"),
            (1_i32, "second"),
        )
        .await
        .expect("lwt2");
    let rows = result.rows.expect("rows");
    let applied = rows[0].columns[0].as_ref().and_then(|v| match v {
        CqlValue::Boolean(b) => Some(*b),
        _ => None,
    });
    assert_eq!(applied, Some(false), "conflicting LWT should not apply");

    // The stored value should still be "first".
    let result = pool
        .query(
            &format!("SELECT v FROM prax_test.{table} WHERE id = ?"),
            (1_i32,),
        )
        .await
        .expect("select");
    let rows = result.rows.expect("rows");
    let v = rows[0].columns[0].as_ref().and_then(|v| v.as_text()).cloned();
    assert_eq!(v.as_deref(), Some("first"));

    drop_table(&pool, &table).await;
}

#[tokio::test]
#[ignore = "requires running ScyllaDB via docker-compose"]
async fn e2e_logged_batch_atomicity() {
    if skip_unless_e2e().is_none() {
        return;
    }
    let pool = pool().await;
    let table = unique_table("batch");
    drop_table(&pool, &table).await;

    pool.session()
        .query_unpaged(
            format!(
                "CREATE TABLE prax_test.{table} (
                    id INT PRIMARY KEY,
                    v INT
                )"
            ),
            &[],
        )
        .await
        .expect("create");

    let engine = pool.engine();
    engine
        .batch()
        .logged()
        .add(&format!(
            "INSERT INTO prax_test.{table} (id, v) VALUES (1, 100)"
        ))
        .add(&format!(
            "INSERT INTO prax_test.{table} (id, v) VALUES (2, 200)"
        ))
        .add(&format!(
            "INSERT INTO prax_test.{table} (id, v) VALUES (3, 300)"
        ))
        .execute()
        .await
        .expect("batch execute");

    let result = pool
        .query(&format!("SELECT COUNT(*) FROM prax_test.{table}"), ())
        .await
        .expect("count");
    let rows = result.rows.expect("rows");
    let count = rows[0].columns[0].as_ref().and_then(|v| match v {
        CqlValue::BigInt(n) => Some(*n),
        _ => None,
    });
    assert_eq!(count, Some(3));

    drop_table(&pool, &table).await;
}

#[tokio::test]
#[ignore = "requires running ScyllaDB via docker-compose"]
async fn e2e_prepared_statement_cache_is_used() {
    if skip_unless_e2e().is_none() {
        return;
    }
    let pool = pool().await;
    let table = unique_table("prep");
    drop_table(&pool, &table).await;

    pool.session()
        .query_unpaged(
            format!(
                "CREATE TABLE prax_test.{table} (id INT PRIMARY KEY, v INT)"
            ),
            &[],
        )
        .await
        .expect("create");

    let stats_before = pool.stats();
    // execute() uses the prepared-statement cache; a second call with
    // the same CQL should not re-prepare.
    let cql = format!("INSERT INTO prax_test.{table} (id, v) VALUES (?, ?)");
    for i in 0..10_i32 {
        pool.execute(&cql, (i, i * 10)).await.expect("execute");
    }
    let stats_after = pool.stats();
    assert!(
        stats_after.cached_statements > stats_before.cached_statements,
        "expected at least one cached prepared statement"
    );

    drop_table(&pool, &table).await;
}

#[tokio::test]
#[ignore = "requires running ScyllaDB via docker-compose"]
async fn e2e_pool_is_healthy() {
    if skip_unless_e2e().is_none() {
        return;
    }
    let pool = pool().await;
    assert!(pool.is_healthy().await);
}
