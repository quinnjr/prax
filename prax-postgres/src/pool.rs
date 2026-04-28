//! Connection pool for PostgreSQL.

use std::sync::Arc;
use std::time::Duration;

use deadpool_postgres::{Manager, ManagerConfig, Pool, RecyclingMethod, Runtime};
use tokio_postgres::NoTls;
use tracing::{debug, info};

use crate::config::PgConfig;
use crate::connection::PgConnection;
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
            recycling_method: RecyclingMethod::Fast,
        };

        let mgr = Manager::from_config(pg_config, NoTls, mgr_config);

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

        // Set runtime for timeout support
        builder = builder.runtime(Runtime::Tokio1);

        let pool = builder
            .build()
            .map_err(|e| PgError::config(format!("failed to create pool: {}", e)))?;

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

/// Configuration for the connection pool.
#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// Maximum number of connections in the pool.
    pub max_connections: usize,
    /// Minimum number of connections to keep alive.
    pub min_connections: usize,
    /// Maximum time to wait for a connection.
    pub connection_timeout: Option<Duration>,
    /// Maximum idle time before a connection is closed.
    pub idle_timeout: Option<Duration>,
    /// Maximum lifetime of a connection.
    pub max_lifetime: Option<Duration>,
    /// Size of the prepared statement cache per connection.
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
