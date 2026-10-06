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
            recycling_method: pool_config.recycling.into(),
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
        if let Some(timeout) = pool_config.idle_timeout {
            builder = builder.recycle_timeout(Some(timeout));
        }

        // Enforce `max_lifetime`: deadpool runs `pre_recycle` hooks at
        // checkout, and a hook error discards the object and creates a new
        // one. Connections are therefore retired lazily, when an aged one
        // is next checked out.
        if let Some(max_lifetime) = pool_config.max_lifetime {
            builder = builder.pre_recycle(Hook::sync_fn(move |_, metrics| {
                if metrics.created.elapsed() > max_lifetime {
                    Err(HookError::message("max_lifetime exceeded"))
                } else {
                    Ok(())
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
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PgRecycling {
    /// Only discard connections that are already closed (default).
    #[default]
    Fast,
    /// Run a trivial query to verify the connection is alive.
    Verified,
    /// `Verified` plus a session-state reset: closes cursors, resets
    /// session settings and authorization, `UNLISTEN *`, releases advisory
    /// locks, and drops temp tables and sequence state. Prepared
    /// statements are preserved, so Prax's statement cache stays valid.
    Clean,
}

impl From<PgRecycling> for RecyclingMethod {
    fn from(r: PgRecycling) -> Self {
        match r {
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
    /// Timeout for recycling a connection when it is returned to the pool.
    ///
    /// Mapped to deadpool's `recycle_timeout`, which bounds the recycle check
    /// performed on connection return — it is **not** an idle-reaping timeout.
    /// deadpool-postgres 0.14 exposes no true idle timeout, so connections are
    /// not closed after a fixed idle period.
    pub idle_timeout: Option<Duration>,
    /// Maximum lifetime of a connection.
    ///
    /// Enforced at checkout: a connection older than this is discarded and
    /// replaced instead of being handed out. Connections are not reaped
    /// while idle. `Duration::ZERO` replaces the connection on every
    /// checkout, which effectively disables pooling.
    pub max_lifetime: Option<Duration>,
    /// Recycling method applied when a connection is checked out again.
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

    /// Set the idle timeout.
    pub fn idle_timeout(mut self, timeout: Duration) -> Self {
        self.pool_config.idle_timeout = Some(timeout);
        self
    }

    /// Set the maximum connection lifetime.
    pub fn max_lifetime(mut self, lifetime: Duration) -> Self {
        self.pool_config.max_lifetime = Some(lifetime);
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
            RecyclingMethod::from(PgRecycling::Fast),
            RecyclingMethod::Fast
        ));
        assert!(matches!(
            RecyclingMethod::from(PgRecycling::Verified),
            RecyclingMethod::Verified
        ));
        assert!(matches!(
            RecyclingMethod::from(PgRecycling::Clean),
            RecyclingMethod::Clean
        ));
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
