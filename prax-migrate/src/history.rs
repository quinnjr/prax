//! Migration history tracking.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::MigrateResult;

/// A record of an applied migration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationRecord {
    /// Migration ID/name.
    pub id: String,
    /// Checksum of the migration content.
    pub checksum: String,
    /// When the migration was applied.
    pub applied_at: DateTime<Utc>,
    /// Duration of the migration in milliseconds.
    pub duration_ms: i64,
    /// Whether this migration was rolled back.
    pub rolled_back: bool,
}

/// Migration history repository.
#[async_trait::async_trait]
pub trait MigrationHistoryRepository: Send + Sync {
    /// Initialize the migrations table.
    async fn initialize(&self) -> MigrateResult<()>;

    /// Get all applied migrations.
    async fn get_applied(&self) -> MigrateResult<Vec<MigrationRecord>>;

    /// Check if a migration has been applied.
    async fn is_applied(&self, id: &str) -> MigrateResult<bool>;

    /// Record a migration as applied.
    async fn record_applied(&self, id: &str, checksum: &str, duration_ms: i64)
    -> MigrateResult<()>;

    /// Mark a migration as rolled back.
    async fn record_rollback(&self, id: &str) -> MigrateResult<()>;

    /// Get the last applied migration.
    async fn get_last_applied(&self) -> MigrateResult<Option<MigrationRecord>>;

    /// Acquire an exclusive lock for migrations.
    async fn acquire_lock(&self) -> MigrateResult<MigrationLock>;
}

/// Migration lock to prevent concurrent migrations.
pub struct MigrationLock {
    lock_id: i64,
    release_fn: Option<Box<dyn FnOnce() + Send>>,
}

impl MigrationLock {
    /// Create a new migration lock.
    pub fn new(lock_id: i64, release: impl FnOnce() + Send + 'static) -> Self {
        Self {
            lock_id,
            release_fn: Some(Box::new(release)),
        }
    }

    /// Get the lock ID.
    pub fn id(&self) -> i64 {
        self.lock_id
    }
}

impl Drop for MigrationLock {
    fn drop(&mut self) {
        if let Some(release) = self.release_fn.take() {
            release();
        }
    }
}

/// SQL for initializing the migrations table (PostgreSQL).
pub const POSTGRES_INIT_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS "_prax_migrations" (
    id VARCHAR(255) PRIMARY KEY,
    checksum VARCHAR(64) NOT NULL,
    applied_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    duration_ms BIGINT NOT NULL DEFAULT 0,
    rolled_back BOOLEAN NOT NULL DEFAULT FALSE
);

CREATE INDEX IF NOT EXISTS "_prax_migrations_applied_at_idx"
    ON "_prax_migrations" (applied_at DESC);
"#;

/// SQL for advisory lock (PostgreSQL).
pub const POSTGRES_LOCK_SQL: &str = "SELECT pg_advisory_lock(42424242)";
pub const POSTGRES_UNLOCK_SQL: &str = "SELECT pg_advisory_unlock(42424242)";

/// SQL for initializing the event log table (PostgreSQL V2).
pub const POSTGRES_EVENT_LOG_INIT_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS "_prax_migrations" (
    event_id BIGSERIAL PRIMARY KEY,
    migration_id VARCHAR(255) NOT NULL,
    event_type VARCHAR(50) NOT NULL,
    event_data JSONB NOT NULL,
    created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    CONSTRAINT valid_event_type CHECK (event_type IN ('applied', 'rolled_back', 'failed', 'resolved'))
);

CREATE INDEX IF NOT EXISTS "_prax_migrations_migration_id_created_at_idx"
    ON "_prax_migrations" (migration_id, created_at DESC);

CREATE INDEX IF NOT EXISTS "_prax_migrations_event_type_idx"
    ON "_prax_migrations" (event_type);

CREATE INDEX IF NOT EXISTS "_prax_migrations_created_at_idx"
    ON "_prax_migrations" (created_at DESC);
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_migration_record() {
        let record = MigrationRecord {
            id: "20231215_create_users".to_string(),
            checksum: "abc123".to_string(),
            applied_at: Utc::now(),
            duration_ms: 150,
            rolled_back: false,
        };

        assert!(!record.rolled_back);
        assert!(record.duration_ms > 0);
    }

    #[test]
    fn test_init_sql_has_table() {
        assert!(POSTGRES_INIT_SQL.contains("_prax_migrations"));
        assert!(POSTGRES_INIT_SQL.contains("checksum"));
    }
}
