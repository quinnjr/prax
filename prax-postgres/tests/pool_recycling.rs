//! E2E: configurable pool recycling (`PgRecycling`) and enforced
//! `max_lifetime`. Recycling runs at checkout, so each test drops a
//! connection and then checks the pool out again.
//!
//! Gated by `PRAX_E2E=1` and `POSTGRES_URL`.

#![cfg(test)]

use std::time::Duration;

use prax_postgres::{PgPool, PgPoolBuilder, PgRecycling};

fn url() -> Option<String> {
    if std::env::var("PRAX_E2E").ok().as_deref() != Some("1") {
        return None;
    }
    Some(std::env::var("POSTGRES_URL").unwrap_or_else(|_| {
        "postgres://prax:prax_test_password@localhost:5432/prax_test".to_string()
    }))
}

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
