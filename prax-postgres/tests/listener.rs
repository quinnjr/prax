//! End-to-end tests for `PgListener` (LISTEN/NOTIFY) and the `notify`
//! helpers on `PgConnection` / `PgTransaction`.
//!
//! Gated by `PRAX_E2E=1`; `POSTGRES_URL` points at a reachable Postgres
//! (default `postgres://prax:prax_test_password@localhost:5432/prax_test`).
//! Tests are `#[ignore]`-marked so a plain `cargo test` skips them:
//!
//! ```sh
//! docker compose up -d postgres
//! PRAX_E2E=1 cargo test -p prax-postgres --all-features --test listener -- --include-ignored
//! ```
//!
//! Every test uses its own channel names (suffixed with the process id and a
//! counter) so concurrently running tests never see each other's
//! notifications.

#![cfg(test)]

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use futures::StreamExt;
use prax_postgres::{
    PgConfig, PgError, PgListener, PgListenerEvent, PgNotification, PgPool, PgPoolBuilder,
};

/// Upper bound for waiting on a notification that is expected to arrive.
const DELIVERY_TIMEOUT: Duration = Duration::from_secs(10);

static CHANNEL_COUNTER: AtomicU32 = AtomicU32::new(0);

fn unique_channel(prefix: &str) -> String {
    let n = CHANNEL_COUNTER.fetch_add(1, Ordering::SeqCst);
    let pid = std::process::id();
    format!("{prefix}_{pid}_{n}")
}

fn e2e_url() -> Option<String> {
    if std::env::var("PRAX_E2E").ok().as_deref() != Some("1") {
        return None;
    }
    Some(std::env::var("POSTGRES_URL").unwrap_or_else(|_| {
        "postgres://prax:prax_test_password@localhost:5432/prax_test".to_owned()
    }))
}

async fn pool(url: &str) -> PgPool {
    PgPoolBuilder::new()
        .url(url)
        .max_connections(2)
        .connection_timeout(Duration::from_secs(10))
        .build()
        .await
        .expect("connect to postgres")
}

async fn listener(url: &str) -> PgListener {
    let config = PgConfig::from_url(url).expect("parse POSTGRES_URL");
    PgListener::connect(&config)
        .await
        .expect("connect listener")
}

/// Receive the next event (bounded by [`DELIVERY_TIMEOUT`]) and require it to
/// be a notification.
async fn next_notification(listener: &mut PgListener) -> PgNotification {
    let event = tokio::time::timeout(DELIVERY_TIMEOUT, listener.recv())
        .await
        .expect("notification within timeout")
        .expect("recv ok");
    let PgListenerEvent::Notification(n) = event else {
        panic!("expected a notification, got {event:?}");
    };
    n
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn notifications_arrive_in_order() {
    let Some(url) = e2e_url() else { return };
    let pool = pool(&url).await;
    let mut listener = listener(&url).await;
    let channel = unique_channel("events");
    listener.listen(&channel).await.expect("listen");

    let conn = pool.get().await.expect("pool conn");
    for payload in ["1", "2", "3"] {
        conn.notify(&channel, payload).await.expect("notify");
    }

    let mut payloads = Vec::new();
    for _ in 0..3 {
        let n = next_notification(&mut listener).await;
        assert_eq!(n.channel, channel);
        assert!(n.process_id > 0);
        payloads.push(n.payload);
    }
    assert_eq!(payloads, ["1", "2", "3"]);
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn notify_in_transaction_delivers_only_on_commit() {
    let Some(url) = e2e_url() else { return };
    let pool = pool(&url).await;
    let mut listener = listener(&url).await;
    let channel = unique_channel("txc");
    listener.listen(&channel).await.expect("listen");

    let mut conn = pool.get().await.expect("pool conn");

    let tx = conn.transaction().await.expect("begin");
    tx.notify(&channel, "a").await.expect("notify in tx");
    assert!(
        tokio::time::timeout(Duration::from_millis(300), listener.recv())
            .await
            .is_err(),
        "notification must not be delivered before commit"
    );
    tx.commit().await.expect("commit");
    let n = next_notification(&mut listener).await;
    assert_eq!(n.channel, channel);
    assert_eq!(n.payload, "a");

    let tx = conn.transaction().await.expect("begin");
    tx.notify(&channel, "b").await.expect("notify in tx");
    tx.rollback().await.expect("rollback");
    assert!(
        tokio::time::timeout(Duration::from_millis(500), listener.recv())
            .await
            .is_err(),
        "a rolled-back notification must never be delivered"
    );
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn mixed_case_and_quoted_channel_names_match() {
    let Some(url) = e2e_url() else { return };
    let pool = pool(&url).await;
    let mut listener = listener(&url).await;
    let channel = unique_channel("Mixed\"Case");
    listener.listen(&channel).await.expect("listen");

    let conn = pool.get().await.expect("pool conn");
    conn.notify(&channel, "x").await.expect("notify");

    let n = next_notification(&mut listener).await;
    assert_eq!(n.channel, channel);
    assert_eq!(n.payload, "x");
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn unlisten_stops_delivery() {
    let Some(url) = e2e_url() else { return };
    let pool = pool(&url).await;
    let mut listener = listener(&url).await;
    let channel = unique_channel("u");
    listener.listen(&channel).await.expect("listen");
    listener.unlisten(&channel).await.expect("unlisten");

    let conn = pool.get().await.expect("pool conn");
    conn.notify(&channel, "ignored").await.expect("notify");

    assert!(
        tokio::time::timeout(Duration::from_millis(500), listener.recv())
            .await
            .is_err(),
        "no notification after UNLISTEN"
    );
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn listen_all_then_unlisten_all() {
    let Some(url) = e2e_url() else { return };
    let pool = pool(&url).await;
    let mut listener = listener(&url).await;
    let a = unique_channel("all_a");
    let b = unique_channel("all_b");
    listener.listen_all([&a, &b]).await.expect("listen_all");

    let conn = pool.get().await.expect("pool conn");
    conn.notify(&a, "to-a").await.expect("notify a");
    conn.notify(&b, "to-b").await.expect("notify b");
    let first = next_notification(&mut listener).await;
    let second = next_notification(&mut listener).await;
    assert_eq!((first.channel, first.payload), (a.clone(), "to-a".into()));
    assert_eq!((second.channel, second.payload), (b.clone(), "to-b".into()));

    listener.unlisten_all().await.expect("unlisten_all");
    conn.notify(&a, "ignored").await.expect("notify a");
    conn.notify(&b, "ignored").await.expect("notify b");
    assert!(
        tokio::time::timeout(Duration::from_millis(500), listener.recv())
            .await
            .is_err(),
        "no notification after UNLISTEN *"
    );
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn oversized_payload_rejected_client_side() {
    let Some(url) = e2e_url() else { return };
    let pool = pool(&url).await;
    let channel = unique_channel("c");
    let conn = pool.get().await.expect("pool conn");

    let err = conn
        .notify(&channel, &"x".repeat(8000))
        .await
        .expect_err("8000-byte payload must be rejected");
    assert!(matches!(err, PgError::Query(_)), "got {err:?}");

    conn.notify(&channel, &"x".repeat(7999))
        .await
        .expect("7999-byte payload is within the limit");
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn into_stream_yields_notifications() {
    let Some(url) = e2e_url() else { return };
    let pool = pool(&url).await;
    let mut listener = listener(&url).await;
    let channel = unique_channel("stream");
    listener.listen(&channel).await.expect("listen");

    let conn = pool.get().await.expect("pool conn");
    conn.notify(&channel, "s").await.expect("notify");

    let mut stream = std::pin::pin!(listener.into_stream());
    let item = tokio::time::timeout(DELIVERY_TIMEOUT, stream.next())
        .await
        .expect("stream item within timeout");
    let Some(Ok(PgListenerEvent::Notification(n))) = item else {
        panic!("expected Some(Ok(Notification(..))), got {item:?}");
    };
    assert_eq!(n.channel, channel);
    assert_eq!(n.payload, "s");
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn from_pool_listens_with_pool_config() {
    let Some(url) = e2e_url() else { return };
    let pool = pool(&url).await;
    let mut listener = PgListener::from_pool(&pool).await.expect("from_pool");
    let channel = unique_channel("frompool");
    listener.listen(&channel).await.expect("listen");

    pool.get()
        .await
        .expect("pool conn")
        .notify(&channel, "p")
        .await
        .expect("notify");

    let n = next_notification(&mut listener).await;
    assert_eq!(n.payload, "p");
}
