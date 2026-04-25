//! Bootstrap module for migrating from V1 to V2 migration table format.
//!
//! The V1 format used a single boolean `rolled_back` column to track state.
//! The V2 format uses an event sourcing approach with `event_type` and structured event data.

/// SQL to check if a V1 format migration table exists.
/// V1 tables have a `rolled_back` boolean column.
pub const CHECK_V1_FORMAT_SQL: &str = r#"
SELECT EXISTS (
    SELECT 1
    FROM information_schema.columns
    WHERE table_name = '_prax_migrations'
      AND column_name = 'rolled_back'
      AND data_type = 'boolean'
) AS has_v1_format;
"#;

/// SQL to check if a V2 format migration table exists.
/// V2 tables have an `event_type` column.
pub const CHECK_V2_FORMAT_SQL: &str = r#"
SELECT EXISTS (
    SELECT 1
    FROM information_schema.columns
    WHERE table_name = '_prax_migrations'
      AND column_name = 'event_type'
) AS has_v2_format;
"#;

/// SQL to rename the V1 table to a backup name.
pub const RENAME_V1_TABLE_SQL: &str = r#"
ALTER TABLE _prax_migrations
RENAME TO _prax_migrations_v1_backup;
"#;

/// SQL to migrate V1 records to V2 event format.
/// Creates Applied events from all V1 records where rolled_back = false.
pub const MIGRATE_V1_TO_V2_SQL: &str = r#"
INSERT INTO _prax_migrations (
    migration_id,
    event_type,
    event_data,
    created_at
)
SELECT
    id AS migration_id,
    'Applied' AS event_type,
    jsonb_build_object(
        'checksum', checksum,
        'duration_ms', duration_ms,
        'applied_by', NULL,
        'up_sql_preview', NULL,
        'auto_generated', false
    ) AS event_data,
    applied_at AS created_at
FROM _prax_migrations_v1_backup
WHERE rolled_back = false
ORDER BY applied_at ASC;
"#;

/// SQL to verify migration from V1 to V2.
/// Counts records in both tables to ensure data consistency.
pub const VERIFY_MIGRATION_SQL: &str = r#"
SELECT
    (SELECT COUNT(*) FROM _prax_migrations_v1_backup WHERE rolled_back = false) AS v1_applied_count,
    (SELECT COUNT(*) FROM _prax_migrations) AS v2_event_count;
"#;

/// Bootstrap helper for V1-to-V2 migration.
pub struct Bootstrap;

impl Bootstrap {
    /// Returns formatted instructions for manual V1-to-V2 migration.
    ///
    /// This provides a step-by-step guide for database administrators
    /// to safely migrate from V1 to V2 format.
    pub fn migration_instructions() -> String {
        format!(
            r#"
┌─────────────────────────────────────────────────────────────┐
│ Prax Migration Table Bootstrap: V1 → V2                    │
└─────────────────────────────────────────────────────────────┘

Your database uses the legacy V1 migration format. To upgrade to V2
(event sourcing), follow these steps:

1. BACKUP YOUR DATABASE
   The migration is non-destructive (renames V1 table to backup),
   but you should always have a backup before schema changes.

2. Check current format:
   {}

3. Rename V1 table to backup:
   {}

4. Create V2 migration table (run `prax migrate init`)

5. Migrate V1 records to V2 events:
   {}

6. Verify migration:
   {}

   The v1_applied_count and v2_event_count should match.

7. After verification, you can drop the backup table:
   DROP TABLE _prax_migrations_v1_backup;

   Or keep it for audit purposes.

For automated migration, use `prax migrate bootstrap`.
"#,
            CHECK_V1_FORMAT_SQL.trim(),
            RENAME_V1_TABLE_SQL.trim(),
            MIGRATE_V1_TO_V2_SQL.trim(),
            VERIFY_MIGRATION_SQL.trim()
        )
    }

    /// Determines if migration from V1 to V2 is needed.
    ///
    /// # Arguments
    /// * `has_v1` - Whether V1 format table exists (has `rolled_back` column)
    /// * `has_v2` - Whether V2 format table exists (has `event_type` column)
    ///
    /// # Returns
    /// `true` if V1 exists but V2 doesn't (migration needed)
    pub fn needs_migration(has_v1: bool, has_v2: bool) -> bool {
        has_v1 && !has_v2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_v1_format_sql_contains_rolled_back() {
        assert!(CHECK_V1_FORMAT_SQL.contains("rolled_back"));
        assert!(CHECK_V1_FORMAT_SQL.contains("boolean"));
    }

    #[test]
    fn test_check_v2_format_sql_contains_event_type() {
        assert!(CHECK_V2_FORMAT_SQL.contains("event_type"));
    }

    #[test]
    fn test_rename_v1_table_sql() {
        assert!(RENAME_V1_TABLE_SQL.contains("_prax_migrations_v1_backup"));
        assert!(RENAME_V1_TABLE_SQL.contains("ALTER TABLE"));
    }

    #[test]
    fn test_migrate_v1_to_v2_sql_structure() {
        assert!(MIGRATE_V1_TO_V2_SQL.contains("INSERT INTO _prax_migrations"));
        assert!(MIGRATE_V1_TO_V2_SQL.contains("event_type"));
        assert!(MIGRATE_V1_TO_V2_SQL.contains("event_data"));
        assert!(MIGRATE_V1_TO_V2_SQL.contains("rolled_back = false"));
    }

    #[test]
    fn test_verify_migration_sql() {
        assert!(VERIFY_MIGRATION_SQL.contains("v1_applied_count"));
        assert!(VERIFY_MIGRATION_SQL.contains("v2_event_count"));
    }

    #[test]
    fn test_migration_instructions_format() {
        let instructions = Bootstrap::migration_instructions();
        assert!(instructions.contains("V1 → V2"));
        assert!(instructions.contains("BACKUP"));
        assert!(instructions.contains("prax migrate bootstrap"));
    }

    #[test]
    fn test_needs_migration_v1_only() {
        assert!(Bootstrap::needs_migration(true, false));
    }

    #[test]
    fn test_needs_migration_v2_exists() {
        assert!(!Bootstrap::needs_migration(true, true));
    }

    #[test]
    fn test_needs_migration_no_v1() {
        assert!(!Bootstrap::needs_migration(false, false));
        assert!(!Bootstrap::needs_migration(false, true));
    }
}
