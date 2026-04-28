//! Connection pool handle for a Cassandra cluster.

use std::sync::Arc;

use crate::config::CassandraConfig;
use crate::connection::CassandraConnection;
use crate::error::CassandraResult;

/// Public pool handle for executing queries against a Cassandra cluster.
///
/// cdrs-tokio manages its own per-node connection pool; this wrapper
/// exposes a stable type for the prax-cassandra public API. `Clone` is
/// cheap (the underlying `Arc<CassandraConnection>` is reference-counted)
/// so callers — including the `QueryEngine` trait impl — can clone the
/// pool into each per-query future without contention.
#[derive(Clone)]
pub struct CassandraPool {
    connection: Arc<CassandraConnection>,
}

impl CassandraPool {
    /// Connect to the cluster with the given configuration.
    pub async fn connect(config: CassandraConfig) -> CassandraResult<Self> {
        let connection = CassandraConnection::connect(config).await?;
        Ok(Self {
            connection: Arc::new(connection),
        })
    }

    /// Close the pool, terminating all connections.
    ///
    /// This consumes the pool so further queries produce a type error at
    /// compile time.
    pub async fn close(self) -> CassandraResult<()> {
        // cdrs-tokio sessions close when dropped; the Arc drop cascades.
        Ok(())
    }

    /// Borrow the underlying connection.
    pub fn connection(&self) -> &CassandraConnection {
        &self.connection
    }

    /// Clone the inner Arc for sharing across tasks.
    pub fn shared(&self) -> Arc<CassandraConnection> {
        Arc::clone(&self.connection)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_pool_connect_returns_error_without_cluster() {
        let config = CassandraConfig::builder()
            .known_nodes(["127.0.0.1:9042".to_string()])
            .build();

        let result = CassandraPool::connect(config).await;
        assert!(result.is_err());
    }
}
