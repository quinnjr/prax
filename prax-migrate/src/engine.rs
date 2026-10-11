//! Migration engine implementation.

use std::path::PathBuf;
use std::time::Instant;

use chrono::Utc;

use crate::diff::{SchemaDiff, SchemaDiffer};
use crate::error::{MigrateResult, MigrationError};
use crate::event::MigrationEvent;
use crate::event_store::MigrationEventStore;
use crate::file::{MigrationFile, MigrationFileManager};
use crate::history::{MigrationHistoryRepository, MigrationRecord};
use crate::resolution::{Resolution, ResolutionConfig};
use crate::sql::{MigrationSql, PostgresSqlGenerator};
use crate::state::MigrationState;

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
    ///
    /// V1 → V2 (event sourcing) transition: applied state is derived by
    /// replaying the event log (`MigrationState::from_events`) whenever the
    /// event store contains events. The legacy V1 history repository is
    /// only consulted while the event store is still empty, i.e. before the
    /// first event-sourced migration run.
    pub async fn plan(&self, current_schema: &prax_schema::Schema) -> MigrateResult<MigrationPlan> {
        let mut plan = MigrationPlan::empty();

        // Get applied migrations (event replay preferred; V1 history is the
        // fallback while the event store is empty — see method docs).
        let events = self.event_store.get_all_events().await?;
        let applied_checksums: std::collections::HashMap<String, String> = if events.is_empty() {
            self.history
                .get_applied()
                .await?
                .into_iter()
                .map(|r| (r.id, r.checksum))
                .collect()
        } else {
            let state = MigrationState::from_events(events);
            state
                .get_applied()
                .into_iter()
                .map(|s| (s.migration_id.clone(), s.checksum.clone()))
                .collect()
        };

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
            if self.resolutions.is_baseline(&file.id)
                && !applied_checksums.contains_key(file.id.as_str())
            {
                plan.baselines.push(file.id.clone());
                continue;
            }

            // Check for renamed migrations
            let effective_id = self
                .resolutions
                .get_renamed(&file.id)
                .map(String::from)
                .unwrap_or_else(|| file.id.clone());

            if !applied_checksums.contains_key(effective_id.as_str()) {
                plan.pending.push(file);
            } else if let Some(record_checksum) = applied_checksums.get(effective_id.as_str()) {
                // Check for checksum mismatch
                if record_checksum != &file.checksum {
                    // An unexpired ForceApply resolution treats the modified
                    // migration as PENDING again: it lands in `plan.pending`
                    // so `migrate()` attempts to apply it (which currently
                    // fails honestly with `ExecutionUnavailable`).
                    if self.resolutions.is_force_applied(&file.id) {
                        plan.pending.push(file);
                    } else if self.resolutions.accepts_checksum(
                        &file.id,
                        record_checksum,
                        &file.checksum,
                    ) {
                        // Checksum change is resolved
                        if let Some(resolution) = self.resolutions.get(&file.id) {
                            plan.resolved_checksums.push(ChecksumResolution {
                                migration_id: file.id.clone(),
                                expected: record_checksum.clone(),
                                actual: file.checksum.clone(),
                                reason: resolution.reason.clone(),
                            });
                        }
                    } else {
                        // Unresolved checksum mismatch
                        plan.unresolved_checksums.push(ChecksumMismatch {
                            migration_id: file.id.clone(),
                            expected: record_checksum.clone(),
                            actual: file.checksum.clone(),
                        });

                        if self.config.fail_on_checksum_mismatch {
                            plan.warnings.push(format!(
                                "Migration '{}' has been modified since it was applied. \
                                 Add a resolution to accept this change: \
                                 prax migrate resolve checksum {} {} {}",
                                file.id, file.id, record_checksum, file.checksum
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
                plan.warnings.extend(data_loss_warnings(&diff));
            }

            let sql = self.sql_generator.generate(&diff);
            plan.diff = Some(diff);
            plan.sql = Some(sql);
        }

        Ok(plan)
    }

    /// Apply pending migrations.
    ///
    /// **Execution is not wired into this engine.** Applying a pending
    /// migration fails with [`MigrationError::ExecutionUnavailable`] before
    /// anything is recorded: the engine never records success for work that
    /// did not happen, so no applied-history is written for it. Baseline
    /// resolutions are still recorded as applied without running (that is
    /// their documented semantic), and dry-run mode only reports what would
    /// happen.
    ///
    /// The event-sourced path (`dev`) is intended to become the primary
    /// create-and-apply API once it is implemented; see [`Self::dev`].
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
                let record = applied.iter().find(|r| r.id == effective_id);
                let checksum_changed = record.is_some_and(|r| r.checksum != file.checksum);

                // An unexpired ForceApply resolution on a modified migration
                // treats it as PENDING again: skip the checksum gate and fall
                // through to the apply path below.
                if !(checksum_changed && self.resolutions.is_force_applied(&file.id)) {
                    // Check for unresolved checksum mismatch
                    if let Some(record) = record
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
    ///
    /// No SQL executor is wired into this engine, so this always fails with
    /// [`MigrationError::ExecutionUnavailable`] instead of returning `Ok`
    /// for work that did not happen. This keeps the record-applied path in
    /// `migrate` unreachable until a real executor lands.
    async fn apply_migration(&self, migration: &MigrationFile) -> MigrateResult<()> {
        Err(MigrationError::execution_unavailable(format!(
            "cannot apply migration '{}': no SQL executor is configured on this engine",
            migration.id
        )))
    }

    /// Rollback the last migration.
    ///
    /// **Execution is not wired into this engine.** Rolling back fails with
    /// [`MigrationError::ExecutionUnavailable`] before anything is
    /// recorded: the down SQL is never executed and no rollback history is
    /// written for work that did not happen. Dry-run mode only reports what
    /// would happen.
    ///
    /// The event-sourced path (`rollback_with_event`) is intended to become
    /// the primary rollback API once it is implemented; see
    /// [`Self::rollback_with_event`].
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
    ///
    /// No SQL executor is wired into this engine, so this always fails with
    /// [`MigrationError::ExecutionUnavailable`] instead of returning `Ok`
    /// for work that did not happen. This keeps the record-rollback path in
    /// `rollback` unreachable until a real executor lands.
    async fn rollback_migration(&self, migration: &MigrationFile) -> MigrateResult<()> {
        Err(MigrationError::execution_unavailable(format!(
            "cannot roll back migration '{}': no SQL executor is configured on this engine",
            migration.id
        )))
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
    ///
    /// V1 → V2 (event sourcing) transition: applied state is derived by
    /// replaying the event log (`MigrationState::from_events`) whenever the
    /// event store contains events. The legacy V1 history repository is
    /// only consulted while the event store is still empty, i.e. before the
    /// first event-sourced migration run.
    pub async fn status(&self) -> MigrateResult<MigrationStatus> {
        let events = self.event_store.get_all_events().await?;
        let files = self.file_manager.list_migrations().await?;

        let applied: Vec<MigrationRecord> = if events.is_empty() {
            self.history.get_applied().await?
        } else {
            let state = MigrationState::from_events(events);
            state
                .get_applied()
                .into_iter()
                .map(|s| MigrationRecord {
                    id: s.migration_id.clone(),
                    checksum: s.checksum.clone(),
                    applied_at: s.last_applied_at.unwrap_or_else(Utc::now),
                    duration_ms: 0,
                    rolled_back: false,
                })
                .collect()
        };

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
    /// **Not yet implemented.** When implemented, this is the event-sourced
    /// create-and-apply path: it will generate a migration from the schema
    /// diff, execute it through a SQL executor, and append an
    /// [`EventType::Applied`](crate::event::EventType::Applied) event to
    /// the event store. Until an executor lands it returns
    /// [`MigrationError::NotImplemented`]; no event is appended and no
    /// history is recorded.
    pub async fn dev(
        &self,
        _name: &str,
        _schema: &prax_schema::Schema,
        _applied_by: Option<String>,
    ) -> MigrateResult<DevResult> {
        Err(MigrationError::not_implemented(
            "dev: event-sourced create-and-apply requires a SQL executor and event append",
        ))
    }

    /// Rollback a migration with event sourcing support.
    ///
    /// **Not yet implemented.** When implemented, this is the event-sourced
    /// rollback path: it will execute the down migration through a SQL
    /// executor and append a
    /// [`EventType::RolledBack`](crate::event::EventType::RolledBack) event
    /// to the event store (a new immutable event — prior events are never
    /// mutated). Until an executor lands it returns
    /// [`MigrationError::NotImplemented`]; no event is appended and no
    /// history is recorded.
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
        Err(MigrationError::not_implemented(
            "rollback_with_event: event-sourced rollback requires a SQL executor and event append",
        ))
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

/// Warnings for changes that lose data and need `allow_data_loss`.
fn data_loss_warnings(diff: &crate::diff::SchemaDiff) -> Vec<String> {
    let mut warnings = Vec::new();
    if !diff.drop_models.is_empty() {
        warnings.push(format!(
            "Would drop {} tables: {}. Set allow_data_loss=true to proceed.",
            diff.drop_models.len(),
            diff.drop_models.join(", ")
        ));
    }

    for alter in &diff.alter_models {
        if !alter.drop_fields.is_empty() {
            warnings.push(format!(
                "Would drop columns in '{}': {}. Set allow_data_loss=true to proceed.",
                alter.name,
                alter.drop_fields.join(", ")
            ));
        }
    }

    let proceed = "Set allow_data_loss=true to proceed.";
    warnings.extend(
        diff.timescale
            .gated_notes()
            .iter()
            .map(|note| crate::diff::append_sentence(note, proceed)),
    );
    warnings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{EventData, EventType};
    use crate::event_store::InMemoryEventStore;
    use crate::history::MigrationLock;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Stub history repository that counts `record_*` calls so tests can
    /// assert nothing was recorded.
    #[derive(Default)]
    struct StubHistory {
        applied: Vec<MigrationRecord>,
        record_applied_calls: Arc<AtomicUsize>,
        record_rollback_calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl MigrationHistoryRepository for StubHistory {
        async fn initialize(&self) -> MigrateResult<()> {
            Ok(())
        }

        async fn get_applied(&self) -> MigrateResult<Vec<MigrationRecord>> {
            Ok(self.applied.clone())
        }

        async fn is_applied(&self, id: &str) -> MigrateResult<bool> {
            Ok(self.applied.iter().any(|r| r.id == id))
        }

        async fn record_applied(
            &self,
            _id: &str,
            _checksum: &str,
            _duration_ms: i64,
        ) -> MigrateResult<()> {
            self.record_applied_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn record_rollback(&self, _id: &str) -> MigrateResult<()> {
            self.record_rollback_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn get_last_applied(&self) -> MigrateResult<Option<MigrationRecord>> {
            Ok(self.applied.iter().rev().find(|r| !r.rolled_back).cloned())
        }

        async fn acquire_lock(&self) -> MigrateResult<MigrationLock> {
            Ok(MigrationLock::new(1, || {}))
        }
    }

    /// Write a well-formed migration directory (`<dir_name>/up.sql` [+ `down.sql`]).
    async fn write_migration(dir: &std::path::Path, dir_name: &str, up: &str, down: &str) {
        let migration_dir = dir.join(dir_name);
        tokio::fs::create_dir_all(&migration_dir).await.unwrap();
        tokio::fs::write(migration_dir.join("up.sql"), up)
            .await
            .unwrap();
        if !down.is_empty() {
            tokio::fs::write(migration_dir.join("down.sql"), down)
                .await
                .unwrap();
        }
    }

    fn applied_record(id: &str) -> MigrationRecord {
        MigrationRecord {
            id: id.to_string(),
            checksum: "abc123".to_string(),
            applied_at: Utc::now(),
            duration_ms: 1,
            rolled_back: false,
        }
    }

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

    #[tokio::test]
    async fn test_migrate_fails_with_execution_unavailable_and_records_nothing() {
        let temp = tempfile::tempdir().unwrap();
        write_migration(
            temp.path(),
            "20240101000000_create_users",
            "CREATE TABLE users (id INT);",
            "DROP TABLE users;",
        )
        .await;

        let history = StubHistory::default();
        let applied_calls = history.record_applied_calls.clone();
        let config = MigrationConfig::new().migrations_dir(temp.path());
        let engine = MigrationEngine::new(config, history, InMemoryEventStore::new());

        let err = engine.migrate().await.unwrap_err();
        assert!(
            matches!(err, MigrationError::ExecutionUnavailable(_)),
            "expected ExecutionUnavailable, got: {err:?}"
        );
        assert_eq!(
            applied_calls.load(Ordering::SeqCst),
            0,
            "record_applied must not be called when execution never happened"
        );
    }

    #[tokio::test]
    async fn test_plan_force_apply_sends_modified_migration_to_pending() {
        let temp = tempfile::tempdir().unwrap();
        write_migration(
            temp.path(),
            "20240101000000_create_users",
            "CREATE TABLE users (id INT);",
            "",
        )
        .await;

        // Applied with a different checksum (file modified since apply), and
        // an unexpired ForceApply resolution: treated as pending again.
        let history = StubHistory {
            applied: vec![applied_record("20240101000000")],
            ..Default::default()
        };
        let mut resolutions = ResolutionConfig::new();
        resolutions.add(Resolution::force_apply(
            "20240101000000",
            "Re-apply intentionally modified migration",
        ));
        let config = MigrationConfig::new().migrations_dir(temp.path());
        let engine = MigrationEngine::with_resolutions(
            config,
            history,
            InMemoryEventStore::new(),
            resolutions,
        );
        let plan = engine.plan(&prax_schema::Schema::default()).await.unwrap();

        assert_eq!(plan.pending.len(), 1);
        assert_eq!(plan.pending[0].id, "20240101000000");
        assert!(plan.unresolved_checksums.is_empty());
        assert!(plan.resolved_checksums.is_empty());
    }

    #[tokio::test]
    async fn test_plan_expired_force_apply_lands_in_unresolved_checksums() {
        let temp = tempfile::tempdir().unwrap();
        write_migration(
            temp.path(),
            "20240101000000_create_users",
            "CREATE TABLE users (id INT);",
            "",
        )
        .await;

        let history = StubHistory {
            applied: vec![applied_record("20240101000000")],
            ..Default::default()
        };
        let mut resolutions = ResolutionConfig::new();
        resolutions.add(
            Resolution::force_apply("20240101000000", "Expired force apply")
                .with_expiration(Utc::now() - chrono::Duration::hours(1)),
        );
        let config = MigrationConfig::new().migrations_dir(temp.path());
        let engine = MigrationEngine::with_resolutions(
            config,
            history,
            InMemoryEventStore::new(),
            resolutions,
        );
        let plan = engine.plan(&prax_schema::Schema::default()).await.unwrap();

        assert!(plan.pending.is_empty());
        assert!(plan.resolved_checksums.is_empty());
        assert_eq!(plan.unresolved_checksums.len(), 1);
        assert_eq!(plan.unresolved_checksums[0].migration_id, "20240101000000");
        assert!(plan.has_blocking_issues());
    }

    #[tokio::test]
    async fn test_migrate_force_apply_attempts_application() {
        let temp = tempfile::tempdir().unwrap();
        write_migration(
            temp.path(),
            "20240101000000_create_users",
            "CREATE TABLE users (id INT);",
            "",
        )
        .await;

        // ForceApply falls through to the apply path, which errors honestly
        // with ExecutionUnavailable (no executor is wired into this engine).
        let history = StubHistory {
            applied: vec![applied_record("20240101000000")],
            ..Default::default()
        };
        let mut resolutions = ResolutionConfig::new();
        resolutions.add(Resolution::force_apply("20240101000000", "Re-apply"));
        let config = MigrationConfig::new().migrations_dir(temp.path());
        let engine = MigrationEngine::with_resolutions(
            config,
            history,
            InMemoryEventStore::new(),
            resolutions,
        );
        let err = engine.migrate().await.unwrap_err();
        assert!(
            matches!(err, MigrationError::ExecutionUnavailable(_)),
            "expected ExecutionUnavailable, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn test_migrate_expired_force_apply_errors_checksum_mismatch() {
        let temp = tempfile::tempdir().unwrap();
        write_migration(
            temp.path(),
            "20240101000000_create_users",
            "CREATE TABLE users (id INT);",
            "",
        )
        .await;

        let history = StubHistory {
            applied: vec![applied_record("20240101000000")],
            ..Default::default()
        };
        let mut resolutions = ResolutionConfig::new();
        resolutions.add(
            Resolution::force_apply("20240101000000", "Expired force apply")
                .with_expiration(Utc::now() - chrono::Duration::hours(1)),
        );
        let config = MigrationConfig::new().migrations_dir(temp.path());
        let engine = MigrationEngine::with_resolutions(
            config,
            history,
            InMemoryEventStore::new(),
            resolutions,
        );
        let err = engine.migrate().await.unwrap_err();
        assert!(
            matches!(err, MigrationError::ChecksumMismatch { .. }),
            "expected ChecksumMismatch, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn test_rollback_fails_with_execution_unavailable_and_records_nothing() {
        let temp = tempfile::tempdir().unwrap();
        write_migration(
            temp.path(),
            "20240101000000_create_users",
            "CREATE TABLE users (id INT);",
            "DROP TABLE users;",
        )
        .await;

        // V1 history claims the migration is applied; its file has down SQL.
        let history = StubHistory {
            applied: vec![applied_record("20240101000000")],
            ..Default::default()
        };
        let rollback_calls = history.record_rollback_calls.clone();
        let config = MigrationConfig::new().migrations_dir(temp.path());
        let engine = MigrationEngine::new(config, history, InMemoryEventStore::new());

        let err = engine.rollback().await.unwrap_err();
        assert!(
            matches!(err, MigrationError::ExecutionUnavailable(_)),
            "expected ExecutionUnavailable, got: {err:?}"
        );
        assert_eq!(
            rollback_calls.load(Ordering::SeqCst),
            0,
            "record_rollback must not be called when execution never happened"
        );
    }

    #[tokio::test]
    async fn test_status_prefers_event_store_over_v1_history() {
        let temp = tempfile::tempdir().unwrap();
        write_migration(
            temp.path(),
            "20240101000000_create_users",
            "CREATE TABLE users (id INT);",
            "",
        )
        .await;

        // V1 history is empty, but the event store says the migration applied.
        let event_store = InMemoryEventStore::new();
        event_store
            .append_event(
                "20240101000000",
                EventType::Applied,
                EventData::Applied {
                    checksum: "abc123".to_string(),
                    duration_ms: 1,
                    applied_by: None,
                    up_sql_preview: None,
                    auto_generated: false,
                },
            )
            .await
            .unwrap();

        let config = MigrationConfig::new().migrations_dir(temp.path());
        let engine = MigrationEngine::new(config, StubHistory::default(), event_store);

        let status = engine.status().await.unwrap();
        assert_eq!(status.total_applied, 1);
        assert_eq!(status.total_pending, 0);
        assert_eq!(status.applied[0].id, "20240101000000");
    }

    #[tokio::test]
    async fn test_status_falls_back_to_v1_history_when_event_store_empty() {
        let temp = tempfile::tempdir().unwrap();
        write_migration(
            temp.path(),
            "20240101000000_create_users",
            "CREATE TABLE users (id INT);",
            "",
        )
        .await;

        let history = StubHistory {
            applied: vec![applied_record("20240101000000")],
            ..Default::default()
        };
        let config = MigrationConfig::new().migrations_dir(temp.path());
        let engine = MigrationEngine::new(config, history, InMemoryEventStore::new());

        let status = engine.status().await.unwrap();
        assert_eq!(status.total_applied, 1);
        assert_eq!(status.total_pending, 0);
        assert_eq!(status.applied[0].id, "20240101000000");
    }

    #[tokio::test]
    async fn test_dev_and_rollback_with_event_report_not_implemented() {
        let temp = tempfile::tempdir().unwrap();
        let config = MigrationConfig::new().migrations_dir(temp.path());
        let engine =
            MigrationEngine::new(config, StubHistory::default(), InMemoryEventStore::new());

        let err = engine
            .dev("test", &prax_schema::Schema::default(), None)
            .await
            .unwrap_err();
        assert!(
            matches!(err, MigrationError::NotImplemented(_)),
            "expected NotImplemented, got: {err:?}"
        );

        let err = engine
            .rollback_with_event(None, None, None)
            .await
            .unwrap_err();
        assert!(
            matches!(err, MigrationError::NotImplemented(_)),
            "expected NotImplemented, got: {err:?}"
        );
    }

    #[test]
    fn test_data_loss_warnings_report_gated_timescale_changes() {
        use crate::diff::SchemaDiff;
        use crate::timescale::{TimescaleChange, TimescaleChangeKind};

        let mut diff = SchemaDiff::default();
        diff.timescale.changes.push(TimescaleChange {
            kind: TimescaleChangeKind::ReplaceRetentionPolicy {
                table: "Metric".into(),
                old: None,
                new: None,
            },
            gated: true,
            note: Some("Adding retention to \"Metric\" drops old chunks".into()),
        });
        let warnings = data_loss_warnings(&diff);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("allow_data_loss=true"), "{warnings:?}");
        assert!(warnings[0].contains("drops old chunks"), "{warnings:?}");
    }
}
