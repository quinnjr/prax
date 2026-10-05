//! `LISTEN` / `NOTIFY` on a dedicated connection.
//!
//! A [`PgListener`] owns one **non-pooled** connection that subscribes to
//! notification channels. Notifications are buffered in a bounded queue and
//! read with [`PgListener::recv`] (or as a stream via
//! [`PgListener::into_stream`]).
//!
//! Sending is done from any ordinary connection or transaction with
//! [`PgConnection::notify`](crate::connection::PgConnection::notify) /
//! [`PgTransaction::notify`](crate::connection::PgTransaction::notify), which
//! run `SELECT pg_notify($1, $2)`. Inside a transaction the notification is
//! delivered on commit and discarded on rollback.
//!
//! ```rust,ignore
//! use prax_postgres::{PgListener, PgListenerEvent};
//!
//! let mut listener = PgListener::connect(&config).await?;
//! listener.listen("events").await?;
//! loop {
//!     match listener.recv().await? {
//!         PgListenerEvent::Notification(n) => println!("{}: {}", n.channel, n.payload),
//!         PgListenerEvent::Reconnected | PgListenerEvent::Lagged { .. } => {
//!             // Notifications may have been missed: resynchronise.
//!         }
//!         _ => {}
//!     }
//! }
//! ```
//!
//! # Architecture
//!
//! - A **supervisor task** owns the `tokio_postgres::Client` and the set of
//!   tracked channels, and executes `LISTEN` / `UNLISTEN` commands sent by the
//!   [`PgListener`] handle.
//! - A **driver task** per connection polls `Connection::poll_message`. It
//!   never blocks: each notification is `try_send`-ed into the bounded queue,
//!   and when the queue is full the notification is dropped and counted. The
//!   driver tells the supervisor when the connection ends.
//! - State that must never be lost to a full queue (the drop count, the
//!   reconnect flag, the terminal error) lives in atomics / a mutex shared with
//!   the handle and is checked at the top of [`PgListener::recv`].
//!
//! Dropping the [`PgListener`] closes the command channel; the supervisor then
//! exits and drops the client, which ends the driver task.

use std::collections::BTreeSet;
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use futures::{Stream, StreamExt};
use prax_query::sql::escape_identifier;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, oneshot};
use tokio_postgres::{AsyncMessage, NoTls};
use tracing::{debug, warn};

use crate::config::PgConfig;
use crate::connector::{Connector, select_connector};
use crate::error::{PgError, PgResult};
use crate::pool::PgPool;

/// Longest channel name PostgreSQL accepts (`NAMEDATALEN - 1`), in bytes.
const MAX_CHANNEL_LEN: usize = 63;

/// PostgreSQL's notification payload limit in bytes; a payload must be
/// strictly shorter than this.
const PAYLOAD_LIMIT: usize = 8000;

/// Tuning for a [`PgListener`]; see [`PgListener::builder`].
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct PgListenerOptions {
    /// Reconnect after the connection is lost, re-issuing `LISTEN` for every
    /// tracked channel and reporting [`PgListenerEvent::Reconnected`]. When
    /// `false`, connection loss is terminal and [`PgListener::recv`] returns
    /// the error. Default: `true`.
    pub reconnect: bool,
    /// First reconnect delay; doubles per failed attempt (with jitter).
    /// Default: 100 ms.
    pub backoff_initial: Duration,
    /// Upper bound on the reconnect delay. Default: 30 s.
    pub backoff_max: Duration,
    /// TCP keepalive idle time on the listener connection. Default: 60 s.
    pub keepalive_idle: Duration,
    /// Interval after which an idle connection is checked with `SELECT 1`.
    /// Default: 30 s.
    pub ping_interval: Duration,
    /// Capacity of the notification queue. When it is full, further
    /// notifications are dropped and reported as
    /// [`PgListenerEvent::Lagged`]. Must be at least 1. Default: 1024.
    pub buffer: usize,
}

impl Default for PgListenerOptions {
    fn default() -> Self {
        Self {
            reconnect: true,
            backoff_initial: Duration::from_millis(100),
            backoff_max: Duration::from_secs(30),
            keepalive_idle: Duration::from_secs(60),
            ping_interval: Duration::from_secs(30),
            buffer: 1024,
        }
    }
}

/// An event delivered by [`PgListener::recv`].
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PgListenerEvent {
    /// A notification on a channel this listener is subscribed to.
    Notification(PgNotification),
    /// The connection was lost and re-established; notifications sent in
    /// between were missed. Resynchronise any state derived from them.
    Reconnected,
    /// The notification queue overflowed and `dropped` notifications were
    /// discarded. Resynchronise any state derived from them.
    Lagged {
        /// Number of notifications discarded since the last report.
        dropped: u64,
    },
}

/// A notification received from PostgreSQL.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgNotification {
    /// The channel the notification was sent on, exactly as given to
    /// `NOTIFY` / `pg_notify` (case preserved).
    pub channel: String,
    /// The notification payload (empty when none was given).
    pub payload: String,
    /// Process id of the server backend that sent the notification.
    pub process_id: i32,
}

/// Builder for a [`PgListener`] with non-default [`PgListenerOptions`].
#[derive(Debug, Clone)]
pub struct PgListenerBuilder {
    config: PgConfig,
    options: PgListenerOptions,
}

impl PgListenerBuilder {
    /// Whether to reconnect after the connection is lost.
    pub fn reconnect(mut self, reconnect: bool) -> Self {
        self.options.reconnect = reconnect;
        self
    }

    /// Reconnect backoff: the first delay and the upper bound.
    pub fn backoff(mut self, initial: Duration, max: Duration) -> Self {
        self.options.backoff_initial = initial;
        self.options.backoff_max = max;
        self
    }

    /// TCP keepalive idle time on the listener connection.
    pub fn keepalive_idle(mut self, idle: Duration) -> Self {
        self.options.keepalive_idle = idle;
        self
    }

    /// Interval after which an idle connection is checked with `SELECT 1`.
    pub fn ping_interval(mut self, interval: Duration) -> Self {
        self.options.ping_interval = interval;
        self
    }

    /// Capacity of the notification queue (at least 1).
    pub fn buffer(mut self, buffer: usize) -> Self {
        self.options.buffer = buffer;
        self
    }

    /// Replace all options at once.
    pub fn options(mut self, options: PgListenerOptions) -> Self {
        self.options = options;
        self
    }

    /// Open the listener connection.
    ///
    /// Fails if the options are invalid, the TLS configuration is unusable
    /// (see the pool's `ssl_mode` rules), or the connection cannot be
    /// established.
    pub async fn connect(self) -> PgResult<PgListener> {
        let Self { config, options } = self;
        if options.buffer == 0 {
            return Err(PgError::config("PgListener buffer must be at least 1"));
        }

        let connector = select_connector(&config)?;
        let mut pg_config = config.to_pg_config();
        pg_config
            .keepalives(true)
            .keepalives_idle(options.keepalive_idle);

        let (events_tx, events) = mpsc::channel(options.buffer);
        let shared = Arc::new(Shared::default());
        let supervisor = Supervisor {
            pg_config,
            connector,
            events: events_tx,
            shared: Arc::clone(&shared),
            channels: BTreeSet::new(),
        };
        let session = supervisor.connect().await?;

        // Methods take `&mut self`, so at most one command is in flight.
        let (cmd_tx, cmd_rx) = mpsc::channel(1);
        tokio::spawn(supervisor.run(session, cmd_rx));

        debug!(
            host = %config.host,
            port = %config.port,
            database = %config.database,
            "PostgreSQL listener connected"
        );
        Ok(PgListener {
            cmd_tx,
            events,
            shared,
        })
    }
}

/// A dedicated connection receiving PostgreSQL `NOTIFY` messages.
///
/// The connection is not taken from a pool. Dropping the listener closes it
/// and stops its background tasks.
pub struct PgListener {
    cmd_tx: mpsc::Sender<Command>,
    events: mpsc::Receiver<PgNotification>,
    shared: Arc<Shared>,
}

impl fmt::Debug for PgListener {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PgListener").finish_non_exhaustive()
    }
}

impl PgListener {
    /// Connect with default [`PgListenerOptions`].
    pub async fn connect(config: &PgConfig) -> PgResult<Self> {
        Self::builder(config).connect().await
    }

    /// Connect a dedicated listener using `pool`'s configuration (including
    /// its TLS settings). The connection is not taken from the pool.
    pub async fn from_pool(pool: &PgPool) -> PgResult<Self> {
        Self::connect(pool.config()).await
    }

    /// Start building a listener with custom [`PgListenerOptions`].
    pub fn builder(config: &PgConfig) -> PgListenerBuilder {
        PgListenerBuilder {
            config: config.clone(),
            options: PgListenerOptions::default(),
        }
    }

    /// Subscribe to `channel`.
    ///
    /// The name is used verbatim (quoted as an identifier, case preserved), so
    /// it matches `pg_notify(channel, ..)` / [`PgConnection::notify`] with the
    /// same string. It must be non-empty, at most 63 bytes and free of NUL.
    /// Subscribing to a channel twice is a no-op.
    ///
    /// [`PgConnection::notify`]: crate::connection::PgConnection::notify
    pub async fn listen(&mut self, channel: &str) -> PgResult<()> {
        validate_channel(channel)?;
        self.command(|reply| Command::Listen(channel.to_owned(), reply))
            .await
    }

    /// Subscribe to every channel in `channels`.
    ///
    /// All names are validated before any is subscribed; on a server error
    /// the channels before the failing one stay subscribed.
    pub async fn listen_all<I, S>(&mut self, channels: I) -> PgResult<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let channels: Vec<String> = channels
            .into_iter()
            .map(|channel| channel.as_ref().to_owned())
            .collect();
        for channel in &channels {
            validate_channel(channel)?;
        }
        for channel in channels {
            self.command(|reply| Command::Listen(channel, reply))
                .await?;
        }
        Ok(())
    }

    /// Unsubscribe from `channel`. Unsubscribing from a channel that is not
    /// subscribed is a no-op.
    pub async fn unlisten(&mut self, channel: &str) -> PgResult<()> {
        validate_channel(channel)?;
        self.command(|reply| Command::Unlisten(channel.to_owned(), reply))
            .await
    }

    /// Unsubscribe from every channel (`UNLISTEN *`).
    pub async fn unlisten_all(&mut self) -> PgResult<()> {
        self.command(Command::UnlistenAll).await
    }

    /// Wait for the next event.
    ///
    /// Events are reported in priority order: [`PgListenerEvent::Reconnected`]
    /// first, then [`PgListenerEvent::Lagged`], then the next buffered
    /// notification. Once the connection is gone for good and every buffered
    /// notification has been read, this returns the error that ended it (and
    /// a generic "listener closed" error on later calls).
    ///
    /// Cancel safe: if the future is dropped before completing, no event is
    /// lost.
    pub async fn recv(&mut self) -> PgResult<PgListenerEvent> {
        if self.shared.reconnected.swap(false, Ordering::AcqRel) {
            return Ok(PgListenerEvent::Reconnected);
        }
        let dropped = self.shared.dropped.swap(0, Ordering::Relaxed);
        if dropped > 0 {
            return Ok(PgListenerEvent::Lagged { dropped });
        }
        match self.events.recv().await {
            Some(notification) => Ok(PgListenerEvent::Notification(notification)),
            None => Err(self.shared.take_terminal().unwrap_or_else(listener_closed)),
        }
    }

    /// Turn the listener into a stream of [`PgListener::recv`] results.
    ///
    /// The stream ends after yielding the error that closed the listener.
    pub fn into_stream(self) -> impl Stream<Item = PgResult<PgListenerEvent>> + Send {
        futures::stream::unfold(Some(self), |state| async move {
            let mut listener = state?;
            match listener.recv().await {
                Ok(event) => Some((Ok(event), Some(listener))),
                Err(e) => Some((Err(e), None)),
            }
        })
    }

    /// Send a command to the supervisor and wait for its result.
    async fn command(&self, command: impl FnOnce(Reply) -> Command) -> PgResult<()> {
        let (reply, response) = oneshot::channel();
        self.cmd_tx
            .send(command(reply))
            .await
            .map_err(|_| listener_closed())?;
        response.await.map_err(|_| listener_closed())?
    }
}

/// Error for operations on a listener whose connection is gone.
fn listener_closed() -> PgError {
    PgError::connection("listener closed")
}

/// Validate a `LISTEN` / `NOTIFY` channel name: non-empty, at most 63 bytes,
/// no NUL. Channel names are identifiers and cannot be bound as parameters
/// in `LISTEN`, so this runs before the name is quoted into SQL.
pub(crate) fn validate_channel(name: &str) -> PgResult<()> {
    if name.is_empty() {
        return Err(PgError::query(
            "notification channel name must not be empty",
        ));
    }
    if name.len() > MAX_CHANNEL_LEN {
        return Err(PgError::query(format!(
            "notification channel name is {} bytes; PostgreSQL allows at most {MAX_CHANNEL_LEN}",
            name.len()
        )));
    }
    if name.contains('\0') {
        return Err(PgError::query(
            "notification channel name must not contain NUL",
        ));
    }
    Ok(())
}

/// Validate a notification payload against PostgreSQL's limit (it must be
/// shorter than 8000 bytes).
pub(crate) fn validate_payload(payload: &str) -> PgResult<()> {
    if payload.len() >= PAYLOAD_LIMIT {
        return Err(PgError::query(format!(
            "notification payload is {} bytes; PostgreSQL requires fewer than {PAYLOAD_LIMIT}",
            payload.len()
        )));
    }
    Ok(())
}

/// `LISTEN` statement for a validated channel name.
pub(crate) fn listen_sql(channel: &str) -> String {
    format!("LISTEN {}", escape_identifier(channel))
}

/// `UNLISTEN` statement for a validated channel name.
pub(crate) fn unlisten_sql(channel: &str) -> String {
    format!("UNLISTEN {}", escape_identifier(channel))
}

/// Reply slot for a [`Command`].
type Reply = oneshot::Sender<PgResult<()>>;

/// A request from the [`PgListener`] handle to its supervisor.
#[derive(Debug)]
enum Command {
    Listen(String, Reply),
    Unlisten(String, Reply),
    UnlistenAll(Reply),
}

/// State shared between the handle, the supervisor and the driver that must
/// never be lost to a full notification queue.
#[derive(Debug, Default)]
struct Shared {
    /// Notifications discarded because the queue was full, not yet reported.
    dropped: AtomicU64,
    /// Set after a reconnect; reported once as `Reconnected`.
    reconnected: AtomicBool,
    /// Why the listener stopped, returned once the queue is drained.
    terminal: Mutex<Option<PgError>>,
}

impl Shared {
    fn set_terminal(&self, error: PgError) {
        *self.terminal.lock().unwrap_or_else(PoisonError::into_inner) = Some(error);
    }

    fn take_terminal(&self) -> Option<PgError> {
        self.terminal
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }
}

/// One live connection: the client commands run on, and the driver task's
/// end-of-connection signal (`Some(error)` on a connection error, `None` on a
/// clean close).
struct Session {
    client: tokio_postgres::Client,
    closed: oneshot::Receiver<Option<tokio_postgres::Error>>,
}

/// Background task owning the client and the tracked channel set.
struct Supervisor {
    pg_config: tokio_postgres::Config,
    connector: Connector,
    events: mpsc::Sender<PgNotification>,
    shared: Arc<Shared>,
    /// Channels currently subscribed, to re-`LISTEN` after a reconnect.
    channels: BTreeSet<String>,
}

impl Supervisor {
    /// Open a connection and spawn its driver task.
    async fn connect(&self) -> PgResult<Session> {
        let (client, closed) = match &self.connector {
            Connector::NoTls => {
                let (client, connection) = self.pg_config.connect(NoTls).await?;
                (client, self.spawn_driver(connection))
            }
            #[cfg(feature = "tls")]
            Connector::Rustls(tls) => {
                let (client, connection) = self.pg_config.connect(tls.clone()).await?;
                (client, self.spawn_driver(connection))
            }
        };
        Ok(Session { client, closed })
    }

    /// Spawn the task that drives `connection`, forwarding notifications into
    /// the queue without ever blocking.
    fn spawn_driver<S, T>(
        &self,
        mut connection: tokio_postgres::Connection<S, T>,
    ) -> oneshot::Receiver<Option<tokio_postgres::Error>>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
        T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let events = self.events.clone();
        let shared = Arc::clone(&self.shared);
        let (closed_tx, closed_rx) = oneshot::channel();
        tokio::spawn(async move {
            let mut messages = futures::stream::poll_fn(move |cx| connection.poll_message(cx));
            let outcome = loop {
                match messages.next().await {
                    Some(Ok(AsyncMessage::Notification(n))) => {
                        let notification = PgNotification {
                            channel: n.channel().to_owned(),
                            payload: n.payload().to_owned(),
                            process_id: n.process_id(),
                        };
                        // `Closed` means the handle is gone; keep driving the
                        // connection until the supervisor drops the client.
                        if let Err(mpsc::error::TrySendError::Full(_)) =
                            events.try_send(notification)
                        {
                            shared.dropped.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    Some(Ok(AsyncMessage::Notice(notice))) => debug!(
                        severity = %notice.severity(),
                        code = %notice.code().code(),
                        message = %notice.message(),
                        "notice on PostgreSQL listener connection"
                    ),
                    Some(Ok(_)) => {}
                    Some(Err(e)) => break Some(e),
                    None => break None,
                }
            };
            // The supervisor may already be gone (listener dropped).
            let _ = closed_tx.send(outcome);
        });
        closed_rx
    }

    /// Serve commands until the handle is dropped or the connection is lost.
    async fn run(mut self, mut session: Session, mut commands: mpsc::Receiver<Command>) {
        loop {
            tokio::select! {
                biased;
                outcome = &mut session.closed => {
                    match outcome {
                        Ok(Some(e)) => warn!(error = %e, "PostgreSQL listener connection lost"),
                        Ok(None) => warn!("PostgreSQL listener connection closed"),
                        Err(_) => warn!("PostgreSQL listener driver task ended unexpectedly"),
                    }
                    // Reconnecting is not implemented yet, so connection loss
                    // is terminal regardless of `PgListenerOptions::reconnect`.
                    self.shared
                        .set_terminal(PgError::connection("listener connection closed"));
                    return;
                }
                command = commands.recv() => match command {
                    Some(command) => self.execute(&session.client, command).await,
                    // The handle was dropped: dropping the client ends the
                    // driver task.
                    None => return,
                },
            }
        }
    }

    /// Run one command and keep the tracked channel set in step with the
    /// server.
    async fn execute(&mut self, client: &tokio_postgres::Client, command: Command) {
        match command {
            Command::Listen(channel, reply) => {
                let result = client.batch_execute(&listen_sql(&channel)).await;
                if result.is_ok() {
                    self.channels.insert(channel);
                }
                let _ = reply.send(result.map_err(PgError::from));
            }
            Command::Unlisten(channel, reply) => {
                let result = client.batch_execute(&unlisten_sql(&channel)).await;
                if result.is_ok() {
                    self.channels.remove(&channel);
                }
                let _ = reply.send(result.map_err(PgError::from));
            }
            Command::UnlistenAll(reply) => {
                let result = client.batch_execute("UNLISTEN *").await;
                if result.is_ok() {
                    self.channels.clear();
                }
                let _ = reply.send(result.map_err(PgError::from));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_channel_rejects_empty_too_long_and_nul() {
        assert!(matches!(validate_channel(""), Err(PgError::Query(_))));
        assert!(matches!(
            validate_channel(&"a".repeat(64)),
            Err(PgError::Query(_))
        ));
        assert!(matches!(validate_channel("a\0b"), Err(PgError::Query(_))));
    }

    #[test]
    fn validate_channel_accepts_max_length_and_quotes() {
        assert!(validate_channel(&"a".repeat(63)).is_ok());
        assert!(validate_channel("Mixed\"Case").is_ok());
    }

    #[test]
    fn validate_payload_enforces_8000_byte_limit() {
        assert!(validate_payload(&"x".repeat(7999)).is_ok());
        assert!(matches!(
            validate_payload(&"x".repeat(8000)),
            Err(PgError::Query(_))
        ));
    }

    #[test]
    fn listen_sql_quotes_identifier() {
        assert_eq!(listen_sql("Mixed\"Case"), "LISTEN \"Mixed\"\"Case\"");
        assert_eq!(listen_sql("events"), "LISTEN \"events\"");
    }

    #[test]
    fn unlisten_sql_quotes_identifier() {
        assert_eq!(unlisten_sql("Mixed\"Case"), "UNLISTEN \"Mixed\"\"Case\"");
    }

    #[test]
    fn options_default_matches_spec() {
        let o = PgListenerOptions::default();
        assert!(o.reconnect);
        assert_eq!(o.backoff_initial, Duration::from_millis(100));
        assert_eq!(o.backoff_max, Duration::from_secs(30));
        assert_eq!(o.keepalive_idle, Duration::from_secs(60));
        assert_eq!(o.ping_interval, Duration::from_secs(30));
        assert_eq!(o.buffer, 1024);
    }
}
