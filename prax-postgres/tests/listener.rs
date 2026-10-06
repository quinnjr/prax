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
//! notifications. Tests that kill the listener's connection tag it with a
//! unique `application_name` and terminate only that backend.

#![cfg(test)]

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

mod common;

use common::e2e_url;
use futures::StreamExt;
use prax_postgres::{
    PgConfig, PgConnection, PgError, PgListener, PgListenerEvent, PgNotification, PgPool,
    PgPoolBuilder,
};
use tokio::io::AsyncReadExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

/// Upper bound for waiting on a notification that is expected to arrive.
const DELIVERY_TIMEOUT: Duration = Duration::from_secs(10);

static CHANNEL_COUNTER: AtomicU32 = AtomicU32::new(0);

fn unique_channel(prefix: &str) -> String {
    let n = CHANNEL_COUNTER.fetch_add(1, Ordering::SeqCst);
    let pid = std::process::id();
    format!("{prefix}_{pid}_{n}")
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

/// Listener config tagged with a unique `application_name`, so a test can
/// find (and terminate) exactly its own listener's backend.
fn tagged_config(url: &str) -> (PgConfig, String) {
    let app = unique_channel("prax_listener");
    let mut config = PgConfig::from_url(url).expect("parse POSTGRES_URL");
    config.application_name = Some(app.clone());
    (config, app)
}

/// Terminate the backends tagged `app` (only the calling test's listener);
/// returns how many were signalled.
async fn terminate_backend(pool: &PgPool, app: &str) -> usize {
    pool.get()
        .await
        .expect("pool conn")
        .query(
            "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE application_name = $1",
            &[&app],
        )
        .await
        .expect("pg_terminate_backend")
        .len()
}

/// Number of server backends tagged `app`.
async fn backend_count(pool: &PgPool, app: &str) -> i64 {
    pool.get()
        .await
        .expect("pool conn")
        .query_one(
            "SELECT count(*) FROM pg_stat_activity WHERE application_name = $1",
            &[&app],
        )
        .await
        .expect("count backends")
        .get(0)
}

/// A TCP proxy in front of Postgres that can fake three failures: a frozen
/// connection stays open and accepts the client's bytes but never answers
/// (a peer that ACKs but is stuck); while refusing, new connections are
/// closed at once (a server that is down); and while silent, new connections
/// are accepted but never answered, not even the startup handshake (a wedged
/// server or proxy).
struct TestProxy {
    port: u16,
    accepted: Arc<AtomicUsize>,
    refusing: Arc<AtomicBool>,
    silent: Arc<AtomicBool>,
    freezers: Arc<Mutex<Vec<oneshot::Sender<()>>>>,
    /// Frozen or silenced connections the client has since closed.
    hung_up: Arc<AtomicUsize>,
}

impl TestProxy {
    async fn start(host: String, port: u16) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind proxy");
        let proxy = Self {
            port: listener.local_addr().expect("proxy addr").port(),
            accepted: Arc::default(),
            refusing: Arc::default(),
            silent: Arc::default(),
            freezers: Arc::default(),
            hung_up: Arc::default(),
        };
        let accepted = Arc::clone(&proxy.accepted);
        let refusing = Arc::clone(&proxy.refusing);
        let silent = Arc::clone(&proxy.silent);
        let freezers = Arc::clone(&proxy.freezers);
        let hung_up = Arc::clone(&proxy.hung_up);
        tokio::spawn(async move {
            while let Ok((mut client, _)) = listener.accept().await {
                accepted.fetch_add(1, Ordering::SeqCst);
                if refusing.load(Ordering::SeqCst) {
                    continue;
                }
                if silent.load(Ordering::SeqCst) {
                    // Swallow the handshake and never answer, until the
                    // client hangs up.
                    let hung_up = Arc::clone(&hung_up);
                    tokio::spawn(async move {
                        let mut sink = [0u8; 1024];
                        while matches!(client.read(&mut sink).await, Ok(n) if n > 0) {}
                        hung_up.fetch_add(1, Ordering::SeqCst);
                    });
                    continue;
                }
                let Ok(mut server) = TcpStream::connect((host.as_str(), port)).await else {
                    continue;
                };
                let (freeze_tx, freeze_rx) = oneshot::channel::<()>();
                freezers.lock().unwrap().push(freeze_tx);
                let hung_up = Arc::clone(&hung_up);
                tokio::spawn(async move {
                    tokio::select! {
                        _ = tokio::io::copy_bidirectional(&mut client, &mut server) => {}
                        Ok(()) = freeze_rx => {
                            // Swallow whatever the client sends and never
                            // answer, until the client hangs up.
                            let mut sink = [0u8; 1024];
                            while matches!(client.read(&mut sink).await, Ok(n) if n > 0) {}
                            hung_up.fetch_add(1, Ordering::SeqCst);
                        }
                    }
                });
            }
        });
        proxy
    }

    /// `config` with its host and port pointed at this proxy.
    fn route(&self, mut config: PgConfig) -> PgConfig {
        config.host = "127.0.0.1".to_owned();
        config.port = self.port;
        config
    }

    /// Freeze every connection currently open through the proxy.
    fn freeze(&self) {
        for freeze in self.freezers.lock().unwrap().drain(..) {
            let _ = freeze.send(());
        }
    }

    /// Close new connections at once; open ones keep forwarding.
    fn refuse_new(&self) {
        self.refusing.store(true, Ordering::SeqCst);
    }

    /// Accept new connections but never answer them; open ones keep
    /// forwarding.
    fn silence_new(&self) {
        self.silent.store(true, Ordering::SeqCst);
    }

    /// Forward new connections to the server again.
    fn heal(&self) {
        self.refusing.store(false, Ordering::SeqCst);
        self.silent.store(false, Ordering::SeqCst);
    }

    /// Connections accepted so far (refused ones included).
    fn accepted(&self) -> usize {
        self.accepted.load(Ordering::SeqCst)
    }

    /// Frozen or silenced connections the client has closed.
    fn hung_up(&self) -> usize {
        self.hung_up.load(Ordering::SeqCst)
    }
}

/// Config for a LATIN1-encoded database (created on first use and kept, like
/// the other test databases), in which PostgreSQL rejects channel names it
/// cannot encode, e.g. `LISTEN "日本"` fails with SQLSTATE 22P05.
async fn latin1_config(url: &str, pool: &PgPool) -> PgConfig {
    const DATABASE: &str = "prax_listener_latin1";
    let conn = pool.get().await.expect("pool conn");
    if !database_exists(&conn, DATABASE).await {
        // A concurrent test run may win the race; existence is all we need.
        let _ = conn
            .batch_execute(&format!(
                "CREATE DATABASE {DATABASE} ENCODING 'LATIN1' LC_COLLATE 'C' LC_CTYPE 'C' \
                 TEMPLATE template0"
            ))
            .await;
        assert!(database_exists(&conn, DATABASE).await, "create {DATABASE}");
    }
    let mut config = PgConfig::from_url(url).expect("parse POSTGRES_URL");
    config.database = DATABASE.to_owned();
    config
}

async fn database_exists(conn: &PgConnection, name: &str) -> bool {
    conn.query_one(
        "SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname = $1)",
        &[&name],
    )
    .await
    .expect("check database")
    .get(0)
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

/// Require that notifications on `channels` are not delivered: send one on
/// each, then one on `sentinel` (which the listener is subscribed to), all
/// from `conn`. Notifications are delivered in commit order, so if the next
/// event is the sentinel's, none of the others is coming.
async fn assert_not_delivered(
    listener: &mut PgListener,
    conn: &PgConnection,
    channels: &[&str],
    sentinel: &str,
) {
    for channel in channels {
        conn.notify(channel, "must-not-arrive")
            .await
            .expect("notify");
    }
    conn.notify(sentinel, "sentinel")
        .await
        .expect("notify sentinel");
    let n = next_notification(listener).await;
    assert_eq!(
        (n.channel.as_str(), n.payload.as_str()),
        (sentinel, "sentinel"),
        "nothing on {channels:?} may arrive before the sentinel"
    );
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
    // Notifications are delivered in commit order: one committed by another
    // session while the transaction is open must come first.
    let other = pool.get().await.expect("second pool conn");

    let tx = conn.transaction().await.expect("begin");
    tx.notify(&channel, "a").await.expect("notify in tx");
    other
        .notify(&channel, "before-commit")
        .await
        .expect("notify");
    let n = next_notification(&mut listener).await;
    assert_eq!(
        n.payload, "before-commit",
        "notification must not be delivered before commit"
    );
    tx.commit().await.expect("commit");
    let n = next_notification(&mut listener).await;
    assert_eq!(n.channel, channel);
    assert_eq!(n.payload, "a");

    let tx = conn.transaction().await.expect("begin");
    tx.notify(&channel, "b").await.expect("notify in tx");
    tx.rollback().await.expect("rollback");
    other
        .notify(&channel, "after-rollback")
        .await
        .expect("notify");
    let n = next_notification(&mut listener).await;
    assert_eq!(
        n.payload, "after-rollback",
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
    let sentinel = unique_channel("u_sentinel");
    listener.listen(&channel).await.expect("listen");
    listener.listen(&sentinel).await.expect("listen sentinel");
    listener.unlisten(&channel).await.expect("unlisten");

    let conn = pool.get().await.expect("pool conn");
    assert_not_delivered(&mut listener, &conn, &[&channel], &sentinel).await;
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
    let sentinel = unique_channel("all_sentinel");
    listener.listen(&sentinel).await.expect("listen sentinel");
    assert_not_delivered(&mut listener, &conn, &[&a, &b], &sentinel).await;
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

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn reconnects_and_resubscribes_after_backend_termination() {
    let Some(url) = e2e_url() else { return };
    let pool = pool(&url).await;
    let (config, app) = tagged_config(&url);
    let mut listener = PgListener::connect(&config)
        .await
        .expect("connect listener");
    let channel = unique_channel("r");
    listener.listen(&channel).await.expect("listen");

    // `recv` is polled first, so it is already parked on the idle queue when
    // the backend dies: `Reconnected` must wake a blocked caller.
    let (event, terminated) = tokio::join!(
        tokio::time::timeout(DELIVERY_TIMEOUT, listener.recv()),
        async {
            tokio::time::sleep(Duration::from_millis(200)).await;
            terminate_backend(&pool, &app).await
        },
    );
    assert_eq!(
        terminated, 1,
        "exactly the listener's backend is terminated"
    );
    assert_eq!(
        event.expect("Reconnected within timeout").expect("recv ok"),
        PgListenerEvent::Reconnected
    );

    // The tracked channel was re-subscribed on the new connection.
    pool.get()
        .await
        .expect("pool conn")
        .notify(&channel, "after")
        .await
        .expect("notify");
    let n = next_notification(&mut listener).await;
    assert_eq!((n.channel, n.payload), (channel, "after".to_owned()));
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn overflow_reports_lagged() {
    let Some(url) = e2e_url() else { return };
    let pool = pool(&url).await;
    let config = PgConfig::from_url(&url).expect("parse POSTGRES_URL");
    let mut listener = PgListener::builder(&config)
        .buffer(4)
        .connect()
        .await
        .expect("connect listener");
    let channel = unique_channel("lag");
    listener.listen(&channel).await.expect("listen");

    let conn = pool.get().await.expect("pool conn");
    for i in 0..20 {
        conn.notify(&channel, &i.to_string()).await.expect("notify");
    }
    tokio::time::sleep(Duration::from_millis(500)).await;

    let mut events = Vec::new();
    while let Ok(event) = tokio::time::timeout(Duration::from_millis(500), listener.recv()).await {
        events.push(event.expect("recv ok"));
    }
    // Nobody read while they arrived: the first four filled the queue and
    // the rest were dropped. `Lagged` keeps its place in the stream, after
    // the four that were queued ahead of it.
    let queued: Vec<&str> = events
        .iter()
        .take(4)
        .map(|event| match event {
            PgListenerEvent::Notification(n) => n.payload.as_str(),
            other => panic!("expected the queued notifications first, got {other:?} in {events:?}"),
        })
        .collect();
    assert_eq!(queued, ["0", "1", "2", "3"], "{events:?}");
    assert!(
        matches!(events.get(4), Some(PgListenerEvent::Lagged { dropped, .. }) if *dropped > 0),
        "expected Lagged right after the queued notifications: {events:?}"
    );
    let notifications = events
        .iter()
        .filter(|event| matches!(event, PgListenerEvent::Notification(_)))
        .count();
    let dropped: u64 = events
        .iter()
        .filter_map(|event| match event {
            PgListenerEvent::Lagged { dropped, .. } => Some(*dropped),
            _ => None,
        })
        .sum();
    assert_eq!(dropped as usize + notifications, 20, "{events:?}");
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn reconnect_disabled_surfaces_error() {
    let Some(url) = e2e_url() else { return };
    let pool = pool(&url).await;
    let (config, app) = tagged_config(&url);
    let mut listener = PgListener::builder(&config)
        .reconnect(false)
        .connect()
        .await
        .expect("connect listener");
    listener
        .listen(&unique_channel("nr"))
        .await
        .expect("listen");

    let (result, terminated) = tokio::join!(
        tokio::time::timeout(DELIVERY_TIMEOUT, listener.recv()),
        async {
            tokio::time::sleep(Duration::from_millis(200)).await;
            terminate_backend(&pool, &app).await
        },
    );
    assert_eq!(
        terminated, 1,
        "exactly the listener's backend is terminated"
    );
    let err = result
        .expect("error within timeout")
        .expect_err("connection loss is terminal");
    // The server's own error (57P01 admin_shutdown) is surfaced, not just
    // "db error": structured when the driver reported it, described in the
    // message otherwise.
    let cause = match &err {
        PgError::Postgres(e) => {
            let db = e.as_db_error().unwrap_or_else(|| panic!("{err:?}"));
            format!("{} {}", db.code().code(), db.message())
        }
        PgError::Connection(message) => message.clone(),
        other => panic!("expected the connection's cause, got {other:?}"),
    };
    assert!(
        cause.contains("57P01") || cause.contains("terminating connection"),
        "the error carries its cause: {err:?}"
    );

    // Afterwards the listener reports itself closed.
    let err = tokio::time::timeout(DELIVERY_TIMEOUT, listener.recv())
        .await
        .expect("recv returns at once")
        .expect_err("listener closed");
    assert!(
        matches!(&err, PgError::Connection(m) if m == "listener closed"),
        "got {err:?}"
    );
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn dropping_listener_during_backoff_does_not_hang() {
    let Some(url) = e2e_url() else { return };
    tokio::time::timeout(Duration::from_secs(5), async {
        let pool = pool(&url).await;
        let (config, app) = tagged_config(&url);
        let proxy = TestProxy::start(config.host.clone(), config.port).await;
        let mut listener = PgListener::builder(&proxy.route(config))
            .backoff(Duration::from_millis(20), Duration::from_millis(50))
            .connect()
            .await
            .expect("connect listener through proxy");
        listener
            .listen(&unique_channel("drop"))
            .await
            .expect("listen");

        // The server now looks down: every reconnect attempt fails, so the
        // supervisor keeps backing off and retrying.
        proxy.refuse_new();
        assert_eq!(terminate_backend(&pool, &app).await, 1);
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(proxy.accepted() > 1, "the listener was retrying");

        drop(listener);
        // Let an attempt already in flight finish, then let the server
        // answer again and require silence: a supervisor that missed the
        // drop would retry every 25-50 ms, and now succeed.
        tokio::time::sleep(Duration::from_millis(100)).await;
        let attempts = proxy.accepted();
        proxy.heal();
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(
            proxy.accepted(),
            attempts,
            "no reconnect attempts after the listener was dropped"
        );
        assert_eq!(backend_count(&pool, &app).await, 0);
    })
    .await
    .expect("test finishes within 5 s");
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn listen_while_reconnecting_takes_effect_after_reconnect() {
    let Some(url) = e2e_url() else { return };
    let pool = pool(&url).await;
    let (config, app) = tagged_config(&url);
    let proxy = TestProxy::start(config.host.clone(), config.port).await;
    let mut listener = PgListener::builder(&proxy.route(config))
        .backoff(Duration::from_millis(50), Duration::from_millis(200))
        .connect()
        .await
        .expect("connect listener through proxy");
    let first = unique_channel("first");
    listener.listen(&first).await.expect("listen");

    // The server now looks down, so the listener stays disconnected until
    // the proxy heals; one refused attempt proves it noticed the loss.
    proxy.refuse_new();
    let before = proxy.accepted();
    assert_eq!(terminate_backend(&pool, &app).await, 1);
    assert!(
        wait_for_accepted(&proxy, before + 1, DELIVERY_TIMEOUT).await,
        "the listener tried to reconnect"
    );

    // Disconnected: the subscription is recorded without waiting for the
    // connection to come back.
    let late = unique_channel("late");
    tokio::time::timeout(Duration::from_secs(1), listener.listen(&late))
        .await
        .expect("listen does not wait for the reconnect")
        .expect("listen while reconnecting");

    proxy.heal();
    let event = tokio::time::timeout(DELIVERY_TIMEOUT, listener.recv())
        .await
        .expect("Reconnected within timeout")
        .expect("recv ok");
    assert_eq!(event, PgListenerEvent::Reconnected);

    let conn = pool.get().await.expect("pool conn");
    conn.notify(&first, "f").await.expect("notify first");
    conn.notify(&late, "l").await.expect("notify late");
    let a = next_notification(&mut listener).await;
    let b = next_notification(&mut listener).await;
    assert_eq!((a.channel, a.payload), (first, "f".to_owned()));
    assert_eq!((b.channel, b.payload), (late, "l".to_owned()));
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn ping_detects_unresponsive_connection_and_reconnects() {
    let Some(url) = e2e_url() else { return };
    let pool = pool(&url).await;
    let config = PgConfig::from_url(&url).expect("parse POSTGRES_URL");
    let proxy = TestProxy::start(config.host.clone(), config.port).await;

    // Long enough that a healthy ping on a loaded machine is never late.
    let ping = Duration::from_secs(1);
    let mut listener = PgListener::builder(&proxy.route(config))
        .ping_interval(ping)
        .backoff(Duration::from_millis(50), Duration::from_secs(1))
        .connect()
        .await
        .expect("connect listener through proxy");
    let channel = unique_channel("ping");
    listener.listen(&channel).await.expect("listen");

    // Healthy and idle: pings succeed and nothing is reported.
    assert!(
        tokio::time::timeout(ping * 3, listener.recv())
            .await
            .is_err(),
        "no event while the connection is healthy"
    );

    // No FIN, no RST: only the unanswered ping reveals the dead connection.
    proxy.freeze();
    let event = tokio::time::timeout(DELIVERY_TIMEOUT, listener.recv())
        .await
        .expect("Reconnected within timeout")
        .expect("recv ok");
    assert_eq!(event, PgListenerEvent::Reconnected);

    pool.get()
        .await
        .expect("pool conn")
        .notify(&channel, "alive")
        .await
        .expect("notify");
    let n = next_notification(&mut listener).await;
    assert_eq!((n.channel, n.payload), (channel, "alive".to_owned()));
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn command_on_unresponsive_connection_times_out_and_reconnects() {
    let Some(url) = e2e_url() else { return };
    let pool = pool(&url).await;
    let config = PgConfig::from_url(&url).expect("parse POSTGRES_URL");
    let proxy = TestProxy::start(config.host.clone(), config.port).await;

    let ping = Duration::from_secs(1);
    let mut listener = PgListener::builder(&proxy.route(config))
        .ping_interval(ping)
        .backoff(Duration::from_millis(50), Duration::from_secs(1))
        .connect()
        .await
        .expect("connect listener through proxy");
    let first = unique_channel("cmd");
    listener.listen(&first).await.expect("listen");

    // The first ping is only due one interval after connecting, so it is
    // this LISTEN that meets the frozen connection.
    proxy.freeze();
    let stuck = unique_channel("stuck");
    let err = tokio::time::timeout(ping * 3, listener.listen(&stuck))
        .await
        .expect("listen returns instead of hanging")
        .expect_err("LISTEN on a frozen connection fails");
    assert!(err.is_timeout(), "got {err:?}");

    // The timeout counted as connection loss.
    let event = tokio::time::timeout(DELIVERY_TIMEOUT, listener.recv())
        .await
        .expect("Reconnected within timeout")
        .expect("recv ok");
    assert_eq!(event, PgListenerEvent::Reconnected);

    // The tracked channel was re-subscribed and commands work again.
    listener
        .listen(&stuck)
        .await
        .expect("listen after reconnect");
    let conn = pool.get().await.expect("pool conn");
    conn.notify(&first, "a").await.expect("notify first");
    conn.notify(&stuck, "b").await.expect("notify stuck");
    let a = next_notification(&mut listener).await;
    let b = next_notification(&mut listener).await;
    assert_eq!((a.channel, a.payload), (first, "a".to_owned()));
    assert_eq!((b.channel, b.payload), (stuck, "b".to_owned()));
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn dropping_listener_mid_command_closes_its_connection() {
    let Some(url) = e2e_url() else { return };
    tokio::time::timeout(Duration::from_secs(5), async {
        let config = PgConfig::from_url(&url).expect("parse POSTGRES_URL");
        let proxy = TestProxy::start(config.host.clone(), config.port).await;
        // Default 30 s ping interval: the command's own timeout cannot fire
        // during this test.
        let mut listener = PgListener::connect(&proxy.route(config))
            .await
            .expect("connect listener through proxy");
        listener
            .listen(&unique_channel("mid"))
            .await
            .expect("listen");

        proxy.freeze();
        assert!(
            tokio::time::timeout(
                Duration::from_millis(200),
                listener.listen(&unique_channel("never"))
            )
            .await
            .is_err(),
            "LISTEN is stuck on the frozen connection"
        );
        drop(listener);

        // The supervisor must give up the in-flight query and close the
        // socket, not wait for an answer that never comes.
        while proxy.hung_up() == 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("connection closed within 5 s of dropping the listener");
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn reconnect_drops_channels_the_server_rejects() {
    let Some(url) = e2e_url() else { return };
    let pool = pool(&url).await;
    let latin1 = latin1_config(&url, &pool).await;
    let latin1_pool = PgPoolBuilder::new()
        .config(latin1.clone())
        .max_connections(1)
        .connection_timeout(Duration::from_secs(10))
        .build()
        .await
        .expect("connect to the LATIN1 database");

    let app = unique_channel("prax_listener");
    let mut config = latin1;
    config.application_name = Some(app.clone());
    let proxy = TestProxy::start(config.host.clone(), config.port).await;
    let mut listener = PgListener::builder(&proxy.route(config))
        .backoff(Duration::from_millis(50), Duration::from_millis(200))
        .connect()
        .await
        .expect("connect listener through proxy");
    let good = unique_channel("good");
    listener.listen(&good).await.expect("listen");

    // Recorded while disconnected, so the server only sees it on
    // reconnect, where LATIN1 cannot encode it.
    proxy.refuse_new();
    let before = proxy.accepted();
    assert_eq!(terminate_backend(&pool, &app).await, 1);
    assert!(
        wait_for_accepted(&proxy, before + 1, DELIVERY_TIMEOUT).await,
        "the listener tried to reconnect"
    );
    let bad = format!("{}_日本", unique_channel("bad"));
    listener
        .listen(&bad)
        .await
        .expect("recorded while reconnecting");

    // Without the per-channel fallback every attempt fails on `bad`. The
    // dropped channel is reported first, then the reconnect.
    proxy.heal();
    let event = tokio::time::timeout(DELIVERY_TIMEOUT, listener.recv())
        .await
        .expect("ChannelsDropped within timeout")
        .expect("recv ok");
    let PgListenerEvent::ChannelsDropped { channels, .. } = &event else {
        panic!("expected ChannelsDropped, got {event:?}");
    };
    assert_eq!(channels, &[bad]);
    let event = tokio::time::timeout(DELIVERY_TIMEOUT, listener.recv())
        .await
        .expect("Reconnected within timeout")
        .expect("recv ok");
    assert_eq!(event, PgListenerEvent::Reconnected);

    latin1_pool
        .get()
        .await
        .expect("pool conn")
        .notify(&good, "kept")
        .await
        .expect("notify");
    let n = next_notification(&mut listener).await;
    assert_eq!((n.channel, n.payload), (good, "kept".to_owned()));
}

/// Wait (up to `limit`) until `proxy` has accepted at least `count`
/// connections; returns whether it did.
async fn wait_for_accepted(proxy: &TestProxy, count: usize, limit: Duration) -> bool {
    tokio::time::timeout(limit, async {
        while proxy.accepted() < count {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .is_ok()
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn connect_fails_when_the_server_never_answers_the_handshake() {
    let Some(url) = e2e_url() else { return };
    let mut config = PgConfig::from_url(&url).expect("parse POSTGRES_URL");
    config.connect_timeout = Duration::from_secs(1);
    let proxy = TestProxy::start(config.host.clone(), config.port).await;
    proxy.silence_new();

    // The TCP connect succeeds at once, so `connect_timeout` alone never
    // fires; the handshake is bounded by max(connect_timeout, ping_interval)
    // = 1 s here.
    let started = tokio::time::Instant::now();
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        PgListener::builder(&proxy.route(config))
            .ping_interval(Duration::from_millis(500))
            .connect(),
    )
    .await
    .expect("connect returns instead of hanging");
    let err = result.expect_err("a handshake that is never answered fails");
    assert!(
        matches!(&err, PgError::Connection(m) if m.contains("handshake")),
        "got {err:?}"
    );
    // Bounded by connect_timeout (1 s), not the shorter ping_interval.
    let elapsed = started.elapsed();
    assert!(
        elapsed >= Duration::from_millis(900) && elapsed < Duration::from_secs(3),
        "failed after {elapsed:?}"
    );
    assert_eq!(proxy.accepted(), 1, "the attempt reached the server");
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn reconnect_keeps_retrying_while_the_server_never_answers() {
    let Some(url) = e2e_url() else { return };
    let pool = pool(&url).await;
    let (mut config, app) = tagged_config(&url);
    config.connect_timeout = Duration::from_secs(1);
    let proxy = TestProxy::start(config.host.clone(), config.port).await;
    let mut listener = PgListener::builder(&proxy.route(config))
        .ping_interval(Duration::from_secs(1))
        .backoff(Duration::from_millis(50), Duration::from_millis(200))
        .connect()
        .await
        .expect("connect listener through proxy");
    let first = unique_channel("silent");
    listener.listen(&first).await.expect("listen");

    // New connections now hang in the handshake, and the live one dies.
    proxy.silence_new();
    let before = proxy.accepted();
    assert_eq!(terminate_backend(&pool, &app).await, 1);

    // Each attempt gives up after the handshake bound (1 s) and the next
    // one follows: attempts keep coming instead of stalling on the first.
    assert!(
        wait_for_accepted(&proxy, before + 3, Duration::from_secs(10)).await,
        "only {} reconnect attempts",
        proxy.accepted() - before
    );

    // A command waits for at most the attempt in flight, then is recorded.
    let late = unique_channel("silent_late");
    tokio::time::timeout(Duration::from_secs(3), listener.listen(&late))
        .await
        .expect("listen returns while the server is silent")
        .expect("listen while reconnecting");

    proxy.heal();
    let event = tokio::time::timeout(DELIVERY_TIMEOUT, listener.recv())
        .await
        .expect("Reconnected within timeout")
        .expect("recv ok");
    assert_eq!(event, PgListenerEvent::Reconnected);

    let conn = pool.get().await.expect("pool conn");
    conn.notify(&first, "f").await.expect("notify first");
    conn.notify(&late, "l").await.expect("notify late");
    let a = next_notification(&mut listener).await;
    let b = next_notification(&mut listener).await;
    assert_eq!((a.channel, a.payload), (first, "f".to_owned()));
    assert_eq!((b.channel, b.payload), (late, "l".to_owned()));
}

/// The SQLSTATE of a server error.
fn sqlstate(err: &PgError) -> Option<&str> {
    match err {
        PgError::Postgres(e) => e.as_db_error().map(|db| db.code().code()),
        _ => None,
    }
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn listen_rejected_by_server_fails_only_the_call() {
    let Some(url) = e2e_url() else { return };
    let pool = pool(&url).await;
    let latin1 = latin1_config(&url, &pool).await;
    let latin1_pool = PgPoolBuilder::new()
        .config(latin1.clone())
        .max_connections(1)
        .connection_timeout(Duration::from_secs(10))
        .build()
        .await
        .expect("connect to the LATIN1 database");
    let app = unique_channel("prax_listener");
    let mut config = latin1;
    config.application_name = Some(app.clone());
    let mut listener = PgListener::connect(&config)
        .await
        .expect("connect listener");
    let good = unique_channel("good");
    listener.listen(&good).await.expect("listen");

    // LATIN1 cannot encode the name: the server refuses this LISTEN only.
    let bad = format!("{}_日本", unique_channel("bad"));
    let err = listener
        .listen(&bad)
        .await
        .expect_err("the server rejects the name");
    assert_eq!(sqlstate(&err), Some("22P05"), "got {err:?}");

    // The connection was not treated as lost: the next event is the good
    // channel's notification, not Reconnected, on the same backend.
    latin1_pool
        .get()
        .await
        .expect("pool conn")
        .notify(&good, "still-subscribed")
        .await
        .expect("notify");
    let n = next_notification(&mut listener).await;
    assert_eq!(
        (n.channel, n.payload),
        (good, "still-subscribed".to_owned())
    );
    assert_eq!(backend_count(&pool, &app).await, 1);
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn invalid_channel_names_fail_before_reaching_the_server() {
    let Some(url) = e2e_url() else { return };
    let pool = pool(&url).await;
    let (config, app) = tagged_config(&url);
    let mut listener = PgListener::connect(&config)
        .await
        .expect("connect listener");
    let sentinel = unique_channel("sentinel");
    listener.listen(&sentinel).await.expect("listen sentinel");
    let mut conn = pool.get().await.expect("pool conn");

    // The server would truncate the long name (with a notice) and reject
    // the others with its own errors: a `Query` error means the call never
    // got that far.
    let too_long = "a".repeat(64);
    for bad in ["", too_long.as_str(), "a\0b"] {
        let result = listener.listen(bad).await;
        assert!(
            matches!(result, Err(PgError::Query(_))),
            "listen {bad:?}: {result:?}"
        );
        let result = listener.unlisten(bad).await;
        assert!(
            matches!(result, Err(PgError::Query(_))),
            "unlisten {bad:?}: {result:?}"
        );
        let result = conn.notify(bad, "x").await;
        assert!(
            matches!(result, Err(PgError::Query(_))),
            "notify {bad:?}: {result:?}"
        );

        // A server error would abort the transaction; this one stays usable.
        let tx = conn.transaction().await.expect("begin");
        let result = tx.notify(bad, "x").await;
        assert!(
            matches!(result, Err(PgError::Query(_))),
            "tx.notify {bad:?}: {result:?}"
        );
        tx.notify(&sentinel, "tx-ok")
            .await
            .expect("the transaction is still usable");
        tx.commit().await.expect("commit");
        let n = next_notification(&mut listener).await;
        assert_eq!(
            (n.channel.as_str(), n.payload.as_str()),
            (sentinel.as_str(), "tx-ok")
        );
    }

    // Every name is validated before any is subscribed.
    let good = unique_channel("good");
    let result = listener.listen_all([good.as_str(), "a\0b"]).await;
    assert!(matches!(result, Err(PgError::Query(_))), "{result:?}");
    assert_not_delivered(&mut listener, &conn, &[&good], &sentinel).await;

    // None of it touched the listener's connection.
    assert_eq!(backend_count(&pool, &app).await, 1);
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn reconnect_backoff_keeps_growing_while_sessions_keep_dying() {
    let Some(url) = e2e_url() else { return };
    let pool = pool(&url).await;
    let (config, app) = tagged_config(&url);
    // No session lives for a whole ping interval here, so the backoff never
    // starts over: reconnect `k` waits 200 ms * 2^k, scaled by 0.5-1.
    let initial = Duration::from_millis(200);
    let mut listener = PgListener::builder(&config)
        .backoff(initial, Duration::from_secs(10))
        .connect()
        .await
        .expect("connect listener");

    let mut delays = Vec::new();
    for _ in 0..4 {
        let started = tokio::time::Instant::now();
        assert!(terminate_backend(&pool, &app).await >= 1);
        let event = tokio::time::timeout(DELIVERY_TIMEOUT, listener.recv())
            .await
            .expect("Reconnected within timeout")
            .expect("recv ok");
        assert_eq!(event, PgListenerEvent::Reconnected);
        delays.push(started.elapsed());
    }
    // Restarting from `initial` each time would keep every delay under
    // 200 ms (plus connect time).
    assert!(delays[2] >= 2 * initial, "{delays:?}");
    assert!(delays[3] >= 4 * initial, "{delays:?}");
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn reconnect_gives_up_after_max_reconnect_attempts() {
    let Some(url) = e2e_url() else { return };
    let pool = pool(&url).await;
    let (config, app) = tagged_config(&url);
    let proxy = TestProxy::start(config.host.clone(), config.port).await;
    let mut listener = PgListener::builder(&proxy.route(config))
        .backoff(Duration::from_millis(20), Duration::from_millis(50))
        .max_reconnect_attempts(3)
        .connect()
        .await
        .expect("connect listener through proxy");
    listener
        .listen(&unique_channel("max"))
        .await
        .expect("listen");

    proxy.refuse_new();
    let before = proxy.accepted();
    assert_eq!(terminate_backend(&pool, &app).await, 1);

    // After three refused attempts the listener stops with the last error.
    let err = tokio::time::timeout(DELIVERY_TIMEOUT, listener.recv())
        .await
        .expect("recv returns once the listener gives up")
        .expect_err("giving up is terminal");
    assert!(
        !matches!(&err, PgError::Connection(m) if m == "listener closed"),
        "the error says why: {err:?}"
    );
    proxy.heal();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(proxy.accepted(), before + 3, "exactly three attempts");
    assert_eq!(backend_count(&pool, &app).await, 0);

    let err = tokio::time::timeout(DELIVERY_TIMEOUT, listener.recv())
        .await
        .expect("recv returns at once")
        .expect_err("listener closed");
    assert!(
        matches!(&err, PgError::Connection(m) if m == "listener closed"),
        "got {err:?}"
    );
    let result = listener.listen(&unique_channel("after")).await;
    assert!(result.is_err(), "commands fail once stopped: {result:?}");
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn reconnect_gives_up_when_the_database_is_gone() {
    let Some(url) = e2e_url() else { return };
    let pool = pool(&url).await;
    let database = unique_channel("prax_listener_gone");
    let conn = pool.get().await.expect("pool conn");
    conn.batch_execute(&format!("CREATE DATABASE {database}"))
        .await
        .expect("create database");

    let (mut config, _app) = tagged_config(&url);
    config.database = database.clone();
    // Retrying forever is the default; a missing database still ends it.
    let mut listener = PgListener::builder(&config)
        .backoff(Duration::from_millis(20), Duration::from_millis(50))
        .connect()
        .await
        .expect("connect listener");
    listener
        .listen(&unique_channel("gone"))
        .await
        .expect("listen");

    // Terminates the listener's backend, then drops the database.
    conn.batch_execute(&format!("DROP DATABASE {database} WITH (FORCE)"))
        .await
        .expect("drop database");

    let err = tokio::time::timeout(DELIVERY_TIMEOUT, listener.recv())
        .await
        .expect("recv returns once the listener gives up")
        .expect_err("a missing database is terminal");
    assert_eq!(sqlstate(&err), Some("3D000"), "got {err:?}");
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn idle_session_timeout_does_not_close_the_listener() {
    let Some(url) = e2e_url() else { return };
    let pool = pool(&url).await;
    let (mut config, app) = tagged_config(&url);
    // A default that would end the listener's session after 300 ms idle.
    config
        .options
        .push(("idle_session_timeout".to_owned(), "300".to_owned()));
    let mut listener = PgListener::connect(&config)
        .await
        .expect("connect listener");
    let channel = unique_channel("idle");
    listener.listen(&channel).await.expect("listen");

    tokio::time::sleep(Duration::from_secs(1)).await;
    pool.get()
        .await
        .expect("pool conn")
        .notify(&channel, "still-here")
        .await
        .expect("notify");
    // A Reconnected here would mean the server closed the idle session.
    let n = next_notification(&mut listener).await;
    assert_eq!(n.payload, "still-here");
    assert_eq!(backend_count(&pool, &app).await, 1);
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn dropping_idle_listener_closes_its_backend() {
    let Some(url) = e2e_url() else { return };
    let pool = pool(&url).await;
    let (config, app) = tagged_config(&url);
    let mut listener = PgListener::connect(&config)
        .await
        .expect("connect listener");
    listener
        .listen(&unique_channel("idle_drop"))
        .await
        .expect("listen");
    assert_eq!(backend_count(&pool, &app).await, 1);

    drop(listener);
    let closed = tokio::time::timeout(Duration::from_secs(5), async {
        while backend_count(&pool, &app).await != 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(closed.is_ok(), "the backend is gone within 5 s of the drop");
}

#[tokio::test]
#[ignore = "requires PRAX_E2E=1 and a live POSTGRES_URL"]
async fn dropping_listener_mid_reconnect_attempt_stops_at_once() {
    let Some(url) = e2e_url() else { return };
    let pool = pool(&url).await;
    let (mut config, app) = tagged_config(&url);
    // The handshake bound (30 s) cannot end the stuck attempt in this test.
    config.connect_timeout = Duration::from_secs(30);
    let proxy = TestProxy::start(config.host.clone(), config.port).await;
    let mut listener = PgListener::builder(&proxy.route(config))
        .backoff(Duration::from_millis(20), Duration::from_millis(50))
        .connect()
        .await
        .expect("connect listener through proxy");
    listener
        .listen(&unique_channel("mid_attempt"))
        .await
        .expect("listen");

    // The next attempt hangs in its handshake.
    proxy.silence_new();
    let before = proxy.accepted();
    assert_eq!(terminate_backend(&pool, &app).await, 1);
    assert!(
        wait_for_accepted(&proxy, before + 1, DELIVERY_TIMEOUT).await,
        "the listener tried to reconnect"
    );

    let hung_up = proxy.hung_up();
    drop(listener);
    let abandoned = tokio::time::timeout(Duration::from_secs(2), async {
        while proxy.hung_up() == hung_up {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(
        abandoned.is_ok(),
        "the stuck attempt's socket is closed at once, not after the handshake bound"
    );

    // Nothing follows, even once the server answers again.
    let attempts = proxy.accepted();
    proxy.heal();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(proxy.accepted(), attempts, "no attempt after the drop");
    assert_eq!(backend_count(&pool, &app).await, 0);
}
