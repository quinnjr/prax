//! E2E: configurable pool recycling (`PgRecycling`) and enforced
//! `max_lifetime`. Recycling runs at checkout, so each test drops a
//! connection and then checks the pool out again.
//!
//! Gated by `PRAX_E2E=1` and `POSTGRES_URL`.

#![cfg(test)]

use std::time::Duration;

mod common;

use common::e2e_url as url;
use prax_postgres::{PgPool, PgPoolBuilder, PgRecycling};

/// Per-test, per-process advisory-lock key (fits the 32-bit `objid` half).
fn lock_key(test: i64) -> i64 {
    1_000_000 + (std::process::id() as i64 % 100_000) * 10 + test
}

async fn pool(url: &str, recycling: PgRecycling, lifetime: Option<Duration>) -> PgPool {
    let mut b = PgPoolBuilder::new()
        .url(url)
        .max_connections(1)
        .min_connections(0)
        .connection_timeout(Duration::from_secs(10))
        .recycling(recycling);
    if let Some(d) = lifetime {
        b = b.max_lifetime(d);
    }
    b.build().await.expect("connect to postgres")
}

async fn advisory_count(observer: &PgPool, key: i64) -> i64 {
    let conn = observer.get().await.expect("observer conn");
    let row = conn
        .query_one(
            "SELECT count(*) FROM pg_locks WHERE locktype = 'advisory' AND objid::bigint = $1",
            &[&key],
        )
        .await
        .expect("count locks");
    row.get::<_, i64>(0)
}

async fn backend_pid(pool: &PgPool) -> i32 {
    let conn = pool.get().await.expect("conn");
    let row = conn
        .query_one("SELECT pg_backend_pid()", &[])
        .await
        .expect("pid");
    row.get::<_, i32>(0)
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn clean_recycling_releases_advisory_locks() {
    let Some(url) = url() else { return };
    let key = lock_key(1);
    let pool = pool(&url, PgRecycling::Clean, None).await;
    let observer = pool_observer(&url).await;

    let c = pool.get().await.expect("conn");
    c.execute("SELECT pg_advisory_lock($1)", &[&key])
        .await
        .expect("lock");
    drop(c);
    drop(pool.get().await.expect("recycled conn")); // checkout runs Clean

    let n = advisory_count(&observer, key).await;
    assert_eq!(n, 0, "Clean recycling must release session advisory locks");
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn fast_recycling_keeps_advisory_locks() {
    let Some(url) = url() else { return };
    let key = lock_key(2);
    let pool = pool(&url, PgRecycling::Fast, None).await;
    let observer = pool_observer(&url).await;

    let c = pool.get().await.expect("conn");
    c.execute("SELECT pg_advisory_lock($1)", &[&key])
        .await
        .expect("lock");
    drop(c);
    let c = pool.get().await.expect("recycled conn");

    let n = advisory_count(&observer, key).await;
    // Cleanup before asserting so a failure never leaks the lock.
    c.execute("SELECT pg_advisory_unlock_all()", &[])
        .await
        .expect("unlock");
    assert_eq!(n, 1, "Fast recycling leaves session state untouched");
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn max_lifetime_replaces_aged_connections() {
    let Some(url) = url() else { return };
    let pool = pool(&url, PgRecycling::Fast, Some(Duration::from_secs(1))).await;

    let pid1 = backend_pid(&pool).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let pid2 = backend_pid(&pool).await;
    assert_ne!(pid1, pid2, "aged connection must be replaced");
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn unexpired_max_lifetime_keeps_connection() {
    let Some(url) = url() else { return };
    // No explicit lifetime: relies on the builder's default max_lifetime
    // (30 minutes), far above the sleep below.
    let pool = pool(&url, PgRecycling::Fast, None).await;

    let pid1 = backend_pid(&pool).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let pid2 = backend_pid(&pool).await;
    assert_eq!(pid1, pid2, "control: young connection is reused");
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn clean_recycling_keeps_prepared_statements() {
    let Some(url) = url() else { return };
    let pool = pool(&url, PgRecycling::Clean, None).await;

    let (pid1, before) = {
        let c = pool.get().await.expect("conn");
        let n: i32 = 7;
        let row = c.query_one("SELECT $1::int4", &[&n]).await.expect("q1");
        assert_eq!(row.get::<_, i32>(0), 7);
        let pid = c
            .query_one("SELECT pg_backend_pid()", &[])
            .await
            .expect("pid")
            .get::<_, i32>(0);
        let cnt = c
            .query_one("SELECT count(*) FROM pg_prepared_statements", &[])
            .await
            .expect("count")
            .get::<_, i64>(0);
        (pid, cnt)
    };
    assert!(before > 0, "statement prepared on first checkout");

    // Next checkout runs Clean on the same backend.
    let c = pool.get().await.expect("recycled conn");
    let pid2 = c
        .query_one("SELECT pg_backend_pid()", &[])
        .await
        .expect("pid")
        .get::<_, i32>(0);
    assert_eq!(pid1, pid2, "same backend reused");
    let after = c
        .query_one("SELECT count(*) FROM pg_prepared_statements", &[])
        .await
        .expect("count")
        .get::<_, i64>(0);
    assert!(
        after > 0,
        "Clean recycling must preserve prepared statements"
    );
    let n: i32 = 8;
    let row = c.query_one("SELECT $1::int4", &[&n]).await.expect("q2");
    assert_eq!(row.get::<_, i32>(0), 8);
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn verified_recycling_replaces_terminated_connection() {
    let Some(url) = url() else { return };
    let pool = pool(&url, PgRecycling::Verified, None).await;
    let observer = pool_observer(&url).await;

    let pid1 = backend_pid(&pool).await;
    {
        let o = observer.get().await.expect("observer conn");
        o.query_one("SELECT pg_terminate_backend($1)", &[&pid1])
            .await
            .expect("terminate");
    }
    tokio::time::sleep(Duration::from_millis(300)).await;

    let pid2 = backend_pid(&pool).await;
    assert_ne!(pid1, pid2, "terminated connection must be replaced");
}

async fn pool_observer(url: &str) -> PgPool {
    PgPoolBuilder::new()
        .url(url)
        .max_connections(1)
        .min_connections(0)
        .connection_timeout(Duration::from_secs(10))
        .build()
        .await
        .expect("observer pool")
}
