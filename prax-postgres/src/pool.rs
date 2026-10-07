//! Connection pool for PostgreSQL.

use std::sync::Arc;
use std::time::Duration;

use deadpool_postgres::{Hook, HookError, Manager, ManagerConfig, Pool, RecyclingMethod, Runtime};
use tokio_postgres::NoTls;
use tracing::{debug, info, warn};

use crate::config::PgConfig;
use crate::connection::PgConnection;
use crate::connector::{Connector, select_connector};
use crate::error::{PgError, PgResult};
use crate::statement::PreparedStatementCache;

/// A connection pool for PostgreSQL.
#[derive(Clone)]
pub struct PgPool {
    inner: Pool,
    config: Arc<PgConfig>,
    statement_cache: Arc<PreparedStatementCache>,
}

impl PgPool {
    /// Create a new connection pool from configuration.
    pub async fn new(config: PgConfig) -> PgResult<Self> {
        Self::with_pool_config(config, PoolConfig::default()).await
    }

    /// Create a new connection pool with custom pool configuration.
    pub async fn with_pool_config(config: PgConfig, pool_config: PoolConfig) -> PgResult<Self> {
        let pg_config = config.to_pg_config();

        let mgr_config = ManagerConfig {
            recycling_method: pool_config.recycling.to_deadpool(),
        };

        // Select the TLS connector by ssl_mode (see `select_connector` for the
        // rules). A TLS-requiring mode without the `tls` feature, or an
        // unreadable `ssl_root_cert`, fails pool construction here.
        let mgr = match select_connector(&config)? {
            Connector::NoTls => Manager::from_config(pg_config, NoTls, mgr_config),
            #[cfg(feature = "tls")]
            Connector::Rustls(connector) => Manager::from_config(pg_config, connector, mgr_config),
        };

        // Build pool - set runtime to tokio for timeout support
        let mut builder = Pool::builder(mgr).max_size(pool_config.max_connections);

        // Only set timeouts if they are configured
        if let Some(timeout) = pool_config.connection_timeout {
            builder = builder
                .wait_timeout(Some(timeout))
                .create_timeout(Some(timeout));
        }
        // Bounds the recycle check (Verified/Clean round trip) at checkout.
        // A recycle that times out discards the connection and a new one is
        // created, so a hung connection cannot stall `get()` forever.
        if let Some(timeout) = pool_config.recycle_timeout {
            builder = builder.recycle_timeout(Some(timeout));
        }

        // Enforce `max_lifetime` and `idle_timeout`: deadpool runs
        // `pre_recycle` hooks at checkout, and a hook error discards the
        // object and creates a new one. Connections are therefore retired
        // lazily, when an aged or idle one is next checked out. Each
        // connection's lifetime deadline is shortened by a stable
        // per-connection jitter (up to ~10%) so a batch of connections
        // created together does not expire together.
        if let Some(policy) =
            RetirementPolicy::new(pool_config.max_lifetime, pool_config.idle_timeout)
        {
            builder = builder.pre_recycle(Hook::sync_fn(move |_, metrics| {
                let age = metrics.age();
                // deadpool records when a connection was last handed out
                // (`recycled`, else `created`), not when it was returned.
                let idle = metrics.last_used();
                let jitter = policy.max_lifetime.map_or(Duration::ZERO, |max_lifetime| {
                    lifetime_jitter(max_lifetime, &metrics.created)
                });
                match policy.check(age, idle, jitter) {
                    None => Ok(()),
                    Some(reason) => {
                        debug!(
                            age = ?age,
                            idle = ?idle,
                            reason = reason.as_str(),
                            "retiring pooled connection"
                        );
                        Err(HookError::message(reason.as_str()))
                    }
                }
            }));
        }

        // Set runtime for timeout support
        builder = builder.runtime(Runtime::Tokio1);

        let pool = builder
            .build()
            .map_err(|e| PgError::config(format!("failed to create pool: {}", e)))?;

        // deadpool has no native min-connections knob, so pre-establish
        // `min_connections` on a background task. Acquired connections are
        // released back to the pool, which retains them (deadpool never
        // reaps idle connections). Best-effort: pool creation neither
        // blocks on nor fails because of this warmup.
        let min_connections = pool_config.min_connections.min(pool_config.max_connections);
        if min_connections > 0 {
            let warm_pool = pool.clone();
            tokio::spawn(async move {
                let mut held = Vec::with_capacity(min_connections);
                for _ in 0..min_connections {
                    match warm_pool.get().await {
                        Ok(conn) => {
                            debug!(
                                established = held.len() + 1,
                                min_connections = min_connections,
                                "min_connections warmup established connection"
                            );
                            held.push(conn);
                        }
                        Err(e) => {
                            // Operator-visible: the pool is starting below
                            // its configured minimum.
                            warn!(
                                error = %e,
                                min_connections = min_connections,
                                established = held.len(),
                                "min_connections warmup could not acquire connection; \
                                 pool starts below its configured minimum"
                            );
                            break;
                        }
                    }
                }
                drop(held);
            });
        }

        info!(
            host = %config.host,
            port = %config.port,
            database = %config.database,
            max_connections = %pool_config.max_connections,
            "PostgreSQL connection pool created"
        );

        Ok(Self {
            inner: pool,
            config: Arc::new(config),
            statement_cache: Arc::new(PreparedStatementCache::new(
                pool_config.statement_cache_size,
            )),
        })
    }

    /// Get a connection from the pool.
    pub async fn get(&self) -> PgResult<PgConnection> {
        debug!("Acquiring connection from pool");
        let client = self.inner.get().await?;
        Ok(PgConnection::new(client, self.statement_cache.clone()))
    }

    /// Borrow the underlying `deadpool_postgres::Pool`.
    ///
    /// Reserved for intra-crate paths that need a raw `Object` (e.g.
    /// [`crate::engine::PgEngine::transaction`], which pins a single
    /// connection for the lifetime of an in-flight transaction). The
    /// standard path is [`PgPool::get`], which returns a
    /// cache-wrapped [`PgConnection`].
    pub(crate) fn inner(&self) -> &Pool {
        &self.inner
    }

    /// Get the current pool status.
    pub fn status(&self) -> PoolStatus {
        let status = self.inner.status();
        PoolStatus {
            available: status.available,
            size: status.size,
            max_size: status.max_size,
            waiting: status.waiting,
        }
    }

    /// Get the pool configuration.
    pub fn config(&self) -> &PgConfig {
        &self.config
    }

    /// Check if the pool is healthy by attempting to get a connection.
    pub async fn is_healthy(&self) -> bool {
        match self.inner.get().await {
            Ok(client) => {
                // Try a simple query to verify the connection is actually working
                client.query_one("SELECT 1", &[]).await.is_ok()
            }
            Err(_) => false,
        }
    }

    /// Close the pool and all connections.
    pub fn close(&self) {
        self.inner.close();
        info!("PostgreSQL connection pool closed");
    }

    /// Create a builder for configuring the pool.
    pub fn builder() -> PgPoolBuilder {
        PgPoolBuilder::new()
    }

    /// Warm up the connection pool by pre-establishing connections.
    ///
    /// This eliminates the latency of establishing connections on the first queries.
    /// The `count` parameter specifies how many connections to pre-establish.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let pool = PgPool::builder()
    ///     .url("postgresql://localhost/db")
    ///     .max_connections(10)
    ///     .build()
    ///     .await?;
    ///
    /// // Pre-establish 5 connections
    /// pool.warmup(5).await?;
    /// ```
    pub async fn warmup(&self, count: usize) -> PgResult<()> {
        info!(count = count, "Warming up connection pool");

        let count = count.min(self.inner.status().max_size);
        let mut connections = Vec::with_capacity(count);

        // Acquire connections to force establishment
        for i in 0..count {
            match self.inner.get().await {
                Ok(conn) => {
                    // Validate the connection with a simple query
                    if let Err(e) = conn.query_one("SELECT 1", &[]).await {
                        debug!(error = %e, "Warmup connection {} failed validation", i);
                    } else {
                        debug!("Warmup connection {} established", i);
                        connections.push(conn);
                    }
                }
                Err(e) => {
                    debug!(error = %e, "Failed to establish warmup connection {}", i);
                }
            }
        }

        // Connections are returned to pool when dropped
        let established = connections.len();
        drop(connections);

        info!(
            established = established,
            requested = count,
            "Connection pool warmup complete"
        );

        Ok(())
    }

    /// Warm up with common prepared statements.
    ///
    /// This pre-prepares common SQL statements on warmed connections,
    /// eliminating the prepare latency on first use.
    pub async fn warmup_with_statements(&self, count: usize, statements: &[&str]) -> PgResult<()> {
        info!(
            count = count,
            statements = statements.len(),
            "Warming up connection pool with prepared statements"
        );

        let count = count.min(self.inner.status().max_size);
        let mut connections = Vec::with_capacity(count);

        for i in 0..count {
            match self.inner.get().await {
                Ok(conn) => {
                    // Pre-prepare all statements
                    for sql in statements {
                        if let Err(e) = conn.prepare_cached(sql).await {
                            debug!(error = %e, sql = %sql, "Failed to prepare statement");
                        }
                    }
                    debug!(
                        connection = i,
                        statements = statements.len(),
                        "Prepared statements on connection"
                    );
                    connections.push(conn);
                }
                Err(e) => {
                    debug!(error = %e, "Failed to establish warmup connection {}", i);
                }
            }
        }

        let established = connections.len();
        drop(connections);

        info!(
            established = established,
            "Connection pool warmup with statements complete"
        );

        Ok(())
    }
}

/// Why the pool replaces a connection at checkout instead of handing it out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Retirement {
    /// Older than `max_lifetime` (less its jitter).
    MaxLifetime,
    /// Not handed out for longer than `idle_timeout`.
    IdleTimeout,
}

impl Retirement {
    fn as_str(self) -> &'static str {
        match self {
            Self::MaxLifetime => "max_lifetime exceeded",
            Self::IdleTimeout => "idle_timeout exceeded",
        }
    }
}

/// The limits the pool's `pre_recycle` hook enforces at checkout.
#[derive(Debug, Clone, Copy)]
struct RetirementPolicy {
    max_lifetime: Option<Duration>,
    idle_timeout: Option<Duration>,
}

impl RetirementPolicy {
    /// `None` when neither limit is set (no hook is needed).
    fn new(max_lifetime: Option<Duration>, idle_timeout: Option<Duration>) -> Option<Self> {
        (max_lifetime.is_some() || idle_timeout.is_some()).then_some(Self {
            max_lifetime,
            idle_timeout,
        })
    }

    /// Whether a connection `age` old, last handed out `idle` ago and with
    /// lifetime `jitter`, must be replaced. The lifetime is checked first.
    fn check(&self, age: Duration, idle: Duration, jitter: Duration) -> Option<Retirement> {
        if let Some(max_lifetime) = self.max_lifetime
            && age > max_lifetime.saturating_sub(jitter)
        {
            return Some(Retirement::MaxLifetime);
        }
        if let Some(idle_timeout) = self.idle_timeout
            && idle > idle_timeout
        {
            return Some(Retirement::IdleTimeout);
        }
        None
    }
}

/// Stable per-connection offset in `[0, max_lifetime / 10)`, derived from the
/// connection's creation instant so it is identical on every checkout.
fn lifetime_jitter(max_lifetime: Duration, created: &std::time::Instant) -> Duration {
    use std::hash::{Hash, Hasher};

    let window = (max_lifetime / 10).as_nanos();
    if window == 0 {
        return Duration::ZERO;
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    created.hash(&mut hasher);
    let nanos = (hasher.finish() as u128) % window;
    Duration::from_nanos(nanos as u64)
}

/// Pool status information.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct PoolStatus {
    /// Number of available (idle) connections.
    pub available: usize,
    /// Current total size of the pool.
    pub size: usize,
    /// Maximum size of the pool.
    pub max_size: usize,
    /// Number of tasks waiting for a connection.
    pub waiting: usize,
}

/// How the pool validates a connection when it is checked out again.
///
/// None of these methods roll back an open transaction; `Clean` resets
/// session state but runs inside whatever transaction is still open.
///
/// **Session state and multi-tenant / RLS use.** Under `Fast` and `Verified`,
/// session state (`SET`, `SET ROLE`, `search_path`, custom GUCs such as a
/// tenant id read by row-level-security policies, advisory locks, temp
/// tables) persists across checkouts: the next caller of the same physical
/// connection inherits it. If you rely on such state for isolation, use
/// [`PgRecycling::Clean`] or scope it with `SET LOCAL` inside a transaction.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PgRecycling {
    /// Only discard connections that are already closed (default).
    ///
    /// Does **not** reset session state; see the type-level warning about
    /// leaking `SET` / `SET ROLE` / tenant GUCs between checkouts. Prefer
    /// [`PgRecycling::Clean`] for RLS or tenant-scoped workloads.
    #[default]
    Fast,
    /// Run a trivial query to verify the connection is alive.
    ///
    /// The check is bounded by [`PoolConfig::recycle_timeout`]. Session
    /// state is **not** reset (see the type-level warning).
    Verified,
    /// `Verified` plus a session-state reset: closes cursors, resets
    /// session settings and authorization, `UNLISTEN *`, releases advisory
    /// locks, and drops temp tables and sequence state. Prepared
    /// statements are preserved, so Prax's statement cache stays valid.
    ///
    /// The check and reset are bounded by [`PoolConfig::recycle_timeout`].
    /// Recommended for RLS / tenant users.
    Clean,
}

impl PgRecycling {
    pub(crate) fn to_deadpool(self) -> RecyclingMethod {
        match self {
            PgRecycling::Fast => RecyclingMethod::Fast,
            PgRecycling::Verified => RecyclingMethod::Verified,
            PgRecycling::Clean => RecyclingMethod::Clean,
        }
    }
}

/// Configuration for the connection pool.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// Maximum number of connections in the pool.
    pub max_connections: usize,
    /// Minimum number of connections to pre-establish and keep alive.
    ///
    /// deadpool exposes no minimum-size knob, so these are established by a
    /// best-effort background warmup when the pool is created; the released
    /// connections are then retained by the pool.
    pub min_connections: usize,
    /// Maximum time to wait for a connection.
    pub connection_timeout: Option<Duration>,
    /// Idle timeout: a connection that was not handed out for longer than
    /// this is discarded and replaced at its next checkout instead of being
    /// reused (so a connection a server, proxy or NAT may have dropped while
    /// idle is not handed out).
    ///
    /// Enforced at checkout, like [`max_lifetime`](Self::max_lifetime):
    /// idle connections are not reaped in the background. The idle time is
    /// measured from when the connection was last **handed out** (deadpool
    /// does not record when it is returned), so a connection held for
    /// longer than this is also replaced at its next checkout.
    /// `Duration::ZERO` replaces every reused connection; `None` disables
    /// the limit. It does not bound the recycle check; use
    /// [`PoolConfig::recycle_timeout`] for that. Default: 10 minutes.
    pub idle_timeout: Option<Duration>,
    /// Upper bound on the recycle check run when a connection is checked out
    /// again (the `Verified` / `Clean` round trip).
    ///
    /// Mapped to deadpool's `recycle_timeout`. A recycle that exceeds it
    /// discards the connection and a fresh one is created. `None` disables
    /// the bound. Default: 5 seconds.
    pub recycle_timeout: Option<Duration>,
    /// Maximum lifetime of a connection.
    ///
    /// Enforced at checkout: a connection older than this is discarded and
    /// replaced instead of being handed out. Connections are not reaped
    /// while idle. Each connection's limit is shortened by a stable jitter of
    /// up to ~10% so connections created together do not all expire at once.
    /// `Duration::ZERO` replaces the connection on every checkout, which
    /// effectively disables pooling.
    pub max_lifetime: Option<Duration>,
    /// Recycling method applied when a connection is checked out again.
    ///
    /// Defaults to [`PgRecycling::Fast`], which does not reset session state
    /// (`SET`, `SET ROLE`, tenant GUCs persist across checkouts). Use
    /// [`PgRecycling::Clean`] for RLS / tenant-scoped workloads.
    pub recycling: PgRecycling,
    /// Number of SQL strings tracked for prepared-statement cache metrics.
    ///
    /// [`PreparedStatementCache`] records only which SQL strings have been
    /// seen (hit/miss tracing); actual statement caching is per-connection
    /// inside tokio-postgres and is not bounded by this value.
    pub statement_cache_size: usize,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            max_connections: 10,
            min_connections: 1,
            connection_timeout: Some(Duration::from_secs(30)),
            idle_timeout: Some(Duration::from_secs(600)), // 10 minutes
            recycle_timeout: Some(Duration::from_secs(5)),
            max_lifetime: Some(Duration::from_secs(1800)), // 30 minutes
            recycling: PgRecycling::Fast,
            statement_cache_size: 100,
        }
    }
}

/// Builder for creating a connection pool.
#[derive(Debug, Default)]
pub struct PgPoolBuilder {
    config: Option<PgConfig>,
    url: Option<String>,
    pool_config: PoolConfig,
}

impl PgPoolBuilder {
    /// Create a new pool builder.
    pub fn new() -> Self {
        Self {
            config: None,
            url: None,
            pool_config: PoolConfig::default(),
        }
    }

    /// Set the database URL.
    pub fn url(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }

    /// Set the configuration.
    pub fn config(mut self, config: PgConfig) -> Self {
        self.config = Some(config);
        self
    }

    /// Set the maximum number of connections.
    pub fn max_connections(mut self, n: usize) -> Self {
        self.pool_config.max_connections = n;
        self
    }

    /// Set the minimum number of connections.
    pub fn min_connections(mut self, n: usize) -> Self {
        self.pool_config.min_connections = n;
        self
    }

    /// Set the connection timeout.
    pub fn connection_timeout(mut self, timeout: Duration) -> Self {
        self.pool_config.connection_timeout = Some(timeout);
        self
    }

    /// Set the idle timeout enforced at checkout (see
    /// [`PoolConfig::idle_timeout`]). Pass `None` to disable.
    pub fn idle_timeout(mut self, timeout: impl Into<Option<Duration>>) -> Self {
        self.pool_config.idle_timeout = timeout.into();
        self
    }

    /// Set the bound on the recycle check at checkout
    /// (see [`PoolConfig::recycle_timeout`]). Pass `None` to disable.
    pub fn recycle_timeout(mut self, timeout: impl Into<Option<Duration>>) -> Self {
        self.pool_config.recycle_timeout = timeout.into();
        self
    }

    /// Set the maximum connection lifetime. Pass `None` to disable.
    pub fn max_lifetime(mut self, lifetime: impl Into<Option<Duration>>) -> Self {
        self.pool_config.max_lifetime = lifetime.into();
        self
    }

    /// Set the recycling method used when a connection is checked out again.
    pub fn recycling(mut self, recycling: PgRecycling) -> Self {
        self.pool_config.recycling = recycling;
        self
    }

    /// Set the prepared statement cache size.
    pub fn statement_cache_size(mut self, size: usize) -> Self {
        self.pool_config.statement_cache_size = size;
        self
    }

    /// Build the connection pool.
    pub async fn build(self) -> PgResult<PgPool> {
        let config = if let Some(config) = self.config {
            config
        } else if let Some(url) = self.url {
            PgConfig::from_url(url)?
        } else {
            return Err(PgError::config("no database URL or config provided"));
        };

        PgPool::with_pool_config(config, self.pool_config).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pool_config_default() {
        let config = PoolConfig::default();
        assert_eq!(config.max_connections, 10);
        assert_eq!(config.min_connections, 1);
        assert_eq!(config.statement_cache_size, 100);
    }

    #[test]
    fn test_pool_config_default_recycling_is_fast() {
        assert_eq!(PoolConfig::default().recycling, PgRecycling::Fast);
    }

    #[test]
    fn test_pg_recycling_maps_to_deadpool() {
        assert!(matches!(
            PgRecycling::Fast.to_deadpool(),
            RecyclingMethod::Fast
        ));
        assert!(matches!(
            PgRecycling::Verified.to_deadpool(),
            RecyclingMethod::Verified
        ));
        assert!(matches!(
            PgRecycling::Clean.to_deadpool(),
            RecyclingMethod::Clean
        ));
    }

    #[test]
    fn test_recycle_timeout_default_and_plumbing() {
        assert_eq!(
            PoolConfig::default().recycle_timeout,
            Some(Duration::from_secs(5))
        );
        let b = PgPoolBuilder::new().recycle_timeout(Duration::from_millis(250));
        assert_eq!(
            b.pool_config.recycle_timeout,
            Some(Duration::from_millis(250))
        );
        let b = PgPoolBuilder::new().recycle_timeout(None);
        assert_eq!(b.pool_config.recycle_timeout, None);
    }

    #[test]
    fn test_max_lifetime_accepts_duration_and_none() {
        let b = PgPoolBuilder::new().max_lifetime(Duration::from_secs(60));
        assert_eq!(b.pool_config.max_lifetime, Some(Duration::from_secs(60)));
        let b = PgPoolBuilder::new().max_lifetime(None);
        assert_eq!(b.pool_config.max_lifetime, None);
    }

    #[test]
    fn test_lifetime_jitter_bounds_and_stability() {
        let max = Duration::from_secs(1800);
        let created = std::time::Instant::now();
        let j = lifetime_jitter(max, &created);
        assert!(j < max / 10);
        assert_eq!(j, lifetime_jitter(max, &created), "stable per connection");
        assert_eq!(
            lifetime_jitter(Duration::ZERO, &created),
            Duration::ZERO,
            "ZERO guarded"
        );
    }

    #[test]
    fn retirement_checks_lifetime_then_idle_time() {
        let minutes = |m: u64| Duration::from_secs(m * 60);
        let policy = RetirementPolicy::new(Some(minutes(30)), Some(minutes(10))).unwrap();
        let none = Duration::ZERO;
        assert_eq!(policy.check(minutes(5), minutes(1), none), None);
        assert_eq!(
            policy.check(minutes(31), minutes(1), none),
            Some(Retirement::MaxLifetime)
        );
        // The jitter brings the lifetime deadline forward.
        assert_eq!(
            policy.check(minutes(29), minutes(1), minutes(2)),
            Some(Retirement::MaxLifetime)
        );
        assert_eq!(
            policy.check(minutes(20), minutes(11), none),
            Some(Retirement::IdleTimeout)
        );
        // Only idle *longer* than the timeout retires.
        assert_eq!(policy.check(minutes(20), minutes(10), none), None);
        // Both exceeded: the lifetime is reported.
        assert_eq!(
            policy.check(minutes(31), minutes(11), none),
            Some(Retirement::MaxLifetime)
        );
    }

    #[test]
    fn retirement_limits_apply_independently() {
        let secs = Duration::from_secs;
        assert!(
            RetirementPolicy::new(None, None).is_none(),
            "no hook needed"
        );
        let idle_only = RetirementPolicy::new(None, Some(secs(60))).unwrap();
        assert_eq!(idle_only.check(secs(86_400), secs(1), Duration::ZERO), None);
        assert_eq!(
            idle_only.check(secs(5), secs(61), Duration::ZERO),
            Some(Retirement::IdleTimeout)
        );
        let lifetime_only = RetirementPolicy::new(Some(secs(60)), None).unwrap();
        assert_eq!(
            lifetime_only.check(secs(5), secs(86_400), Duration::ZERO),
            None
        );
        // `Duration::ZERO` idle timeout replaces every reused connection.
        let zero = RetirementPolicy::new(None, Some(Duration::ZERO)).unwrap();
        assert_eq!(
            zero.check(secs(1), Duration::from_nanos(1), Duration::ZERO),
            Some(Retirement::IdleTimeout)
        );
    }

    #[test]
    fn test_idle_timeout_accepts_duration_and_none() {
        let b = PgPoolBuilder::new().idle_timeout(Duration::from_secs(60));
        assert_eq!(b.pool_config.idle_timeout, Some(Duration::from_secs(60)));
        let b = PgPoolBuilder::new().idle_timeout(None);
        assert_eq!(b.pool_config.idle_timeout, None);
    }

    #[test]
    fn test_pool_builder() {
        let builder = PgPoolBuilder::new()
            .url("postgresql://localhost/test")
            .max_connections(20)
            .statement_cache_size(200);

        assert!(builder.url.is_some());
        assert_eq!(builder.pool_config.max_connections, 20);
        assert_eq!(builder.pool_config.statement_cache_size, 200);
    }
}
