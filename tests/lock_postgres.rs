//! End-to-end row-locking tests (`FOR UPDATE SKIP LOCKED` / `NOWAIT`)
//! against a live PostgreSQL server, using two concurrent
//! `PraxClient::transaction`s on separate pooled connections.
//!
//! Gated by `PRAX_E2E=1` and `#[ignore]`:
//!
//! ```sh
//! docker compose up -d postgres
//! PRAX_E2E=1 cargo test --test lock_postgres -- --include-ignored --nocapture
//! ```
//!
//! Transactions are coordinated with oneshot channels (no sleeps), and
//! each test namespaces its rows by `tag`.

#![cfg(test)]

mod common;

use std::collections::HashSet;
use std::time::Duration;

use common::e2e_url;
use prax_orm::{Model, PraxClient, client};
use prax_postgres::{PgEngine, PgPool, PgPoolBuilder};
use prax_query::advanced::RowLock;
use prax_query::error::QueryError;
use prax_query::raw::Sql;
use prax_query::types::OrderByField;
use tokio::sync::oneshot;

#[derive(Debug, Model)]
#[prax(table = "lock_pg_jobs")]
struct Job {
    #[prax(id, auto)]
    id: i32,
    tag: String,
}

client!(Job);

async fn build_pool(url: String) -> PgPool {
    PgPoolBuilder::new()
        .url(url)
        .max_connections(4)
        .connection_timeout(Duration::from_secs(10))
        .build()
        .await
        .expect("connect to postgres")
}

/// Build a client and ensure `lock_pg_jobs` exists (advisory-lock guarded
/// so concurrent test binaries don't race on catalog inserts).
async fn setup() -> Option<PraxClient<PgEngine>> {
    let url = e2e_url()?;
    let pool = build_pool(url).await;

    let conn = pool.get().await.expect("acquire conn for setup");
    conn.batch_execute(
        "BEGIN;
         SELECT pg_advisory_xact_lock(0x6c6f636b5f706a);
         CREATE TABLE IF NOT EXISTS lock_pg_jobs (
             id SERIAL PRIMARY KEY,
             tag TEXT NOT NULL
         );
         COMMIT",
    )
    .await
    .expect("create lock_pg_jobs");
    drop(conn);

    Some(PraxClient::new(PgEngine::new(pool)))
}

async fn seed(client: &PraxClient<PgEngine>, tag: &str, n: usize) -> Vec<i32> {
    client
        .execute_raw(Sql::new("DELETE FROM lock_pg_jobs WHERE tag = ").bind(tag.to_string()))
        .await
        .expect("pre-clean");
    let mut ids = Vec::new();
    for _ in 0..n {
        let j = client
            .job()
            .create()
            .set("tag", tag)
            .exec()
            .await
            .expect("seed row");
        ids.push(j.id);
    }
    ids
}

async fn cleanup(client: &PraxClient<PgEngine>, tag: &str) {
    client
        .execute_raw(Sql::new("DELETE FROM lock_pg_jobs WHERE tag = ").bind(tag.to_string()))
        .await
        .expect("cleanup");
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn skip_locked_claims_are_disjoint() {
    let Some(client) = setup().await else {
        eprintln!("skipping: PRAX_E2E not set");
        return;
    };
    let tag = format!("lock_skip_locked_{}", std::process::id());
    seed(&client, &tag, 10).await;

    let (locked_tx, locked_rx) = oneshot::channel::<()>();
    let (done_tx, done_rx) = oneshot::channel::<()>();

    // Tx B waits until A holds its locks, claims its own batch, then
    // tells A it may commit.
    let client_b = client.clone();
    let tag_b = tag.clone();
    let b = tokio::spawn(async move {
        locked_rx.await.expect("A signalled");
        client_b
            .transaction(move |tx| async move {
                let rows = tx
                    .job()
                    .find_many()
                    .r#where(job::tag::equals(tag_b))
                    .order_by(OrderByField::asc("id"))
                    .take(5)
                    .lock(RowLock::for_update().skip_locked())
                    .exec()
                    .await?;
                let ids: Vec<i32> = rows.iter().map(|j| j.id).collect();
                let _ = done_tx.send(());
                Ok(ids)
            })
            .await
    });

    let tag_a = tag.clone();
    let ids_a: Vec<i32> = client
        .transaction(move |tx| async move {
            let rows = tx
                .job()
                .find_many()
                .r#where(job::tag::equals(tag_a))
                .order_by(OrderByField::asc("id"))
                .take(5)
                .lock(RowLock::for_update().skip_locked())
                .exec()
                .await?;
            let ids: Vec<i32> = rows.iter().map(|j| j.id).collect();
            let _ = locked_tx.send(());
            // Hold the locks until B has finished its claim.
            tokio::time::timeout(Duration::from_secs(10), done_rx)
                .await
                .map_err(|_| QueryError::internal("timed out waiting for B"))?
                .map_err(|_| QueryError::internal("B dropped before finishing"))?;
            Ok(ids)
        })
        .await
        .expect("tx A");

    let ids_b = b.await.expect("join B").expect("tx B");

    assert_eq!(ids_a.len(), 5, "A should claim 5 rows");
    assert_eq!(ids_b.len(), 5, "B should claim the other 5 rows");
    let a: HashSet<_> = ids_a.iter().collect();
    assert!(
        ids_b.iter().all(|id| !a.contains(id)),
        "claims overlap: a={ids_a:?} b={ids_b:?}"
    );

    cleanup(&client, &tag).await;
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn nowait_on_locked_row_errors_immediately() {
    let Some(client) = setup().await else {
        eprintln!("skipping: PRAX_E2E not set");
        return;
    };
    let tag = format!("lock_nowait_{}", std::process::id());
    let ids = seed(&client, &tag, 1).await;
    let target = ids[0];

    let (locked_tx, locked_rx) = oneshot::channel::<()>();
    let (done_tx, done_rx) = oneshot::channel::<()>();

    // Tx A locks the row and holds it until B has reported.
    let client_a = client.clone();
    let a = tokio::spawn(async move {
        client_a
            .transaction(move |tx| async move {
                let rows = tx
                    .job()
                    .find_many()
                    .r#where(job::id::equals(target))
                    .for_update()
                    .exec()
                    .await?;
                assert_eq!(rows.len(), 1);
                let _ = locked_tx.send(());
                tokio::time::timeout(Duration::from_secs(10), done_rx)
                    .await
                    .map_err(|_| QueryError::internal("timed out waiting for B"))?
                    .map_err(|_| QueryError::internal("B dropped before finishing"))?;
                Ok(())
            })
            .await
    });

    locked_rx.await.expect("A holds the lock");

    let result = tokio::time::timeout(
        Duration::from_secs(2),
        client.transaction(move |tx| async move {
            tx.job()
                .find_unique()
                .r#where(job::id::equals(target))
                .lock(RowLock::for_update().nowait())
                .exec_optional()
                .await
        }),
    )
    .await;

    let _ = done_tx.send(());
    a.await.expect("join A").expect("tx A");

    let err = match result {
        Ok(Err(e)) => e,
        Ok(Ok(v)) => panic!("NOWAIT on a locked row must error, got {v:?}"),
        Err(_) => panic!("NOWAIT blocked past 2s instead of failing immediately"),
    };
    // SQLSTATE 55P03 (lock_not_available).
    assert_eq!(
        err.sqlstate(),
        Some("55P03"),
        "expected SQLSTATE 55P03, got: {err:?}"
    );

    cleanup(&client, &tag).await;
}
