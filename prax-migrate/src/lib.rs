//! # prax-migrate
//!
//! Migration engine for the Prax ORM.
//!
//! This crate provides functionality for:
//! - Schema diffing between Prax schema definitions and database state
//! - SQL migration generation for PostgreSQL (with MySQL/SQLite planned)
//! - Migration file management on the filesystem
//! - Migration history tracking in the database
//! - Safe, transactional migration application and rollback
//! - **Resolution system** for handling migration conflicts and checksums
//!
//! ## Architecture
//!
//! The migration engine compares your Prax schema definition with the current
//! database state and generates SQL scripts to bring the database up to date.
//! It tracks applied migrations in a `_prax_migrations` table.
//!
//! ```text
//! ┌──────────────┐     ┌────────────────┐     ┌─────────────┐
//! │ Prax Schema  │────▶│ Schema Differ  │────▶│ SQL Gen     │
//! └──────────────┘     └────────────────┘     └─────────────┘
//!                              │                     │
//!                              ▼                     ▼
//!                      ┌────────────────┐     ┌─────────────┐
//!                      │ Migration Plan │────▶│ Apply SQL   │
//!                      └────────────────┘     └─────────────┘
//!                                                    │
//!                                                    ▼
//!                                            ┌─────────────┐
//!                                            │ History Tbl │
//!                                            └─────────────┘
//! ```
//!
//! ## Example
//!
//! ```rust,ignore
//! use prax_migrate::{MigrationConfig, MigrationEngine};
//!
//! async fn run_migrations() -> Result<(), Box<dyn std::error::Error>> {
//!     // Parse your schema
//!     let schema = prax_schema::parse_schema(r#"
//!         model User {
//!             id      Int @id @auto
//!             email   String @unique
//!             name    String?
//!         }
//!     "#)?;
//!
//!     // Configure migrations
//!     let config = MigrationConfig::new()
//!         .migrations_dir("./migrations");
//!
//!     // Create engine with your history repository
//!     let history = /* your history implementation */;
//!     let engine = MigrationEngine::new(config, history);
//!
//!     // Initialize (creates migrations table)
//!     engine.initialize().await?;
//!
//!     // Plan migrations
//!     let plan = engine.plan(&schema).await?;
//!     println!("Plan: {}", plan.summary());
//!
//!     // Apply migrations
//!     let result = engine.migrate().await?;
//!     println!("Applied {} migrations in {}ms",
//!         result.applied_count, result.duration_ms);
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Migration Files
//!
//! Migrations are stored as directories with `up.sql` and `down.sql` files:
//!
//! ```text
//! migrations/
//! ├── 20231215120000_create_users/
//! │   ├── up.sql
//! │   └── down.sql
//! ├── 20231216090000_add_posts/
//! │   ├── up.sql
//! │   └── down.sql
//! └── resolutions.toml        # Migration resolutions
//! ```
//!
//! ## Resolution System
//!
//! The resolution system handles common migration issues:
//!
//! - **Checksum Mismatches**: When a migration is modified after being applied
//! - **Skipped Migrations**: Intentionally skip migrations (e.g., legacy tables)
//! - **Baseline Migrations**: Mark migrations as applied without running them
//! - **Renamed Migrations**: Map old migration IDs to new ones
//! - **Conflict Resolution**: Handle conflicts between migrations
//!
//! ```rust,ignore
//! use prax_migrate::{Resolution, ResolutionConfig};
//!
//! let mut resolutions = ResolutionConfig::new();
//!
//! // Accept a checksum change
//! resolutions.add(Resolution::accept_checksum(
//!     "20240101_create_users",
//!     "old_checksum",
//!     "new_checksum",
//!     "Fixed column type",
//! ));
//!
//! // Skip a migration
//! resolutions.add(Resolution::skip(
//!     "20240102_legacy_table",
//!     "Already exists in production",
//! ));
//!
//! // Mark as baseline (applied without running)
//! resolutions.add(Resolution::baseline(
//!     "20240103_initial",
//!     "Database was imported from backup",
//! ));
//!
//! // Save to file
//! resolutions.save("migrations/resolutions.toml").await?;
//! ```

pub mod bootstrap;
pub mod diff;
pub mod engine;
pub mod error;
pub mod event;
pub mod event_store;
pub mod file;
pub mod history;
pub mod introspect;
pub mod procedure;
pub mod resolution;
pub mod shadow;
pub mod sql;
pub mod state;

// Re-exports
pub use bootstrap::Bootstrap;
pub use diff::{
    EnumAlterDiff, EnumDiff, FieldAlterDiff, FieldDiff, IndexDiff, ModelAlterDiff, ModelDiff,
    SchemaDiff, SchemaDiffer, UniqueConstraint,
};
pub use engine::{
    DevResult, MigrationConfig, MigrationEngine, MigrationPlan, MigrationResult, MigrationStatus,
};
pub use error::{MigrateResult, MigrationError};
pub use event::{EventData, EventType, MigrationEvent};
pub use event_store::{InMemoryEventStore, MigrationEventStore};
pub use file::{MigrationFile, MigrationFileManager};
pub use history::{MigrationHistoryRepository, MigrationLock, MigrationRecord};
pub use introspect::{
    ColumnInfo, ConstraintInfo, EnumInfo, IndexInfo, IntrospectionConfig, IntrospectionResult,
    Introspector, SchemaBuilder, SkippedTable, TableInfo,
};
pub use procedure::{
    // MongoDB Atlas Triggers
    AtlasOperation,
    AtlasTrigger,
    AtlasTriggerType,
    AuthOperation,
    // Procedure types
    ChangeType,
    // Event Scheduler types (MySQL)
    EventAlterDiff,
    EventDiff,
    EventInterval,
    EventSchedule,
    IntervalUnit,
    // SQL Agent types (MSSQL)
    JobSchedule,
    JobStep,
    NotifyLevel,
    OnCompletion,
    ParallelSafety,
    ParameterMode,
    ProcedureAlterDiff,
    ProcedureChange,
    ProcedureDefinition,
    ProcedureDiff,
    ProcedureDiffer,
    ProcedureHistoryEntry,
    ProcedureLanguage,
    ProcedureParameter,
    ProcedureSqlGenerator,
    ProcedureStore,
    ReturnColumn,
    ScheduleFrequency,
    ScheduledEvent,
    SqlAgentJob,
    StepAction,
    StepType,
    // Trigger types
    TriggerAlterDiff,
    TriggerDefinition,
    TriggerEvent,
    TriggerLevel,
    TriggerTiming,
    Volatility,
    Weekday,
};
pub use resolution::{
    ConflictStrategy, Resolution, ResolutionAction, ResolutionBuilder, ResolutionConfig,
    ResolutionCounts, ResolutionWarning,
};
pub use shadow::{
    FieldDrift, IndexDrift, SchemaDrift, ShadowConfig, ShadowDatabase, ShadowDatabaseManager,
    ShadowDiffResult, ShadowState, detect_drift,
};
pub use sql::{MigrationSql, PostgresSqlGenerator};
pub use state::MigrationState;
// Note: state::MigrationStatus not re-exported to avoid conflict with engine::MigrationStatus
// Access via prax_migrate::state::MigrationStatus if needed
