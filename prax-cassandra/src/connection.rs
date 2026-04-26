//! Connection wrapper around a cdrs-tokio Session.

use std::sync::Arc;

use crate::config::CassandraConfig;
use crate::error::CassandraResult;

/// A handle to an established Cassandra session.
///
/// Wraps a cdrs-tokio Session. cdrs-tokio manages its own internal
/// connection pool per node; this wrapper provides a stable prax-cassandra
/// type for consumers while delegating the low-level protocol work to
/// cdrs-tokio.
pub struct CassandraConnection {
    config: CassandraConfig,
    // The concrete cdrs-tokio session type requires generic parameters
    // (LoadBalancingStrategy, ConnectionManager, etc.) that are wired up
    // in `connect`. We erase those details behind an Arc<dyn> boundary.
    #[allow(dead_code)]
    session: Arc<CdrsSessionHandle>,
}

/// Internal opaque wrapper for the cdrs-tokio Session.
///
/// The cdrs-tokio Session is generic over three type parameters
/// (LoadBalancingStrategy, ConnectionManager, Transport). We erase those
/// with this wrapper so the public CassandraConnection has a stable type.
pub(crate) struct CdrsSessionHandle {
    // Populated in `connect` with the concrete cdrs-tokio Session.
    // Stored as an opaque `Box<dyn Any + Send + Sync>` for type erasure.
    #[allow(dead_code)]
    inner: Box<dyn std::any::Any + Send + Sync>,
}

impl CassandraConnection {
    /// Connect to the cluster using the provided configuration.
    ///
    /// Returns an error if the configuration is invalid or the cluster
    /// is unreachable. Runs a health check (`SELECT now() FROM system.local`)
    /// after the session is established.
    pub async fn connect(config: CassandraConfig) -> CassandraResult<Self> {
        // cdrs-tokio connection setup:
        //
        // 1. Build a NodeTcpConfigBuilder from config.known_nodes
        // 2. Attach auth (CassandraAuth::Password -> cdrs_tokio's StaticPasswordAuthenticator)
        // 3. Build cluster config via cdrs_tokio::cluster::session::TcpSessionBuilder
        // 4. Call .build().await to get a Session
        // 5. Wrap session in CdrsSessionHandle
        //
        // The exact API requires importing from cdrs_tokio::cluster::*,
        // cdrs_tokio::authenticators::*, cdrs_tokio::load_balancing::*,
        // and cdrs_tokio::cluster::session::*. See cdrs-tokio docs for details.
        //
        // Placeholder implementation until live testing is wired up in
        // a follow-up task.
        Err(crate::error::CassandraError::Connection(format!(
            "CassandraConnection::connect is not yet wired to cdrs-tokio (nodes: {:?})",
            config.known_nodes
        )))
    }

    /// Borrow the configuration this connection was built from.
    pub fn config(&self) -> &CassandraConfig {
        &self.config
    }

    /// Ping the cluster with `SELECT now() FROM system.local`.
    pub async fn ping(&self) -> CassandraResult<()> {
        // Will execute on the wrapped session once connect() is live.
        Err(crate::error::CassandraError::Connection(
            "ping requires a live cdrs-tokio session".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_connect_without_live_cluster_returns_error() {
        let config = CassandraConfig::builder()
            .known_nodes(["127.0.0.1:9042".to_string()])
            .build();

        let result = CassandraConnection::connect(config).await;
        assert!(result.is_err());
    }
}
