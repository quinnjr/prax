//! Error types for PostgreSQL operations.

use prax_query::QueryError;
use thiserror::Error;

/// Result type for PostgreSQL operations.
pub type PgResult<T> = Result<T, PgError>;

/// Errors that can occur during PostgreSQL operations.
#[derive(Error, Debug)]
pub enum PgError {
    /// Connection pool error.
    #[error("pool error: {0}")]
    Pool(#[from] deadpool_postgres::PoolError),

    /// PostgreSQL error.
    #[error("postgres error: {0}")]
    Postgres(#[from] tokio_postgres::Error),

    /// Configuration error.
    #[error("configuration error: {0}")]
    Config(String),

    /// Connection error.
    #[error("connection error: {0}")]
    Connection(String),

    /// Query execution error.
    #[error("query error: {0}")]
    Query(String),

    /// Row deserialization error.
    #[error("deserialization error: {0}")]
    Deserialization(String),

    /// Type conversion error.
    #[error("type conversion error: {0}")]
    TypeConversion(String),

    /// Timeout error.
    #[error("operation timed out after {0}ms")]
    Timeout(u64),

    /// Internal error.
    #[error("internal error: {0}")]
    Internal(String),
}

impl PgError {
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

    /// Create a deserialization error.
    pub fn deserialization(message: impl Into<String>) -> Self {
        Self::Deserialization(message.into())
    }

    /// Create a type conversion error.
    pub fn type_conversion(message: impl Into<String>) -> Self {
        Self::TypeConversion(message.into())
    }

    /// Check if this is a connection error.
    pub fn is_connection_error(&self) -> bool {
        matches!(self, Self::Pool(_) | Self::Connection(_))
    }

    /// Check if this is a timeout error.
    pub fn is_timeout(&self) -> bool {
        matches!(self, Self::Timeout(_))
    }
}

/// Map a PostgreSQL SQLSTATE code (and message) to a [`QueryError`].
///
/// Split out from the `From<PgError>` impl so the classification can be
/// exercised directly in tests — a `tokio_postgres::Error` carrying a chosen
/// SQLSTATE cannot be constructed by hand.
///
/// Recognized classes:
///   * `23505` unique, `23503` foreign key, `23514` check → constraint
///     violations.
///   * `23502` not-null → invalid input.
///   * `0A000` "cached plan must not change result type" → a *retryable*
///     stale-plan error, so a pooled prepared statement invalidated by DDL is
///     classified retryable (see [`QueryError::stale_plan`]) instead of an
///     opaque generic database error. `0A000` is the shared
///     `FEATURE_NOT_SUPPORTED` class, so this is gated on the server's message
///     — other `0A000` conditions (genuinely unsupported features) are
///     terminal and stay generic.
///   * anything else → a generic database error.
///
/// `display` is the driver error's `Display` text (used as the error message);
/// `detail` is the server's message string (`DbError::message`), used only for
/// the `0A000` gate — a `tokio_postgres::Error` renders a DB error as just
/// "db error" via `Display`, so the cached-plan text is only visible in the
/// DbError. When `detail` is `None` (no DbError, e.g. a synthesized error) the
/// gate falls back to `display`.
pub(crate) fn classify_sqlstate(
    code: Option<&str>,
    display: &str,
    detail: Option<&str>,
) -> QueryError {
    let gate_text = detail.unwrap_or(display);
    match code {
        // Unique / foreign key / check violations.
        Some("23505") | Some("23503") | Some("23514") => {
            QueryError::constraint_violation("", display)
        }
        // Not null violation.
        Some("23502") => QueryError::invalid_input("", display),
        // Stale server-side prepared plan after DDL — transient, retry. The
        // 0A000 class also covers real "feature not supported" errors, which
        // are terminal, so match the specific cached-plan message.
        Some("0A000") if gate_text.contains("cached plan must not change result type") => {
            QueryError::stale_plan(display)
        }
        _ => QueryError::database(display),
    }
}

impl From<PgError> for QueryError {
    fn from(err: PgError) -> Self {
        match err {
            PgError::Pool(e) => QueryError::connection(e.to_string()),
            PgError::Postgres(e) => {
                // Categorize by SQLSTATE while preserving the driver error
                // as the source. The cached-plan gate needs the server's
                // message, which lives in the DbError — `Display` renders a DB
                // error as just "db error".
                let code_str = e.code().map(|c| c.code().to_owned());
                let display = e.to_string();
                let detail = e.as_db_error().map(|db| db.message().to_owned());
                let mapped = classify_sqlstate(code_str.as_deref(), &display, detail.as_deref());
                mapped.with_source(e)
            }
            PgError::Config(msg) => QueryError::connection(msg),
            PgError::Connection(msg) => QueryError::connection(msg),
            PgError::Query(msg) => QueryError::database(msg),
            PgError::Deserialization(msg) => QueryError::serialization(msg),
            PgError::TypeConversion(msg) => QueryError::serialization(msg),
            PgError::Timeout(ms) => QueryError::timeout(ms),
            PgError::Internal(msg) => QueryError::internal(msg),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_creation() {
        let err = PgError::config("invalid URL");
        assert!(matches!(err, PgError::Config(_)));

        let err = PgError::connection("connection refused");
        assert!(err.is_connection_error());

        let err = PgError::Timeout(5000);
        assert!(err.is_timeout());
    }

    #[test]
    fn test_into_query_error() {
        let pg_err = PgError::Timeout(1000);
        let query_err: QueryError = pg_err.into();
        assert!(query_err.is_timeout());
    }

    #[test]
    fn test_classify_constraint_sqlstates() {
        use prax_query::ErrorCode;
        // Unique, foreign key, and check all classify as constraint violations.
        for code in ["23505", "23503", "23514"] {
            let e = classify_sqlstate(Some(code), "boom", None);
            assert_eq!(e.code, ErrorCode::UniqueConstraint, "code {code}");
            assert!(e.is_constraint_violation(), "code {code}");
        }
        // Not-null maps to invalid input.
        assert_eq!(
            classify_sqlstate(Some("23502"), "boom", None).code,
            ErrorCode::InvalidParameter
        );
    }

    #[test]
    fn test_classify_stale_cached_plan_gates_on_detail() {
        use prax_query::ErrorCode;
        // Real Postgres path: Display is just "db error", the cached-plan text
        // is only in the DbError detail. The gate must read `detail`.
        let e = classify_sqlstate(
            Some("0A000"),
            "db error",
            Some("cached plan must not change result type"),
        );
        assert_eq!(e.code, ErrorCode::SerializationFailure);
        assert!(e.is_retryable());

        // If the cached-plan text were only in Display (no detail), the
        // fallback still catches it — defends the synthesized-error path.
        let e = classify_sqlstate(
            Some("0A000"),
            "cached plan must not change result type",
            None,
        );
        assert_eq!(e.code, ErrorCode::SerializationFailure);
    }

    #[test]
    fn test_classify_other_0a000_stays_generic() {
        use prax_query::ErrorCode;
        // A genuine "feature not supported" 0A000 is terminal, not retryable —
        // even though Display is "db error", the detail is not the cached-plan
        // text.
        let e = classify_sqlstate(Some("0A000"), "db error", Some("cannot insert into a view"));
        assert_eq!(e.code, ErrorCode::DatabaseError);
        assert!(!e.is_retryable());
    }

    #[test]
    fn test_classify_unknown_sqlstate_is_generic() {
        use prax_query::ErrorCode;
        assert_eq!(
            classify_sqlstate(Some("40P01"), "deadlock-ish", None).code,
            ErrorCode::DatabaseError
        );
        assert_eq!(
            classify_sqlstate(None, "no code", None).code,
            ErrorCode::DatabaseError
        );
    }
}
