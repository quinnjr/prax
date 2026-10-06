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
//!         PgListenerEvent::ChannelsDropped { channels, .. } => {
//!             eprintln!("no longer subscribed to {channels:?}");
//!         }
//!         _ => {}
//!     }
//! }
//! ```
//!
//! # Security
//!
//! Any role that can connect to the database can `NOTIFY` any channel; there
//! are no per-channel privileges. Treat a notification's channel and payload
//! as untrusted input: use it as a hint to re-read the authoritative row
//! (with the reader's own permissions), or authenticate the payload (for
//! example an HMAC with a key only the senders hold) before acting on it.
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
//!   reconnect flag and the channels a reconnect dropped, the terminal error)
//!   lives in atomics / mutexes shared with the handle and is collected at the
//!   top of [`PgListener::recv`], which then returns it after the
//!   notifications queued ahead of it. Raising a flag also wakes a `recv`
//!   that is parked on an empty queue.
//! - The supervisor watches the connection: when the driver reports its end,
//!   or when a `SELECT 1` ping (sent after a [`ping_interval`] without any
//!   notification) or a `LISTEN` / `UNLISTEN` command fails on the connection
//!   or goes unanswered for a [`ping_interval`], the connection is lost. It
//!   then either reconnects with exponential backoff,
//!   re-`LISTEN`s every tracked channel and reports
//!   [`PgListenerEvent::Reconnected`], or (with [`reconnect`] disabled)
//!   records the error and stops. It also stops once reconnecting gives up
//!   (see [`max_reconnect_attempts`]). Every connection attempt, handshake
//!   included, is bounded by `PgConfig::connect_timeout` plus
//!   [`ping_interval`]; one that runs out counts as a failed attempt.
//! - Each new connection runs `SET idle_session_timeout = 0` (bounded by
//!   [`ping_interval`]; skipped before PostgreSQL 14), so a server default
//!   cannot close a listener that is idle while it waits.
//!
//! Dropping the [`PgListener`] closes the command channel; the supervisor then
//! exits (also while waiting to reconnect) and drops the client, which ends
//! the driver task.
//!
//! [`ping_interval`]: PgListenerOptions::ping_interval
//! [`reconnect`]: PgListenerOptions::reconnect
//! [`max_reconnect_attempts`]: PgListenerOptions::max_reconnect_attempts

mod backoff;
mod supervisor;

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use futures::Stream;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
#[cfg(test)]
use tokio::time;
use tracing::debug;

use crate::config::PgConfig;
use crate::connector::select_connector;
use crate::error::{PgError, PgResult};
use crate::notify::validate_channel;
use crate::pool::PgPool;
use supervisor::{Command, Reply, Shared, Supervisor};

/// Largest notification queue tokio's bounded channel supports (it panics
/// above this).
const MAX_BUFFER: usize = tokio::sync::Semaphore::MAX_PERMITS;

/// Shortest accepted `ping_interval`. Below this, the ping and command
/// bound would fail healthy connections on an ordinary network hiccup.
const MIN_PING_INTERVAL: Duration = Duration::from_millis(100);

/// Longest accepted `ping_interval`. Far beyond any useful liveness check,
/// and it keeps tokio's `Instant + period` arithmetic (which panics on
/// overflow) safe.
const MAX_PING_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// Bound on one whole connection attempt (TCP connect, TLS, startup
/// handshake). tokio-postgres applies `connect_timeout` to each address the
/// host resolves to, so the bound adds `ping_interval` to it: an address that
/// times out still leaves time to try the next one and finish the handshake.
fn handshake_bound(connect_timeout: Duration, ping_interval: Duration) -> Duration {
    connect_timeout.saturating_add(ping_interval)
}

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
    /// attempt, up to [`backoff_max`](Self::backoff_max), and only starts
    /// over once a re-established connection has stayed up for a
    /// [`ping_interval`](Self::ping_interval) (so a server that keeps
    /// dropping new connections is not hammered). Each delay is scaled by a
    /// random factor between 0.5 and 1 (jitter). Must be non-zero and at
    /// most `backoff_max`. Default: 100 ms.
    pub backoff_initial: Duration,
    /// Upper bound on the reconnect delay. Default: 30 s.
    pub backoff_max: Duration,
    /// TCP keepalive idle time on the listener connection. Counted in whole
    /// seconds; must be at least 1 s. Default: 60 s.
    pub keepalive_idle: Duration,
    /// Liveness check period. When no notification arrived during an
    /// interval, `SELECT 1` is sent; if it fails or is not answered within
    /// another interval, the connection is treated as lost. This detects
    /// half-open connections long before TCP keepalive does. The same bound
    /// applies to `LISTEN` / `UNLISTEN` commands: one not answered in time
    /// fails with a timeout error and the connection is treated as lost.
    /// Each connection attempt (TCP connect, TLS and the startup handshake)
    /// is bounded by [`PgConfig::connect_timeout`] plus this (the driver
    /// applies `connect_timeout` to each address the host resolves to, so
    /// an address that times out leaves this long for the next one and the
    /// handshake). Must be at least 100 ms and at most one day.
    /// Default: 30 s.
    pub ping_interval: Duration,
    /// Capacity of the notification queue. When it is full, further
    /// notifications are dropped and reported as
    /// [`PgListenerEvent::Lagged`]. Must be at least 1 and at most tokio's
    /// [`Semaphore::MAX_PERMITS`](tokio::sync::Semaphore::MAX_PERMITS), the
    /// largest bounded channel tokio supports. Default: 1024.
    pub buffer: usize,
    /// Give up after this many failed reconnect attempts in a row (counted
    /// per outage): the listener then stops, and [`PgListener::recv`]
    /// returns the last attempt's error once the queue is drained. `None`
    /// retries for as long as the listener exists. Must be at least 1;
    /// to never reconnect, disable [`reconnect`](Self::reconnect).
    ///
    /// Whatever this is set to, the listener also gives up after three
    /// attempts in a row that the server refuses with SQLSTATE class 28
    /// (invalid authorization: a wrong password, a `pg_hba.conf` rejection)
    /// or 3D000 (the database does not exist). Retrying cannot fix those,
    /// and retrying bad credentials forever can lock the role or trip
    /// intrusion detection. Default: `None`.
    pub max_reconnect_attempts: Option<u32>,
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
        if self.ping_interval < MIN_PING_INTERVAL {
            return Err(PgError::config(format!(
                "PgListener ping_interval must be at least {MIN_PING_INTERVAL:?}"
            )));
        }
        // TCP keepalive counts whole seconds; less than one becomes zero,
        // which the OS rejects (EINVAL) when the socket is opened.
        if self.keepalive_idle < Duration::from_secs(1) {
            return Err(PgError::config(
                "PgListener keepalive_idle must be at least one second",
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
        if self.max_reconnect_attempts == Some(0) {
            return Err(PgError::config(
                "PgListener max_reconnect_attempts must be at least 1 (disable reconnect instead)",
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
            max_reconnect_attempts: None,
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
    #[non_exhaustive]
    Lagged {
        /// Number of notifications discarded since the last report.
        dropped: u64,
    },
    /// After a reconnect, the server refused to subscribe these channels
    /// again (for example names its encoding cannot represent); they are no
    /// longer tracked and receive nothing until listened to again. Reported
    /// right before the [`Reconnected`](Self::Reconnected) of that
    /// reconnect.
    #[non_exhaustive]
    ChannelsDropped {
        /// The channels that were dropped, in no particular order.
        channels: Vec<String>,
    },
}

/// A notification received from PostgreSQL.
///
/// # Security
///
/// Any role that can connect to the database can send a notification on any
/// channel: [`channel`](Self::channel) and [`payload`](Self::payload) are
/// untrusted input (and [`process_id`](Self::process_id) only names the
/// sending backend). Re-read the authoritative row, or verify an HMAC in
/// the payload, before acting on it; see [`PgListener`](PgListener#security).
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

impl PgNotification {
    /// A notification as the server would deliver it (for tests and fakes of
    /// code that consumes [`PgListenerEvent`]s).
    pub fn new(channel: impl Into<String>, payload: impl Into<String>, process_id: i32) -> Self {
        Self {
            channel: channel.into(),
            payload: payload.into(),
            process_id,
        }
    }
}

/// Builder for a [`PgListener`] with non-default [`PgListenerOptions`].
#[derive(Clone)]
pub struct PgListenerBuilder {
    config: PgConfig,
    options: PgListenerOptions,
}

/// Shows the options only: the config holds the password.
impl fmt::Debug for PgListenerBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PgListenerBuilder")
            .field("options", &self.options)
            .finish_non_exhaustive()
    }
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

    /// TCP keepalive idle time on the listener connection (at least 1 s).
    pub fn keepalive_idle(mut self, idle: Duration) -> Self {
        self.options.keepalive_idle = idle;
        self
    }

    /// Liveness check period (at least 100 ms, at most one day); see
    /// [`PgListenerOptions::ping_interval`].
    pub fn ping_interval(mut self, interval: Duration) -> Self {
        self.options.ping_interval = interval;
        self
    }

    /// Capacity of the notification queue (at least 1, at most tokio's
    /// `Semaphore::MAX_PERMITS`); see [`PgListenerOptions::buffer`].
    pub fn buffer(mut self, buffer: usize) -> Self {
        self.options.buffer = buffer;
        self
    }

    /// Give up after `attempts` failed reconnect attempts in a row (`None`:
    /// never); see [`PgListenerOptions::max_reconnect_attempts`].
    pub fn max_reconnect_attempts(mut self, attempts: impl Into<Option<u32>>) -> Self {
        self.options.max_reconnect_attempts = attempts.into();
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
    /// established. The attempt, including the TLS and startup handshake,
    /// is bounded by [`PgConfig::connect_timeout`] plus
    /// [`PgListenerOptions::ping_interval`]; past that it fails with a
    /// connection error.
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
        let supervisor = Supervisor::new(
            pg_config,
            connector,
            handshake_bound(config.connect_timeout, options.ping_interval),
            options,
            events_tx,
            Arc::clone(&shared),
        );
        let session = supervisor.connect().await?;

        // Methods take `&mut self`, so at most one command is in flight.
        let (cmd_tx, cmd_rx) = mpsc::channel(1);
        let supervisor = tokio::spawn(supervisor.run(session, cmd_rx));

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
            pending: Pending::default(),
            supervisor: Some(supervisor),
        })
    }
}

/// A dedicated connection receiving PostgreSQL `NOTIFY` messages.
///
/// The connection is not taken from a pool. Dropping the listener closes it
/// and stops its background tasks.
///
/// # Connect directly to PostgreSQL
///
/// A `LISTEN` subscription belongs to one server session, so the listener
/// must connect to PostgreSQL itself, not through a transaction-mode pooler
/// (such as PgBouncer with `pool_mode = transaction`) or a multiplexing
/// proxy such as Amazon RDS Proxy, which do not keep the client connection
/// tied to one server session: notifications would be lost or never
/// arrive. When the application's pool goes through such a proxy, build the
/// listener from a [`PgConfig`] that points at the database server directly
/// rather than with [`from_pool`](Self::from_pool), which copies the pool's
/// host.
///
/// # Security
///
/// PostgreSQL has no privileges on channels: any role that can connect to
/// the database can `NOTIFY` on any channel, including the ones this
/// listener subscribes to. A notification proves nothing about who sent it
/// or whether its payload is true. Use it as a cue to re-read the
/// authoritative data, or verify an authenticated payload (such as an HMAC
/// over it with a key only legitimate senders hold) before acting on it.
pub struct PgListener {
    cmd_tx: mpsc::Sender<Command>,
    events: mpsc::Receiver<PgNotification>,
    shared: Arc<Shared>,
    pending: Pending,
    /// The supervisor task, until `recv` has seen how it ended. Dropping it
    /// detaches the task, which exits on its own once the handle is gone.
    supervisor: Option<JoinHandle<()>>,
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
    /// its TLS settings and host). The connection is not taken from the pool.
    ///
    /// If the pool connects through a transaction-mode pooler or a proxy
    /// such as Amazon RDS Proxy, use [`connect`](Self::connect) with a
    /// config that points at PostgreSQL directly instead; see
    /// [Connect directly to PostgreSQL](Self#connect-directly-to-postgresql).
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
    /// its encoding cannot represent), the channel is dropped and reported as
    /// [`PgListenerEvent::ChannelsDropped`].
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
    /// Events keep their order relative to the notifications: a
    /// [`PgListenerEvent::Reconnected`] or [`PgListenerEvent::Lagged`] is
    /// returned after the notifications that were already queued when this
    /// first saw it, and before any queued later. On an empty queue it is
    /// returned at once, also while this is waiting. A
    /// [`PgListenerEvent::ChannelsDropped`] comes right before the
    /// `Reconnected` of the reconnect that dropped the channels; when both
    /// are due at once, `Reconnected` comes before `Lagged`. Several
    /// reconnects that happen before this sees the first are reported as a
    /// single `Reconnected`.
    ///
    /// Once the listener has stopped (the connection was lost with
    /// [`PgListenerOptions::reconnect`] disabled, or reconnecting gave up)
    /// and every queued event has been read, this returns the error that
    /// ended it, and a generic "listener closed" error on later calls.
    ///
    /// Cancel safe: if the future is dropped before completing, no event is
    /// lost.
    pub async fn recv(&mut self) -> PgResult<PgListenerEvent> {
        loop {
            self.pending.collect(&self.shared, self.events.len());
            if let Some(event) = self.pending.due() {
                return Ok(event);
            }
            tokio::select! {
                // Checked first so a newly raised flag is collected before
                // the next notification. `Notify` keeps a permit when nobody
                // waits, so a flag raised between the collection above and
                // this await still wakes us; a stale permit only costs one
                // extra loop.
                biased;
                () = self.shared.wake.notified() => {}
                notification = self.events.recv() => match notification {
                    Some(notification) => {
                        self.pending.delivered();
                        return Ok(PgListenerEvent::Notification(notification));
                    }
                    None => {
                        // Stopped: report what was raised last, then why.
                        self.pending.collect(&self.shared, 0);
                        if let Some(event) = self.pending.due() {
                            return Ok(event);
                        }
                        return Err(self.stopped_error().await);
                    }
                },
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

    /// Why the listener stopped, once its queue is closed and drained: the
    /// terminal error (once), else a panic of the supervisor task, else the
    /// generic "listener closed".
    async fn stopped_error(&mut self) -> PgError {
        if let Some(error) = self.shared.take_terminal() {
            return error;
        }
        if let Some(supervisor) = self.supervisor.as_mut() {
            // The queue closes when the supervisor's sender is dropped, so
            // the task has ended or is about to: this returns promptly. The
            // handle is cleared as soon as it completes, never polled again.
            let outcome = supervisor.await;
            self.supervisor = None;
            if let Err(error) = outcome
                && error.is_panic()
            {
                return PgError::Internal(format!("listener supervisor panicked: {error}"));
            }
        }
        listener_closed()
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

/// Events [`PgListener::recv`] has taken from [`Shared`] but not returned
/// yet. Each waits for the notifications that were already queued when it
/// was taken; kept in the handle, so a cancelled `recv` loses none.
#[derive(Debug, Default)]
struct Pending {
    /// A reconnect: notifications still ahead of it, and the channels it
    /// dropped (reported first, as `ChannelsDropped`).
    reconnected: Option<(usize, Vec<String>)>,
    /// Notifications still ahead of a `Lagged`, and its count.
    lagged: Option<(usize, u64)>,
}

impl Pending {
    /// Take events newly raised in `shared`, behind the `queued`
    /// notifications now in the queue. One of a kind is taken at a time:
    /// another one raised meanwhile stays in `shared` (where repeats
    /// coalesce) until this one has been returned.
    fn collect(&mut self, shared: &Shared, queued: usize) {
        if self.reconnected.is_none()
            && let Some(channels) = shared.take_reconnect()
        {
            self.reconnected = Some((queued, channels));
        }
        if self.lagged.is_none() {
            let dropped = shared.take_dropped();
            if dropped > 0 {
                self.lagged = Some((queued, dropped));
            }
        }
    }

    /// The next event with no notification left ahead of it.
    fn due(&mut self) -> Option<PgListenerEvent> {
        if let Some((0, channels)) = &mut self.reconnected {
            if !channels.is_empty() {
                return Some(PgListenerEvent::ChannelsDropped {
                    channels: std::mem::take(channels),
                });
            }
            self.reconnected = None;
            return Some(PgListenerEvent::Reconnected);
        }
        if let Some((0, dropped)) = self.lagged {
            self.lagged = None;
            return Some(PgListenerEvent::Lagged { dropped });
        }
        None
    }

    /// A notification was returned: one fewer ahead of each pending event.
    fn delivered(&mut self) {
        if let Some((ahead, _)) = &mut self.reconnected {
            *ahead = ahead.saturating_sub(1);
        }
        if let Some((ahead, _)) = &mut self.lagged {
            *ahead = ahead.saturating_sub(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_debug_omits_the_config() {
        let config =
            PgConfig::from_url("postgres://prax:hunter2-secret@db.internal:5432/prax").unwrap();
        let debug = format!("{:?}", PgListener::builder(&config));
        assert!(!debug.contains("hunter2-secret"), "{debug}");
        assert!(!debug.contains("db.internal"), "{debug}");
        assert!(debug.contains("ping_interval"), "{debug}");
    }

    #[test]
    fn handshake_bound_leaves_room_after_a_timed_out_address() {
        // tokio-postgres applies `connect_timeout` per resolved address: an
        // address that times out must still leave time for the next one.
        let connect = Duration::from_secs(5);
        let ping = Duration::from_secs(30);
        assert_eq!(handshake_bound(connect, ping), connect + ping);
        assert!(handshake_bound(ping, connect) > ping.max(connect));
        assert_eq!(handshake_bound(Duration::MAX, ping), Duration::MAX);
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
        assert_eq!(o.max_reconnect_attempts, None);
    }

    /// A listener without background tasks: the test plays the driver (the
    /// returned queue sender) and the supervisor (the returned `Shared`).
    fn detached_listener(buffer: usize) -> (PgListener, mpsc::Sender<PgNotification>, Arc<Shared>) {
        let (cmd_tx, _) = mpsc::channel(1);
        let (events_tx, events) = mpsc::channel(buffer);
        let shared = Arc::new(Shared::default());
        let listener = PgListener {
            cmd_tx,
            events,
            shared: Arc::clone(&shared),
            pending: Pending::default(),
            supervisor: None,
        };
        (listener, events_tx, shared)
    }

    fn notification(payload: &str) -> PgNotification {
        PgNotification::new("events", payload, 7)
    }

    fn delivered(payload: &str) -> PgListenerEvent {
        PgListenerEvent::Notification(notification(payload))
    }

    async fn next_event(listener: &mut PgListener) -> PgListenerEvent {
        time::timeout(Duration::from_secs(5), listener.recv())
            .await
            .expect("an event within 5 s")
            .expect("recv ok")
    }

    async fn assert_no_event(listener: &mut PgListener) {
        let result = time::timeout(Duration::from_millis(50), listener.recv()).await;
        assert!(result.is_err(), "unexpected {result:?}");
    }

    #[tokio::test]
    async fn reconnected_follows_the_notifications_queued_before_it() {
        let (mut listener, tx, shared) = detached_listener(8);
        tx.try_send(notification("1")).unwrap();
        tx.try_send(notification("2")).unwrap();
        shared.report_reconnect(Vec::new());
        assert_eq!(next_event(&mut listener).await, delivered("1"));
        // Arrived after `recv` saw the reconnect: it comes after it.
        tx.try_send(notification("3")).unwrap();
        assert_eq!(next_event(&mut listener).await, delivered("2"));
        assert_eq!(
            next_event(&mut listener).await,
            PgListenerEvent::Reconnected
        );
        assert_eq!(next_event(&mut listener).await, delivered("3"));
        assert_no_event(&mut listener).await;
    }

    #[tokio::test]
    async fn lagged_follows_the_queue_that_overflowed() {
        let (mut listener, tx, shared) = detached_listener(2);
        tx.try_send(notification("1")).unwrap();
        tx.try_send(notification("2")).unwrap();
        for _ in 0..3 {
            shared.notification_dropped();
        }
        assert_eq!(next_event(&mut listener).await, delivered("1"));
        tx.try_send(notification("3")).unwrap();
        assert_eq!(next_event(&mut listener).await, delivered("2"));
        assert_eq!(
            next_event(&mut listener).await,
            PgListenerEvent::Lagged { dropped: 3 }
        );
        assert_eq!(next_event(&mut listener).await, delivered("3"));
        assert_no_event(&mut listener).await;
    }

    #[tokio::test]
    async fn channels_dropped_comes_right_before_its_reconnected() {
        let (mut listener, _tx, shared) = detached_listener(8);
        shared.notification_dropped();
        shared.report_reconnect(vec!["bad".to_owned()]);
        assert_eq!(
            next_event(&mut listener).await,
            PgListenerEvent::ChannelsDropped {
                channels: vec!["bad".to_owned()]
            }
        );
        assert_eq!(
            next_event(&mut listener).await,
            PgListenerEvent::Reconnected
        );
        // Due at the same time: after the reconnect.
        assert_eq!(
            next_event(&mut listener).await,
            PgListenerEvent::Lagged { dropped: 1 }
        );
        assert_no_event(&mut listener).await;
    }

    #[tokio::test]
    async fn event_raised_while_waiting_is_returned_at_once() {
        let (mut listener, _tx, shared) = detached_listener(8);
        let raise = tokio::spawn(async move {
            time::sleep(Duration::from_millis(50)).await;
            shared.report_reconnect(Vec::new());
        });
        assert_eq!(
            next_event(&mut listener).await,
            PgListenerEvent::Reconnected
        );
        raise.await.unwrap();
    }

    #[tokio::test]
    async fn one_reconnected_per_reconnect_seen() {
        let (mut listener, tx, shared) = detached_listener(8);
        // Two reconnects before `recv` saw either: one report.
        shared.report_reconnect(Vec::new());
        shared.report_reconnect(Vec::new());
        assert_eq!(
            next_event(&mut listener).await,
            PgListenerEvent::Reconnected
        );
        assert_no_event(&mut listener).await;

        // A reconnect raised after `recv` took the first (which waited
        // behind "1") is reported on its own, in its own place.
        tx.try_send(notification("1")).unwrap();
        shared.report_reconnect(Vec::new());
        assert_eq!(next_event(&mut listener).await, delivered("1"));
        shared.report_reconnect(Vec::new());
        tx.try_send(notification("2")).unwrap();
        assert_eq!(
            next_event(&mut listener).await,
            PgListenerEvent::Reconnected
        );
        assert_eq!(next_event(&mut listener).await, delivered("2"));
        assert_eq!(
            next_event(&mut listener).await,
            PgListenerEvent::Reconnected
        );
        assert_no_event(&mut listener).await;
    }

    #[tokio::test]
    async fn closed_queue_reports_pending_events_then_closes() {
        let (mut listener, tx, shared) = detached_listener(8);
        tx.try_send(notification("1")).unwrap();
        drop(tx);
        shared.notification_dropped();
        assert_eq!(next_event(&mut listener).await, delivered("1"));
        assert_eq!(
            next_event(&mut listener).await,
            PgListenerEvent::Lagged { dropped: 1 }
        );
        let err = listener.recv().await.expect_err("closed");
        assert!(
            matches!(&err, PgError::Connection(m) if m == "listener closed"),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn supervisor_panic_is_reported_once() {
        let (mut listener, tx, _shared) = detached_listener(8);
        listener.supervisor = Some(tokio::spawn(async move {
            // Holds the queue open until it panics, like the supervisor.
            let _tx = tx;
            panic!("supervisor test panic");
        }));
        let err = time::timeout(Duration::from_secs(5), listener.recv())
            .await
            .expect("recv returns")
            .expect_err("the listener stopped");
        assert!(
            matches!(&err, PgError::Internal(m)
                if m.contains("listener supervisor panicked") && m.contains("supervisor test panic")),
            "{err:?}"
        );
        let err = listener.recv().await.expect_err("still stopped");
        assert!(
            matches!(&err, PgError::Connection(m) if m == "listener closed"),
            "{err:?}"
        );
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
                    buffer: MAX_BUFFER + 1,
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
                "ping_interval below 100 ms",
                PgListenerOptions {
                    ping_interval: MIN_PING_INTERVAL - Duration::from_nanos(1),
                    ..base.clone()
                },
            ),
            (
                "ping_interval above one day",
                PgListenerOptions {
                    ping_interval: MAX_PING_INTERVAL + Duration::from_nanos(1),
                    ..base.clone()
                },
            ),
            (
                "zero keepalive_idle",
                PgListenerOptions {
                    keepalive_idle: Duration::ZERO,
                    ..base.clone()
                },
            ),
            (
                "sub-second keepalive_idle (TCP keepalive counts whole seconds)",
                PgListenerOptions {
                    keepalive_idle: Duration::from_millis(999),
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
                    ..base.clone()
                },
            ),
            (
                "zero max_reconnect_attempts",
                PgListenerOptions {
                    max_reconnect_attempts: Some(0),
                    ..base
                },
            ),
        ]
    }

    #[test]
    fn options_validate_accepts_defaults_and_boundaries() {
        assert!(PgListenerOptions::default().validate().is_ok());
        let boundaries = PgListenerOptions {
            buffer: MAX_BUFFER,
            backoff_initial: Duration::from_secs(1),
            backoff_max: Duration::from_secs(1),
            ping_interval: MIN_PING_INTERVAL,
            max_reconnect_attempts: Some(1),
            ..PgListenerOptions::default()
        };
        assert!(boundaries.validate().is_ok());
        let shortest_keepalive = PgListenerOptions {
            keepalive_idle: Duration::from_secs(1),
            ..PgListenerOptions::default()
        };
        assert!(shortest_keepalive.validate().is_ok());
        let longest_ping = PgListenerOptions {
            ping_interval: MAX_PING_INTERVAL,
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
            .buffer(MAX_BUFFER)
            .connect()
            .await;
        assert!(
            matches!(result, Err(ref e) if !matches!(e, PgError::Config(_))),
            "{result:?}"
        );
    }
}
