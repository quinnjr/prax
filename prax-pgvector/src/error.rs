//! Error types for pgvector operations.

use prax_query::QueryError;
use thiserror::Error;

/// Result type for pgvector operations.
pub type VectorResult<T> = Result<T, VectorError>;

/// Errors that can occur during pgvector operations.
#[derive(Error, Debug)]
pub enum VectorError {
    /// The pgvector extension is not installed.
    #[error("pgvector extension not installed: run CREATE EXTENSION vector")]
    ExtensionNotInstalled,

    /// Dimension mismatch between vectors.
    #[error("dimension mismatch: expected {expected}, got {actual}")]
    DimensionMismatch {
        /// Expected number of dimensions.
        expected: usize,
        /// Actual number of dimensions.
        actual: usize,
    },

    /// Empty vector provided.
    #[error("empty vector: vectors must have at least one dimension")]
    EmptyVector,

    /// Invalid dimensions for an operation.
    #[error("invalid dimensions: {0}")]
    InvalidDimensions(String),

    /// Index creation error.
    #[error("index error: {0}")]
    Index(String),

    /// PostgreSQL error.
    #[error("postgres error: {0}")]
    Postgres(#[from] prax_postgres::PgError),

    /// Query execution error.
    #[error("query error: {0}")]
    Query(String),

    /// Type conversion error.
    #[error("type conversion error: {0}")]
    TypeConversion(String),

    /// Configuration error.
    #[error("configuration error: {0}")]
    Config(String),
}

impl VectorError {
    /// Create a dimension mismatch error.
    pub fn dimension_mismatch(expected: usize, actual: usize) -> Self {
        Self::DimensionMismatch { expected, actual }
    }

    /// Create an index error.
    pub fn index(message: impl Into<String>) -> Self {
        Self::Index(message.into())
    }

    /// Create a query error.
    pub fn query(message: impl Into<String>) -> Self {
        Self::Query(message.into())
    }

    /// Create a type conversion error.
    pub fn type_conversion(message: impl Into<String>) -> Self {
        Self::TypeConversion(message.into())
    }

    /// Create a configuration error.
    pub fn config(message: impl Into<String>) -> Self {
        Self::Config(message.into())
    }

    /// Check if this is a dimension mismatch error.
    pub fn is_dimension_mismatch(&self) -> bool {
        matches!(self, Self::DimensionMismatch { .. })
    }

    /// Check if this is an extension not installed error.
    pub fn is_extension_not_installed(&self) -> bool {
        matches!(self, Self::ExtensionNotInstalled)
    }
}

impl From<VectorError> for QueryError {
    fn from(err: VectorError) -> Self {
        match err {
            VectorError::ExtensionNotInstalled => {
                QueryError::database("pgvector extension not installed".to_string())
            }
            VectorError::DimensionMismatch { expected, actual } => QueryError::invalid_input(
                "vector",
                format!("dimension mismatch: expected {expected}, got {actual}"),
            ),
            VectorError::EmptyVector => {
                QueryError::invalid_input("vector", "empty vector".to_string())
            }
            VectorError::InvalidDimensions(msg) => QueryError::invalid_input("vector", msg),
            VectorError::Index(msg) => QueryError::database(msg),
            VectorError::Postgres(e) => QueryError::from(e),
            VectorError::Query(msg) => QueryError::database(msg),
            VectorError::TypeConversion(msg) => QueryError::serialization(msg),
            VectorError::Config(msg) => QueryError::connection(msg),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_creation() {
        let err = VectorError::dimension_mismatch(3, 5);
        assert!(err.is_dimension_mismatch());
        assert!(err.to_string().contains("expected 3"));
        assert!(err.to_string().contains("got 5"));
    }

    #[test]
    fn test_extension_not_installed() {
        let err = VectorError::ExtensionNotInstalled;
        assert!(err.is_extension_not_installed());
        assert!(err.to_string().contains("pgvector"));
    }

    #[test]
    fn test_empty_vector() {
        let err = VectorError::EmptyVector;
        assert!(err.to_string().contains("empty vector"));
    }

    #[test]
    fn test_into_query_error() {
        let err = VectorError::dimension_mismatch(3, 5);
        let query_err: QueryError = err.into();
        assert!(query_err.to_string().contains("dimension mismatch"));
    }

    #[test]
    fn test_index_error() {
        let err = VectorError::index("failed to create HNSW index");
        assert!(err.to_string().contains("HNSW"));
    }

    #[test]
    fn test_config_error() {
        let err = VectorError::config("invalid probes value");
        assert!(err.to_string().contains("probes"));
    }
}
