//! ScyllaDB connection pool.
//!
//! Note: The ScyllaDB Rust driver (`scylla`) already includes built-in connection
//! pooling and automatic reconnection. This module provides a higher-level wrapper
//! that integrates with the Prax ORM ecosystem.

use parking_lot::RwLock;
use scylla::Session;
use std::sync::Arc;

use crate::config::ScyllaConfig;
use crate::connection::{ScyllaConnection, connect};
use crate::engine::ScyllaEngine;
#[allow(unused_imports)]
use crate::error::ScyllaError;
use crate::error::ScyllaResult;

/// A connection pool for ScyllaDB.
///
/// The ScyllaDB driver already handles connection pooling internally,
/// managing connections to all nodes in the cluster. This pool provides
/// a higher-level interface for acquiring connections and managing
/// prepared statements.
#[derive(Clone)]
pub struct ScyllaPool {
    connection: Arc<ScyllaConnection>,
    config: Arc<ScyllaConfig>,
    /// Cache of prepared statements
    prepared_cache: Arc<
        RwLock<std::collections::HashMap<String, scylla::prepared_statement::PreparedStatement>>,
    >,
}

impl ScyllaPool {
    /// Connect to a ScyllaDB cluster and create a pool.
    pub async fn connect(config: ScyllaConfig) -> ScyllaResult<Self> {
        let connection = connect(config.clone()).await?;

        Ok(Self {
            connection: Arc::new(connection),
            config: Arc::new(config),
            prepared_cache: Arc::new(RwLock::new(std::collections::HashMap::new())),
        })
    }

    /// Connect using a URL.
    ///
    /// URL format: `scylla://[user:pass@]host1:port1[,host2:port2,...]/keyspace[?options]`
    pub async fn from_url(url: &str) -> ScyllaResult<Self> {
        let config = ScyllaConfig::from_url(url)?;
        Self::connect(config).await
    }

    /// Get a connection from the pool.
    ///
    /// Note: This returns a clone of the shared connection since ScyllaDB
    /// sessions are inherently pooled and thread-safe.
    #[must_use]
    pub fn get(&self) -> ScyllaConnection {
        (*self.connection).clone()
    }

    /// Get a reference to the underlying session.
    #[must_use]
    pub fn session(&self) -> &Session {
        self.connection.session()
    }

    /// Get a reference to the configuration.
    #[must_use]
    pub fn config(&self) -> &ScyllaConfig {
        &self.config
    }

    /// Create a query engine from this pool.
    #[must_use]
    pub fn engine(&self) -> ScyllaEngine {
        ScyllaEngine::new(self.clone())
    }

    /// Execute a raw CQL query.
    pub async fn query(
        &self,
        query: &str,
        values: impl scylla::serialize::row::SerializeRow,
    ) -> ScyllaResult<scylla::QueryResult> {
        self.connection
            .session()
            .query_unpaged(query, values)
            .await
            .map_err(Into::into)
    }

    /// Execute a prepared statement with caching.
    pub async fn execute<V: scylla::serialize::row::SerializeRow>(
        &self,
        query: &str,
        values: V,
    ) -> ScyllaResult<scylla::QueryResult> {
        let prepared = self.prepare(query).await?;
        self.connection
            .session()
            .execute_unpaged(&prepared, values)
            .await
            .map_err(Into::into)
    }

    /// Prepare a statement (cached).
    pub async fn prepare(
        &self,
        query: &str,
    ) -> ScyllaResult<scylla::prepared_statement::PreparedStatement> {
        // Check cache first
        {
            let cache = self.prepared_cache.read();
            if let Some(stmt) = cache.get(query) {
                return Ok(stmt.clone());
            }
        }

        // Prepare and cache
        let stmt = self.connection.session().prepare(query).await?;
        {
            let mut cache = self.prepared_cache.write();
            cache.insert(query.to_string(), stmt.clone());
        }

        Ok(stmt)
    }

    /// Clear the prepared statement cache.
    pub fn clear_cache(&self) {
        let mut cache = self.prepared_cache.write();
        cache.clear();
    }

    /// Check if the pool is healthy.
    pub async fn is_healthy(&self) -> bool {
        self.connection.is_healthy().await
    }

    /// Use a specific keyspace.
    pub async fn use_keyspace(&self, keyspace: &str) -> ScyllaResult<()> {
        self.connection.use_keyspace(keyspace).await
    }

    /// Get pool statistics.
    #[must_use]
    pub fn stats(&self) -> PoolStats {
        PoolStats {
            cached_statements: self.prepared_cache.read().len(),
            known_nodes: self.config.known_nodes().len(),
        }
    }
}

impl std::fmt::Debug for ScyllaPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScyllaPool")
            .field("keyspace", &self.config.default_keyspace())
            .field("nodes", &self.config.known_nodes())
            .field("cached_statements", &self.prepared_cache.read().len())
            .finish()
    }
}

/// Statistics about the connection pool.
#[derive(Debug, Clone)]
pub struct PoolStats {
    /// Number of cached prepared statements.
    pub cached_statements: usize,
    /// Number of known nodes in the cluster.
    pub known_nodes: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pool_stats() {
        let stats = PoolStats {
            cached_statements: 10,
            known_nodes: 3,
        };
        assert_eq!(stats.cached_statements, 10);
        assert_eq!(stats.known_nodes, 3);
    }
}
