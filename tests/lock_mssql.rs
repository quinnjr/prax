//! End-to-end row-locking tests (`UPDLOCK, READPAST` / `NOWAIT`) against a
//! live Microsoft SQL Server, using two concurrent
//! `PraxClient::transaction`s on separate pooled connections.
//!
//! Gated by `PRAX_E2E=1` and `#[ignore]`:
//!
//! ```sh
//! docker compose up -d mssql
//! PRAX_E2E=1 cargo test --all-features --test lock_mssql -- --include-ignored --nocapture
//! ```
//!
//! Transactions are coordinated with oneshot channels (no sleeps), and
//! each test namespaces its rows by a pid-suffixed `tag`.

#![cfg(test)]

mod common;

use std::collections::HashSet;
use std::time::Duration;

use common::e2e_enabled;
use prax_mssql::{MssqlConfig, MssqlEngine, MssqlPool};
use prax_orm::{Model, PraxClient, client};
use prax_query::advanced::RowLock;
use prax_query::error::QueryError;
use prax_query::types::OrderByField;
use tokio::sync::oneshot;

#[derive(Debug, Model)]
#[prax(table = "lock_mssql_jobs")]
struct Job {
    #[prax(id, auto)]
    id: i32,
    tag: String,
}

client!(Job);

fn mssql_url() -> Option<String> {
    if !e2e_enabled() {
        return None;
    }
    Some(std::env::var("MSSQL_URL").unwrap_or_else(|_| {
        // Bare `localhost,1433` (no `tcp:` prefix) per tests/client_mssql.rs.
        "server=localhost,1433;database=master;\
         user=sa;password=Prax_Test_Password123!;\
         trustservercertificate=true"
            .into()
    }))
}

async fn build_pool(url: String) -> MssqlPool {
    let config = MssqlConfig::from_connection_string(&url).expect("parse mssql url");
    MssqlPool::builder()
        .config(config)
        .max_connections(4)
        .connection_timeout(Duration::from_secs(15))
        .trust_cert(true)
        .build()
        .await
        .expect("connect to mssql")
}

async fn setup() -> Option<PraxClient<MssqlEngine>> {
    let url = mssql_url()?;
    let pool = build_pool(url).await;

    let mut conn = pool.get().await.expect("acquire conn for setup");
    // T-SQL has no `CREATE TABLE IF NOT EXISTS`, and the OBJECT_ID guard is
    // racy across parallel tests, so tolerate "already exists".
    let _ = conn
        .batch_execute(
            "IF OBJECT_ID('dbo.lock_mssql_jobs', 'U') IS NULL \
                 CREATE TABLE dbo.lock_mssql_jobs ( \
                     id INT IDENTITY(1,1) PRIMARY KEY, \
                     tag NVARCHAR(255) NOT NULL \
                 );",
        )
        .await;
    drop(conn);

    Some(PraxClient::new(MssqlEngine::new(pool)))
}

async fn seed(client: &PraxClient<MssqlEngine>, tag: &str, n: usize) -> Vec<i32> {
    cleanup(client, tag).await;
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

async fn cleanup(client: &PraxClient<MssqlEngine>, tag: &str) {
    client
        .job()
        .delete_many()
        .r#where(job::tag::equals(tag.to_string()))
        .exec()
        .await
        .expect("cleanup");
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live MSSQL_URL"]
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
#[ignore = "requires PRAX_E2E=1 and a live MSSQL_URL"]
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
    // Error 1222: "Lock request time out period exceeded" (NOWAIT is
    // `LOCK_TIMEOUT 0`). Match on the rendered error chain rather than
    // downcasting to the driver type.
    let source_dbg = std::error::Error::source(&err)
        .map(|s| format!("{s:?}"))
        .unwrap_or_default();
    assert!(
        source_dbg.contains("1222") || format!("{err:?}").contains("1222"),
        "expected MSSQL error 1222, got: {err} / {source_dbg}"
    );

    cleanup(&client, &tag).await;
}
