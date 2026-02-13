//! Error types for DuckDB operations.

use std::fmt;

use prax_query::error::QueryError;

/// Result type for DuckDB operations.
pub type DuckDbResult<T> = Result<T, DuckDbError>;

/// Error type for DuckDB operations.
#[derive(Debug)]
pub enum DuckDbError {
    /// Pool error.
    Pool(String),
    /// DuckDB driver error.
    DuckDb(duckdb::Error),
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
    /// File I/O error.
    FileIo(String),
    /// Parquet error.
    Parquet(String),
    /// Internal error.
    Internal(String),
}

impl DuckDbError {
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

    /// Create a file I/O error.
    pub fn file_io(msg: impl Into<String>) -> Self {
        Self::FileIo(msg.into())
    }

    /// Create a Parquet error.
    pub fn parquet(msg: impl Into<String>) -> Self {
        Self::Parquet(msg.into())
    }

    /// Create an internal error.
    pub fn internal(msg: impl Into<String>) -> Self {
        Self::Internal(msg.into())
    }
}

impl fmt::Display for DuckDbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pool(msg) => write!(f, "Pool error: {}", msg),
            Self::DuckDb(e) => write!(f, "DuckDB error: {}", e),
            Self::Config(msg) => write!(f, "Configuration error: {}", msg),
            Self::Connection(msg) => write!(f, "Connection error: {}", msg),
            Self::Query(msg) => write!(f, "Query error: {}", msg),
            Self::Deserialization(msg) => write!(f, "Deserialization error: {}", msg),
            Self::TypeConversion(msg) => write!(f, "Type conversion error: {}", msg),
            Self::Timeout(msg) => write!(f, "Timeout error: {}", msg),
            Self::FileIo(msg) => write!(f, "File I/O error: {}", msg),
            Self::Parquet(msg) => write!(f, "Parquet error: {}", msg),
            Self::Internal(msg) => write!(f, "Internal error: {}", msg),
        }
    }
}

impl std::error::Error for DuckDbError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::DuckDb(e) => Some(e),
            _ => None,
        }
    }
}

impl From<duckdb::Error> for DuckDbError {
    fn from(err: duckdb::Error) -> Self {
        Self::DuckDb(err)
    }
}

impl From<std::io::Error> for DuckDbError {
    fn from(err: std::io::Error) -> Self {
        Self::FileIo(err.to_string())
    }
}

impl From<DuckDbError> for QueryError {
    fn from(err: DuckDbError) -> Self {
        match err {
            DuckDbError::Pool(msg) => QueryError::connection(msg),
            DuckDbError::DuckDb(e) => QueryError::database(e.to_string()),
            DuckDbError::Config(msg) => QueryError::internal(format!("config: {}", msg)),
            DuckDbError::Connection(msg) => QueryError::connection(msg),
            DuckDbError::Query(msg) => QueryError::database(msg),
            DuckDbError::Deserialization(msg) => QueryError::serialization(msg),
            DuckDbError::TypeConversion(msg) => QueryError::serialization(format!("type: {}", msg)),
            DuckDbError::Timeout(_) => QueryError::timeout(5000),
            DuckDbError::FileIo(msg) => QueryError::internal(format!("file: {}", msg)),
            DuckDbError::Parquet(msg) => QueryError::internal(format!("parquet: {}", msg)),
            DuckDbError::Internal(msg) => QueryError::internal(msg),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = DuckDbError::config("invalid path");
        assert!(err.to_string().contains("Configuration error"));
        assert!(err.to_string().contains("invalid path"));
    }

    #[test]
    fn test_error_constructors() {
        assert!(matches!(DuckDbError::pool("test"), DuckDbError::Pool(_)));
        assert!(matches!(
            DuckDbError::config("test"),
            DuckDbError::Config(_)
        ));
        assert!(matches!(
            DuckDbError::connection("test"),
            DuckDbError::Connection(_)
        ));
        assert!(matches!(DuckDbError::query("test"), DuckDbError::Query(_)));
        assert!(matches!(
            DuckDbError::parquet("test"),
            DuckDbError::Parquet(_)
        ));
    }

    #[test]
    fn test_error_conversion() {
        let err = DuckDbError::timeout("connection timed out");
        let query_err: QueryError = err.into();
        assert!(query_err.is_timeout());
    }
}
