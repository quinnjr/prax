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
//!   the handle and is checked at the top of [`PgListener::recv`]. Raising a
//!   flag also wakes a `recv` that is parked on an empty queue.
//! - The supervisor watches the connection: when the driver reports its end,
//!   or when a `SELECT 1` ping (sent after a [`ping_interval`] without any
//!   notification) or a `LISTEN` / `UNLISTEN` command fails on the connection
//!   or goes unanswered for a [`ping_interval`], the connection is lost. It
//!   then either reconnects with exponential backoff,
//!   re-`LISTEN`s every tracked channel and reports
//!   [`PgListenerEvent::Reconnected`], or (with [`reconnect`] disabled)
//!   records the error and stops.
//!
//! Dropping the [`PgListener`] closes the command channel; the supervisor then
//! exits (also while waiting to reconnect) and drops the client, which ends
//! the driver task.
//!
//! [`ping_interval`]: PgListenerOptions::ping_interval
//! [`reconnect`]: PgListenerOptions::reconnect

use std::collections::BTreeSet;
use std::fmt;
use std::hash::BuildHasher;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use futures::{Stream, StreamExt};
use prax_query::sql::escape_identifier;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{Notify, mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::{self, Interval, MissedTickBehavior};
use tokio_postgres::{AsyncMessage, NoTls};
use tracing::{debug, info, warn};

use crate::config::PgConfig;
use crate::connector::{Connector, select_connector};
use crate::error::{PgError, PgResult};
use crate::pool::PgPool;

/// Longest channel name PostgreSQL accepts (`NAMEDATALEN - 1`), in bytes.
const MAX_CHANNEL_LEN: usize = 63;

/// PostgreSQL's notification payload limit in bytes; a payload must be
/// strictly shorter than this.
const PAYLOAD_LIMIT: usize = 8000;

/// Largest notification queue tokio's bounded channel supports (it panics
/// above this).
const MAX_BUFFER: usize = tokio::sync::Semaphore::MAX_PERMITS;

/// Longest accepted `ping_interval`. Far beyond any useful liveness check,
/// and it keeps tokio's `Instant + period` arithmetic (which panics on
/// overflow) safe.
const MAX_PING_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// Tuning for a [`PgListener`]; see [`PgListener::builder`].
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct PgListenerOptions {
    /// Reconnect after the connection is lost, re-issuing `LISTEN` for every
    /// tracked channel and reporting [`PgListenerEvent::Reconnected`]. When
    /// `false`, connection loss is terminal and [`PgListener::recv`] returns
    /// the error. Default: `true`.
    pub reconnect: bool,
    /// Delay before the first reconnect attempt; it doubles after every
    /// failed attempt, up to [`backoff_max`](Self::backoff_max). Each delay
    /// is scaled by a random factor between 0.5 and 1 (jitter). Must be
    /// non-zero and at most `backoff_max`. Default: 100 ms.
    pub backoff_initial: Duration,
    /// Upper bound on the reconnect delay. Default: 30 s.
    pub backoff_max: Duration,
    /// TCP keepalive idle time on the listener connection. Default: 60 s.
    pub keepalive_idle: Duration,
    /// Liveness check period. When no notification arrived during an
    /// interval, `SELECT 1` is sent; if it fails or is not answered within
    /// another interval, the connection is treated as lost. This detects
    /// half-open connections long before TCP keepalive does. The same bound
    /// applies to `LISTEN` / `UNLISTEN` commands: one not answered in time
    /// fails with a timeout error and the connection is treated as lost.
    /// Must be non-zero and at most one day. Default: 30 s.
    pub ping_interval: Duration,
    /// Capacity of the notification queue. When it is full, further
    /// notifications are dropped and reported as
    /// [`PgListenerEvent::Lagged`]. Must be at least 1 and at most
    /// `usize::MAX >> 3` (tokio's channel limit). Default: 1024.
    pub buffer: usize,
}

impl PgListenerOptions {
    /// Reject values the listener cannot run with (several would otherwise
    /// panic inside tokio).
    fn validate(&self) -> PgResult<()> {
        if self.buffer == 0 {
            return Err(PgError::config("PgListener buffer must be at least 1"));
        }
        if self.buffer > MAX_BUFFER {
            return Err(PgError::config(format!(
                "PgListener buffer must be at most {MAX_BUFFER}"
            )));
        }
        if self.ping_interval.is_zero() {
            return Err(PgError::config(
                "PgListener ping_interval must be greater than zero",
            ));
        }
        if self.ping_interval > MAX_PING_INTERVAL {
            return Err(PgError::config(
                "PgListener ping_interval must be at most one day",
            ));
        }
        if self.backoff_initial.is_zero() {
            return Err(PgError::config(
                "PgListener backoff_initial must be greater than zero",
            ));
        }
        if self.backoff_initial > self.backoff_max {
            return Err(PgError::config(
                "PgListener backoff_initial must not exceed backoff_max",
            ));
        }
        Ok(())
    }
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

    /// Reconnect backoff: the first delay (non-zero) and the upper bound (at
    /// least `initial`).
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

    /// Liveness check period (non-zero, at most one day); see
    /// [`PgListenerOptions::ping_interval`].
    pub fn ping_interval(mut self, interval: Duration) -> Self {
        self.options.ping_interval = interval;
        self
    }

    /// Capacity of the notification queue (at least 1, at most
    /// `usize::MAX >> 3`).
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
        options.validate()?;

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
            options,
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
    /// If the server does not answer within
    /// [`ping_interval`](PgListenerOptions::ping_interval), this fails with a
    /// timeout error and the listener treats the connection as lost.
    ///
    /// After a connection loss, while the listener is backing off before its
    /// next reconnect attempt, this (like [`unlisten`](Self::unlisten) and
    /// [`unlisten_all`](Self::unlisten_all)) returns without waiting for the
    /// server: the channel is recorded and subscribed when the connection is
    /// re-established. (During the attempt itself it waits for the attempt to
    /// finish.) If the server then rejects the channel name (for example one
    /// its encoding cannot represent), the channel is dropped and a warning is
    /// logged.
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
    /// notification. A `Reconnected` or `Lagged` raised while this is waiting
    /// is returned at once. Reconnects that happen before the caller gets to
    /// read them are reported as a single `Reconnected`.
    ///
    /// With [`PgListenerOptions::reconnect`] disabled, once the connection is
    /// lost and every buffered notification has been read, this returns the
    /// error that ended it (and a generic "listener closed" error on later
    /// calls).
    ///
    /// Cancel safe: if the future is dropped before completing, no event is
    /// lost.
    pub async fn recv(&mut self) -> PgResult<PgListenerEvent> {
        loop {
            if self.shared.reconnected.swap(false, Ordering::AcqRel) {
                return Ok(PgListenerEvent::Reconnected);
            }
            let dropped = self.shared.dropped.swap(0, Ordering::Relaxed);
            if dropped > 0 {
                return Ok(PgListenerEvent::Lagged { dropped });
            }
            tokio::select! {
                // Checked first so a raised flag outranks a queued
                // notification. `Notify` keeps a permit when nobody waits,
                // so a flag raised between the checks above and this await
                // still wakes us; a stale permit only costs one extra loop.
                biased;
                () = self.shared.wake.notified() => {}
                notification = self.events.recv() => {
                    return match notification {
                        Some(notification) => Ok(PgListenerEvent::Notification(notification)),
                        None => Err(self.shared.take_terminal().unwrap_or_else(listener_closed)),
                    };
                }
            }
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

/// Delay before reconnect attempt `attempt` (0-based):
/// `min(initial * 2^attempt, max)` scaled by `jitter`, which is clamped to
/// `[0.5, 1.0]` (NaN counts as 1.0). Never panics, whatever the inputs.
fn backoff_delay(attempt: u32, initial: Duration, max: Duration, jitter: f64) -> Duration {
    let capped = 2u32
        .checked_pow(attempt)
        .and_then(|factor| initial.checked_mul(factor))
        .map_or(max, |delay| delay.min(max));
    let jitter = if jitter.is_nan() {
        1.0
    } else {
        jitter.clamp(0.5, 1.0)
    };
    // Not `mul_f64`: rounding can push a delay near `Duration::MAX` past it,
    // which panics there.
    Duration::try_from_secs_f64(capped.as_secs_f64() * jitter)
        .map_or(capped, |delay| delay.min(capped))
}

/// A jitter factor in `[0.5, 1.0)` for [`backoff_delay`], taken from std's
/// randomly keyed hasher (every `RandomState` gets fresh keys), so no `rand`
/// dependency is needed.
fn jitter() -> f64 {
    let bits = std::collections::hash_map::RandomState::new().hash_one(());
    // The top 53 bits as a fraction in [0, 1).
    let unit = (bits >> 11) as f64 / (1u64 << 53) as f64;
    0.5 + unit / 2.0
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

impl Command {
    /// The statement that carries out this command on the server.
    fn sql(&self) -> String {
        match self {
            Self::Listen(channel, _) => listen_sql(channel),
            Self::Unlisten(channel, _) => unlisten_sql(channel),
            Self::UnlistenAll(_) => "UNLISTEN *".to_owned(),
        }
    }

    /// Report failure to the caller; the tracked channel set is unchanged.
    fn fail(self, error: PgError) {
        let (Self::Listen(_, reply) | Self::Unlisten(_, reply) | Self::UnlistenAll(reply)) = self;
        let _ = reply.send(Err(error));
    }
}

/// State shared between the handle, the supervisor and the driver that must
/// never be lost to a full notification queue.
#[derive(Debug, Default)]
struct Shared {
    /// Notifications discarded because the queue was full, not yet reported.
    dropped: AtomicU64,
    /// Set after a reconnect; reported once as `Reconnected`.
    reconnected: AtomicBool,
    /// Set by the driver whenever a notification (or another asynchronous
    /// message) arrives, cleared by the supervisor's ping tick: a tick that
    /// finds it unset sends a ping.
    traffic: AtomicBool,
    /// Wakes a `recv` parked on the empty queue when `reconnected` or
    /// `dropped` is raised.
    wake: Notify,
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

/// One live connection: the client commands run on, the driver task's
/// end-of-connection signal (`Some(error)` on a connection error, `None` on a
/// clean close), and the driver task itself.
struct Session {
    client: tokio_postgres::Client,
    closed: oneshot::Receiver<Option<tokio_postgres::Error>>,
    driver: DriverGuard,
}

/// Owns a connection's driver task and aborts it when dropped.
///
/// Dropping only the client is not enough to close an abandoned connection:
/// while a query is still in flight the driver keeps waiting for its answer,
/// which a stuck peer that still ACKs (a stopped backend, a wedged proxy)
/// never sends, and TCP keepalive never fires either. Aborting closes the
/// socket at once. An idle connection is [`detach`](Self::detach)ed instead,
/// so that it closes gracefully.
struct DriverGuard(Option<JoinHandle<()>>);

impl DriverGuard {
    fn abort(&self) {
        if let Some(driver) = &self.0 {
            driver.abort();
        }
    }

    /// Let the driver run to completion: once the client is dropped and no
    /// query is pending, it sends `Terminate` and ends.
    fn detach(mut self) {
        self.0 = None;
    }
}

impl Drop for DriverGuard {
    fn drop(&mut self) {
        self.abort();
    }
}

/// What talking to the server revealed about the connection.
enum Liveness {
    Alive,
    /// The connection is unusable, for the given reason.
    Lost(String),
    /// The handle was dropped while a query was in flight.
    HandleDropped,
}

/// How a query on the listener connection ended; see [`Supervisor::query`].
enum QueryOutcome<T> {
    Done(Result<T, tokio_postgres::Error>),
    /// No answer within `ping_interval`.
    TimedOut,
    /// The handle was dropped while waiting.
    HandleDropped,
}

/// A ping ticker whose first tick is one `period` from now. `period` is at
/// most [`MAX_PING_INTERVAL`] (see `validate`), so tokio's `now + period`
/// arithmetic cannot overflow.
fn ping_ticks(period: Duration) -> Interval {
    let mut ticks = time::interval_at(time::Instant::now() + period, period);
    // A tick missed while busy (a slow command) is taken late, not in a burst.
    ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
    ticks
}

/// The error a caller gets when the server did not answer within `limit`.
fn timeout_error(limit: Duration) -> PgError {
    PgError::Timeout(u64::try_from(limit.as_millis()).unwrap_or(u64::MAX))
}

/// Whether the server refused `LISTEN` because of the channel name itself:
/// SQLSTATE class 22 (data exception, e.g. a name the server encoding cannot
/// represent) or 42 (syntax error or access rule violation). Anything else,
/// such as a hot standby refusing `LISTEN` altogether, is a server-wide
/// condition that a later reconnect attempt may not hit.
fn rejects_channel(error: &PgError) -> bool {
    let PgError::Postgres(error) = error else {
        return false;
    };
    error.as_db_error().is_some_and(|db| {
        let code = db.code().code();
        code.starts_with("22") || code.starts_with("42")
    })
}

/// Background task owning the client and the tracked channel set.
struct Supervisor {
    pg_config: tokio_postgres::Config,
    connector: Connector,
    options: PgListenerOptions,
    events: mpsc::Sender<PgNotification>,
    shared: Arc<Shared>,
    /// Channels currently subscribed, to re-`LISTEN` after a reconnect.
    channels: BTreeSet<String>,
}

impl Supervisor {
    /// Open a connection and spawn its driver task.
    async fn connect(&self) -> PgResult<Session> {
        let (client, (closed, driver)) = match &self.connector {
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
        Ok(Session {
            client,
            closed,
            driver: DriverGuard(Some(driver)),
        })
    }

    /// Spawn the task that drives `connection`, forwarding notifications into
    /// the queue without ever blocking.
    fn spawn_driver<S, T>(
        &self,
        mut connection: tokio_postgres::Connection<S, T>,
    ) -> (
        oneshot::Receiver<Option<tokio_postgres::Error>>,
        JoinHandle<()>,
    )
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
        T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let events = self.events.clone();
        let shared = Arc::clone(&self.shared);
        let (closed_tx, closed_rx) = oneshot::channel();
        let driver = tokio::spawn(async move {
            let mut messages = futures::stream::poll_fn(move |cx| connection.poll_message(cx));
            let outcome = loop {
                let message = match messages.next().await {
                    Some(Ok(message)) => message,
                    Some(Err(e)) => break Some(e),
                    None => break None,
                };
                shared.traffic.store(true, Ordering::Relaxed);
                match message {
                    AsyncMessage::Notification(n) => {
                        let notification = PgNotification {
                            channel: n.channel().to_owned(),
                            payload: n.payload().to_owned(),
                            process_id: n.process_id(),
                        };
                        // `Closed` means the handle is gone; keep driving the
                        // connection until the supervisor drops the client.
                        if let Err(mpsc::error::TrySendError::Full(_)) =
                            events.try_send(notification)
                            && shared.dropped.fetch_add(1, Ordering::Relaxed) == 0
                        {
                            // First drop since the last report: wake `recv`.
                            shared.wake.notify_one();
                        }
                    }
                    AsyncMessage::Notice(notice) => debug!(
                        severity = %notice.severity(),
                        code = %notice.code().code(),
                        message = %notice.message(),
                        "notice on PostgreSQL listener connection"
                    ),
                    _ => {}
                }
            };
            // The supervisor may already be gone (listener dropped).
            let _ = closed_tx.send(outcome);
        });
        (closed_rx, driver)
    }

    /// Serve commands and watch the connection until the handle is dropped,
    /// or until the connection is lost while reconnecting is disabled.
    async fn run(mut self, mut session: Session, mut commands: mpsc::Receiver<Command>) {
        let mut ticks = ping_ticks(self.options.ping_interval);
        loop {
            let cause = tokio::select! {
                biased;
                outcome = &mut session.closed => match outcome {
                    Ok(Some(e)) => e.to_string(),
                    Ok(None) => "connection closed by the server".to_owned(),
                    Err(_) => "connection driver ended unexpectedly".to_owned(),
                },
                command = commands.recv() => match command {
                    Some(command) => match self.execute(&session.client, command).await {
                        Liveness::Alive => continue,
                        Liveness::Lost(cause) => cause,
                        // Dropping the session aborts the driver, abandoning
                        // the query in flight.
                        Liveness::HandleDropped => return,
                    },
                    // The handle was dropped while idle: dropping the client
                    // lets the driver close the connection gracefully.
                    None => {
                        session.driver.detach();
                        return;
                    }
                },
                _ = ticks.tick() => match self.check_alive(&session.client).await {
                    Liveness::Alive => continue,
                    Liveness::Lost(cause) => cause,
                    Liveness::HandleDropped => return,
                },
            };

            // The session is lost; from here on it is only replaced or
            // dropped, never polled again (`closed` may have completed).
            // Aborting closes a half-open socket now rather than when (or if)
            // TCP keepalive gives up; a finished driver ignores it.
            session.driver.abort();
            warn!(cause = %cause, "PostgreSQL listener connection lost");
            if !self.options.reconnect {
                self.shared.set_terminal(PgError::connection(format!(
                    "listener connection closed: {cause}"
                )));
                return;
            }
            let Some(new_session) = self.reconnect(&mut commands).await else {
                // The handle was dropped while reconnecting.
                return;
            };
            session = new_session;
            // Every tracked channel is subscribed again: report it.
            self.shared.reconnected.store(true, Ordering::Release);
            self.shared.wake.notify_one();
            ticks.reset();
        }
    }

    /// Await a query on the listener connection, bounded by `ping_interval`
    /// (a half-open socket never answers) and abandoned as soon as the
    /// handle is dropped.
    async fn query<T>(
        &self,
        query: impl Future<Output = Result<T, tokio_postgres::Error>>,
    ) -> QueryOutcome<T> {
        tokio::select! {
            biased;
            () = self.events.closed() => QueryOutcome::HandleDropped,
            result = time::timeout(self.options.ping_interval, query) => match result {
                Ok(result) => QueryOutcome::Done(result),
                Err(_) => QueryOutcome::TimedOut,
            },
        }
    }

    /// On a ping tick: unless a notification arrived since the last tick,
    /// check the connection with `SELECT 1`.
    async fn check_alive(&self, client: &tokio_postgres::Client) -> Liveness {
        if self.shared.traffic.swap(false, Ordering::Relaxed) {
            return Liveness::Alive;
        }
        match self.query(client.simple_query("SELECT 1")).await {
            QueryOutcome::Done(Ok(_)) => Liveness::Alive,
            QueryOutcome::Done(Err(e)) => Liveness::Lost(format!("ping failed: {e}")),
            QueryOutcome::TimedOut => Liveness::Lost(format!(
                "ping not answered within {:?}",
                self.options.ping_interval
            )),
            QueryOutcome::HandleDropped => Liveness::HandleDropped,
        }
    }

    /// Run one command on the live connection, bounded like a ping.
    ///
    /// Success updates the tracked channel set. A server-side error (the
    /// connection itself is fine) only goes back to the caller; a timeout or
    /// a connection error goes back to the caller *and* counts as connection
    /// loss.
    async fn execute(&mut self, client: &tokio_postgres::Client, command: Command) -> Liveness {
        let sql = command.sql();
        match self.query(client.batch_execute(&sql)).await {
            QueryOutcome::Done(Ok(())) => {
                self.track(command);
                Liveness::Alive
            }
            QueryOutcome::Done(Err(e)) if e.as_db_error().is_some() => {
                command.fail(e.into());
                Liveness::Alive
            }
            QueryOutcome::Done(Err(e)) => {
                let cause = format!("{sql} failed: {e}");
                command.fail(e.into());
                Liveness::Lost(cause)
            }
            QueryOutcome::TimedOut => {
                let limit = self.options.ping_interval;
                command.fail(timeout_error(limit));
                Liveness::Lost(format!("{sql} not answered within {limit:?}"))
            }
            QueryOutcome::HandleDropped => Liveness::HandleDropped,
        }
    }

    /// Re-establish the connection: before each attempt wait out its
    /// [`backoff_delay`] (meanwhile recording commands against the tracked
    /// channel set), then connect and re-`LISTEN` every tracked channel.
    /// Returns `None` as soon as the handle is dropped.
    ///
    /// The whole retry loop lives here, so the lost session's (possibly
    /// completed) `closed` receiver is never polled again.
    async fn reconnect(&mut self, commands: &mut mpsc::Receiver<Command>) -> Option<Session> {
        let mut attempt: u32 = 0;
        loop {
            let delay = backoff_delay(
                attempt,
                self.options.backoff_initial,
                self.options.backoff_max,
                jitter(),
            );
            let backoff = time::sleep(delay);
            tokio::pin!(backoff);
            loop {
                tokio::select! {
                    biased;
                    command = commands.recv() => match command {
                        Some(command) => self.track(command),
                        None => return None,
                    },
                    () = &mut backoff => break,
                }
            }

            // Dropping an unfinished attempt aborts its driver.
            let outcome = tokio::select! {
                biased;
                () = self.events.closed() => return None,
                outcome = self.open_subscribed() => outcome,
            };
            attempt = attempt.saturating_add(1);
            match outcome {
                Ok((session, rejected)) => {
                    for channel in &rejected {
                        self.channels.remove(channel);
                    }
                    info!(
                        attempts = attempt,
                        channels = self.channels.len(),
                        "PostgreSQL listener reconnected"
                    );
                    return Some(session);
                }
                Err(e) => warn!(
                    error = %e,
                    attempt,
                    "PostgreSQL listener reconnect attempt failed"
                ),
            }
        }
    }

    /// Connect and re-`LISTEN` every tracked channel, returning the session
    /// and the channels the server rejected by name (see
    /// [`rejects_channel`]).
    ///
    /// All channels go in one batch. A batch runs as one implicit
    /// transaction, so when the server rejects a channel name nothing is
    /// subscribed, and the channels are retried one by one to single out the
    /// rejected ones. Each statement is bounded by `ping_interval`; any other
    /// failure fails the attempt.
    async fn open_subscribed(&self) -> PgResult<(Session, Vec<String>)> {
        let session = self.connect().await?;
        let mut rejected = Vec::new();
        if self.channels.is_empty() {
            return Ok((session, rejected));
        }
        let batch = self
            .channels
            .iter()
            .map(|channel| listen_sql(channel))
            .collect::<Vec<_>>()
            .join("; ");
        match self.execute_bounded(&session.client, &batch).await {
            Ok(()) => {}
            Err(e) if rejects_channel(&e) => {
                for channel in &self.channels {
                    match self
                        .execute_bounded(&session.client, &listen_sql(channel))
                        .await
                    {
                        Ok(()) => {}
                        Err(e) if rejects_channel(&e) => {
                            warn!(
                                channel = %channel,
                                error = %e,
                                "PostgreSQL rejected LISTEN after reconnect; dropping the channel"
                            );
                            rejected.push(channel.clone());
                        }
                        Err(e) => return Err(e),
                    }
                }
            }
            Err(e) => return Err(e),
        }
        Ok((session, rejected))
    }

    /// Run `sql` on a fresh connection, bounded by `ping_interval`.
    async fn execute_bounded(&self, client: &tokio_postgres::Client, sql: &str) -> PgResult<()> {
        let limit = self.options.ping_interval;
        match time::timeout(limit, client.batch_execute(sql)).await {
            Ok(result) => Ok(result?),
            Err(_) => Err(timeout_error(limit)),
        }
    }

    /// Apply `command` to the tracked channel set and acknowledge it: after
    /// the server carried it out, or while disconnected (the server then
    /// catches up on reconnect).
    fn track(&mut self, command: Command) {
        let reply = match command {
            Command::Listen(channel, reply) => {
                self.channels.insert(channel);
                reply
            }
            Command::Unlisten(channel, reply) => {
                self.channels.remove(&channel);
                reply
            }
            Command::UnlistenAll(reply) => {
                self.channels.clear();
                reply
            }
        };
        let _ = reply.send(Ok(()));
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

    #[test]
    fn backoff_delay_doubles_from_initial_up_to_max() {
        let initial = Duration::from_millis(100);
        let max = Duration::from_secs(30);
        assert_eq!(
            backoff_delay(0, initial, max, 1.0),
            Duration::from_millis(100)
        );
        assert_eq!(
            backoff_delay(3, initial, max, 1.0),
            Duration::from_millis(800)
        );
        assert_eq!(backoff_delay(20, initial, max, 1.0), max);
    }

    #[test]
    fn backoff_delay_scales_by_jitter() {
        let initial = Duration::from_millis(100);
        let max = Duration::from_secs(30);
        assert_eq!(
            backoff_delay(3, initial, max, 0.5),
            Duration::from_millis(400)
        );
    }

    #[test]
    fn backoff_delay_saturates_and_clamps_jitter() {
        let initial = Duration::from_millis(100);
        let max = Duration::from_secs(30);
        // `2^attempt` and `initial * 2^attempt` overflow: the cap applies.
        assert_eq!(backoff_delay(u32::MAX, initial, max, 1.0), max);
        assert_eq!(
            backoff_delay(70, Duration::from_secs(1), Duration::MAX, 1.0),
            Duration::MAX
        );
        // Jitter outside [0.5, 1.0] (or NaN) never shortens the delay below
        // half or stretches it past the computed value.
        assert_eq!(
            backoff_delay(0, initial, max, 0.0),
            Duration::from_millis(50)
        );
        assert_eq!(backoff_delay(0, initial, max, 7.0), initial);
        assert_eq!(backoff_delay(0, initial, max, f64::NAN), initial);
    }

    #[test]
    fn jitter_is_in_range_and_varies() {
        let samples: Vec<f64> = (0..256).map(|_| jitter()).collect();
        assert!(
            samples.iter().all(|j| (0.5..1.0).contains(j)),
            "{samples:?}"
        );
        assert!(samples.iter().any(|j| *j != samples[0]), "{samples:?}");
    }

    /// One invalid option set per rule `PgListenerOptions::validate` enforces.
    fn invalid_options() -> Vec<(&'static str, PgListenerOptions)> {
        let base = PgListenerOptions::default();
        vec![
            (
                "zero buffer",
                PgListenerOptions {
                    buffer: 0,
                    ..base.clone()
                },
            ),
            (
                "buffer above tokio's channel limit",
                PgListenerOptions {
                    buffer: (usize::MAX >> 3) + 1,
                    ..base.clone()
                },
            ),
            (
                "zero ping_interval",
                PgListenerOptions {
                    ping_interval: Duration::ZERO,
                    ..base.clone()
                },
            ),
            (
                "ping_interval above one day",
                PgListenerOptions {
                    ping_interval: Duration::from_secs(24 * 60 * 60) + Duration::from_nanos(1),
                    ..base.clone()
                },
            ),
            (
                "zero backoff_initial",
                PgListenerOptions {
                    backoff_initial: Duration::ZERO,
                    ..base.clone()
                },
            ),
            (
                "backoff_initial above backoff_max",
                PgListenerOptions {
                    backoff_initial: Duration::from_secs(2),
                    backoff_max: Duration::from_secs(1),
                    ..base
                },
            ),
        ]
    }

    #[test]
    fn options_validate_accepts_defaults_and_boundaries() {
        assert!(PgListenerOptions::default().validate().is_ok());
        let boundaries = PgListenerOptions {
            buffer: usize::MAX >> 3,
            backoff_initial: Duration::from_secs(1),
            backoff_max: Duration::from_secs(1),
            ping_interval: Duration::from_nanos(1),
            ..PgListenerOptions::default()
        };
        assert!(boundaries.validate().is_ok());
        let longest_ping = PgListenerOptions {
            ping_interval: Duration::from_secs(24 * 60 * 60),
            ..PgListenerOptions::default()
        };
        assert!(longest_ping.validate().is_ok());
    }

    #[test]
    fn options_validate_rejects_unusable_values() {
        for (case, options) in invalid_options() {
            assert!(
                matches!(options.validate(), Err(PgError::Config(_))),
                "{case}"
            );
        }
    }

    #[tokio::test]
    async fn connect_rejects_invalid_options_without_panicking() {
        // Nothing listens on port 1: validation must fail before connecting.
        let config = PgConfig::from_url("postgres://prax@127.0.0.1:1/prax").unwrap();
        for (case, options) in invalid_options() {
            let result = PgListener::builder(&config)
                .options(options)
                .connect()
                .await;
            assert!(
                matches!(result, Err(PgError::Config(_))),
                "{case}: {result:?}"
            );
        }
        // The largest accepted buffer really builds the channel (tokio panics
        // one above it); only the unreachable server makes this fail.
        let result = PgListener::builder(&config)
            .buffer(usize::MAX >> 3)
            .connect()
            .await;
        assert!(
            matches!(result, Err(ref e) if !matches!(e, PgError::Config(_))),
            "{result:?}"
        );
    }
}
