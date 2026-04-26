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

    #[test]
    fn test_cql_dialect_name() {
        use crate::cql::CqlDialect;
        assert_eq!(CqlDialect::name(), "cql");
    }

    #[test]
    fn test_cql_dialect_event_log_table() {
        use crate::cql::CqlDialect;
        assert_eq!(CqlDialect::event_log_table(), "_prax_cql_migrations");
    }

    #[test]
    fn test_cql_dialect_generates_empty_migration_from_empty_diff() {
        use crate::cql::{CqlDialect, CqlSchemaDiff};
        let diff = CqlSchemaDiff::default();
        let migration = CqlDialect::generate(&diff);
        assert!(migration.is_empty());
    }

    #[test]
    fn test_sql_dialect_matches_postgres_generator_directly() {
        use crate::diff::{FieldDiff, ModelDiff};

        let mut diff = SchemaDiff::default();
        diff.create_models.push(ModelDiff {
            name: "User".to_string(),
            table_name: "users".to_string(),
            fields: vec![FieldDiff {
                name: "id".to_string(),
                column_name: "id".to_string(),
                sql_type: "BIGINT".to_string(),
                nullable: false,
                default: None,
                is_primary_key: true,
                is_auto_increment: true,
                is_unique: false,
            }],
            primary_key: vec!["id".to_string()],
            indexes: Vec::new(),
            unique_constraints: Vec::new(),
            foreign_keys: Vec::new(),
        });

        let via_trait = SqlDialect::generate(&diff);
        let via_direct = PostgresSqlGenerator.generate(&diff);

        assert_eq!(via_trait.up, via_direct.up);
        assert_eq!(via_trait.down, via_direct.down);
        assert_eq!(via_trait.warnings, via_direct.warnings);
    }
}
