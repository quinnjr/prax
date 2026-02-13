//! Error types for MongoDB operations.

use prax_query::QueryError;
use thiserror::Error;

/// Result type for MongoDB operations.
pub type MongoResult<T> = Result<T, MongoError>;

/// Errors that can occur during MongoDB operations.
#[derive(Error, Debug)]
pub enum MongoError {
    /// MongoDB driver error.
    #[error("mongodb error: {0}")]
    Driver(#[from] mongodb::error::Error),

    /// BSON serialization/deserialization error.
    #[error("bson error: {0}")]
    Bson(#[from] bson::ser::Error),

    /// BSON deserialization error.
    #[error("bson deserialization error: {0}")]
    BsonDe(#[from] bson::de::Error),

    /// Configuration error.
    #[error("configuration error: {0}")]
    Config(String),

    /// Connection error.
    #[error("connection error: {0}")]
    Connection(String),

    /// Query execution error.
    #[error("query error: {0}")]
    Query(String),

    /// Document not found.
    #[error("document not found: {0}")]
    NotFound(String),

    /// Document serialization error.
    #[error("serialization error: {0}")]
    Serialization(String),

    /// Invalid ObjectId.
    #[error("invalid object id: {0}")]
    InvalidObjectId(String),

    /// Timeout error.
    #[error("operation timed out after {0}ms")]
    Timeout(u64),

    /// Internal error.
    #[error("internal error: {0}")]
    Internal(String),
}

impl MongoError {
    /// Create a configuration error.
    pub fn config(message: impl Into<String>) -> Self {
        Self::Config(message.into())
    }

    /// Create a connection error.
    pub fn connection(message: impl Into<String>) -> Self {
        Self::Connection(message.into())
    }

    /// Create a query error.
    pub fn query(message: impl Into<String>) -> Self {
        Self::Query(message.into())
    }

    /// Create a not found error.
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound(message.into())
    }

    /// Create a serialization error.
    pub fn serialization(message: impl Into<String>) -> Self {
        Self::Serialization(message.into())
    }

    /// Create an invalid object id error.
    pub fn invalid_object_id(message: impl Into<String>) -> Self {
        Self::InvalidObjectId(message.into())
    }

    /// Check if this is a connection error.
    pub fn is_connection_error(&self) -> bool {
        matches!(self, Self::Connection(_))
    }

    /// Check if this is a timeout error.
    pub fn is_timeout(&self) -> bool {
        matches!(self, Self::Timeout(_))
    }

    /// Check if this is a not found error.
    pub fn is_not_found(&self) -> bool {
        matches!(self, Self::NotFound(_))
    }
}

impl From<bson::oid::Error> for MongoError {
    fn from(err: bson::oid::Error) -> Self {
        MongoError::InvalidObjectId(err.to_string())
    }
}

impl From<MongoError> for QueryError {
    fn from(err: MongoError) -> Self {
        match err {
            MongoError::Driver(e) => {
                let msg = e.to_string();

                // Check for specific MongoDB error types
                if msg.contains("duplicate key") {
                    return QueryError::constraint_violation("_id", msg);
                }
                if msg.contains("connection") || msg.contains("timeout") {
                    return QueryError::connection(msg);
                }

                QueryError::database(msg)
            }
            MongoError::Bson(e) => QueryError::serialization(e.to_string()),
            MongoError::BsonDe(e) => QueryError::serialization(e.to_string()),
            MongoError::Config(msg) => QueryError::connection(msg),
            MongoError::Connection(msg) => QueryError::connection(msg),
            MongoError::Query(msg) => QueryError::database(msg),
            MongoError::NotFound(msg) => QueryError::not_found(&msg),
            MongoError::Serialization(msg) => QueryError::serialization(msg),
            MongoError::InvalidObjectId(msg) => QueryError::invalid_input("_id", msg),
            MongoError::Timeout(ms) => QueryError::timeout(ms),
            MongoError::Internal(msg) => QueryError::internal(msg),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_creation() {
        let err = MongoError::config("invalid URI");
        assert!(matches!(err, MongoError::Config(_)));

        let err = MongoError::connection("connection refused");
        assert!(err.is_connection_error());

        let err = MongoError::Timeout(5000);
        assert!(err.is_timeout());

        let err = MongoError::not_found("user");
        assert!(err.is_not_found());
    }

    #[test]
    fn test_error_display() {
        let err = MongoError::config("test error");
        assert_eq!(err.to_string(), "configuration error: test error");

        let err = MongoError::NotFound("user".to_string());
        assert_eq!(err.to_string(), "document not found: user");
    }

    #[test]
    fn test_into_query_error() {
        let mongo_err = MongoError::Timeout(1000);
        let query_err: QueryError = mongo_err.into();
        assert!(query_err.is_timeout());

        let mongo_err = MongoError::not_found("User");
        let query_err: QueryError = mongo_err.into();
        assert!(query_err.is_not_found());
    }
}
