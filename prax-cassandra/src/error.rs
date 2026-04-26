//! Error types for the prax-cassandra driver.

use std::time::Duration;

/// Errors produced by the prax-cassandra driver.
#[derive(Debug, thiserror::Error)]
pub enum CassandraError {
    /// A connection-level failure (network, TCP, cluster resolution).
    #[error("Connection error: {0}")]
    Connection(String),

    /// Authentication was rejected by the cluster.
    #[error("Authentication failed: {0}")]
    Authentication(String),

    /// A query failed to execute.
    #[error("Query execution failed: {0}")]
    Query(String),

    /// A row could not be deserialized into the requested type.
    #[error("Row deserialization failed: {0}")]
    Deserialization(String),

    /// An operation exceeded its timeout.
    #[error("Timeout after {duration:?}: {operation}")]
    Timeout {
        /// Name of the operation that timed out.
        operation: String,
        /// Elapsed duration before timeout.
        duration: Duration,
    },

    /// The provided configuration was invalid.
    #[error("Configuration error: {0}")]
    Config(String),

    /// A TLS error occurred during connection setup.
    #[error("TLS error: {0}")]
    Tls(String),

    /// A lightweight transaction did not apply (CAS failed).
    #[error("Lightweight transaction not applied")]
    LwtNotApplied,
}

/// Convenience alias for `Result<T, CassandraError>`.
pub type CassandraResult<T> = Result<T, CassandraError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connection_error_display() {
        let err = CassandraError::Connection("refused".into());
        assert_eq!(err.to_string(), "Connection error: refused");
    }

    #[test]
    fn test_timeout_error_display() {
        let err = CassandraError::Timeout {
            operation: "query".into(),
            duration: Duration::from_secs(5),
        };
        assert!(err.to_string().contains("query"));
        assert!(err.to_string().contains("5s"));
    }

    #[test]
    fn test_lwt_not_applied_is_no_data() {
        let err = CassandraError::LwtNotApplied;
        assert_eq!(err.to_string(), "Lightweight transaction not applied");
    }
}
