//! Error types for SQLx operations.

use prax_query::QueryError;
use thiserror::Error;

/// Result type alias for SQLx operations.
pub type SqlxResult<T> = Result<T, SqlxError>;

/// Errors that can occur during SQLx operations.
#[derive(Error, Debug)]
pub enum SqlxError {
    /// SQLx database error
    #[error("Database error: {0}")]
    Sqlx(#[from] sqlx::Error),

    /// Configuration error
    #[error("Configuration error: {0}")]
    Config(String),

    /// Connection error
    #[error("Connection error: {0}")]
    Connection(String),

    /// Query execution error
    #[error("Query error: {0}")]
    Query(String),

    /// Row deserialization error
    #[error("Deserialization error: {0}")]
    Deserialization(String),

    /// Type conversion error
    #[error("Type conversion error: {0}")]
    TypeConversion(String),

    /// Pool error
    #[error("Pool error: {0}")]
    Pool(String),

    /// Timeout error
    #[error("Operation timed out after {0}ms")]
    Timeout(u64),

    /// Migration error
    #[error("Migration error: {0}")]
    Migration(String),

    /// Internal error
    #[error("Internal error: {0}")]
    Internal(String),
}

impl From<SqlxError> for QueryError {
    fn from(err: SqlxError) -> Self {
        match err {
            SqlxError::Sqlx(e) => match &e {
                // Transport/pool failures are connection problems even though
                // their messages ("error communicating with database") never
                // contain the word "connection".
                sqlx::Error::Io(_) | sqlx::Error::PoolTimedOut | sqlx::Error::PoolClosed => {
                    QueryError::connection(e.to_string())
                }
                sqlx::Error::RowNotFound => QueryError::not_found("row"),
                // Classify database errors by SQLSTATE code rather than by
                // substring-matching the message (mirrors prax-postgres).
                sqlx::Error::Database(db_err) => match db_err.code().as_deref() {
                    // unique / foreign key / check violation
                    Some("23505") | Some("23503") | Some("23514") => {
                        QueryError::constraint_violation("", e.to_string())
                    }
                    // not-null violation
                    Some("23502") => QueryError::invalid_input("", e.to_string()),
                    // Stale server-side prepared plan after DDL — transient,
                    // retryable. 0A000 is the shared FEATURE_NOT_SUPPORTED
                    // class, so gate on the specific cached-plan message from
                    // the DbError (not the outer Display); other 0A000
                    // conditions stay terminal.
                    Some("0A000")
                        if db_err
                            .message()
                            .contains("cached plan must not change result type") =>
                    {
                        QueryError::stale_plan(e.to_string())
                    }
                    _ => QueryError::database(e.to_string()),
                },
                _ => QueryError::database(e.to_string()),
            },
            SqlxError::Config(msg) => QueryError::connection(msg),
            SqlxError::Connection(msg) => QueryError::connection(msg),
            SqlxError::Query(msg) => QueryError::database(msg),
            SqlxError::Deserialization(msg) => QueryError::serialization(msg),
            SqlxError::TypeConversion(msg) => QueryError::serialization(msg),
            SqlxError::Pool(msg) => QueryError::connection(msg),
            SqlxError::Timeout(ms) => QueryError::timeout(ms),
            SqlxError::Migration(msg) => QueryError::database(msg),
            SqlxError::Internal(msg) => QueryError::internal(msg),
        }
    }
}

impl SqlxError {
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

    /// Create a pool error.
    pub fn pool(msg: impl Into<String>) -> Self {
        Self::Pool(msg.into())
    }

    /// Create a timeout error.
    pub fn timeout(ms: u64) -> Self {
        Self::Timeout(ms)
    }

    /// Create a type conversion error.
    pub fn type_conversion(msg: impl Into<String>) -> Self {
        Self::TypeConversion(msg.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_creation() {
        let err = SqlxError::config("test config error");
        assert!(matches!(err, SqlxError::Config(_)));

        let err = SqlxError::connection("test connection error");
        assert!(matches!(err, SqlxError::Connection(_)));

        let err = SqlxError::timeout(5000);
        assert!(matches!(err, SqlxError::Timeout(5000)));
    }

    #[test]
    fn test_error_to_query_error() {
        let err = SqlxError::connection("connection failed");
        let query_err: QueryError = err.into();
        assert!(query_err.to_string().contains("connection"));

        let err = SqlxError::timeout(1000);
        let query_err: QueryError = err.into();
        assert!(
            query_err.to_string().contains("timeout") || query_err.to_string().contains("1000")
        );
    }

    /// Minimal `sqlx::error::DatabaseError` implementation so SQLSTATE-code
    /// classification can be exercised without a live database.
    #[derive(Debug)]
    struct MockDbError {
        message: String,
        code: Option<String>,
    }

    impl std::fmt::Display for MockDbError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(&self.message)
        }
    }

    impl std::error::Error for MockDbError {}

    impl sqlx::error::DatabaseError for MockDbError {
        fn message(&self) -> &str {
            &self.message
        }

        fn code(&self) -> Option<std::borrow::Cow<'_, str>> {
            self.code.as_deref().map(std::borrow::Cow::Borrowed)
        }

        fn as_error(&self) -> &(dyn std::error::Error + Send + Sync + 'static) {
            self
        }

        fn as_error_mut(&mut self) -> &mut (dyn std::error::Error + Send + Sync + 'static) {
            self
        }

        fn into_error(self: Box<Self>) -> Box<dyn std::error::Error + Send + Sync + 'static> {
            self
        }

        fn kind(&self) -> sqlx::error::ErrorKind {
            sqlx::error::ErrorKind::Other
        }
    }

    #[test]
    fn test_sqlx_database_error_code_mapping() {
        // SQLSTATE 23505 (unique violation) classifies by code, not message.
        let db_err = MockDbError {
            message: "db says no".to_string(),
            code: Some("23505".to_string()),
        };
        let err = SqlxError::from(sqlx::Error::Database(Box::new(db_err)));
        let query_err: QueryError = err.into();
        assert_eq!(query_err.code, prax_query::ErrorCode::UniqueConstraint);

        // SQLSTATE 23502 (not-null violation) maps to invalid input.
        let db_err = MockDbError {
            message: "db says no".to_string(),
            code: Some("23502".to_string()),
        };
        let err = SqlxError::from(sqlx::Error::Database(Box::new(db_err)));
        let query_err: QueryError = err.into();
        assert_eq!(query_err.code, prax_query::ErrorCode::InvalidParameter);

        // An unrecognised SQLSTATE falls back to a generic database error.
        let db_err = MockDbError {
            message: "deadlock detected".to_string(),
            code: Some("40P01".to_string()),
        };
        let err = SqlxError::from(sqlx::Error::Database(Box::new(db_err)));
        let query_err: QueryError = err.into();
        assert_eq!(query_err.code, prax_query::ErrorCode::DatabaseError);
    }

    #[test]
    fn test_sqlx_check_violation_maps_to_constraint() {
        // SQLSTATE 23514 (check violation) classifies as a constraint error.
        let db_err = MockDbError {
            message: "new row violates check constraint".to_string(),
            code: Some("23514".to_string()),
        };
        let err = SqlxError::from(sqlx::Error::Database(Box::new(db_err)));
        let query_err: QueryError = err.into();
        assert!(query_err.is_constraint_violation());
    }

    #[test]
    fn test_sqlx_stale_cached_plan_is_retryable() {
        // 0A000 with the cached-plan message is transient and retryable.
        let db_err = MockDbError {
            message: "cached plan must not change result type".to_string(),
            code: Some("0A000".to_string()),
        };
        let err = SqlxError::from(sqlx::Error::Database(Box::new(db_err)));
        let query_err: QueryError = err.into();
        assert_eq!(query_err.code, prax_query::ErrorCode::SerializationFailure);
        assert!(query_err.is_retryable());
    }

    #[test]
    fn test_sqlx_other_0a000_stays_generic() {
        // A genuine "feature not supported" 0A000 is terminal.
        let db_err = MockDbError {
            message: "cannot insert into a view".to_string(),
            code: Some("0A000".to_string()),
        };
        let err = SqlxError::from(sqlx::Error::Database(Box::new(db_err)));
        let query_err: QueryError = err.into();
        assert_eq!(query_err.code, prax_query::ErrorCode::DatabaseError);
        assert!(!query_err.is_retryable());
    }

    #[test]
    fn test_sqlx_io_error_maps_to_connection() {
        // I/O errors display as "error communicating with database" — no
        // "connection" substring — but must still classify as connection errors.
        let io_err = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "refused");
        let err = SqlxError::from(sqlx::Error::Io(io_err));
        let query_err: QueryError = err.into();
        assert_eq!(query_err.code, prax_query::ErrorCode::ConnectionFailed);
    }
}
