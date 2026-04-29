//! Connection wrapper around a cdrs-tokio Session.

use std::sync::Arc;

use cdrs_tokio::authenticators::{NoneAuthenticatorProvider, StaticPasswordAuthenticatorProvider};
use cdrs_tokio::cluster::session::{Session, SessionBuilder, TcpSessionBuilder};
use cdrs_tokio::cluster::{NodeAddress, NodeTcpConfigBuilder, TcpConnectionManager};
use cdrs_tokio::load_balancing::RoundRobinLoadBalancingStrategy;
use cdrs_tokio::transport::TransportTcp;

use crate::config::{CassandraAuth, CassandraConfig};
use crate::error::{CassandraError, CassandraResult};

/// Concrete cdrs-tokio session type used by prax-cassandra. We pin the
/// load-balancing strategy to round-robin so the outer type is
/// nameable (otherwise we'd need to box it behind `dyn Any`).
pub(crate) type CdrsSession = Session<
    TransportTcp,
    TcpConnectionManager,
    RoundRobinLoadBalancingStrategy<TransportTcp, TcpConnectionManager>,
>;

/// A handle to an established Cassandra session.
pub struct CassandraConnection {
    config: CassandraConfig,
    pub(crate) session: Arc<CdrsSession>,
}

impl CassandraConnection {
    /// Connect to the cluster using the provided configuration.
    pub async fn connect(config: CassandraConfig) -> CassandraResult<Self> {
        if config.known_nodes.is_empty() {
            return Err(CassandraError::Connection(
                "at least one contact point is required".into(),
            ));
        }

        let mut builder = NodeTcpConfigBuilder::new();
        for node in &config.known_nodes {
            let addr: NodeAddress = node.as_str().into();
            builder = builder.with_contact_point(addr);
        }
        if let Some(CassandraAuth::Password { username, password }) = &config.auth {
            builder = builder.with_authenticator_provider(Arc::new(
                StaticPasswordAuthenticatorProvider::new(username, password),
            ));
        } else {
            // Explicit no-auth is the default but make it loud so readers
            // don't wonder why the builder skipped the auth branch.
            builder = builder.with_authenticator_provider(Arc::new(NoneAuthenticatorProvider));
        }

        let node_config = builder
            .build()
            .await
            .map_err(|e| CassandraError::Connection(format!("resolve contact points: {e}")))?;

        let lb = RoundRobinLoadBalancingStrategy::<TransportTcp, TcpConnectionManager>::new();
        let session = TcpSessionBuilder::new(lb, node_config)
            .build()
            .await
            .map_err(|e| CassandraError::Connection(format!("build session: {e}")))?;

        Ok(Self {
            config,
            session: Arc::new(session),
        })
    }

    /// Borrow the configuration this connection was built from.
    pub fn config(&self) -> &CassandraConfig {
        &self.config
    }

    /// Borrow the underlying cdrs-tokio session.
    pub(crate) fn session(&self) -> &CdrsSession {
        &self.session
    }

    /// Ping the cluster with `SELECT now() FROM system.local`.
    pub async fn ping(&self) -> CassandraResult<()> {
        self.session()
            .query("SELECT now() FROM system.local")
            .await
            .map_err(|e| CassandraError::Connection(format!("ping failed: {e}")))?;
        Ok(())
    }
}

impl std::fmt::Debug for CassandraConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CassandraConnection")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_known_nodes_is_an_error() {
        // Building a connection with no contact points should fail
        // fast rather than wait for cdrs-tokio to complain. Keep
        // this as a fast unit test; the live-cluster connect path
        // is exercised by the e2e integration tests.
        let config = CassandraConfig::builder().build();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = rt.block_on(CassandraConnection::connect(config));
        assert!(result.is_err(), "expected connect to fail with no nodes");
    }
}
