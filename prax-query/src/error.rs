//! Comprehensive error types for query operations with actionable messages.
//!
//! This module provides detailed error types that include:
//! - Error codes for programmatic handling
//! - Actionable suggestions for fixing issues
//! - Context about what operation failed
//! - Help text and documentation links
//!
//! # Error Codes
//!
//! Error codes follow a pattern: P{category}{number}
//! - 1xxx: Query errors (not found, invalid filter, etc.)
//! - 2xxx: Constraint violations (unique, foreign key, etc.)
//! - 3xxx: Connection errors (timeout, pool, auth)
//! - 4xxx: Transaction errors (deadlock, serialization)
//! - 5xxx: Execution errors (timeout, syntax, params)
//! - 6xxx: Data errors (type, serialization)
//! - 7xxx: Configuration errors
//! - 8xxx: Migration errors
//! - 9xxx: Tenant errors
//!
//! ```rust
//! use prax_query::ErrorCode;
//!
//! // Error codes have string representations
//! let code = ErrorCode::RecordNotFound;
//! let code = ErrorCode::UniqueConstraint;
//! let code = ErrorCode::ConnectionFailed;
//! ```
//!
//! # Creating Errors
//!
//! ```rust
//! use prax_query::{QueryError, ErrorCode};
//!
//! // Not found error
//! let err = QueryError::not_found("User");
//! assert_eq!(err.code, ErrorCode::RecordNotFound);
//!
//! // Generic error with code
//! let err = QueryError::new(ErrorCode::UniqueConstraint, "Email already exists");
//! assert_eq!(err.code, ErrorCode::UniqueConstraint);
//! ```
//!
//! # Error Properties
//!
//! ```rust
//! use prax_query::{QueryError, ErrorCode};
//!
//! let err = QueryError::not_found("User");
//!
//! // Access error code (public field)
//! assert_eq!(err.code, ErrorCode::RecordNotFound);
//!
//! // Access error message
//! let message = err.to_string();
//! assert!(message.contains("User"));
//! ```

use std::fmt;
use thiserror::Error;

/// Result type for query operations.
pub type QueryResult<T> = Result<T, QueryError>;

/// Error codes for programmatic error handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorCode {
    // Query errors (1xxx)
    /// Record not found (P1001).
    RecordNotFound = 1001,
    /// Multiple records found when expecting one (P1002).
    NotUnique = 1002,
    /// Invalid filter or where clause (P1003).
    InvalidFilter = 1003,
    /// Invalid select or include (P1004).
    InvalidSelect = 1004,
    /// Required field missing (P1005).
    RequiredFieldMissing = 1005,

    // Constraint errors (2xxx)
    /// Unique constraint violation (P2001).
    UniqueConstraint = 2001,
    /// Foreign key constraint violation (P2002).
    ForeignKeyConstraint = 2002,
    /// Check constraint violation (P2003).
    CheckConstraint = 2003,
    /// Not null constraint violation (P2004).
    NotNullConstraint = 2004,

    // Connection errors (3xxx)
    /// Database connection failed (P3001).
    ConnectionFailed = 3001,
    /// Connection pool exhausted (P3002).
    PoolExhausted = 3002,
    /// Connection timeout (P3003).
    ConnectionTimeout = 3003,
    /// Authentication failed (P3004).
    AuthenticationFailed = 3004,
    /// SSL/TLS error (P3005).
    SslError = 3005,

    // Transaction errors (4xxx)
    /// Transaction failed (P4001).
    TransactionFailed = 4001,
    /// Deadlock detected (P4002).
    Deadlock = 4002,
    /// Serialization failure (P4003).
    SerializationFailure = 4003,
    /// Transaction already committed/rolled back (P4004).
    TransactionClosed = 4004,

    // Query execution errors (5xxx)
    /// Query timeout (P5001).
    QueryTimeout = 5001,
    /// SQL syntax error (P5002).
    SqlSyntax = 5002,
    /// Invalid parameter (P5003).
    InvalidParameter = 5003,
    /// Query too complex (P5004).
    QueryTooComplex = 5004,
    /// General database error (P5005).
    DatabaseError = 5005,

    // Data errors (6xxx)
    /// Invalid data type (P6001).
    InvalidDataType = 6001,
    /// Serialization error (P6002).
    SerializationError = 6002,
    /// Deserialization error (P6003).
    DeserializationError = 6003,
    /// Data truncation (P6004).
    DataTruncation = 6004,

    // Configuration errors (7xxx)
    /// Invalid configuration (P7001).
    InvalidConfiguration = 7001,
    /// Missing configuration (P7002).
    MissingConfiguration = 7002,
    /// Invalid connection string (P7003).
    InvalidConnectionString = 7003,

    // Internal errors (9xxx)
    /// Internal error (P9001).
    Internal = 9001,
    /// Unknown error (P9999).
    Unknown = 9999,
}

impl ErrorCode {
    /// Get the error code string (e.g., "P1001").
    pub fn code(&self) -> String {
        format!("P{}", *self as u16)
    }

    /// Get a short description of the error code.
    pub fn description(&self) -> &'static str {
        match self {
            Self::RecordNotFound => "Record not found",
            Self::NotUnique => "Multiple records found",
            Self::InvalidFilter => "Invalid filter condition",
            Self::InvalidSelect => "Invalid select or include",
            Self::RequiredFieldMissing => "Required field missing",
            Self::UniqueConstraint => "Unique constraint violation",
            Self::ForeignKeyConstraint => "Foreign key constraint violation",
            Self::CheckConstraint => "Check constraint violation",
            Self::NotNullConstraint => "Not null constraint violation",
            Self::ConnectionFailed => "Database connection failed",
            Self::PoolExhausted => "Connection pool exhausted",
            Self::ConnectionTimeout => "Connection timeout",
            Self::AuthenticationFailed => "Authentication failed",
            Self::SslError => "SSL/TLS error",
            Self::TransactionFailed => "Transaction failed",
            Self::Deadlock => "Deadlock detected",
            Self::SerializationFailure => "Serialization failure",
            Self::TransactionClosed => "Transaction already closed",
            Self::QueryTimeout => "Query timeout",
            Self::SqlSyntax => "SQL syntax error",
            Self::InvalidParameter => "Invalid parameter",
            Self::QueryTooComplex => "Query too complex",
            Self::DatabaseError => "Database error",
            Self::InvalidDataType => "Invalid data type",
            Self::SerializationError => "Serialization error",
            Self::DeserializationError => "Deserialization error",
            Self::DataTruncation => "Data truncation",
            Self::InvalidConfiguration => "Invalid configuration",
            Self::MissingConfiguration => "Missing configuration",
            Self::InvalidConnectionString => "Invalid connection string",
            Self::Internal => "Internal error",
            Self::Unknown => "Unknown error",
        }
    }

    /// Get the documentation URL for this error.
    pub fn docs_url(&self) -> String {
        format!("https://prax.rs/docs/errors/{}", self.code())
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.code())
    }
}

/// Suggestion for fixing an error.
#[derive(Debug, Clone)]
pub struct Suggestion {
    /// The suggestion text.
    pub text: String,
    /// Optional code example.
    pub code: Option<String>,
}

impl Suggestion {
    /// Create a new suggestion.
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            code: None,
        }
    }

    /// Add a code example.
    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }
}

/// Additional context for an error.
#[derive(Debug, Clone, Default)]
pub struct ErrorContext {
    /// The operation that was being performed.
    pub operation: Option<String>,
    /// The model involved.
    pub model: Option<String>,
    /// The field involved.
    pub field: Option<String>,
    /// The SQL query (if available).
    pub sql: Option<String>,
    /// Suggestions for fixing the error.
    pub suggestions: Vec<Suggestion>,
    /// Help text.
    pub help: Option<String>,
    /// Related errors.
    pub related: Vec<String>,
}

impl ErrorContext {
    /// Create new empty context.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the operation.
    pub fn operation(mut self, op: impl Into<String>) -> Self {
        self.operation = Some(op.into());
        self
    }

    /// Set the model.
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Set the field.
    pub fn field(mut self, field: impl Into<String>) -> Self {
        self.field = Some(field.into());
        self
    }

    /// Set the SQL query.
    pub fn sql(mut self, sql: impl Into<String>) -> Self {
        self.sql = Some(sql.into());
        self
    }

    /// Add a suggestion.
    pub fn suggestion(mut self, suggestion: Suggestion) -> Self {
        self.suggestions.push(suggestion);
        self
    }

    /// Add a text suggestion.
    pub fn suggest(mut self, text: impl Into<String>) -> Self {
        self.suggestions.push(Suggestion::new(text));
        self
    }

    /// Set help text.
    pub fn help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }
}

/// Errors that can occur during query operations.
#[derive(Error, Debug)]
pub struct QueryError {
    /// The error code.
    pub code: ErrorCode,
    /// The error message.
    pub message: String,
    /// Additional context.
    pub context: ErrorContext,
    /// The source error (if any).
    #[source]
    pub source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl fmt::Display for QueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.code.code(), self.message)
    }
}

impl QueryError {
    /// Create a new error with the given code and message.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            context: ErrorContext::default(),
            source: None,
        }
    }

    /// Add context about the operation.
    pub fn with_context(mut self, operation: impl Into<String>) -> Self {
        self.context.operation = Some(operation.into());
        self
    }

    /// Add a suggestion for fixing the error.
    pub fn with_suggestion(mut self, suggestion: impl Into<String>) -> Self {
        self.context.suggestions.push(Suggestion::new(suggestion));
        self
    }

    /// Add a code suggestion.
    pub fn with_code_suggestion(
        mut self,
        text: impl Into<String>,
        code: impl Into<String>,
    ) -> Self {
        self.context
            .suggestions
            .push(Suggestion::new(text).with_code(code));
        self
    }

    /// Add help text.
    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.context.help = Some(help.into());
        self
    }

    /// Set the model.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.context.model = Some(model.into());
        self
    }

    /// Set the field.
    pub fn with_field(mut self, field: impl Into<String>) -> Self {
        self.context.field = Some(field.into());
        self
    }

    /// Set the SQL query.
    pub fn with_sql(mut self, sql: impl Into<String>) -> Self {
        self.context.sql = Some(sql.into());
        self
    }

    /// Set the source error.
    pub fn with_source<E: std::error::Error + Send + Sync + 'static>(mut self, source: E) -> Self {
        self.source = Some(Box::new(source));
        self
    }

    // ============== Constructor Functions ==============

    /// Create a not found error.
    pub fn not_found(model: impl Into<String>) -> Self {
        let model = model.into();
        Self::new(
            ErrorCode::RecordNotFound,
            format!("No {} record found matching the query", model),
        )
        .with_model(&model)
        .with_suggestion(format!("Verify the {} exists before querying", model))
        .with_code_suggestion(
            "Use findFirst() instead to get None instead of an error",
            format!(
                "client.{}().find_first().r#where(...).exec().await",
                model.to_lowercase()
            ),
        )
    }

    /// Create a not unique error.
    pub fn not_unique(model: impl Into<String>) -> Self {
        let model = model.into();
        Self::new(
            ErrorCode::NotUnique,
            format!("Expected unique {} record but found multiple", model),
        )
        .with_model(&model)
        .with_suggestion("Add more specific filters to narrow down to a single record")
        .with_suggestion("Use find_many() if you expect multiple results")
    }

    /// Create a constraint violation error.
    pub fn constraint_violation(model: impl Into<String>, message: impl Into<String>) -> Self {
        let model = model.into();
        let message = message.into();
        Self::new(
            ErrorCode::UniqueConstraint,
            format!("Constraint violation on {}: {}", model, message),
        )
        .with_model(&model)
    }

    /// Create a unique constraint violation error.
    pub fn unique_violation(model: impl Into<String>, field: impl Into<String>) -> Self {
        let model = model.into();
        let field = field.into();
        Self::new(
            ErrorCode::UniqueConstraint,
            format!("Unique constraint violated on {}.{}", model, field),
        )
        .with_model(&model)
        .with_field(&field)
        .with_suggestion(format!("A record with this {} already exists", field))
        .with_code_suggestion(
            "Use upsert() to update if exists, create if not",
            format!(
                "client.{}().upsert()\n  .r#where({}::{}::equals(value))\n  .create(...)\n  .update(...)\n  .exec().await",
                model.to_lowercase(), model.to_lowercase(), field
            ),
        )
    }

    /// Create a foreign key violation error.
    pub fn foreign_key_violation(model: impl Into<String>, relation: impl Into<String>) -> Self {
        let model = model.into();
        let relation = relation.into();
        Self::new(
            ErrorCode::ForeignKeyConstraint,
            format!("Foreign key constraint violated: {} -> {}", model, relation),
        )
        .with_model(&model)
        .with_field(&relation)
        .with_suggestion(format!(
            "Ensure the related {} record exists before creating this {}",
            relation, model
        ))
        .with_suggestion("Check for typos in the relation ID")
    }

    /// Create a not null violation error.
    pub fn not_null_violation(model: impl Into<String>, field: impl Into<String>) -> Self {
        let model = model.into();
        let field = field.into();
        Self::new(
            ErrorCode::NotNullConstraint,
            format!("Cannot set {}.{} to null - field is required", model, field),
        )
        .with_model(&model)
        .with_field(&field)
        .with_suggestion(format!("Provide a value for the {} field", field))
        .with_help("Make the field optional in your schema if null should be allowed")
    }

    /// Create an invalid input error.
    pub fn invalid_input(field: impl Into<String>, message: impl Into<String>) -> Self {
        let field = field.into();
        let message = message.into();
        Self::new(
            ErrorCode::InvalidParameter,
            format!("Invalid input for {}: {}", field, message),
        )
        .with_field(&field)
    }

    /// Create a connection error.
    pub fn connection(message: impl Into<String>) -> Self {
        let message = message.into();
        Self::new(
            ErrorCode::ConnectionFailed,
            format!("Connection error: {}", message),
        )
        .with_suggestion("Check that the database server is running")
        .with_suggestion("Verify the connection URL is correct")
        .with_suggestion("Check firewall settings allow the connection")
    }

    /// Create a connection timeout error.
    pub fn connection_timeout(duration_ms: u64) -> Self {
        Self::new(
            ErrorCode::ConnectionTimeout,
            format!("Connection timed out after {}ms", duration_ms),
        )
        .with_suggestion("Increase the connect_timeout in your connection string")
        .with_suggestion("Check network connectivity to the database server")
        .with_code_suggestion(
            "Add connect_timeout to your connection URL",
            "postgres://user:pass@host/db?connect_timeout=30",
        )
    }

    /// Create a pool exhausted error.
    pub fn pool_exhausted(max_connections: u32) -> Self {
        Self::new(
            ErrorCode::PoolExhausted,
            format!("Connection pool exhausted (max {} connections)", max_connections),
        )
        .with_suggestion("Increase max_connections in pool configuration")
        .with_suggestion("Ensure connections are being released properly")
        .with_suggestion("Check for connection leaks in your application")
        .with_help("Consider using connection pooling middleware like PgBouncer for high-traffic applications")
    }

    /// Create an authentication error.
    pub fn authentication_failed(message: impl Into<String>) -> Self {
        let message = message.into();
        Self::new(
            ErrorCode::AuthenticationFailed,
            format!("Authentication failed: {}", message),
        )
        .with_suggestion("Check username and password in connection string")
        .with_suggestion("Verify the user has permission to access the database")
        .with_suggestion("Check pg_hba.conf (PostgreSQL) or user privileges (MySQL)")
    }

    /// Create a timeout error.
    pub fn timeout(duration_ms: u64) -> Self {
        Self::new(
            ErrorCode::QueryTimeout,
            format!("Query timed out after {}ms", duration_ms),
        )
        .with_suggestion("Optimize the query to run faster")
        .with_suggestion("Add indexes to improve query performance")
        .with_suggestion("Increase the query timeout if the query is expected to be slow")
        .with_help("Consider paginating large result sets")
    }

    /// Create a transaction error.
    pub fn transaction(message: impl Into<String>) -> Self {
        let message = message.into();
        Self::new(
            ErrorCode::TransactionFailed,
            format!("Transaction error: {}", message),
        )
    }

    /// Create a deadlock error.
    pub fn deadlock() -> Self {
        Self::new(
            ErrorCode::Deadlock,
            "Deadlock detected - transaction was rolled back".to_string(),
        )
        .with_suggestion("Retry the transaction")
        .with_suggestion("Access tables in a consistent order across transactions")
        .with_suggestion("Keep transactions short to reduce lock contention")
        .with_help("Deadlocks occur when two transactions wait for each other's locks")
    }

    /// Create an SQL syntax error.
    pub fn sql_syntax(message: impl Into<String>, sql: impl Into<String>) -> Self {
        let message = message.into();
        let sql = sql.into();
        Self::new(
            ErrorCode::SqlSyntax,
            format!("SQL syntax error: {}", message),
        )
        .with_sql(&sql)
        .with_suggestion("Check the generated SQL for errors")
        .with_help("This is likely a bug in Prax - please report it")
    }

    /// Create a serialization error.
    pub fn serialization(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::SerializationError, message.into())
    }

    /// Create a deserialization error.
    pub fn deserialization(message: impl Into<String>) -> Self {
        let message = message.into();
        Self::new(
            ErrorCode::DeserializationError,
            format!("Failed to deserialize result: {}", message),
        )
        .with_suggestion("Check that the model matches the database schema")
        .with_suggestion("Ensure data types are compatible")
    }

    /// Create a general database error.
    pub fn database(message: impl Into<String>) -> Self {
        let message = message.into();
        Self::new(ErrorCode::DatabaseError, message)
            .with_suggestion("Check the database logs for more details")
    }

    /// Create an internal error.
    pub fn internal(message: impl Into<String>) -> Self {
        let message = message.into();
        Self::new(ErrorCode::Internal, format!("Internal error: {}", message))
            .with_help("This is likely a bug in Prax ORM - please report it at https://github.com/pegasusheavy/prax-orm/issues")
    }

    /// Create an unsupported operation error.
    pub fn unsupported(message: impl Into<String>) -> Self {
        let message = message.into();
        Self::new(
            ErrorCode::InvalidConfiguration,
            format!("Unsupported: {}", message),
        )
        .with_help("This operation is not supported by the current database driver")
    }

    // ============== Error Checks ==============

    /// Check if this is a not found error.
    pub fn is_not_found(&self) -> bool {
        self.code == ErrorCode::RecordNotFound
    }

    /// Check if this is a constraint violation.
    pub fn is_constraint_violation(&self) -> bool {
        matches!(
            self.code,
            ErrorCode::UniqueConstraint
                | ErrorCode::ForeignKeyConstraint
                | ErrorCode::CheckConstraint
                | ErrorCode::NotNullConstraint
        )
    }

    /// Check if this is a timeout error.
    pub fn is_timeout(&self) -> bool {
        matches!(
            self.code,
            ErrorCode::QueryTimeout | ErrorCode::ConnectionTimeout
        )
    }

    /// Check if this is a connection error.
    pub fn is_connection_error(&self) -> bool {
        matches!(
            self.code,
            ErrorCode::ConnectionFailed
                | ErrorCode::PoolExhausted
                | ErrorCode::ConnectionTimeout
                | ErrorCode::AuthenticationFailed
                | ErrorCode::SslError
        )
    }

    /// Check if this error is retryable.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self.code,
            ErrorCode::ConnectionTimeout
                | ErrorCode::PoolExhausted
                | ErrorCode::QueryTimeout
                | ErrorCode::Deadlock
                | ErrorCode::SerializationFailure
        )
    }

    // ============== Display Functions ==============

    /// Get the error code.
    pub fn error_code(&self) -> &ErrorCode {
        &self.code
    }

    /// Get the documentation URL for this error.
    pub fn docs_url(&self) -> String {
        self.code.docs_url()
    }

    /// Display the full error with all context and suggestions.
    pub fn display_full(&self) -> String {
        let mut output = String::new();

        // Error header
        output.push_str(&format!("Error [{}]: {}\n", self.code.code(), self.message));

        // Context
        if let Some(ref op) = self.context.operation {
            output.push_str(&format!("  → While: {}\n", op));
        }
        if let Some(ref model) = self.context.model {
            output.push_str(&format!("  → Model: {}\n", model));
        }
        if let Some(ref field) = self.context.field {
            output.push_str(&format!("  → Field: {}\n", field));
        }

        // SQL (truncated if too long)
        if let Some(ref sql) = self.context.sql {
            let sql_display = if sql.len() > 200 {
                format!("{}...", &sql[..200])
            } else {
                sql.clone()
            };
            output.push_str(&format!("  → SQL: {}\n", sql_display));
        }

        // Suggestions
        if !self.context.suggestions.is_empty() {
            output.push_str("\nSuggestions:\n");
            for (i, suggestion) in self.context.suggestions.iter().enumerate() {
                output.push_str(&format!("  {}. {}\n", i + 1, suggestion.text));
                if let Some(ref code) = suggestion.code {
                    output.push_str(&format!(
                        "     ```\n     {}\n     ```\n",
                        code.replace('\n', "\n     ")
                    ));
                }
            }
        }

        // Help
        if let Some(ref help) = self.context.help {
            output.push_str(&format!("\nHelp: {}\n", help));
        }

        // Documentation link
        output.push_str(&format!("\nMore info: {}\n", self.docs_url()));

        output
    }

    /// Display error with ANSI colors for terminal output.
    pub fn display_colored(&self) -> String {
        let mut output = String::new();

        // Error header (red)
        output.push_str(&format!(
            "\x1b[1;31mError [{}]\x1b[0m: \x1b[1m{}\x1b[0m\n",
            self.code.code(),
            self.message
        ));

        // Context (dim)
        if let Some(ref op) = self.context.operation {
            output.push_str(&format!("  \x1b[2m→ While:\x1b[0m {}\n", op));
        }
        if let Some(ref model) = self.context.model {
            output.push_str(&format!("  \x1b[2m→ Model:\x1b[0m {}\n", model));
        }
        if let Some(ref field) = self.context.field {
            output.push_str(&format!("  \x1b[2m→ Field:\x1b[0m {}\n", field));
        }

        // Suggestions (yellow)
        if !self.context.suggestions.is_empty() {
            output.push_str("\n\x1b[1;33mSuggestions:\x1b[0m\n");
            for (i, suggestion) in self.context.suggestions.iter().enumerate() {
                output.push_str(&format!(
                    "  \x1b[33m{}.\x1b[0m {}\n",
                    i + 1,
                    suggestion.text
                ));
                if let Some(ref code) = suggestion.code {
                    output.push_str(&format!(
                        "     \x1b[2m```\x1b[0m\n     \x1b[36m{}\x1b[0m\n     \x1b[2m```\x1b[0m\n",
                        code.replace('\n', "\n     ")
                    ));
                }
            }
        }

        // Help (cyan)
        if let Some(ref help) = self.context.help {
            output.push_str(&format!("\n\x1b[1;36mHelp:\x1b[0m {}\n", help));
        }

        // Documentation link (blue)
        output.push_str(&format!(
            "\n\x1b[2mMore info:\x1b[0m \x1b[4;34m{}\x1b[0m\n",
            self.docs_url()
        ));

        output
    }
}

/// Extension trait for converting errors to QueryError.
pub trait IntoQueryError {
    /// Convert to a QueryError.
    fn into_query_error(self) -> QueryError;
}

impl<E: std::error::Error + Send + Sync + 'static> IntoQueryError for E {
    fn into_query_error(self) -> QueryError {
        QueryError::internal(self.to_string()).with_source(self)
    }
}

/// Helper for creating errors with context.
#[macro_export]
macro_rules! query_error {
    ($code:expr, $msg:expr) => {
        $crate::error::QueryError::new($code, $msg)
    };
    ($code:expr, $msg:expr, $($key:ident = $value:expr),+ $(,)?) => {{
        let mut err = $crate::error::QueryError::new($code, $msg);
        $(
            err = err.$key($value);
        )+
        err
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_code_format() {
        assert_eq!(ErrorCode::RecordNotFound.code(), "P1001");
        assert_eq!(ErrorCode::UniqueConstraint.code(), "P2001");
        assert_eq!(ErrorCode::ConnectionFailed.code(), "P3001");
    }

    #[test]
    fn test_not_found_error() {
        let err = QueryError::not_found("User");
        assert!(err.is_not_found());
        assert!(err.message.contains("User"));
        assert!(!err.context.suggestions.is_empty());
    }

    #[test]
    fn test_unique_violation_error() {
        let err = QueryError::unique_violation("User", "email");
        assert!(err.is_constraint_violation());
        assert_eq!(err.context.model, Some("User".to_string()));
        assert_eq!(err.context.field, Some("email".to_string()));
    }

    #[test]
    fn test_timeout_error() {
        let err = QueryError::timeout(5000);
        assert!(err.is_timeout());
        assert!(err.message.contains("5000"));
    }

    #[test]
    fn test_error_with_context() {
        let err = QueryError::not_found("User")
            .with_context("Finding user by email")
            .with_suggestion("Use a different query method");

        assert_eq!(
            err.context.operation,
            Some("Finding user by email".to_string())
        );
        assert!(err.context.suggestions.len() >= 2); // Original + new one
    }

    #[test]
    fn test_retryable_errors() {
        assert!(QueryError::timeout(1000).is_retryable());
        assert!(QueryError::deadlock().is_retryable());
        assert!(QueryError::pool_exhausted(10).is_retryable());
        assert!(!QueryError::not_found("User").is_retryable());
    }

    #[test]
    fn test_connection_errors() {
        assert!(QueryError::connection("failed").is_connection_error());
        assert!(QueryError::authentication_failed("bad password").is_connection_error());
        assert!(QueryError::pool_exhausted(10).is_connection_error());
    }

    #[test]
    fn test_display_full() {
        let err = QueryError::unique_violation("User", "email").with_context("Creating new user");

        let output = err.display_full();
        assert!(output.contains("P2001"));
        assert!(output.contains("User"));
        assert!(output.contains("email"));
        assert!(output.contains("Suggestions"));
    }

    #[test]
    fn test_docs_url() {
        let err = QueryError::not_found("User");
        assert!(err.docs_url().contains("P1001"));
    }

    #[test]
    fn test_error_macro() {
        let err = query_error!(
            ErrorCode::InvalidParameter,
            "Invalid email format",
            with_field = "email",
            with_suggestion = "Use a valid email address"
        );

        assert_eq!(err.code, ErrorCode::InvalidParameter);
        assert_eq!(err.context.field, Some("email".to_string()));
    }

    #[test]
    fn test_suggestion_with_code() {
        let err = QueryError::not_found("User")
            .with_code_suggestion("Try this instead", "client.user().find_first()");

        let suggestion = err.context.suggestions.last().unwrap();
        assert!(suggestion.code.is_some());
    }

    #[test]
    fn with_source_populates_std_error_source_chain() {
        use std::error::Error;
        let io_err = std::io::Error::new(std::io::ErrorKind::ConnectionAborted, "boom");
        let q = QueryError::connection("could not connect").with_source(io_err);
        let src = q
            .source()
            .expect("source() should return the chained error");
        assert!(src.to_string().contains("boom"));
    }
}
