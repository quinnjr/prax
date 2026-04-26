//! Migration dialect trait for abstracting over SQL and CQL backends.

use crate::diff::SchemaDiff;
use crate::sql::{MigrationSql, PostgresSqlGenerator};

/// A migration dialect abstracts the schema diff type, migration output type,
/// and generator for a specific database backend.
pub trait MigrationDialect {
    /// The schema diff type for this dialect.
    type Diff: Default + Send + Sync;

    /// The migration output type for this dialect.
    type Migration: Send + Sync;

    /// Human-readable dialect name (e.g., "sql", "cql").
    fn name() -> &'static str;

    /// Generate a migration from a schema diff.
    fn generate(diff: &Self::Diff) -> Self::Migration;

    /// Event log table name used by this dialect.
    fn event_log_table() -> &'static str;
}

/// The SQL dialect (PostgreSQL, MySQL, SQLite, MSSQL, DuckDB share this).
pub struct SqlDialect;

impl MigrationDialect for SqlDialect {
    type Diff = SchemaDiff;
    type Migration = MigrationSql;

    fn name() -> &'static str {
        "sql"
    }

    fn generate(diff: &SchemaDiff) -> MigrationSql {
        PostgresSqlGenerator.generate(diff)
    }

    fn event_log_table() -> &'static str {
        "_prax_migrations"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sql_dialect_name() {
        assert_eq!(SqlDialect::name(), "sql");
    }

    #[test]
    fn test_sql_dialect_event_log_table() {
        assert_eq!(SqlDialect::event_log_table(), "_prax_migrations");
    }

    #[test]
    fn test_sql_dialect_generates_empty_migration_from_empty_diff() {
        let diff = SchemaDiff::default();
        let migration = SqlDialect::generate(&diff);
        assert!(migration.is_empty());
    }
}
