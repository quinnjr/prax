//! Error types for `ScyllaDB` operations.

#[allow(unused_imports)]
use std::fmt;
use thiserror::Error;

/// Result type for `ScyllaDB` operations.
pub type ScyllaResult<T> = Result<T, ScyllaError>;

/// Errors that can occur during `ScyllaDB` operations.
#[derive(Error, Debug)]
pub enum ScyllaError {
    /// Configuration error.
    #[error("Configuration error: {0}")]
    Configuration(String),

    /// Connection error.
    #[error("Connection error: {0}")]
    Connection(String),

    /// Query execution error.
    #[error("Query error: {0}")]
    Query(String),

    /// Prepared statement error.
    #[error("Prepared statement error: {0}")]
    PreparedStatement(String),

    /// Batch operation error.
    #[error("Batch error: {0}")]
    Batch(String),

    /// Serialization error.
    #[error("Serialization error: {0}")]
    Serialization(String),

    /// Deserialization error.
    #[error("Deserialization error: {0}")]
    Deserialization(String),

    /// Type conversion error.
    #[error("Type conversion error: {0}")]
    TypeConversion(String),

    /// Pool error.
    #[error("Pool error: {0}")]
    Pool(String),

    /// Timeout error.
    #[error("Operation timed out: {0}")]
    Timeout(String),

    /// Keyspace error.
    #[error("Keyspace error: {0}")]
    Keyspace(String),

    /// Authentication error.
    #[error("Authentication failed: {0}")]
    Authentication(String),

    /// Authorization error.
    #[error("Authorization failed: {0}")]
    Authorization(String),

    /// Unavailable error (not enough replicas).
    #[error("Unavailable: {0}")]
    Unavailable(String),

    /// Write timeout error.
    #[error("Write timeout: {0}")]
    WriteTimeout(String),

    /// Read timeout error.
    #[error("Read timeout: {0}")]
    ReadTimeout(String),

    /// Overloaded error.
    #[error("Server overloaded: {0}")]
    Overloaded(String),

    /// Syntax error in CQL.
    #[error("CQL syntax error: {0}")]
    Syntax(String),

    /// Invalid query.
    #[error("Invalid query: {0}")]
    Invalid(String),

    /// Lightweight transaction (LWT) error.
    #[error("LWT error: {0}")]
    Lwt(String),

    /// Internal error.
    #[error("Internal error: {0}")]
    Internal(String),

    /// Row not found.
    #[error("Row not found")]
    NotFound,

    /// Multiple rows returned when one expected.
    #[error("Multiple rows returned when one expected")]
    MultipleRowsReturned,
}

impl ScyllaError {
    /// Create a configuration error.
    pub fn config(msg: impl Into<String>) -> Self {
        Self::Configuration(msg.into())
    }

    /// Create a connection error.
    pub fn connection(msg: impl Into<String>) -> Self {
        Self::Connection(msg.into())
    }

    /// Create a query error.
    pub fn query(msg: impl Into<String>) -> Self {
        Self::Query(msg.into())
    }

    /// Create a type conversion error.
    pub fn type_conversion(msg: impl Into<String>) -> Self {
        Self::TypeConversion(msg.into())
    }

    /// Create a deserialization error.
    pub fn deserialization(msg: impl Into<String>) -> Self {
        Self::Deserialization(msg.into())
    }

    /// Check if error is retryable.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::Connection(_)
                | Self::Timeout(_)
                | Self::Unavailable(_)
                | Self::WriteTimeout(_)
                | Self::ReadTimeout(_)
                | Self::Overloaded(_)
        )
    }

    /// Check if error is a timeout.
    #[must_use]
    pub fn is_timeout(&self) -> bool {
        matches!(
            self,
            Self::Timeout(_) | Self::WriteTimeout(_) | Self::ReadTimeout(_)
        )
    }

    /// Check if error is authentication related.
    #[must_use]
    pub fn is_auth_error(&self) -> bool {
        matches!(self, Self::Authentication(_) | Self::Authorization(_))
    }
}

// Conversion from scylla driver errors
impl From<scylla::transport::errors::NewSessionError> for ScyllaError {
    fn from(err: scylla::transport::errors::NewSessionError) -> Self {
        Self::Connection(err.to_string())
    }
}

impl From<scylla::transport::errors::QueryError> for ScyllaError {
    fn from(err: scylla::transport::errors::QueryError) -> Self {
        use scylla::transport::errors::QueryError;

        match &err {
            QueryError::TimeoutError => Self::Timeout("Query timed out".into()),
            QueryError::DbError(db_err, msg) => {
                use scylla::transport::errors::DbError;
                match db_err {
                    DbError::Unavailable { .. } => Self::Unavailable(msg.clone()),
                    DbError::WriteTimeout { .. } => Self::WriteTimeout(msg.clone()),
                    DbError::ReadTimeout { .. } => Self::ReadTimeout(msg.clone()),
                    DbError::Overloaded => Self::Overloaded(msg.clone()),
                    DbError::SyntaxError => Self::Syntax(msg.clone()),
                    DbError::Invalid => Self::Invalid(msg.clone()),
                    DbError::Unauthorized => Self::Authorization(msg.clone()),
                    DbError::AuthenticationError => Self::Authentication(msg.clone()),
                    _ => Self::Query(format!("{db_err}: {msg}")),
                }
            }
            _ => Self::Query(err.to_string()),
        }
    }
}

impl From<scylla::serialize::SerializationError> for ScyllaError {
    fn from(err: scylla::serialize::SerializationError) -> Self {
        Self::Serialization(err.to_string())
    }
}

impl From<scylla::deserialize::DeserializationError> for ScyllaError {
    fn from(err: scylla::deserialize::DeserializationError) -> Self {
        Self::Deserialization(err.to_string())
    }
}

// Conversion to prax_query::error::QueryError
impl From<ScyllaError> for prax_query::error::QueryError {
    fn from(err: ScyllaError) -> Self {
        use prax_query::error::ErrorCode;

        let code = match &err {
            ScyllaError::Configuration(_) => ErrorCode::InvalidParameter,
            ScyllaError::Connection(_) => ErrorCode::ConnectionFailed,
            ScyllaError::Authentication(_) | ScyllaError::Authorization(_) => {
                ErrorCode::AuthenticationFailed
            }
            ScyllaError::Timeout(_)
            | ScyllaError::WriteTimeout(_)
            | ScyllaError::ReadTimeout(_) => ErrorCode::QueryTimeout,
            ScyllaError::NotFound => ErrorCode::RecordNotFound,
            ScyllaError::Syntax(_) | ScyllaError::Invalid(_) => ErrorCode::SqlSyntax,
            _ => ErrorCode::DatabaseError,
        };

        prax_query::error::QueryError::new(code, err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_retryable() {
        assert!(ScyllaError::Connection("test".into()).is_retryable());
        assert!(ScyllaError::Timeout("test".into()).is_retryable());
        assert!(ScyllaError::Unavailable("test".into()).is_retryable());
        assert!(!ScyllaError::Syntax("test".into()).is_retryable());
        assert!(!ScyllaError::NotFound.is_retryable());
    }

    #[test]
    fn test_error_timeout() {
        assert!(ScyllaError::Timeout("test".into()).is_timeout());
        assert!(ScyllaError::WriteTimeout("test".into()).is_timeout());
        assert!(ScyllaError::ReadTimeout("test".into()).is_timeout());
        assert!(!ScyllaError::Query("test".into()).is_timeout());
    }

    #[test]
    fn test_error_auth() {
        assert!(ScyllaError::Authentication("test".into()).is_auth_error());
        assert!(ScyllaError::Authorization("test".into()).is_auth_error());
        assert!(!ScyllaError::Connection("test".into()).is_auth_error());
    }
}
