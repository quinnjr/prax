//! DuckDB connection pool.
//!
//! DuckDB supports concurrent access within a single process through
//! connection pooling. This module provides a simple connection pool
//! that manages multiple connections to the same database.

use std::sync::Arc;

use parking_lot::Mutex;
use tokio::sync::Semaphore;
use tracing::{debug, info};

use crate::config::DuckDbConfig;
use crate::connection::DuckDbConnection;
use crate::error::{DuckDbError, DuckDbResult};

/// Pool configuration.
#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// Maximum number of connections.
    pub max_connections: usize,
    /// Minimum number of connections to keep open.
    pub min_connections: usize,
    /// Connection timeout in milliseconds.
    pub connection_timeout_ms: u64,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            max_connections: 10,
            min_connections: 1,
            connection_timeout_ms: 30_000,
        }
    }
}

/// A DuckDB connection pool.
///
/// Manages multiple connections to a DuckDB database for concurrent access.
#[derive(Clone)]
pub struct DuckDbPool {
    /// Database configuration.
    config: Arc<DuckDbConfig>,
    /// Pool configuration.
    pool_config: Arc<PoolConfig>,
    /// Available connections.
    connections: Arc<Mutex<Vec<DuckDbConnection>>>,
    /// Semaphore to limit concurrent connections.
    semaphore: Arc<Semaphore>,
}

impl DuckDbPool {
    /// Create a new connection pool.
    pub async fn new(config: DuckDbConfig) -> DuckDbResult<Self> {
        Self::with_pool_config(config, PoolConfig::default()).await
    }

    /// Create a new connection pool with custom pool configuration.
    pub async fn with_pool_config(
        config: DuckDbConfig,
        pool_config: PoolConfig,
    ) -> DuckDbResult<Self> {
        info!(
            max_connections = pool_config.max_connections,
            min_connections = pool_config.min_connections,
            "Creating DuckDB connection pool"
        );

        let pool = Self {
            config: Arc::new(config),
            pool_config: Arc::new(pool_config.clone()),
            connections: Arc::new(Mutex::new(Vec::new())),
            semaphore: Arc::new(Semaphore::new(pool_config.max_connections)),
        };

        // Pre-create minimum connections
        for _ in 0..pool_config.min_connections {
            let conn = pool.create_connection()?;
            pool.connections.lock().push(conn);
        }

        Ok(pool)
    }

    /// Create a builder for the pool.
    pub fn builder() -> DuckDbPoolBuilder {
        DuckDbPoolBuilder::default()
    }

    /// Get a connection from the pool.
    pub async fn get(&self) -> DuckDbResult<PooledConnection> {
        debug!("Acquiring connection from pool");

        // Acquire permit
        let permit = self
            .semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| DuckDbError::pool(format!("Failed to acquire semaphore: {}", e)))?;

        // Try to get an existing connection
        let conn = {
            let mut connections = self.connections.lock();
            connections.pop()
        };

        let conn = match conn {
            Some(c) => c,
            None => self.create_connection()?,
        };

        Ok(PooledConnection {
            conn: Some(conn),
            pool: self.clone(),
            _permit: permit,
        })
    }

    /// Create a new connection.
    fn create_connection(&self) -> DuckDbResult<DuckDbConnection> {
        debug!("Creating new DuckDB connection");
        DuckDbConnection::new(&self.config)
    }

    /// Return a connection to the pool.
    fn return_connection(&self, conn: DuckDbConnection) {
        let mut connections = self.connections.lock();
        if connections.len() < self.pool_config.max_connections {
            connections.push(conn);
        }
        // If pool is full, connection is dropped
    }

    /// Get pool status.
    pub fn status(&self) -> PoolStatus {
        let available = self.connections.lock().len();
        let permits = self.semaphore.available_permits();

        PoolStatus {
            max_connections: self.pool_config.max_connections,
            available_connections: available,
            available_permits: permits,
            in_use: self.pool_config.max_connections - permits,
        }
    }

    /// Get a reference to the database configuration.
    pub fn config(&self) -> &DuckDbConfig {
        &self.config
    }

    /// Get a reference to the pool configuration.
    pub fn pool_config(&self) -> &PoolConfig {
        &self.pool_config
    }
}

impl std::fmt::Debug for DuckDbPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DuckDbPool")
            .field("status", &self.status())
            .finish()
    }
}

/// Pool status information.
#[derive(Debug, Clone)]
pub struct PoolStatus {
    /// Maximum connections in the pool.
    pub max_connections: usize,
    /// Available connections in the pool.
    pub available_connections: usize,
    /// Available permits.
    pub available_permits: usize,
    /// Connections currently in use.
    pub in_use: usize,
}

/// A connection borrowed from the pool.
///
/// When dropped, the connection is returned to the pool.
pub struct PooledConnection {
    conn: Option<DuckDbConnection>,
    pool: DuckDbPool,
    _permit: tokio::sync::OwnedSemaphorePermit,
}

impl PooledConnection {
    /// Get a reference to the underlying connection.
    pub fn connection(&self) -> &DuckDbConnection {
        self.conn.as_ref().expect("Connection already taken")
    }

    /// Query and return all rows as JSON.
    pub async fn query(
        &self,
        sql: &str,
        params: &[prax_query::filter::FilterValue],
    ) -> DuckDbResult<Vec<serde_json::Value>> {
        let conn = self.connection().clone();
        let sql = sql.to_string();
        let params = params.to_vec();

        tokio::task::spawn_blocking(move || conn.query(&sql, &params))
            .await
            .map_err(|e| DuckDbError::internal(format!("Task join error: {}", e)))?
    }

    /// Query and return the first row.
    pub async fn query_one(
        &self,
        sql: &str,
        params: &[prax_query::filter::FilterValue],
    ) -> DuckDbResult<serde_json::Value> {
        let conn = self.connection().clone();
        let sql = sql.to_string();
        let params = params.to_vec();

        tokio::task::spawn_blocking(move || conn.query_one(&sql, &params))
            .await
            .map_err(|e| DuckDbError::internal(format!("Task join error: {}", e)))?
    }

    /// Query and return the first row or None.
    pub async fn query_optional(
        &self,
        sql: &str,
        params: &[prax_query::filter::FilterValue],
    ) -> DuckDbResult<Option<serde_json::Value>> {
        let conn = self.connection().clone();
        let sql = sql.to_string();
        let params = params.to_vec();

        tokio::task::spawn_blocking(move || conn.query_optional(&sql, &params))
            .await
            .map_err(|e| DuckDbError::internal(format!("Task join error: {}", e)))?
    }

    /// Query and return typed row snapshots. Drives the synchronous
    /// `query_rows` on the inner connection from a `spawn_blocking`
    /// so the caller's runtime isn't stalled on DuckDB's blocking API.
    pub async fn query_rows(
        &self,
        sql: &str,
        params: &[prax_query::filter::FilterValue],
    ) -> DuckDbResult<Vec<crate::row_ref::DuckDbRowRef>> {
        let conn = self.connection().clone();
        let sql = sql.to_string();
        let params = params.to_vec();
        tokio::task::spawn_blocking(move || conn.query_rows(&sql, &params))
            .await
            .map_err(|e| DuckDbError::internal(format!("Task join error: {}", e)))?
    }

    /// Execute a statement and return affected rows.
    pub async fn execute(
        &self,
        sql: &str,
        params: &[prax_query::filter::FilterValue],
    ) -> DuckDbResult<usize> {
        let conn = self.connection().clone();
        let sql = sql.to_string();
        let params = params.to_vec();

        tokio::task::spawn_blocking(move || conn.execute(&sql, &params))
            .await
            .map_err(|e| DuckDbError::internal(format!("Task join error: {}", e)))?
    }

    /// Execute a batch of SQL statements.
    pub async fn execute_batch(&self, sql: &str) -> DuckDbResult<()> {
        let conn = self.connection().clone();
        let sql = sql.to_string();

        tokio::task::spawn_blocking(move || conn.execute_batch(&sql))
            .await
            .map_err(|e| DuckDbError::internal(format!("Task join error: {}", e)))?
    }

    /// Copy data to Parquet.
    pub async fn copy_to_parquet(&self, query: &str, path: &str) -> DuckDbResult<()> {
        let conn = self.connection().clone();
        let query = query.to_string();
        let path = path.to_string();

        tokio::task::spawn_blocking(move || conn.copy_to_parquet(&query, &path))
            .await
            .map_err(|e| DuckDbError::internal(format!("Task join error: {}", e)))?
    }

    /// Copy data to CSV.
    pub async fn copy_to_csv(&self, query: &str, path: &str, header: bool) -> DuckDbResult<()> {
        let conn = self.connection().clone();
        let query = query.to_string();
        let path = path.to_string();

        tokio::task::spawn_blocking(move || conn.copy_to_csv(&query, &path, header))
            .await
            .map_err(|e| DuckDbError::internal(format!("Task join error: {}", e)))?
    }

    /// Query a Parquet file.
    pub async fn query_parquet(&self, path: &str) -> DuckDbResult<Vec<serde_json::Value>> {
        let conn = self.connection().clone();
        let path = path.to_string();

        tokio::task::spawn_blocking(move || conn.query_parquet(&path))
            .await
            .map_err(|e| DuckDbError::internal(format!("Task join error: {}", e)))?
    }

    /// Query a CSV file.
    pub async fn query_csv(
        &self,
        path: &str,
        header: bool,
    ) -> DuckDbResult<Vec<serde_json::Value>> {
        let conn = self.connection().clone();
        let path = path.to_string();

        tokio::task::spawn_blocking(move || conn.query_csv(&path, header))
            .await
            .map_err(|e| DuckDbError::internal(format!("Task join error: {}", e)))?
    }

    /// Query a JSON file.
    pub async fn query_json(&self, path: &str) -> DuckDbResult<Vec<serde_json::Value>> {
        let conn = self.connection().clone();
        let path = path.to_string();

        tokio::task::spawn_blocking(move || conn.query_json(&path))
            .await
            .map_err(|e| DuckDbError::internal(format!("Task join error: {}", e)))?
    }
}

impl Drop for PooledConnection {
    fn drop(&mut self) {
        if let Some(conn) = self.conn.take() {
            self.pool.return_connection(conn);
        }
    }
}

impl std::fmt::Debug for PooledConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PooledConnection").finish_non_exhaustive()
    }
}

/// Builder for DuckDB connection pool.
#[derive(Debug, Default)]
pub struct DuckDbPoolBuilder {
    config: Option<DuckDbConfig>,
    pool_config: PoolConfig,
}

impl DuckDbPoolBuilder {
    /// Create a new builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the database configuration.
    pub fn config(mut self, config: DuckDbConfig) -> Self {
        self.config = Some(config);
        self
    }

    /// Set the database path.
    pub fn path(mut self, path: &str) -> Self {
        self.config = Some(DuckDbConfig::from_path(path).unwrap_or_default());
        self
    }

    /// Use an in-memory database.
    pub fn in_memory(mut self) -> Self {
        self.config = Some(DuckDbConfig::in_memory());
        self
    }

    /// Set the database URL.
    pub fn url(mut self, url: &str) -> Self {
        self.config = DuckDbConfig::from_url(url).ok();
        self
    }

    /// Set maximum connections.
    pub fn max_connections(mut self, max: usize) -> Self {
        self.pool_config.max_connections = max;
        self
    }

    /// Set minimum connections.
    pub fn min_connections(mut self, min: usize) -> Self {
        self.pool_config.min_connections = min;
        self
    }

    /// Set connection timeout in milliseconds.
    pub fn connection_timeout_ms(mut self, timeout: u64) -> Self {
        self.pool_config.connection_timeout_ms = timeout;
        self
    }

    /// Build the pool.
    pub async fn build(self) -> DuckDbResult<DuckDbPool> {
        let config = self
            .config
            .ok_or_else(|| DuckDbError::config("Database configuration required"))?;

        DuckDbPool::with_pool_config(config, self.pool_config).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_pool_creation() {
        let pool = DuckDbPool::new(DuckDbConfig::in_memory()).await.unwrap();
        let status = pool.status();
        assert_eq!(status.max_connections, 10);
        assert!(status.available_connections >= 1);
    }

    #[tokio::test]
    async fn test_pool_get_connection() {
        let pool = DuckDbPool::new(DuckDbConfig::in_memory()).await.unwrap();
        let conn = pool.get().await.unwrap();

        // Execute a simple query
        let results = conn.query("SELECT 1 as value", &[]).await.unwrap();
        assert_eq!(results.len(), 1);
    }

    #[tokio::test]
    async fn test_pool_builder() {
        let pool = DuckDbPool::builder()
            .in_memory()
            .max_connections(5)
            .min_connections(2)
            .build()
            .await
            .unwrap();

        let status = pool.status();
        assert_eq!(status.max_connections, 5);
        assert!(status.available_connections >= 2);
    }

    #[tokio::test]
    async fn test_connection_returned_to_pool() {
        let pool = DuckDbPool::builder()
            .in_memory()
            .max_connections(2)
            .min_connections(0)
            .build()
            .await
            .unwrap();

        let initial_permits = pool.semaphore.available_permits();

        {
            let _conn = pool.get().await.unwrap();
            assert_eq!(pool.semaphore.available_permits(), initial_permits - 1);
        }

        // Connection should be returned
        assert_eq!(pool.semaphore.available_permits(), initial_permits);
    }
}
