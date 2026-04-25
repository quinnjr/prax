//! Error types for the migration engine.

use thiserror::Error;

/// Result type alias for migration operations.
pub type MigrateResult<T> = Result<T, MigrationError>;

/// Errors that can occur during migration operations.
#[derive(Debug, Error)]
pub enum MigrationError {
    /// File system error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Database operation error.
    #[error("Database error: {0}")]
    Database(String),

    /// Schema parsing error.
    #[error("Schema error: {0}")]
    Schema(String),

    /// Invalid migration file or format.
    #[error("Invalid migration: {0}")]
    InvalidMigration(String),

    /// Migration checksum mismatch.
    #[error("Checksum mismatch for migration '{id}': expected {expected}, got {actual}")]
    ChecksumMismatch {
        /// Migration ID.
        id: String,
        /// Expected checksum.
        expected: String,
        /// Actual checksum.
        actual: String,
    },

    /// Migration already applied.
    #[error("Migration '{0}' has already been applied")]
    AlreadyApplied(String),

    /// Migration not found.
    #[error("Migration '{0}' not found")]
    NotFound(String),

    /// Data loss would occur.
    #[error("Data loss would occur: {0}")]
    DataLoss(String),

    /// Lock acquisition failed.
    #[error("Failed to acquire migration lock: {0}")]
    LockFailed(String),

    /// No changes to migrate.
    #[error("No schema changes detected")]
    NoChanges,

    /// Rollback not possible.
    #[error("Cannot rollback: {0}")]
    RollbackFailed(String),

    /// Shadow database error.
    #[error("Shadow database error: {0}")]
    ShadowDatabaseError(String),

    /// Resolution file error.
    #[error("Resolution file error: {0}")]
    ResolutionFile(String),

    /// Resolution conflict.
    #[error("Resolution conflict: {0}")]
    ResolutionConflict(String),

    /// Migration conflict detected.
    #[error("Migration conflict: migrations '{0}' and '{1}' conflict")]
    MigrationConflict(String, String),

    /// Bootstrap migration failed.
    #[error("Bootstrap migration failed: {0}")]
    BootstrapFailed(String),

    /// No migrations to rollback.
    #[error("No migrations to rollback")]
    NoMigrationsToRollback,

    /// Parent event not found for rollback.
    #[error("Parent event not found for rollback")]
    ParentEventNotFound,

    /// Migration file not found.
    #[error("Migration file not found: {0}")]
    MigrationFileNotFound(String),

    /// No down migration available.
    #[error("No down migration available for: {0}")]
    NoDownMigration(String),

    /// General migration error.
    #[error("Migration error: {0}")]
    Other(String),
}

impl MigrationError {
    /// Create a database error.
    pub fn database(msg: impl Into<String>) -> Self {
        Self::Database(msg.into())
    }

    /// Create a schema error.
    pub fn schema(msg: impl Into<String>) -> Self {
        Self::Schema(msg.into())
    }

    /// Create a data loss error.
    pub fn data_loss(msg: impl Into<String>) -> Self {
        Self::DataLoss(msg.into())
    }

    /// Create a lock failed error.
    pub fn lock_failed(msg: impl Into<String>) -> Self {
        Self::LockFailed(msg.into())
    }

    /// Create an other error.
    pub fn other(msg: impl Into<String>) -> Self {
        Self::Other(msg.into())
    }

    /// Create a shadow database error.
    pub fn shadow_database(msg: impl Into<String>) -> Self {
        Self::ShadowDatabaseError(msg.into())
    }

    /// Create a resolution file error.
    pub fn resolution_file(msg: impl Into<String>) -> Self {
        Self::ResolutionFile(msg.into())
    }

    /// Create a resolution conflict error.
    pub fn resolution_conflict(msg: impl Into<String>) -> Self {
        Self::ResolutionConflict(msg.into())
    }

    /// Create a migration conflict error.
    pub fn migration_conflict(m1: impl Into<String>, m2: impl Into<String>) -> Self {
        Self::MigrationConflict(m1.into(), m2.into())
    }

    /// Create a migration file error.
    pub fn migration_file(msg: impl Into<String>) -> Self {
        Self::InvalidMigration(msg.into())
    }

    /// Check if this is a recoverable error.
    pub fn is_recoverable(&self) -> bool {
        matches!(
            self,
            Self::LockFailed(_) | Self::AlreadyApplied(_) | Self::NoChanges
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = MigrationError::NotFound("20231215_test".to_string());
        assert!(err.to_string().contains("20231215_test"));
    }

    #[test]
    fn test_checksum_mismatch_display() {
        let err = MigrationError::ChecksumMismatch {
            id: "test".to_string(),
            expected: "abc".to_string(),
            actual: "xyz".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("abc"));
        assert!(msg.contains("xyz"));
    }

    #[test]
    fn test_is_recoverable() {
        assert!(MigrationError::NoChanges.is_recoverable());
        assert!(MigrationError::LockFailed("timeout".to_string()).is_recoverable());
        assert!(!MigrationError::Database("connection".to_string()).is_recoverable());
    }
}
