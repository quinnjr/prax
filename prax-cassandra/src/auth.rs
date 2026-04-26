//! SASL authentication framework for Cassandra.
//!
//! Cassandra supports pluggable authentication mechanisms via SASL.
//! This module provides the [`SaslMechanism`] trait and a `PLAIN` implementation
//! covering username+password authentication.
//!
//! Future crates can implement additional mechanisms (LDAP, GSSAPI/Kerberos)
//! by implementing [`SaslMechanism`].

use async_trait::async_trait;

use crate::error::CassandraResult;

/// A SASL mechanism for authenticating against a Cassandra cluster.
///
/// Implementations are generally stateful — the `evaluate` method is called
/// repeatedly with server challenges until authentication completes.
#[async_trait]
pub trait SaslMechanism: Send + Sync + std::fmt::Debug {
    /// The SASL mechanism name (e.g., "PLAIN", "GSSAPI").
    fn name(&self) -> &str;

    /// Generate the initial client response sent with the SASL AUTHENTICATE.
    async fn initial_response(&self) -> CassandraResult<Vec<u8>>;

    /// Respond to a SASL challenge sent by the server.
    ///
    /// Returns the next client response. For single-round mechanisms like
    /// PLAIN, this returns an empty vector.
    async fn evaluate(&self, challenge: &[u8]) -> CassandraResult<Vec<u8>>;
}

/// PLAIN SASL mechanism: username + password over a single round.
#[derive(Debug, Clone)]
pub struct PlainSasl {
    /// Username for authentication.
    pub username: String,
    /// Password for authentication.
    pub password: String,
}

impl PlainSasl {
    /// Create a new PlainSasl authenticator.
    pub fn new(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            username: username.into(),
            password: password.into(),
        }
    }
}

#[async_trait]
impl SaslMechanism for PlainSasl {
    fn name(&self) -> &str {
        "PLAIN"
    }

    async fn initial_response(&self) -> CassandraResult<Vec<u8>> {
        // PLAIN format: \0username\0password
        let mut buf = Vec::with_capacity(2 + self.username.len() + self.password.len());
        buf.push(0);
        buf.extend_from_slice(self.username.as_bytes());
        buf.push(0);
        buf.extend_from_slice(self.password.as_bytes());
        Ok(buf)
    }

    async fn evaluate(&self, _challenge: &[u8]) -> CassandraResult<Vec<u8>> {
        // PLAIN completes in the initial response; no further challenges.
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_plain_sasl_initial_response_format() {
        let sasl = PlainSasl::new("alice", "s3cret");
        let response = sasl.initial_response().await.unwrap();
        let expected: Vec<u8> = b"\0alice\0s3cret".to_vec();
        assert_eq!(response, expected);
    }

    #[tokio::test]
    async fn test_plain_sasl_evaluate_returns_empty() {
        let sasl = PlainSasl::new("alice", "s3cret");
        let response = sasl.evaluate(b"challenge").await.unwrap();
        assert!(response.is_empty());
    }

    #[test]
    fn test_plain_sasl_name() {
        let sasl = PlainSasl::new("u", "p");
        assert_eq!(sasl.name(), "PLAIN");
    }
}
