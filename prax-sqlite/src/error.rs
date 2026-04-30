//! Error types for SQLite operations.

use std::fmt;

use prax_query::error::QueryError;

/// Result type for SQLite operations.
pub type SqliteResult<T> = Result<T, SqliteError>;

/// Error type for SQLite operations.
#[derive(Debug)]
pub enum SqliteError {
    /// Pool error.
    Pool(String),
    /// SQLite driver error.
    Sqlite(tokio_rusqlite::Error),
    /// Configuration error.
    Config(String),
    /// Connection error.
    Connection(String),
    /// Query error.
    Query(String),
    /// Deserialization error.
    Deserialization(String),
    /// Type conversion error.
    TypeConversion(String),
    /// Timeout error.
    Timeout(String),
    /// Internal error.
    Internal(String),
    /// Vector extension error (only present with the `vector` feature).
    #[cfg(feature = "vector")]
    Vector(crate::vector::error::VectorError),
}

impl SqliteError {
    /// Create a pool error.
    pub fn pool(msg: impl Into<String>) -> Self {
        Self::Pool(msg.into())
    }

    /// Create a configuration error.
    pub fn config(msg: impl Into<String>) -> Self {
        Self::Config(msg.into())
    }

    /// Create a connection error.
    pub fn connection(msg: impl Into<String>) -> Self {
        Self::Connection(msg.into())
    }

    /// Create a query error.
    pub fn query(msg: impl Into<String>) -> Self {
        Self::Query(msg.into())
    }

    /// Create a deserialization error.
    pub fn deserialization(msg: impl Into<String>) -> Self {
        Self::Deserialization(msg.into())
    }

    /// Create a type conversion error.
    pub fn type_conversion(msg: impl Into<String>) -> Self {
        Self::TypeConversion(msg.into())
    }

    /// Create a timeout error.
    pub fn timeout(msg: impl Into<String>) -> Self {
        Self::Timeout(msg.into())
    }

    /// Create an internal error.
    pub fn internal(msg: impl Into<String>) -> Self {
        Self::Internal(msg.into())
    }
}

impl fmt::Display for SqliteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pool(msg) => write!(f, "Pool error: {}", msg),
            Self::Sqlite(e) => write!(f, "SQLite error: {}", e),
            Self::Config(msg) => write!(f, "Configuration error: {}", msg),
            Self::Connection(msg) => write!(f, "Connection error: {}", msg),
            Self::Query(msg) => write!(f, "Query error: {}", msg),
            Self::Deserialization(msg) => write!(f, "Deserialization error: {}", msg),
            Self::TypeConversion(msg) => write!(f, "Type conversion error: {}", msg),
            Self::Timeout(msg) => write!(f, "Timeout error: {}", msg),
            Self::Internal(msg) => write!(f, "Internal error: {}", msg),
            #[cfg(feature = "vector")]
            Self::Vector(e) => write!(f, "Vector error: {}", e),
        }
    }
}

impl std::error::Error for SqliteError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite(e) => Some(e),
            #[cfg(feature = "vector")]
            Self::Vector(e) => Some(e),
            _ => None,
        }
    }
}

impl From<tokio_rusqlite::Error> for SqliteError {
    fn from(err: tokio_rusqlite::Error) -> Self {
        Self::Sqlite(err)
    }
}

impl From<rusqlite::Error> for SqliteError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Sqlite(tokio_rusqlite::Error::Rusqlite(err))
    }
}

#[cfg(feature = "vector")]
impl From<crate::vector::error::VectorError> for SqliteError {
    fn from(err: crate::vector::error::VectorError) -> Self {
        Self::Vector(err)
    }
}

impl From<SqliteError> for QueryError {
    fn from(err: SqliteError) -> Self {
        match err {
            SqliteError::Pool(msg) => QueryError::connection(msg),
            SqliteError::Sqlite(e) => QueryError::database(e.to_string()),
            SqliteError::Config(msg) => QueryError::internal(format!("config: {}", msg)),
            SqliteError::Connection(msg) => QueryError::connection(msg),
            SqliteError::Query(msg) => QueryError::database(msg),
            SqliteError::Deserialization(msg) => QueryError::serialization(msg),
            SqliteError::TypeConversion(msg) => QueryError::serialization(format!("type: {}", msg)),
            SqliteError::Timeout(_) => QueryError::timeout(5000), // Default timeout duration
            SqliteError::Internal(msg) => QueryError::internal(msg),
            #[cfg(feature = "vector")]
            SqliteError::Vector(e) => QueryError::database(e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = SqliteError::config("invalid path");
        assert!(err.to_string().contains("Configuration error"));
        assert!(err.to_string().contains("invalid path"));
    }

    #[test]
    fn test_error_constructors() {
        assert!(matches!(SqliteError::pool("test"), SqliteError::Pool(_)));
        assert!(matches!(
            SqliteError::config("test"),
            SqliteError::Config(_)
        ));
        assert!(matches!(
            SqliteError::connection("test"),
            SqliteError::Connection(_)
        ));
        assert!(matches!(SqliteError::query("test"), SqliteError::Query(_)));
    }

    #[test]
    fn test_error_conversion() {
        let err = SqliteError::timeout("connection timed out");
        let query_err: QueryError = err.into();
        assert!(query_err.is_timeout());
    }
}
