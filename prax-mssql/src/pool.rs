//! Connection pool for Microsoft SQL Server.

use std::sync::Arc;
use std::time::Duration;

use bb8::{Pool, PooledConnection};
use bb8_tiberius::ConnectionManager;
use tracing::{debug, info};

use crate::config::MssqlConfig;
use crate::connection::MssqlConnection;
use crate::error::{MssqlError, MssqlResult};

/// Type alias for the BB8 pool with Tiberius.
type TiberiusPool = Pool<ConnectionManager>;

/// A connection pool for Microsoft SQL Server.
#[derive(Clone)]
pub struct MssqlPool {
    inner: TiberiusPool,
    config: Arc<MssqlConfig>,
    max_size: usize,
}

impl MssqlPool {
    /// Create a new connection pool from configuration.
    pub async fn new(config: MssqlConfig) -> MssqlResult<Self> {
        Self::with_pool_config(config, PoolConfig::default()).await
    }

    /// Create a new connection pool with custom pool configuration.
    pub async fn with_pool_config(
        config: MssqlConfig,
        pool_config: PoolConfig,
    ) -> MssqlResult<Self> {
        let tiberius_config = config.to_tiberius_config()?;

        let mgr = ConnectionManager::new(tiberius_config);

        let pool = Pool::builder()
            .max_size(pool_config.max_connections as u32)
            .min_idle(Some(pool_config.min_connections as u32))
            .connection_timeout(
                pool_config
                    .connection_timeout
                    .unwrap_or(Duration::from_secs(30)),
            )
            .idle_timeout(pool_config.idle_timeout)
            .max_lifetime(pool_config.max_lifetime)
            .build(mgr)
            .await
            .map_err(|e| MssqlError::pool(format!("failed to create pool: {}", e)))?;

        info!(
            host = %config.host,
            port = %config.port,
            database = %config.database,
            max_connections = %pool_config.max_connections,
            "MSSQL connection pool created"
        );

        Ok(Self {
            inner: pool,
            config: Arc::new(config),
            max_size: pool_config.max_connections,
        })
    }

    /// Get a connection from the pool.
    pub async fn get(&self) -> MssqlResult<MssqlConnection<'_>> {
        debug!("Acquiring connection from pool");
        let client = self.inner.get().await?;
        Ok(MssqlConnection::new(client))
    }

    /// Get a raw pooled connection (for advanced use).
    pub async fn get_raw(&self) -> MssqlResult<PooledConnection<'_, ConnectionManager>> {
        let client = self.inner.get().await?;
        Ok(client)
    }

    /// Acquire a raw bb8-owned pooled connection with a `'static`
    /// lifetime. Needed by [`crate::engine::MssqlEngine::transaction`]
    /// so the pinned connection can outlive any particular stack
    /// frame — bb8's borrowed `get()` handle can't cross a closure
    /// boundary into the engine clone we hand the transaction
    /// closure. The usual pool-mode code path still uses
    /// [`MssqlPool::get`].
    pub async fn get_owned(&self) -> MssqlResult<PooledConnection<'static, ConnectionManager>> {
        let client = self.inner.get_owned().await?;
        Ok(client)
    }

    /// Get the current pool status.
    pub fn status(&self) -> PoolStatus {
        let state = self.inner.state();
        PoolStatus {
            connections: state.connections as usize,
            idle_connections: state.idle_connections as usize,
            max_size: self.max_size,
        }
    }

    /// Get the pool configuration.
    pub fn config(&self) -> &MssqlConfig {
        &self.config
    }

    /// Check if the pool is healthy by attempting to get a connection.
    pub async fn is_healthy(&self) -> bool {
        match self.inner.get().await {
            Ok(mut client) => {
                // Try a simple query to verify the connection is actually working
                client.simple_query("SELECT 1").await.is_ok()
            }
            Err(_) => false,
        }
    }

    /// Create a builder for configuring the pool.
    pub fn builder() -> MssqlPoolBuilder {
        MssqlPoolBuilder::new()
    }

    /// Warm up the connection pool by pre-establishing connections.
    pub async fn warmup(&self, count: usize) -> MssqlResult<()> {
        info!(count = count, "Warming up MSSQL connection pool");

        let count = count.min(self.max_size);
        let mut connections = Vec::with_capacity(count);

        for i in 0..count {
            match self.inner.get().await {
                Ok(mut conn) => {
                    // Validate the connection with a simple query
                    if let Err(e) = conn.simple_query("SELECT 1").await {
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

        let established = connections.len();
        drop(connections);

        info!(
            established = established,
            requested = count,
            "MSSQL connection pool warmup complete"
        );

        Ok(())
    }
}

/// Pool status information.
#[derive(Debug, Clone)]
pub struct PoolStatus {
    /// Current number of connections (including idle).
    pub connections: usize,
    /// Number of idle connections.
    pub idle_connections: usize,
    /// Maximum size of the pool.
    pub max_size: usize,
}

/// Configuration for the connection pool.
#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// Maximum number of connections in the pool.
    pub max_connections: usize,
    /// Minimum number of idle connections to keep.
    pub min_connections: usize,
    /// Maximum time to wait for a connection.
    pub connection_timeout: Option<Duration>,
    /// Maximum idle time before a connection is closed.
    pub idle_timeout: Option<Duration>,
    /// Maximum lifetime of a connection.
    pub max_lifetime: Option<Duration>,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            max_connections: 10,
            min_connections: 1,
            connection_timeout: Some(Duration::from_secs(30)),
            idle_timeout: Some(Duration::from_secs(600)), // 10 minutes
            max_lifetime: Some(Duration::from_secs(1800)), // 30 minutes
        }
    }
}

/// Builder for creating a connection pool.
#[derive(Debug, Default)]
pub struct MssqlPoolBuilder {
    config: Option<MssqlConfig>,
    connection_string: Option<String>,
    pool_config: PoolConfig,
}

impl MssqlPoolBuilder {
    /// Create a new pool builder.
    pub fn new() -> Self {
        Self {
            config: None,
            connection_string: None,
            pool_config: PoolConfig::default(),
        }
    }

    /// Set the connection string.
    pub fn connection_string(mut self, conn_str: impl Into<String>) -> Self {
        self.connection_string = Some(conn_str.into());
        self
    }

    /// Set the configuration.
    pub fn config(mut self, config: MssqlConfig) -> Self {
        self.config = Some(config);
        self
    }

    /// Set the server host.
    pub fn host(mut self, host: impl Into<String>) -> Self {
        let config = self.config.get_or_insert_with(MssqlConfig::default);
        config.host = host.into();
        self
    }

    /// Set the server port.
    pub fn port(mut self, port: u16) -> Self {
        let config = self.config.get_or_insert_with(MssqlConfig::default);
        config.port = port;
        self
    }

    /// Set the database name.
    pub fn database(mut self, database: impl Into<String>) -> Self {
        let config = self.config.get_or_insert_with(MssqlConfig::default);
        config.database = database.into();
        self
    }

    /// Set the username.
    pub fn username(mut self, username: impl Into<String>) -> Self {
        let config = self.config.get_or_insert_with(MssqlConfig::default);
        config.username = Some(username.into());
        self
    }

    /// Set the password.
    pub fn password(mut self, password: impl Into<String>) -> Self {
        let config = self.config.get_or_insert_with(MssqlConfig::default);
        config.password = Some(password.into());
        self
    }

    /// Set the maximum number of connections.
    pub fn max_connections(mut self, n: usize) -> Self {
        self.pool_config.max_connections = n;
        self
    }

    /// Set the minimum number of idle connections.
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

    /// Trust the server certificate.
    pub fn trust_cert(mut self, trust: bool) -> Self {
        let config = self.config.get_or_insert_with(MssqlConfig::default);
        config.trust_cert = trust;
        self
    }

    /// Build the connection pool.
    pub async fn build(self) -> MssqlResult<MssqlPool> {
        let config = if let Some(config) = self.config {
            config
        } else if let Some(conn_str) = self.connection_string {
            MssqlConfig::from_connection_string(conn_str)?
        } else {
            return Err(MssqlError::config(
                "no connection string or config provided",
            ));
        };

        MssqlPool::with_pool_config(config, self.pool_config).await
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
    }

    #[test]
    fn test_pool_builder() {
        let builder = MssqlPoolBuilder::new()
            .host("localhost")
            .database("test")
            .username("sa")
            .password("password")
            .max_connections(20);

        assert_eq!(builder.pool_config.max_connections, 20);
        assert!(builder.config.is_some());
    }
}
