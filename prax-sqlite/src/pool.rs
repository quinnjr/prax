//! Connection pool for SQLite.
//!
//! This module provides an optimized connection pool for SQLite databases.
//! Unlike PostgreSQL/MySQL, SQLite has unique characteristics:
//!
//! - In-memory databases: Each connection has its own isolated database
//! - File-based databases: Connections share the same database file
//!
//! For file-based databases, this pool reuses connections to avoid the
//! overhead of opening new connections (~200µs per open).

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use tokio::sync::Semaphore;
use tokio_rusqlite::Connection;
use tracing::{debug, info, trace};

use crate::config::SqliteConfig;
use crate::connection::{PooledConnection, SqliteConnection};
use crate::error::{SqliteError, SqliteResult};

/// A connection pool for SQLite.
///
/// This pool provides connection reuse for file-based SQLite databases,
/// significantly improving performance by avoiding repeated connection opens.
///
/// # Example
///
/// ```rust,ignore
/// use prax_sqlite::{SqlitePool, SqliteConfig};
///
/// let pool = SqlitePool::new(SqliteConfig::file("data.db")).await?;
/// let conn = pool.get().await?;
/// // Use connection...
/// // Connection is returned to pool when dropped
/// ```
#[derive(Clone)]
pub struct SqlitePool {
    config: Arc<SqliteConfig>,
    /// Semaphore to limit concurrent connections.
    semaphore: Arc<Semaphore>,
    /// Pool of idle connections (only for file-based databases).
    idle_connections: Arc<Mutex<VecDeque<PooledConnection>>>,
    pool_config: Arc<PoolConfig>,
    /// Statistics about pool usage.
    stats: Arc<Mutex<PoolStats>>,
}

/// Statistics about pool usage.
#[derive(Debug, Default, Clone)]
pub struct PoolStats {
    /// Number of connection reuses.
    pub reuses: u64,
    /// Number of new connections opened.
    pub opens: u64,
    /// Number of connections closed due to expiration.
    pub expirations: u64,
    /// Number of connections currently in use.
    pub in_use: usize,
}

impl SqlitePool {
    /// Create a new connection pool from configuration.
    pub async fn new(config: SqliteConfig) -> SqliteResult<Self> {
        Self::with_pool_config(config, PoolConfig::default()).await
    }

    /// Create a new connection pool with custom pool configuration.
    pub async fn with_pool_config(
        config: SqliteConfig,
        pool_config: PoolConfig,
    ) -> SqliteResult<Self> {
        info!(
            path = %config.path_str(),
            max_connections = %pool_config.max_connections,
            "SQLite connection pool created"
        );

        // Verify we can open at least one connection
        let test_conn = Self::open_connection(&config).await?;
        drop(test_conn);

        let pool = Self {
            config: Arc::new(config),
            semaphore: Arc::new(Semaphore::new(pool_config.max_connections)),
            idle_connections: Arc::new(Mutex::new(VecDeque::with_capacity(
                pool_config.max_connections,
            ))),
            pool_config: Arc::new(pool_config),
            stats: Arc::new(Mutex::new(PoolStats::default())),
        };

        // Pre-warm the pool with min_connections
        if !pool.config.path.is_memory() && pool.pool_config.min_connections > 0 {
            debug!(
                "Pre-warming pool with {} connections",
                pool.pool_config.min_connections
            );
            for _ in 0..pool.pool_config.min_connections {
                if let Ok(conn) = Self::open_connection(&pool.config).await {
                    let mut idle = pool.idle_connections.lock();
                    idle.push_back(PooledConnection::new(conn));
                }
            }
        }

        Ok(pool)
    }

    /// Open a new connection with the given configuration.
    async fn open_connection(config: &SqliteConfig) -> SqliteResult<Connection> {
        let path = config.path_str().to_string();
        let init_sql = config.init_sql();

        let conn = if config.path.is_memory() {
            Connection::open_in_memory().await?
        } else {
            Connection::open(&path).await?
        };

        // Run initialization SQL
        conn.call(move |conn| {
            conn.execute_batch(&init_sql)?;
            Ok(())
        })
        .await?;

        // Register the vector extension on every connection when the `vector`
        // feature is enabled. Registration is idempotent and per-connection
        // (every rusqlite Connection is a separate SQLite handle; for
        // in-memory databases the handle has its own isolated database).
        //
        // Soft-fails: if the shared library cannot be located, pool creation
        // still succeeds. Vector SQL functions will then be unavailable on
        // that connection and fail at query time with a clear SQLite error.
        //
        // To avoid log spam on every new connection once the library is
        // known to be missing, the warning is emitted at most once per
        // process via a Once guard. Re-running with the library on disk
        // will still produce functional connections.
        #[cfg(feature = "vector")]
        {
            use std::sync::Once;
            static WARN_ONCE: Once = Once::new();

            let _ = conn
                .call(|conn| {
                    if let Err(e) = crate::vector::register_vector_extension(conn) {
                        WARN_ONCE.call_once(|| {
                            tracing::warn!(
                                error = %e,
                                "sqlite-vector-rs extension could not be registered; \
                                 vector SQL functions will be unavailable on this connection. \
                                 Build libsqlite_vector_rs.so and set SQLITE_VECTOR_RS_LIB \
                                 or place it alongside the test/binary. \
                                 (This warning is emitted once per process.)"
                            );
                        });
                    }
                    Ok(())
                })
                .await;
        }

        Ok(conn)
    }

    /// Get a connection from the pool.
    ///
    /// For file-based databases, this will try to reuse an idle connection
    /// before opening a new one. For in-memory databases, always opens a new
    /// connection (since each connection has its own database).
    pub async fn get(&self) -> SqliteResult<SqliteConnection> {
        trace!("Acquiring connection from pool");

        // Wait for a permit (limits concurrent connections)
        let permit = self
            .semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| SqliteError::pool(format!("failed to acquire permit: {}", e)))?;

        // Update stats
        {
            let mut stats = self.stats.lock();
            stats.in_use += 1;
        }

        // For in-memory databases, always create a new connection
        // (each connection has its own separate database)
        if self.config.path.is_memory() {
            let conn = Self::open_connection(&self.config).await?;
            {
                let mut stats = self.stats.lock();
                stats.opens += 1;
            }
            return Ok(SqliteConnection::new_pooled(
                conn, permit, None, // No return channel for in-memory
            ));
        }

        // Try to get an idle connection
        let conn: Option<Connection> = {
            let mut idle = self.idle_connections.lock();

            // Clean up expired connections while searching
            while let Some(pooled) = idle.pop_front() {
                let is_expired = if let Some(lifetime) = self.pool_config.max_lifetime {
                    pooled.created_at.elapsed() > lifetime
                } else {
                    false
                };
                let is_idle_expired = if let Some(timeout) = self.pool_config.idle_timeout {
                    pooled.last_used.elapsed() > timeout
                } else {
                    false
                };

                if is_expired || is_idle_expired {
                    let mut stats = self.stats.lock();
                    stats.expirations += 1;
                    // Connection will be dropped
                    continue;
                }
                // Found a valid connection
                let mut stats = self.stats.lock();
                stats.reuses += 1;
                return Ok(SqliteConnection::new_pooled(
                    pooled.conn,
                    permit,
                    Some(self.idle_connections.clone()),
                ));
            }
            None
        };

        // No idle connection available, open a new one
        if conn.is_none() {
            debug!("No idle connections, opening new connection");
            let new_conn = Self::open_connection(&self.config).await?;
            {
                let mut stats = self.stats.lock();
                stats.opens += 1;
            }
            return Ok(SqliteConnection::new_pooled(
                new_conn,
                permit,
                Some(self.idle_connections.clone()),
            ));
        }

        unreachable!()
    }

    /// Get the pool configuration.
    pub fn config(&self) -> &SqliteConfig {
        &self.config
    }

    /// Get the pool settings.
    pub fn pool_config(&self) -> &PoolConfig {
        &self.pool_config
    }

    /// Get pool statistics.
    pub fn stats(&self) -> PoolStats {
        self.stats.lock().clone()
    }

    /// Reset pool statistics.
    pub fn reset_stats(&self) {
        let mut stats = self.stats.lock();
        *stats = PoolStats::default();
    }

    /// Check if the pool is healthy by attempting to get a connection.
    pub async fn is_healthy(&self) -> bool {
        match Self::open_connection(&self.config).await {
            Ok(conn) => {
                let result = conn
                    .call(|conn| {
                        conn.execute("SELECT 1", [])?;
                        Ok(())
                    })
                    .await;
                result.is_ok()
            }
            Err(_) => false,
        }
    }

    /// Get the number of available permits (potential concurrent connections).
    pub fn available_permits(&self) -> usize {
        self.semaphore.available_permits()
    }

    /// Get the number of idle connections in the pool.
    pub fn idle_count(&self) -> usize {
        self.idle_connections.lock().len()
    }

    /// Create a builder for configuring the pool.
    pub fn builder() -> SqlitePoolBuilder {
        SqlitePoolBuilder::new()
    }
}

/// Configuration for the connection pool.
#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// Maximum number of concurrent connections.
    pub max_connections: usize,
    /// Minimum number of connections to keep in the pool.
    pub min_connections: usize,
    /// Connection timeout.
    pub connection_timeout: Option<Duration>,
    /// Maximum idle time before a connection is closed.
    pub idle_timeout: Option<Duration>,
    /// Maximum lifetime of a connection before it's recycled.
    pub max_lifetime: Option<Duration>,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            max_connections: 5, // SQLite benefits from fewer connections
            min_connections: 1,
            connection_timeout: Some(Duration::from_secs(30)),
            idle_timeout: Some(Duration::from_secs(300)), // 5 minutes
            max_lifetime: Some(Duration::from_secs(1800)), // 30 minutes
        }
    }
}

/// Builder for creating a connection pool.
#[derive(Debug, Default)]
pub struct SqlitePoolBuilder {
    config: Option<SqliteConfig>,
    url: Option<String>,
    pool_config: PoolConfig,
}

impl SqlitePoolBuilder {
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
    pub fn config(mut self, config: SqliteConfig) -> Self {
        self.config = Some(config);
        self
    }

    /// Set the maximum number of connections.
    pub fn max_connections(mut self, n: usize) -> Self {
        self.pool_config.max_connections = n;
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

    /// Build the connection pool.
    pub async fn build(self) -> SqliteResult<SqlitePool> {
        let config = if let Some(config) = self.config {
            config
        } else if let Some(url) = self.url {
            SqliteConfig::from_url(url)?
        } else {
            return Err(SqliteError::config("no database URL or config provided"));
        };

        SqlitePool::with_pool_config(config, self.pool_config).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pool_config_default() {
        let config = PoolConfig::default();
        assert_eq!(config.max_connections, 5);
    }

    #[test]
    fn test_pool_builder() {
        let builder = SqlitePoolBuilder::new()
            .url("sqlite::memory:")
            .max_connections(10);

        assert!(builder.url.is_some());
        assert_eq!(builder.pool_config.max_connections, 10);
    }

    #[tokio::test]
    async fn test_pool_memory() {
        let pool = SqlitePool::new(SqliteConfig::memory()).await.unwrap();
        // For in-memory databases, is_healthy opens a new database
        // so we just verify the pool was created successfully
        assert!(pool.available_permits() > 0);
    }

    #[tokio::test]
    async fn test_pool_get_connection() {
        let pool = SqlitePool::new(SqliteConfig::memory()).await.unwrap();
        let conn = pool.get().await;
        assert!(conn.is_ok());
    }
}
