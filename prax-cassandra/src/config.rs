//! Configuration for a Cassandra connection.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::auth::SaslMechanism;

/// Complete configuration for connecting to a Cassandra cluster.
#[derive(Debug, Clone)]
pub struct CassandraConfig {
    /// Contact points (e.g., "10.0.0.1:9042"). At least one required.
    pub known_nodes: Vec<String>,
    /// Optional default keyspace to use after connecting.
    pub default_keyspace: Option<String>,
    /// Optional authentication configuration.
    pub auth: Option<CassandraAuth>,
    /// Optional TLS configuration.
    pub tls: Option<TlsConfig>,
    /// Target number of connections per node. Default: 4.
    pub pool_size: usize,
    /// Timeout for establishing a connection.
    pub connection_timeout: Duration,
    /// Timeout for individual queries.
    pub request_timeout: Duration,
    /// Default consistency level.
    pub consistency: Consistency,
    /// Retry policy used for failed queries.
    pub retry_policy: RetryPolicyKind,
}

impl CassandraConfig {
    /// Begin building a new configuration.
    pub fn builder() -> CassandraConfigBuilder {
        CassandraConfigBuilder::default()
    }
}

/// Builder for [`CassandraConfig`].
#[derive(Debug, Default)]
pub struct CassandraConfigBuilder {
    known_nodes: Vec<String>,
    default_keyspace: Option<String>,
    auth: Option<CassandraAuth>,
    tls: Option<TlsConfig>,
    pool_size: Option<usize>,
    connection_timeout: Option<Duration>,
    request_timeout: Option<Duration>,
    consistency: Option<Consistency>,
    retry_policy: Option<RetryPolicyKind>,
}

impl CassandraConfigBuilder {
    /// Set contact points.
    pub fn known_nodes(mut self, nodes: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.known_nodes = nodes.into_iter().map(Into::into).collect();
        self
    }

    /// Set the default keyspace.
    pub fn default_keyspace(mut self, keyspace: impl Into<String>) -> Self {
        self.default_keyspace = Some(keyspace.into());
        self
    }

    /// Set the authentication configuration.
    pub fn auth(mut self, auth: CassandraAuth) -> Self {
        self.auth = Some(auth);
        self
    }

    /// Set the TLS configuration.
    pub fn tls(mut self, tls: TlsConfig) -> Self {
        self.tls = Some(tls);
        self
    }

    /// Set the per-node connection pool size.
    pub fn pool_size(mut self, size: usize) -> Self {
        self.pool_size = Some(size);
        self
    }

    /// Set the connection timeout.
    pub fn connection_timeout(mut self, timeout: Duration) -> Self {
        self.connection_timeout = Some(timeout);
        self
    }

    /// Set the request timeout.
    pub fn request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = Some(timeout);
        self
    }

    /// Set the default consistency level.
    pub fn consistency(mut self, consistency: Consistency) -> Self {
        self.consistency = Some(consistency);
        self
    }

    /// Set the retry policy kind.
    pub fn retry_policy(mut self, policy: RetryPolicyKind) -> Self {
        self.retry_policy = Some(policy);
        self
    }

    /// Finalize the configuration with defaults for any unset fields.
    pub fn build(self) -> CassandraConfig {
        CassandraConfig {
            known_nodes: self.known_nodes,
            default_keyspace: self.default_keyspace,
            auth: self.auth,
            tls: self.tls,
            pool_size: self.pool_size.unwrap_or(4),
            connection_timeout: self.connection_timeout.unwrap_or(Duration::from_secs(5)),
            request_timeout: self.request_timeout.unwrap_or(Duration::from_secs(30)),
            consistency: self.consistency.unwrap_or(Consistency::LocalQuorum),
            retry_policy: self.retry_policy.unwrap_or(RetryPolicyKind::Default),
        }
    }
}

/// Authentication configuration.
#[derive(Debug, Clone)]
pub enum CassandraAuth {
    /// Username and password via PLAIN SASL.
    Password {
        /// Username.
        username: String,
        /// Password.
        password: String,
    },
    /// Custom SASL mechanism.
    Sasl(Arc<dyn SaslMechanism>),
}

/// TLS configuration.
#[derive(Debug, Clone, Default)]
pub struct TlsConfig {
    /// Path to the CA certificate file.
    pub ca_cert: Option<PathBuf>,
    /// Path to the client certificate file.
    pub client_cert: Option<PathBuf>,
    /// Path to the client key file.
    pub client_key: Option<PathBuf>,
    /// Whether to verify the server hostname (default: true).
    pub verify_hostname: bool,
}

/// CQL consistency level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Consistency {
    Any,
    One,
    Two,
    Three,
    Quorum,
    All,
    LocalQuorum,
    EachQuorum,
    LocalOne,
    Serial,
    LocalSerial,
}

/// Retry policy kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryPolicyKind {
    /// Default policy: retry on timeout with same consistency.
    Default,
    /// Downgrading policy: retry at a lower consistency on timeout.
    Downgrading,
    /// Never retry.
    Never,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder_defaults() {
        let config = CassandraConfig::builder()
            .known_nodes(["127.0.0.1:9042".to_string()])
            .build();

        assert_eq!(config.known_nodes, vec!["127.0.0.1:9042"]);
        assert_eq!(config.pool_size, 4);
        assert_eq!(config.connection_timeout, Duration::from_secs(5));
        assert_eq!(config.request_timeout, Duration::from_secs(30));
        assert_eq!(config.consistency, Consistency::LocalQuorum);
        assert_eq!(config.retry_policy, RetryPolicyKind::Default);
        assert!(config.auth.is_none());
        assert!(config.tls.is_none());
        assert!(config.default_keyspace.is_none());
    }

    #[test]
    fn test_builder_with_all_options() {
        let config = CassandraConfig::builder()
            .known_nodes(["node1:9042".to_string(), "node2:9042".to_string()])
            .default_keyspace("myapp")
            .auth(CassandraAuth::Password {
                username: "u".into(),
                password: "p".into(),
            })
            .pool_size(16)
            .connection_timeout(Duration::from_secs(10))
            .request_timeout(Duration::from_secs(60))
            .consistency(Consistency::Quorum)
            .retry_policy(RetryPolicyKind::Never)
            .build();

        assert_eq!(config.known_nodes.len(), 2);
        assert_eq!(config.default_keyspace.as_deref(), Some("myapp"));
        assert!(matches!(config.auth, Some(CassandraAuth::Password { .. })));
        assert_eq!(config.pool_size, 16);
        assert_eq!(config.consistency, Consistency::Quorum);
        assert_eq!(config.retry_policy, RetryPolicyKind::Never);
    }

    #[test]
    fn test_tls_config_default() {
        let tls = TlsConfig::default();
        assert!(tls.ca_cert.is_none());
        assert!(!tls.verify_hostname);
    }
}
