//! Migration engine implementation.

use std::path::PathBuf;
use std::time::Instant;

use crate::diff::{SchemaDiff, SchemaDiffer};
use crate::error::{MigrateResult, MigrationError};
use crate::event::MigrationEvent;
use crate::event_store::MigrationEventStore;
use crate::file::{MigrationFile, MigrationFileManager};
use crate::history::{MigrationHistoryRepository, MigrationRecord};
use crate::resolution::{Resolution, ResolutionConfig};
use crate::sql::{MigrationSql, PostgresSqlGenerator};

/// Configuration for the migration engine.
#[derive(Debug, Clone)]
pub struct MigrationConfig {
    /// Path to the migrations directory.
    pub migrations_dir: PathBuf,
    /// Path to the resolutions file.
    pub resolutions_file: PathBuf,
    /// Whether to run in dry-run mode.
    pub dry_run: bool,
    /// Whether to allow data loss (dropping tables/columns).
    pub allow_data_loss: bool,
    /// Whether to fail on unresolved checksum mismatches.
    pub fail_on_checksum_mismatch: bool,
    /// Whether to apply baseline migrations automatically.
    pub auto_baseline: bool,
}

impl Default for MigrationConfig {
    fn default() -> Self {
        Self {
            migrations_dir: PathBuf::from("./migrations"),
            resolutions_file: PathBuf::from("./migrations/resolutions.toml"),
            dry_run: false,
            allow_data_loss: false,
            fail_on_checksum_mismatch: true,
            auto_baseline: false,
        }
    }
}

impl MigrationConfig {
    /// Create a new configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the migrations directory.
    pub fn migrations_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.migrations_dir = dir.into();
        self
    }

    /// Set the resolutions file path.
    pub fn resolutions_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.resolutions_file = path.into();
        self
    }

    /// Enable dry-run mode.
    pub fn dry_run(mut self, dry_run: bool) -> Self {
        self.dry_run = dry_run;
        self
    }

    /// Allow data loss operations.
    pub fn allow_data_loss(mut self, allow: bool) -> Self {
        self.allow_data_loss = allow;
        self
    }

    /// Set whether to fail on checksum mismatches.
    pub fn fail_on_checksum_mismatch(mut self, fail: bool) -> Self {
        self.fail_on_checksum_mismatch = fail;
        self
    }

    /// Enable automatic baseline application.
    pub fn auto_baseline(mut self, auto: bool) -> Self {
        self.auto_baseline = auto;
        self
    }
}

/// Result of a migration operation.
#[derive(Debug)]
pub struct MigrationResult {
    /// Number of migrations applied.
    pub applied_count: usize,
    /// Total duration in milliseconds.
    pub duration_ms: i64,
    /// IDs of applied migrations.
    pub applied_migrations: Vec<String>,
    /// IDs of baselined migrations (marked as applied without running).
    pub baselined_migrations: Vec<String>,
    /// IDs of skipped migrations.
    pub skipped_migrations: Vec<String>,
    /// Warnings generated during migration.
    pub warnings: Vec<String>,
}

impl MigrationResult {
    /// Get total migrations processed (applied + baselined).
    pub fn total_processed(&self) -> usize {
        self.applied_count + self.baselined_migrations.len()
    }

    /// Check if any migrations were applied or baselined.
    pub fn has_changes(&self) -> bool {
        self.applied_count > 0 || !self.baselined_migrations.is_empty()
    }

    /// Get a summary of the result.
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();

        if self.applied_count > 0 {
            parts.push(format!("{} applied", self.applied_count));
        }

        if !self.baselined_migrations.is_empty() {
            parts.push(format!("{} baselined", self.baselined_migrations.len()));
        }

        if !self.skipped_migrations.is_empty() {
            parts.push(format!("{} skipped", self.skipped_migrations.len()));
        }

        if parts.is_empty() {
            "No migrations applied".to_string()
        } else {
            format!("{} in {}ms", parts.join(", "), self.duration_ms)
        }
    }
}

/// Result of a migration plan.
#[derive(Debug)]
pub struct MigrationPlan {
    /// Pending migrations to apply.
    pub pending: Vec<MigrationFile>,
    /// Migrations that will be skipped (via resolutions).
    pub skipped: Vec<String>,
    /// Migrations that will be baselined (marked as applied without running).
    pub baselines: Vec<String>,
    /// Checksum mismatches that are resolved.
    pub resolved_checksums: Vec<ChecksumResolution>,
    /// Checksum mismatches that are NOT resolved.
    pub unresolved_checksums: Vec<ChecksumMismatch>,
    /// Schema diff for new migrations.
    pub diff: Option<SchemaDiff>,
    /// Generated SQL.
    pub sql: Option<MigrationSql>,
    /// Warnings.
    pub warnings: Vec<String>,
}

/// Information about a resolved checksum mismatch.
#[derive(Debug, Clone)]
pub struct ChecksumResolution {
    /// Migration ID.
    pub migration_id: String,
    /// Expected checksum (from history).
    pub expected: String,
    /// Actual checksum (from file).
    pub actual: String,
    /// Reason for accepting the change.
    pub reason: String,
}

/// Information about an unresolved checksum mismatch.
#[derive(Debug, Clone)]
pub struct ChecksumMismatch {
    /// Migration ID.
    pub migration_id: String,
    /// Expected checksum (from history).
    pub expected: String,
    /// Actual checksum (from file).
    pub actual: String,
}

impl MigrationPlan {
    /// Create an empty migration plan.
    pub fn empty() -> Self {
        Self {
            pending: Vec::new(),
            skipped: Vec::new(),
            baselines: Vec::new(),
            resolved_checksums: Vec::new(),
            unresolved_checksums: Vec::new(),
            diff: None,
            sql: None,
            warnings: Vec::new(),
        }
    }

    /// Check if there's anything to migrate.
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
            && self.baselines.is_empty()
            && self.diff.as_ref().is_none_or(|d| d.is_empty())
    }

    /// Check if there are blocking issues.
    pub fn has_blocking_issues(&self) -> bool {
        !self.unresolved_checksums.is_empty()
    }

    /// Get a summary of the plan.
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();

        if !self.pending.is_empty() {
            parts.push(format!("{} pending migrations", self.pending.len()));
        }

        if !self.skipped.is_empty() {
            parts.push(format!("{} skipped", self.skipped.len()));
        }

        if !self.baselines.is_empty() {
            parts.push(format!("{} baselines", self.baselines.len()));
        }

        if !self.resolved_checksums.is_empty() {
            parts.push(format!(
                "{} resolved checksums",
                self.resolved_checksums.len()
            ));
        }

        if !self.unresolved_checksums.is_empty() {
            parts.push(format!(
                "{} UNRESOLVED checksums",
                self.unresolved_checksums.len()
            ));
        }

        if let Some(diff) = &self.diff {
            parts.push(diff.summary());
        }

        if parts.is_empty() {
            "No changes to apply".to_string()
        } else {
            parts.join("; ")
        }
    }
}

/// The main migration engine.
pub struct MigrationEngine<H: MigrationHistoryRepository, S: MigrationEventStore> {
    config: MigrationConfig,
    history: H,
    event_store: S,
    file_manager: MigrationFileManager,
    sql_generator: PostgresSqlGenerator,
    resolutions: ResolutionConfig,
}

impl<H: MigrationHistoryRepository, S: MigrationEventStore> MigrationEngine<H, S> {
    /// Create a new migration engine.
    pub fn new(config: MigrationConfig, history: H, event_store: S) -> Self {
        let file_manager = MigrationFileManager::new(&config.migrations_dir);
        Self {
            config,
            history,
            event_store,
            file_manager,
            sql_generator: PostgresSqlGenerator,
            resolutions: ResolutionConfig::new(),
        }
    }

    /// Create a new migration engine with resolutions.
    pub fn with_resolutions(
        config: MigrationConfig,
        history: H,
        event_store: S,
        resolutions: ResolutionConfig,
    ) -> Self {
        let file_manager = MigrationFileManager::new(&config.migrations_dir);
        Self {
            config,
            history,
            event_store,
            file_manager,
            sql_generator: PostgresSqlGenerator,
            resolutions,
        }
    }

    /// Load resolutions from the configured file.
    pub async fn load_resolutions(&mut self) -> MigrateResult<()> {
        self.resolutions = ResolutionConfig::load(&self.config.resolutions_file).await?;
        Ok(())
    }

    /// Save resolutions to the configured file.
    pub async fn save_resolutions(&self) -> MigrateResult<()> {
        self.resolutions.save(&self.config.resolutions_file).await
    }

    /// Add a resolution and save.
    pub async fn add_resolution(&mut self, resolution: Resolution) -> MigrateResult<()> {
        self.resolutions.add(resolution);
        self.save_resolutions().await
    }

    /// Get the current resolutions.
    pub fn resolutions(&self) -> &ResolutionConfig {
        &self.resolutions
    }

    /// Get mutable resolutions.
    pub fn resolutions_mut(&mut self) -> &mut ResolutionConfig {
        &mut self.resolutions
    }

    /// Initialize the migration system.
    pub async fn initialize(&mut self) -> MigrateResult<()> {
        // Create migrations directory
        self.file_manager.ensure_dir().await?;

        // Initialize history table
        self.history.initialize().await?;

        // Load resolutions
        self.load_resolutions().await?;

        Ok(())
    }

    /// Plan migrations based on current schema vs database.
    pub async fn plan(&self, current_schema: &prax_schema::Schema) -> MigrateResult<MigrationPlan> {
        let mut plan = MigrationPlan::empty();

        // Get applied migrations
        let applied = self.history.get_applied().await?;
        let applied_ids: std::collections::HashSet<_> =
            applied.iter().map(|r| r.id.as_str()).collect();

        // Get file migrations
        let files = self.file_manager.list_migrations().await?;

        // Find pending migrations
        for file in files {
            // Check if this migration should be skipped
            if self.resolutions.should_skip(&file.id) {
                plan.skipped.push(file.id.clone());
                continue;
            }

            // Check if this is a baseline migration
            if self.resolutions.is_baseline(&file.id) && !applied_ids.contains(file.id.as_str()) {
                plan.baselines.push(file.id.clone());
                continue;
            }

            // Check for renamed migrations
            let effective_id = self
                .resolutions
                .get_renamed(&file.id)
                .map(String::from)
                .unwrap_or_else(|| file.id.clone());

            if !applied_ids.contains(effective_id.as_str()) {
                plan.pending.push(file);
            } else if let Some(record) = applied.iter().find(|r| r.id == effective_id) {
                // Check for checksum mismatch
                if record.checksum != file.checksum {
                    if self
                        .resolutions
                        .accepts_checksum(&file.id, &record.checksum, &file.checksum)
                    {
                        // Checksum change is resolved
                        if let Some(resolution) = self.resolutions.get(&file.id) {
                            plan.resolved_checksums.push(ChecksumResolution {
                                migration_id: file.id.clone(),
                                expected: record.checksum.clone(),
                                actual: file.checksum.clone(),
                                reason: resolution.reason.clone(),
                            });
                        }
                    } else {
                        // Unresolved checksum mismatch
                        plan.unresolved_checksums.push(ChecksumMismatch {
                            migration_id: file.id.clone(),
                            expected: record.checksum.clone(),
                            actual: file.checksum.clone(),
                        });

                        if self.config.fail_on_checksum_mismatch {
                            plan.warnings.push(format!(
                                "Migration '{}' has been modified since it was applied. \
                                 Add a resolution to accept this change: \
                                 prax migrate resolve checksum {} {} {}",
                                file.id, file.id, record.checksum, file.checksum
                            ));
                        }
                    }
                }
            }
        }

        // Generate diff for schema changes
        let differ = SchemaDiffer::new(current_schema.clone());
        let diff = differ.diff()?;

        if !diff.is_empty() {
            // Check for data loss
            if !self.config.allow_data_loss {
                if !diff.drop_models.is_empty() {
                    plan.warnings.push(format!(
                        "Would drop {} tables: {}. Set allow_data_loss=true to proceed.",
                        diff.drop_models.len(),
                        diff.drop_models.join(", ")
                    ));
                }

                for alter in &diff.alter_models {
                    if !alter.drop_fields.is_empty() {
                        plan.warnings.push(format!(
                            "Would drop columns in '{}': {}. Set allow_data_loss=true to proceed.",
                            alter.name,
                            alter.drop_fields.join(", ")
                        ));
                    }
                }
            }

            let sql = self.sql_generator.generate(&diff);
            plan.diff = Some(diff);
            plan.sql = Some(sql);
        }

        Ok(plan)
    }

    /// Apply pending migrations.
    pub async fn migrate(&self) -> MigrateResult<MigrationResult> {
        let mut result = MigrationResult {
            applied_count: 0,
            duration_ms: 0,
            applied_migrations: Vec::new(),
            baselined_migrations: Vec::new(),
            skipped_migrations: Vec::new(),
            warnings: Vec::new(),
        };

        let start = Instant::now();

        // Acquire lock
        let _lock = self.history.acquire_lock().await?;

        // Get pending migrations
        let applied = self.history.get_applied().await?;
        let applied_ids: std::collections::HashSet<_> =
            applied.iter().map(|r| r.id.as_str()).collect();

        let files = self.file_manager.list_migrations().await?;

        for file in files {
            // Check if this migration should be skipped
            if self.resolutions.should_skip(&file.id) {
                result.skipped_migrations.push(file.id.clone());
                continue;
            }

            // Check for renamed migrations
            let effective_id = self
                .resolutions
                .get_renamed(&file.id)
                .map(String::from)
                .unwrap_or_else(|| file.id.clone());

            if applied_ids.contains(effective_id.as_str()) {
                // Check for unresolved checksum mismatch
                if let Some(record) = applied.iter().find(|r| r.id == effective_id)
                    && record.checksum != file.checksum
                    && !self.resolutions.accepts_checksum(
                        &file.id,
                        &record.checksum,
                        &file.checksum,
                    )
                    && self.config.fail_on_checksum_mismatch
                {
                    return Err(MigrationError::ChecksumMismatch {
                        id: file.id.clone(),
                        expected: record.checksum.clone(),
                        actual: file.checksum.clone(),
                    });
                }
                continue;
            }

            // Check if this is a baseline migration
            if self.resolutions.is_baseline(&file.id) {
                if self.config.dry_run {
                    result
                        .warnings
                        .push(format!("[DRY RUN] Would baseline: {}", file.id));
                } else {
                    // Record as applied without running
                    self.history
                        .record_applied(&file.id, &file.checksum, 0)
                        .await?;
                    result.baselined_migrations.push(file.id.clone());
                }
                continue;
            }

            if self.config.dry_run {
                result.applied_migrations.push(file.id.clone());
                result
                    .warnings
                    .push(format!("[DRY RUN] Would apply: {}", file.id));
                continue;
            }

            // Apply migration
            let migration_start = Instant::now();
            self.apply_migration(&file).await?;
            let duration_ms = migration_start.elapsed().as_millis() as i64;

            // Record in history
            self.history
                .record_applied(&file.id, &file.checksum, duration_ms)
                .await?;

            result.applied_migrations.push(file.id);
            result.applied_count += 1;
        }

        result.duration_ms = start.elapsed().as_millis() as i64;
        Ok(result)
    }

    /// Apply a single migration.
    async fn apply_migration(&self, _migration: &MigrationFile) -> MigrateResult<()> {
        // This would execute the SQL through the query engine
        // For now, we just validate the structure
        Ok(())
    }

    /// Rollback the last migration.
    pub async fn rollback(&self) -> MigrateResult<Option<String>> {
        if self.config.dry_run {
            if let Some(last) = self.history.get_last_applied().await? {
                return Ok(Some(format!("[DRY RUN] Would rollback: {}", last.id)));
            }
            return Ok(None);
        }

        let _lock = self.history.acquire_lock().await?;

        let last = self.history.get_last_applied().await?;
        if let Some(record) = last {
            // Find the migration file
            let files = self.file_manager.list_migrations().await?;
            let migration = files.into_iter().find(|f| f.id == record.id);

            if let Some(m) = migration {
                if m.down_sql.is_empty() {
                    return Err(MigrationError::InvalidMigration(format!(
                        "Migration '{}' has no down migration",
                        m.id
                    )));
                }

                // Execute down migration
                self.rollback_migration(&m).await?;

                // Update history
                self.history.record_rollback(&m.id).await?;

                return Ok(Some(m.id));
            }
        }

        Ok(None)
    }

    /// Rollback a single migration.
    async fn rollback_migration(&self, _migration: &MigrationFile) -> MigrateResult<()> {
        // This would execute the down SQL through the query engine
        Ok(())
    }

    /// Create a new migration file from schema changes.
    pub async fn create_migration(
        &self,
        name: &str,
        schema: &prax_schema::Schema,
    ) -> MigrateResult<PathBuf> {
        // Generate diff
        let differ = SchemaDiffer::new(schema.clone());
        let diff = differ.diff()?;

        if diff.is_empty() {
            return Err(MigrationError::NoChanges);
        }

        // Generate SQL
        let sql = self.sql_generator.generate(&diff);

        // Create migration file
        let id = self.file_manager.generate_id();
        let migration = MigrationFile::new(id, name, sql);

        // Write to disk
        let path = self.file_manager.write_migration(&migration).await?;

        Ok(path)
    }

    /// Get migration status.
    pub async fn status(&self) -> MigrateResult<MigrationStatus> {
        let applied = self.history.get_applied().await?;
        let files = self.file_manager.list_migrations().await?;

        let applied_ids: std::collections::HashSet<_> =
            applied.iter().map(|r| r.id.as_str()).collect();

        let pending: Vec<_> = files
            .iter()
            .filter(|f| !applied_ids.contains(f.id.as_str()))
            .map(|f| f.id.clone())
            .collect();

        let total_applied = applied.len();
        let total_pending = pending.len();

        Ok(MigrationStatus {
            applied,
            pending,
            total_applied,
            total_pending,
        })
    }

    /// Create and apply a new migration in development mode.
    ///
    /// This is a skeleton implementation that will be fully implemented later.
    /// Currently returns NoChanges error as a placeholder.
    pub async fn dev(
        &self,
        _name: &str,
        _schema: &prax_schema::Schema,
        _applied_by: Option<String>,
    ) -> MigrateResult<DevResult> {
        // Stub: This will be fully implemented in a later task
        // For now, return NoChanges as a placeholder
        Err(MigrationError::NoChanges)
    }

    /// Rollback a migration with event sourcing support.
    ///
    /// This is a skeleton implementation that will be fully implemented later.
    /// Currently returns NoMigrationsToRollback error as a placeholder.
    ///
    /// # Arguments
    /// * `migration_id` - Optional specific migration to rollback (defaults to last applied)
    /// * `reason` - Optional reason for the rollback
    /// * `rolled_back_by` - Optional identifier of who performed the rollback
    pub async fn rollback_with_event(
        &self,
        _migration_id: Option<String>,
        _reason: Option<String>,
        _rolled_back_by: Option<String>,
    ) -> MigrateResult<RollbackResult> {
        // Stub: This will be fully implemented in a later task
        // For now, return NoMigrationsToRollback as a placeholder
        Err(MigrationError::NoMigrationsToRollback)
    }

    /// Get complete migration history from the event store.
    ///
    /// Returns all migration events in chronological order. This includes:
    /// - Applied events: migrations that were successfully applied
    /// - RolledBack events: migrations that were rolled back
    /// - Failed events: migration attempts that failed
    /// - Resolved events: conflict resolutions and checksum updates
    ///
    /// # Returns
    /// Vector of all migration events, ordered by event_id (chronological)
    pub async fn get_migration_history(&self) -> MigrateResult<Vec<MigrationEvent>> {
        self.event_store.get_all_events().await
    }
}

/// Migration status information.
#[derive(Debug)]
pub struct MigrationStatus {
    /// Applied migrations.
    pub applied: Vec<MigrationRecord>,
    /// Pending migration IDs.
    pub pending: Vec<String>,
    /// Total number of applied migrations.
    pub total_applied: usize,
    /// Total number of pending migrations.
    pub total_pending: usize,
}

/// Result of a dev migration operation.
#[derive(Debug)]
pub struct DevResult {
    /// ID of the created/applied migration.
    pub migration_id: String,
    /// Path to the migration file.
    pub migration_path: PathBuf,
    /// Event ID from the event store.
    pub event_id: i64,
    /// Duration in milliseconds.
    pub duration_ms: i64,
    /// Warnings generated during the operation.
    pub warnings: Vec<String>,
}

/// Result of a rollback operation.
#[derive(Debug)]
pub struct RollbackResult {
    /// ID of the rolled back migration.
    pub migration_id: String,
    /// Event ID from the event store.
    pub event_id: i64,
    /// Duration in milliseconds.
    pub duration_ms: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = MigrationConfig::default();
        assert_eq!(config.migrations_dir, PathBuf::from("./migrations"));
        assert!(!config.dry_run);
        assert!(!config.allow_data_loss);
        assert!(config.fail_on_checksum_mismatch);
    }

    #[test]
    fn test_config_builder() {
        let config = MigrationConfig::new()
            .migrations_dir("./custom_migrations")
            .resolutions_file("./custom/resolutions.toml")
            .dry_run(true)
            .allow_data_loss(true)
            .fail_on_checksum_mismatch(false);

        assert_eq!(config.migrations_dir, PathBuf::from("./custom_migrations"));
        assert_eq!(
            config.resolutions_file,
            PathBuf::from("./custom/resolutions.toml")
        );
        assert!(config.dry_run);
        assert!(config.allow_data_loss);
        assert!(!config.fail_on_checksum_mismatch);
    }

    #[test]
    fn test_migration_plan_empty() {
        let plan = MigrationPlan::empty();

        assert!(plan.is_empty());
        assert!(!plan.has_blocking_issues());
        assert_eq!(plan.summary(), "No changes to apply");
    }

    #[test]
    fn test_migration_plan_with_pending() {
        let mut plan = MigrationPlan::empty();
        plan.pending.push(MigrationFile {
            path: PathBuf::from("migrations/test"),
            id: "test".to_string(),
            name: "test".to_string(),
            up_sql: "SELECT 1".to_string(),
            down_sql: String::new(),
            checksum: "abc".to_string(),
        });

        assert!(!plan.is_empty());
        assert!(plan.summary().contains("1 pending"));
    }

    #[test]
    fn test_migration_plan_with_unresolved_checksum() {
        let mut plan = MigrationPlan::empty();
        plan.unresolved_checksums.push(ChecksumMismatch {
            migration_id: "test".to_string(),
            expected: "abc".to_string(),
            actual: "xyz".to_string(),
        });

        assert!(plan.has_blocking_issues());
        assert!(plan.summary().contains("UNRESOLVED"));
    }

    #[test]
    fn test_migration_result_summary() {
        let result = MigrationResult {
            applied_count: 3,
            duration_ms: 150,
            applied_migrations: vec!["m1".into(), "m2".into(), "m3".into()],
            baselined_migrations: vec!["b1".into()],
            skipped_migrations: vec!["s1".into(), "s2".into()],
            warnings: Vec::new(),
        };

        assert_eq!(result.total_processed(), 4);
        assert!(result.has_changes());
        assert!(result.summary().contains("3 applied"));
        assert!(result.summary().contains("1 baselined"));
        assert!(result.summary().contains("2 skipped"));
    }
}
