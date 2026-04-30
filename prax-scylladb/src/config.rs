//! `ScyllaDB` configuration module.
//!
//! Provides configuration options for connecting to `ScyllaDB` clusters.

use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::error::{ScyllaError, ScyllaResult};

/// Configuration for `ScyllaDB` connections.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScyllaConfig {
    /// Known nodes in the cluster (host:port format).
    known_nodes: Vec<String>,

    /// Default keyspace to use.
    default_keyspace: Option<String>,

    /// Username for authentication.
    username: Option<String>,

    /// Password for authentication.
    password: Option<String>,

    /// Connection timeout in seconds.
    connection_timeout_secs: u64,

    /// Request timeout in seconds.
    request_timeout_secs: u64,

    /// Maximum number of connections per node.
    pool_size: usize,

    /// Datacenter for local consistency levels.
    local_datacenter: Option<String>,

    /// Enable SSL/TLS.
    ssl_enabled: bool,

    /// Application name for identification.
    application_name: Option<String>,

    /// Compression algorithm (lz4, snappy, or none).
    compression: Option<String>,

    /// Consistency level for queries.
    consistency: ConsistencyLevel,

    /// Serial consistency level for LWT.
    serial_consistency: Option<SerialConsistencyLevel>,
}

/// CQL consistency levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ConsistencyLevel {
    /// Any node.
    Any,
    /// Single node.
    One,
    /// Two nodes.
    Two,
    /// Three nodes.
    Three,
    /// Quorum of nodes.
    #[default]
    Quorum,
    /// All nodes.
    All,
    /// Local quorum.
    LocalQuorum,
    /// Each quorum.
    EachQuorum,
    /// Local one.
    LocalOne,
}

/// Serial consistency levels for Lightweight Transactions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SerialConsistencyLevel {
    /// Serial consistency.
    #[default]
    Serial,
    /// Local serial consistency.
    LocalSerial,
}

impl ScyllaConfig {
    /// Create a new configuration builder.
    #[must_use]
    pub fn builder() -> ScyllaConfigBuilder {
        ScyllaConfigBuilder::default()
    }

    /// Parse configuration from a URL.
    ///
    /// URL format: `scylla://[user:pass@]host1:port1[,host2:port2,...]/keyspace[?options]`
    ///
    /// # Examples
    ///
    /// ```rust
    /// use prax_scylladb::ScyllaConfig;
    ///
    /// // Simple connection
    /// let config = ScyllaConfig::from_url("scylla://localhost:9042/my_keyspace").unwrap();
    ///
    /// // With authentication
    /// let config = ScyllaConfig::from_url("scylla://user:pass@localhost:9042/keyspace").unwrap();
    ///
    /// // Multiple nodes
    /// let config = ScyllaConfig::from_url("scylla://node1:9042,node2:9042/keyspace").unwrap();
    /// ```
    pub fn from_url(url: &str) -> ScyllaResult<Self> {
        let url = url.trim();

        // Remove scheme
        let rest = url
            .strip_prefix("scylla://")
            .or_else(|| url.strip_prefix("cassandra://"))
            .ok_or_else(|| {
                ScyllaError::Configuration("URL must start with scylla:// or cassandra://".into())
            })?;

        let mut builder = ScyllaConfigBuilder::default();

        // Parse authentication if present
        let (_auth_rest, rest) = if let Some(at_pos) = rest.find('@') {
            let auth = &rest[..at_pos];
            if let Some(colon) = auth.find(':') {
                builder.username = Some(auth[..colon].to_string());
                builder.password = Some(auth[colon + 1..].to_string());
            }
            (true, &rest[at_pos + 1..])
        } else {
            (false, rest)
        };

        // Parse hosts and keyspace
        let (hosts_part, keyspace) = if let Some(slash_pos) = rest.find('/') {
            let hosts = &rest[..slash_pos];
            let path = &rest[slash_pos + 1..];
            // Remove query string if present
            let keyspace = path.split('?').next().unwrap_or(path);
            (hosts, Some(keyspace.to_string()))
        } else {
            // Remove query string if present
            let hosts = rest.split('?').next().unwrap_or(rest);
            (hosts, None)
        };

        // Parse hosts
        let nodes: Vec<String> = hosts_part
            .split(',')
            .map(|s| {
                let s = s.trim();
                // Add default port if not present
                if s.contains(':') {
                    s.to_string()
                } else {
                    format!("{s}:9042")
                }
            })
            .collect();

        if nodes.is_empty() || nodes.iter().any(String::is_empty) {
            return Err(ScyllaError::Configuration(
                "At least one node must be specified".into(),
            ));
        }

        builder.known_nodes = nodes;
        builder.default_keyspace = keyspace;

        // Parse query parameters if present
        if let Some(query_start) = rest.find('?') {
            let query = &rest[query_start + 1..];
            for param in query.split('&') {
                if let Some(eq_pos) = param.find('=') {
                    let key = &param[..eq_pos];
                    let value = &param[eq_pos + 1..];
                    match key {
                        "timeout" => {
                            if let Ok(secs) = value.parse() {
                                builder.request_timeout_secs = secs;
                            }
                        }
                        "pool_size" => {
                            if let Ok(size) = value.parse() {
                                builder.pool_size = size;
                            }
                        }
                        "datacenter" | "dc" => {
                            builder.local_datacenter = Some(value.to_string());
                        }
                        "ssl" => {
                            builder.ssl_enabled = value == "true" || value == "1";
                        }
                        "compression" => {
                            builder.compression = Some(value.to_string());
                        }
                        "consistency" => {
                            builder.consistency = match value.to_uppercase().as_str() {
                                "ANY" => ConsistencyLevel::Any,
                                "ONE" => ConsistencyLevel::One,
                                "TWO" => ConsistencyLevel::Two,
                                "THREE" => ConsistencyLevel::Three,
                                "QUORUM" => ConsistencyLevel::Quorum,
                                "ALL" => ConsistencyLevel::All,
                                "LOCAL_QUORUM" => ConsistencyLevel::LocalQuorum,
                                "EACH_QUORUM" => ConsistencyLevel::EachQuorum,
                                "LOCAL_ONE" => ConsistencyLevel::LocalOne,
                                _ => ConsistencyLevel::Quorum,
                            };
                        }
                        _ => {}
                    }
                }
            }
        }

        Ok(builder.build())
    }

    /// Get known nodes.
    #[must_use]
    pub fn known_nodes(&self) -> &[String] {
        &self.known_nodes
    }

    /// Get default keyspace.
    #[must_use]
    pub fn default_keyspace(&self) -> Option<&str> {
        self.default_keyspace.as_deref()
    }

    /// Get username.
    #[must_use]
    pub fn username(&self) -> Option<&str> {
        self.username.as_deref()
    }

    /// Get password.
    #[must_use]
    pub fn password(&self) -> Option<&str> {
        self.password.as_deref()
    }

    /// Get connection timeout.
    #[must_use]
    pub fn connection_timeout(&self) -> Duration {
        Duration::from_secs(self.connection_timeout_secs)
    }

    /// Get request timeout.
    #[must_use]
    pub fn request_timeout(&self) -> Duration {
        Duration::from_secs(self.request_timeout_secs)
    }

    /// Get pool size.
    #[must_use]
    pub fn pool_size(&self) -> usize {
        self.pool_size
    }

    /// Get local datacenter.
    #[must_use]
    pub fn local_datacenter(&self) -> Option<&str> {
        self.local_datacenter.as_deref()
    }

    /// Check if SSL is enabled.
    #[must_use]
    pub fn ssl_enabled(&self) -> bool {
        self.ssl_enabled
    }

    /// Get application name.
    #[must_use]
    pub fn application_name(&self) -> Option<&str> {
        self.application_name.as_deref()
    }

    /// Get compression algorithm.
    #[must_use]
    pub fn compression(&self) -> Option<&str> {
        self.compression.as_deref()
    }

    /// Get consistency level.
    #[must_use]
    pub fn consistency(&self) -> ConsistencyLevel {
        self.consistency
    }

    /// Get serial consistency level.
    #[must_use]
    pub fn serial_consistency(&self) -> Option<SerialConsistencyLevel> {
        self.serial_consistency
    }
}

impl Default for ScyllaConfig {
    fn default() -> Self {
        Self {
            known_nodes: vec!["127.0.0.1:9042".to_string()],
            default_keyspace: None,
            username: None,
            password: None,
            connection_timeout_secs: 5,
            request_timeout_secs: 12,
            pool_size: 4,
            local_datacenter: None,
            ssl_enabled: false,
            application_name: Some("prax-scylladb".to_string()),
            compression: None,
            consistency: ConsistencyLevel::Quorum,
            serial_consistency: None,
        }
    }
}

/// Builder for `ScyllaConfig`.
#[derive(Debug, Default)]
pub struct ScyllaConfigBuilder {
    known_nodes: Vec<String>,
    default_keyspace: Option<String>,
    username: Option<String>,
    password: Option<String>,
    connection_timeout_secs: u64,
    request_timeout_secs: u64,
    pool_size: usize,
    local_datacenter: Option<String>,
    ssl_enabled: bool,
    application_name: Option<String>,
    compression: Option<String>,
    consistency: ConsistencyLevel,
    serial_consistency: Option<SerialConsistencyLevel>,
}

impl ScyllaConfigBuilder {
    /// Set known nodes.
    #[must_use]
    pub fn known_nodes<I, S>(mut self, nodes: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.known_nodes = nodes.into_iter().map(Into::into).collect();
        self
    }

    /// Add a known node.
    #[must_use]
    pub fn add_node<S: Into<String>>(mut self, node: S) -> Self {
        self.known_nodes.push(node.into());
        self
    }

    /// Set default keyspace.
    #[must_use]
    pub fn default_keyspace<S: Into<String>>(mut self, keyspace: S) -> Self {
        self.default_keyspace = Some(keyspace.into());
        self
    }

    /// Set username for authentication.
    #[must_use]
    pub fn username<S: Into<String>>(mut self, username: S) -> Self {
        self.username = Some(username.into());
        self
    }

    /// Set password for authentication.
    #[must_use]
    pub fn password<S: Into<String>>(mut self, password: S) -> Self {
        self.password = Some(password.into());
        self
    }

    /// Set connection timeout in seconds.
    #[must_use]
    pub fn connection_timeout_secs(mut self, secs: u64) -> Self {
        self.connection_timeout_secs = secs;
        self
    }

    /// Set request timeout in seconds.
    #[must_use]
    pub fn request_timeout_secs(mut self, secs: u64) -> Self {
        self.request_timeout_secs = secs;
        self
    }

    /// Set pool size per node.
    #[must_use]
    pub fn pool_size(mut self, size: usize) -> Self {
        self.pool_size = size;
        self
    }

    /// Set local datacenter.
    #[must_use]
    pub fn local_datacenter<S: Into<String>>(mut self, dc: S) -> Self {
        self.local_datacenter = Some(dc.into());
        self
    }

    /// Enable SSL/TLS.
    #[must_use]
    pub fn ssl_enabled(mut self, enabled: bool) -> Self {
        self.ssl_enabled = enabled;
        self
    }

    /// Set application name.
    #[must_use]
    pub fn application_name<S: Into<String>>(mut self, name: S) -> Self {
        self.application_name = Some(name.into());
        self
    }

    /// Set compression algorithm.
    #[must_use]
    pub fn compression<S: Into<String>>(mut self, compression: S) -> Self {
        self.compression = Some(compression.into());
        self
    }

    /// Set consistency level.
    #[must_use]
    pub fn consistency(mut self, consistency: ConsistencyLevel) -> Self {
        self.consistency = consistency;
        self
    }

    /// Set serial consistency level.
    #[must_use]
    pub fn serial_consistency(mut self, consistency: SerialConsistencyLevel) -> Self {
        self.serial_consistency = Some(consistency);
        self
    }

    /// Build the configuration.
    #[must_use]
    pub fn build(self) -> ScyllaConfig {
        ScyllaConfig {
            known_nodes: if self.known_nodes.is_empty() {
                vec!["127.0.0.1:9042".to_string()]
            } else {
                self.known_nodes
            },
            default_keyspace: self.default_keyspace,
            username: self.username,
            password: self.password,
            connection_timeout_secs: if self.connection_timeout_secs == 0 {
                5
            } else {
                self.connection_timeout_secs
            },
            request_timeout_secs: if self.request_timeout_secs == 0 {
                12
            } else {
                self.request_timeout_secs
            },
            pool_size: if self.pool_size == 0 {
                4
            } else {
                self.pool_size
            },
            local_datacenter: self.local_datacenter,
            ssl_enabled: self.ssl_enabled,
            application_name: self.application_name,
            compression: self.compression,
            consistency: self.consistency,
            serial_consistency: self.serial_consistency,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = ScyllaConfig::default();
        assert_eq!(config.known_nodes(), &["127.0.0.1:9042"]);
        assert_eq!(config.pool_size(), 4);
        assert_eq!(config.connection_timeout(), Duration::from_secs(5));
        assert_eq!(config.consistency(), ConsistencyLevel::Quorum);
    }

    #[test]
    fn test_builder() {
        let config = ScyllaConfig::builder()
            .known_nodes(["node1:9042", "node2:9042"])
            .default_keyspace("test_ks")
            .username("user")
            .password("pass")
            .pool_size(8)
            .consistency(ConsistencyLevel::LocalQuorum)
            .build();

        assert_eq!(config.known_nodes().len(), 2);
        assert_eq!(config.default_keyspace(), Some("test_ks"));
        assert_eq!(config.username(), Some("user"));
        assert_eq!(config.password(), Some("pass"));
        assert_eq!(config.pool_size(), 8);
        assert_eq!(config.consistency(), ConsistencyLevel::LocalQuorum);
    }

    #[test]
    fn test_from_url_simple() {
        let config = ScyllaConfig::from_url("scylla://localhost:9042/my_keyspace").unwrap();
        assert_eq!(config.known_nodes(), &["localhost:9042"]);
        assert_eq!(config.default_keyspace(), Some("my_keyspace"));
    }

    #[test]
    fn test_from_url_with_auth() {
        let config = ScyllaConfig::from_url("scylla://user:pass@localhost:9042/ks").unwrap();
        assert_eq!(config.username(), Some("user"));
        assert_eq!(config.password(), Some("pass"));
    }

    #[test]
    fn test_from_url_multiple_nodes() {
        let config =
            ScyllaConfig::from_url("scylla://node1:9042,node2:9042,node3:9042/ks").unwrap();
        assert_eq!(config.known_nodes().len(), 3);
    }

    #[test]
    fn test_from_url_with_params() {
        let config = ScyllaConfig::from_url(
            "scylla://localhost/ks?timeout=30&pool_size=16&consistency=LOCAL_QUORUM",
        )
        .unwrap();
        assert_eq!(config.request_timeout(), Duration::from_secs(30));
        assert_eq!(config.pool_size(), 16);
        assert_eq!(config.consistency(), ConsistencyLevel::LocalQuorum);
    }

    #[test]
    fn test_from_url_default_port() {
        let config = ScyllaConfig::from_url("scylla://localhost/ks").unwrap();
        assert_eq!(config.known_nodes(), &["localhost:9042"]);
    }
}
