//! `ScyllaDB` connection management.

use scylla::Session;
use std::sync::Arc;

use crate::config::ScyllaConfig;
use crate::error::{ScyllaError, ScyllaResult};

/// A wrapper around a `ScyllaDB` session.
#[derive(Clone)]
pub struct ScyllaConnection {
    session: Arc<Session>,
    config: Arc<ScyllaConfig>,
}

impl ScyllaConnection {
    /// Create a new connection from a session and config.
    pub(crate) fn new(session: Session, config: ScyllaConfig) -> Self {
        Self {
            session: Arc::new(session),
            config: Arc::new(config),
        }
    }

    /// Get a reference to the underlying session.
    #[must_use]
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// Get a reference to the configuration.
    #[must_use]
    pub fn config(&self) -> &ScyllaConfig {
        &self.config
    }

    /// Check if the connection is healthy by executing a simple query.
    pub async fn is_healthy(&self) -> bool {
        self.session
            .query_unpaged("SELECT now() FROM system.local", &[])
            .await
            .is_ok()
    }

    /// Use a specific keyspace for this connection.
    pub async fn use_keyspace(&self, keyspace: &str) -> ScyllaResult<()> {
        self.session
            .use_keyspace(keyspace, true)
            .await
            .map_err(|e| ScyllaError::Keyspace(e.to_string()))
    }

    /// Get the current keyspace.
    #[must_use]
    pub fn current_keyspace(&self) -> Option<&str> {
        self.config.default_keyspace()
    }

    /// Execute a raw CQL query.
    pub async fn execute_raw(&self, query: &str) -> ScyllaResult<scylla::QueryResult> {
        self.session
            .query_unpaged(query, &[])
            .await
            .map_err(Into::into)
    }

    /// Prepare a statement for execution.
    pub async fn prepare(
        &self,
        query: &str,
    ) -> ScyllaResult<scylla::prepared_statement::PreparedStatement> {
        self.session.prepare(query).await.map_err(Into::into)
    }
}

impl std::fmt::Debug for ScyllaConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScyllaConnection")
            .field("keyspace", &self.config.default_keyspace())
            .field("nodes", &self.config.known_nodes())
            .finish()
    }
}

/// Connect to a `ScyllaDB` cluster.
pub async fn connect(config: ScyllaConfig) -> ScyllaResult<ScyllaConnection> {
    use scylla::SessionBuilder;

    let mut builder = SessionBuilder::new()
        .known_nodes(config.known_nodes())
        .connection_timeout(config.connection_timeout());

    // Set default keyspace
    if let Some(keyspace) = config.default_keyspace() {
        builder = builder.use_keyspace(keyspace, true);
    }

    // Set authentication
    if let (Some(username), Some(password)) = (config.username(), config.password()) {
        builder = builder.user(username, password);
    }

    // Set local datacenter if specified
    if let Some(dc) = config.local_datacenter() {
        builder = builder.default_execution_profile_handle(
            scylla::execution_profile::ExecutionProfile::builder()
                .load_balancing_policy(
                    scylla::load_balancing::DefaultPolicy::builder()
                        .prefer_datacenter(dc.to_string())
                        .build(),
                )
                .build()
                .into_handle(),
        );
    }

    // Set compression
    if let Some(compression) = config.compression() {
        let compression = match compression.to_lowercase().as_str() {
            "lz4" => Some(scylla::transport::Compression::Lz4),
            "snappy" => Some(scylla::transport::Compression::Snappy),
            _ => None, // No compression
        };
        builder = builder.compression(compression);
    }

    // Build and connect
    let session = builder.build().await?;

    Ok(ScyllaConnection::new(session, config))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connection_debug() {
        // Can't fully test without a real connection, but we can test the builder
        let config = ScyllaConfig::builder()
            .known_nodes(["localhost:9042"])
            .default_keyspace("test")
            .build();

        assert_eq!(config.default_keyspace(), Some("test"));
    }
}
