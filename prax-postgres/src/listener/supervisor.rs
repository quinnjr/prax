//! The listener's background tasks: the supervisor that owns the connection
//! and the tracked channel set, and the per-connection driver.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use futures::StreamExt;
use prax_query::sql::escape_identifier;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{Notify, mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::{self, Interval, MissedTickBehavior};
use tokio_postgres::{AsyncMessage, NoTls};
use tracing::{debug, error, info, warn};

use super::backoff::{ReconnectBackoff, jitter};
use super::{PgListenerEvent, PgListenerOptions, PgNotification};
use crate::connector::Connector;
use crate::error::{PgError, PgResult};

/// `LISTEN` statement for a validated channel name.
fn listen_sql(channel: &str) -> String {
    format!("LISTEN {}", escape_identifier(channel))
}

/// `UNLISTEN` statement for a validated channel name.
fn unlisten_sql(channel: &str) -> String {
    format!("UNLISTEN {}", escape_identifier(channel))
}

/// Reply slot for a [`Command`].
pub(super) type Reply = oneshot::Sender<PgResult<()>>;

/// A request from the [`PgListener`](super::PgListener) handle to its
/// supervisor.
#[derive(Debug)]
pub(super) enum Command {
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

/// Connection losses and reconnects the handle has not taken yet, in the
/// order they happened.
///
/// The supervisor reports them strictly alternating (a loss, the reconnect
/// that ends it, the next loss, ...). Untaken events are folded so that the
/// handle sees them alternate too: an unseen reconnect, loss and reconnect
/// become one reconnect, and an unseen loss, reconnect and loss become one
/// ongoing loss (keeping the first cause).
#[derive(Debug, Default)]
enum OutageEvents {
    #[default]
    None,
    /// The connection was lost, for the given reason.
    Lost(String),
    /// The connection was re-established.
    Reconnected,
    /// Lost, then re-established.
    LostThenReconnected(String),
    /// Re-established, then lost again.
    ReconnectedThenLost(String),
}

/// [`OutageEvents`] plus the channels the reconnects among them dropped.
#[derive(Debug, Default)]
struct Outage {
    events: OutageEvents,
    /// Channels the server rejected while re-subscribing after a reconnect
    /// not taken yet; reported with that reconnect.
    channels_dropped: Vec<String>,
}

/// State shared between the handle, the supervisor and the driver that must
/// never be lost to a full notification queue.
#[derive(Debug, Default)]
pub(super) struct Shared {
    /// Notifications discarded because the queue was full, not yet reported.
    dropped: AtomicU64,
    /// Connection losses and reconnects not yet taken by the handle. A
    /// short critical section, never held across an await.
    outage: Mutex<Outage>,
    /// Set by the driver whenever a notification (or another asynchronous
    /// message) arrives, cleared by the supervisor's ping tick: a tick that
    /// finds it unset sends a ping.
    traffic: AtomicBool,
    /// Wakes a `recv` parked on the empty queue when a loss, a reconnect or
    /// `dropped` is raised.
    pub(super) wake: Notify,
    /// Why the listener stopped, returned once the queue is drained.
    terminal: Mutex<Option<PgError>>,
}

impl Shared {
    /// Count a notification the full queue could not take, waking `recv`
    /// on the first one since the last report.
    pub(super) fn notification_dropped(&self) {
        if self.dropped.fetch_add(1, Ordering::Relaxed) == 0 {
            self.wake.notify_one();
        }
    }

    /// Take the count of dropped notifications not yet reported.
    pub(super) fn take_dropped(&self) -> u64 {
        self.dropped.swap(0, Ordering::Relaxed)
    }

    /// Report that the connection was lost (for `error`) and reconnecting
    /// begins.
    pub(super) fn report_disconnect(&self, error: String) {
        {
            let mut outage = lock(&self.outage);
            outage.events = match std::mem::take(&mut outage.events) {
                OutageEvents::None => OutageEvents::Lost(error),
                OutageEvents::Reconnected => OutageEvents::ReconnectedThenLost(error),
                // The reconnect was never seen: still the same outage.
                OutageEvents::LostThenReconnected(first) => OutageEvents::Lost(first),
                // Not reported twice in a row; keep the first cause.
                lost @ (OutageEvents::Lost(_) | OutageEvents::ReconnectedThenLost(_)) => lost,
            };
        }
        self.wake.notify_one();
    }

    /// Report a reconnect, after which the server refused to re-subscribe
    /// `channels_dropped`.
    pub(super) fn report_reconnect(&self, channels_dropped: Vec<String>) {
        {
            let mut outage = lock(&self.outage);
            outage.channels_dropped.extend(channels_dropped);
            outage.events = match std::mem::take(&mut outage.events) {
                OutageEvents::None => OutageEvents::Reconnected,
                OutageEvents::Lost(error) => OutageEvents::LostThenReconnected(error),
                // The loss after the unseen reconnect, and its end, were
                // never seen either: one reconnect.
                OutageEvents::ReconnectedThenLost(_) => OutageEvents::Reconnected,
                // Repeats coalesce.
                reconnected
                @ (OutageEvents::Reconnected | OutageEvents::LostThenReconnected(_)) => reconnected,
            };
        }
        self.wake.notify_one();
    }

    /// Take the reported losses and reconnects, in order, as the events
    /// `recv` returns (`ChannelsDropped` right before its `Reconnected`).
    pub(super) fn take_outage(&self) -> Vec<PgListenerEvent> {
        let mut outage = lock(&self.outage);
        let disconnected = |error| PgListenerEvent::Disconnected { error };
        match std::mem::take(&mut outage.events) {
            OutageEvents::None => Vec::new(),
            // The channels wait for the reconnect that ends this loss.
            OutageEvents::Lost(error) => vec![disconnected(error)],
            OutageEvents::Reconnected => reconnected(&mut outage.channels_dropped),
            OutageEvents::LostThenReconnected(error) => {
                let mut events = vec![disconnected(error)];
                events.extend(reconnected(&mut outage.channels_dropped));
                events
            }
            OutageEvents::ReconnectedThenLost(error) => {
                let mut events = reconnected(&mut outage.channels_dropped);
                events.push(disconnected(error));
                events
            }
        }
    }

    fn set_terminal(&self, error: PgError) {
        *lock(&self.terminal) = Some(error);
    }

    pub(super) fn take_terminal(&self) -> Option<PgError> {
        lock(&self.terminal).take()
    }
}

/// A reconnect's events: `ChannelsDropped` (when it dropped any, taken from
/// `channels_dropped`), then `Reconnected`.
fn reconnected(channels_dropped: &mut Vec<String>) -> Vec<PgListenerEvent> {
    let mut events = Vec::with_capacity(2);
    if !channels_dropped.is_empty() {
        events.push(PgListenerEvent::ChannelsDropped {
            channels: std::mem::take(channels_dropped),
        });
    }
    events.push(PgListenerEvent::Reconnected);
    events
}

/// Lock `mutex`, ignoring poisoning (every critical section here is a
/// single assignment, `extend`, `take` or a state transition that cannot
/// panic part-way, so the value is never left inconsistent).
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One live connection: the client commands run on, the driver task's
/// end-of-connection signal (`Some(error)` on a connection error, `None` on a
/// clean close), and the driver task itself.
pub(super) struct Session {
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

/// Render a driver error with its cause, for logs and error messages.
///
/// `tokio_postgres::Error`'s own `Display` is only its kind ("db error",
/// "error connecting to server"), which hides why the connection failed: a
/// server error becomes `"{severity} {code}: {message}"`, any other error
/// gets its `source()` chain appended.
fn describe(error: &tokio_postgres::Error) -> String {
    if let Some(db) = error.as_db_error() {
        return format!("{} {}: {}", db.severity(), db.code().code(), db.message());
    }
    let mut text = error.to_string();
    let mut source = std::error::Error::source(error);
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

/// [`describe`] for any [`PgError`].
fn describe_error(error: &PgError) -> String {
    match error {
        PgError::Postgres(error) => describe(error),
        other => other.to_string(),
    }
}

/// Why a connection was given up.
struct LostCause {
    /// What happened, for logs and the terminal error.
    message: String,
    /// The driver error behind it, unless it already went to a caller.
    error: Option<tokio_postgres::Error>,
}

impl LostCause {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            error: None,
        }
    }

    /// A loss caused by `error`, described after `context` (if any).
    fn driver(context: &str, error: tokio_postgres::Error) -> Self {
        let described = describe(&error);
        let message = if context.is_empty() {
            described
        } else {
            format!("{context}: {described}")
        };
        Self {
            message,
            error: Some(error),
        }
    }

    /// The error `recv` returns when reconnecting is disabled: the driver
    /// error itself when there is one, so callers can read its SQLSTATE,
    /// otherwise a connection error carrying the message.
    fn into_terminal(self) -> PgError {
        match self.error {
            Some(error) => PgError::Postgres(error),
            None => PgError::connection(format!("listener connection closed: {}", self.message)),
        }
    }
}

/// What talking to the server revealed about the connection.
enum Liveness {
    Alive,
    /// The connection is unusable, for the given reason.
    Lost(LostCause),
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
/// most [`MAX_PING_INTERVAL`](super::MAX_PING_INTERVAL) (see `validate`), so
/// tokio's `now + period` arithmetic cannot overflow.
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

/// The SQLSTATE of a server error, if `error` is one.
fn sqlstate(error: &PgError) -> Option<&str> {
    match error {
        PgError::Postgres(error) => error.as_db_error().map(|db| db.code().code()),
        _ => None,
    }
}

/// Whether the server refused `LISTEN` because of the channel name itself;
/// see [`rejects_channel_code`].
fn rejects_channel(error: &PgError) -> bool {
    sqlstate(error).is_some_and(rejects_channel_code)
}

/// Whether SQLSTATE `code` means the channel name itself was refused: class
/// 22 (data exception, e.g. 22P05 for a name the server encoding cannot
/// represent), 42602 (`invalid_name`) or 42622 (`name_too_long`). Anything
/// else, such as a hot standby refusing `LISTEN` altogether (25006) or a
/// permission error (42501), is a condition a later attempt may not hit:
/// it fails the attempt, which is retried, rather than dropping channels.
fn rejects_channel_code(code: &str) -> bool {
    code.starts_with("22") || code == "42602" || code == "42622"
}

/// Consecutive reconnect attempts refused for a reason retrying cannot fix
/// (see [`persistent_failure_code`]) after which the listener gives up.
const PERSISTENT_FAILURE_LIMIT: u32 = 3;

/// Whether SQLSTATE `code` refuses a connection for a reason retrying cannot
/// fix: class 28 (invalid authorization, e.g. a wrong password or a
/// `pg_hba.conf` rejection) or 3D000 (the database does not exist).
fn persistent_failure_code(code: &str) -> bool {
    code.starts_with("28") || code == "3D000"
}

/// Why [`Supervisor::reconnect`] returned without a new session.
enum Stop {
    /// The handle was dropped.
    HandleDropped,
    /// Reconnecting gave up; the listener ends with this error.
    GaveUp(PgError),
}

/// Background task owning the client and the tracked channel set.
pub(super) struct Supervisor {
    pg_config: tokio_postgres::Config,
    connector: Connector,
    /// Bound on a whole connection attempt; see [`Supervisor::connect`].
    handshake_timeout: Duration,
    options: PgListenerOptions,
    backoff: ReconnectBackoff,
    events: mpsc::Sender<PgNotification>,
    shared: Arc<Shared>,
    /// Channels currently subscribed, to re-`LISTEN` after a reconnect.
    channels: BTreeSet<String>,
}

impl Supervisor {
    pub(super) fn new(
        pg_config: tokio_postgres::Config,
        connector: Connector,
        handshake_timeout: Duration,
        options: PgListenerOptions,
        events: mpsc::Sender<PgNotification>,
        shared: Arc<Shared>,
    ) -> Self {
        Self {
            pg_config,
            connector,
            handshake_timeout,
            options,
            backoff: ReconnectBackoff::default(),
            events,
            shared,
            channels: BTreeSet::new(),
        }
    }

    /// Open a connection and spawn its driver task, bounded by
    /// `handshake_timeout` (`PgConfig::connect_timeout` plus
    /// `ping_interval`).
    ///
    /// `tokio_postgres`'s own `connect_timeout` only covers opening the
    /// socket. The TLS and startup handshake (including authentication)
    /// would otherwise wait forever on a server or proxy that accepts the
    /// connection and never answers.
    ///
    /// The session then opts out of `idle_session_timeout` (PostgreSQL 14+),
    /// which a server, database or role default could otherwise use to close
    /// a listener that waits quietly for notifications.
    pub(super) async fn connect(&self) -> PgResult<Session> {
        let limit = self.handshake_timeout;
        let session = match time::timeout(limit, self.open_connection()).await {
            Ok(result) => result?,
            Err(_) => {
                return Err(PgError::connection(format!(
                    "connection handshake not completed within {limit:?}"
                )));
            }
        };
        match self
            .batch_execute_bounded(&session.client, "SET idle_session_timeout = 0")
            .await
        {
            Ok(()) => {}
            // 42704 undefined_object: a server before 14 has no such setting.
            Err(e) if sqlstate(&e) == Some("42704") => {}
            Err(e) => return Err(e),
        }
        Ok(session)
    }

    /// [`connect`](Self::connect) without the overall bound.
    async fn open_connection(&self) -> PgResult<Session> {
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
                        {
                            shared.notification_dropped();
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
    /// until the connection is lost while reconnecting is disabled, or until
    /// reconnecting gives up.
    pub(super) async fn run(mut self, mut session: Session, mut commands: mpsc::Receiver<Command>) {
        let mut ticks = ping_ticks(self.options.ping_interval);
        let mut connected_at = time::Instant::now();
        loop {
            let cause = tokio::select! {
                biased;
                outcome = &mut session.closed => match outcome {
                    Ok(Some(e)) => LostCause::driver("", e),
                    Ok(None) => LostCause::new("connection closed by the server"),
                    Err(_) => LostCause::new("connection driver ended unexpectedly"),
                },
                command = commands.recv() => match command {
                    Some(command) => match self.run_command(&session.client, command).await {
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
            warn!(cause = %cause.message, "PostgreSQL listener connection lost");
            if !self.options.reconnect {
                self.shared.set_terminal(cause.into_terminal());
                return;
            }
            // Reported now, so the application learns of the outage while
            // the reconnect is still under way.
            self.shared.report_disconnect(cause.message);
            self.backoff
                .session_lost(connected_at.elapsed(), self.options.ping_interval);
            let (new_session, channels_dropped) = match self.reconnect(&mut commands).await {
                Ok(reconnected) => reconnected,
                // The handle was dropped while reconnecting.
                Err(Stop::HandleDropped) => return,
                Err(Stop::GaveUp(error)) => {
                    self.shared.set_terminal(error);
                    return;
                }
            };
            session = new_session;
            connected_at = time::Instant::now();
            // Every tracked channel is subscribed again (or reported as
            // dropped): report it.
            self.shared.report_reconnect(channels_dropped);
            // Traffic seen on the lost connection says nothing about the new
            // one: its first tick must ping unless the new one delivers.
            self.shared.traffic.store(false, Ordering::Relaxed);
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
            QueryOutcome::Done(Err(e)) => Liveness::Lost(LostCause::driver("ping failed", e)),
            QueryOutcome::TimedOut => Liveness::Lost(LostCause::new(format!(
                "ping not answered within {:?}",
                self.options.ping_interval
            ))),
            QueryOutcome::HandleDropped => Liveness::HandleDropped,
        }
    }

    /// Run one command on the live connection, bounded like a ping.
    ///
    /// Success updates the tracked channel set. A server-side error (the
    /// connection itself is fine) only goes back to the caller; a timeout or
    /// a connection error goes back to the caller *and* counts as connection
    /// loss.
    async fn run_command(&mut self, client: &tokio_postgres::Client, command: Command) -> Liveness {
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
                // The driver error goes to the caller; the cause keeps its
                // description.
                let cause = LostCause::new(format!("{sql} failed: {}", describe(&e)));
                command.fail(e.into());
                Liveness::Lost(cause)
            }
            QueryOutcome::TimedOut => {
                let limit = self.options.ping_interval;
                command.fail(timeout_error(limit));
                Liveness::Lost(LostCause::new(format!(
                    "{sql} not answered within {limit:?}"
                )))
            }
            QueryOutcome::HandleDropped => Liveness::HandleDropped,
        }
    }

    /// Re-establish the connection: before each attempt wait out its
    /// backoff delay (meanwhile recording commands against the tracked
    /// channel set), then connect and re-`LISTEN` every tracked channel.
    /// Returns the new session and the channels the server refused (no
    /// longer tracked). Stops as soon as the handle is dropped, and gives up
    /// after `max_reconnect_attempts` failed attempts or
    /// [`PERSISTENT_FAILURE_LIMIT`] consecutive persistent failures.
    ///
    /// The delay keeps growing across sessions until one stays up for a
    /// `ping_interval` (see [`ReconnectBackoff`]).
    ///
    /// The whole retry loop lives here, so the lost session's (possibly
    /// completed) `closed` receiver is never polled again.
    async fn reconnect(
        &mut self,
        commands: &mut mpsc::Receiver<Command>,
    ) -> Result<(Session, Vec<String>), Stop> {
        // Attempts in this outage; the backoff step persists across outages.
        let mut outage_attempts: u32 = 0;
        let mut persistent_failures: u32 = 0;
        loop {
            let backoff_step = self.backoff.step();
            let delay = self.backoff.next_delay(&self.options, jitter());
            let backoff = time::sleep(delay);
            tokio::pin!(backoff);
            loop {
                tokio::select! {
                    biased;
                    command = commands.recv() => match command {
                        Some(command) => self.track(command),
                        None => return Err(Stop::HandleDropped),
                    },
                    () = &mut backoff => break,
                }
            }

            // Dropping an unfinished attempt aborts its driver.
            let outcome = tokio::select! {
                biased;
                () = self.events.closed() => return Err(Stop::HandleDropped),
                outcome = self.open_subscribed() => outcome,
            };
            outage_attempts = outage_attempts.saturating_add(1);
            match outcome {
                Ok((session, rejected)) => {
                    for channel in &rejected {
                        self.channels.remove(channel);
                    }
                    info!(
                        outage_attempts,
                        backoff_step,
                        channels = self.channels.len(),
                        "PostgreSQL listener reconnected"
                    );
                    return Ok((session, rejected));
                }
                Err(e) => {
                    let code = sqlstate(&e).unwrap_or_default();
                    if persistent_failure_code(code) {
                        persistent_failures += 1;
                    } else {
                        persistent_failures = 0;
                    }
                    let give_up = if persistent_failures >= PERSISTENT_FAILURE_LIMIT {
                        Some("the server keeps refusing the credentials or the database")
                    } else if self
                        .options
                        .max_reconnect_attempts
                        .is_some_and(|max| outage_attempts >= max)
                    {
                        Some("max_reconnect_attempts reached")
                    } else {
                        None
                    };
                    if let Some(reason) = give_up {
                        error!(
                            error = %describe_error(&e),
                            sqlstate = code,
                            outage_attempts,
                            reason,
                            "PostgreSQL listener gave up reconnecting"
                        );
                        return Err(Stop::GaveUp(e));
                    }
                    warn!(
                        error = %describe_error(&e),
                        sqlstate = code,
                        outage_attempts,
                        backoff_step,
                        "PostgreSQL listener reconnect attempt failed"
                    );
                }
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
        match self.batch_execute_bounded(&session.client, &batch).await {
            Ok(()) => {}
            Err(e) if rejects_channel(&e) => {
                for channel in &self.channels {
                    match self
                        .batch_execute_bounded(&session.client, &listen_sql(channel))
                        .await
                    {
                        Ok(()) => {}
                        Err(e) if rejects_channel(&e) => {
                            error!(
                                channel = %channel,
                                sqlstate = sqlstate(&e).unwrap_or_default(),
                                error = %describe_error(&e),
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

    /// Run the statement(s) in `sql` on `client` (a connection being set up,
    /// which no command can use yet), bounded by `ping_interval`.
    async fn batch_execute_bounded(
        &self,
        client: &tokio_postgres::Client,
        sql: &str,
    ) -> PgResult<()> {
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
    fn listen_sql_quotes_identifier() {
        assert_eq!(listen_sql("Mixed\"Case"), "LISTEN \"Mixed\"\"Case\"");
        assert_eq!(listen_sql("events"), "LISTEN \"events\"");
    }

    #[test]
    fn unlisten_sql_quotes_identifier() {
        assert_eq!(unlisten_sql("Mixed\"Case"), "UNLISTEN \"Mixed\"\"Case\"");
    }

    #[test]
    fn persistent_failures_are_auth_and_missing_database() {
        // invalid_authorization_specification, invalid_password,
        // invalid_catalog_name.
        for code in ["28000", "28P01", "3D000"] {
            assert!(persistent_failure_code(code), "{code}");
        }
        // Transient: shutting down, too many connections, starting up,
        // connection failure; and no SQLSTATE at all (I/O, timeout).
        for code in ["57P01", "53300", "57P03", "08006", "3F000", ""] {
            assert!(!persistent_failure_code(code), "{code}");
        }
    }

    #[test]
    fn channel_rejection_is_limited_to_name_errors() {
        // Class 22 (e.g. 22P05: the server encoding cannot represent the
        // name), invalid_name and name_too_long: the name itself is bad.
        for code in ["22P05", "22021", "22023", "42602", "42622"] {
            assert!(rejects_channel_code(code), "{code}");
        }
        // Everything else fails the attempt, which is retried.
        for code in [
            "42501", "42601", "42P01", "25006", "57P01", "53300", "08006", "XX000",
        ] {
            assert!(!rejects_channel_code(code), "{code}");
        }
    }
}
