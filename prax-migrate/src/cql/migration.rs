//! CQL migration output type.

/// A CQL migration: up script, down script, and warnings.
#[derive(Debug, Clone, Default)]
pub struct MigrationCql {
    /// CQL statements to apply the migration (forward direction).
    pub up: String,
    /// CQL statements to roll back the migration (reverse direction).
    pub down: String,
    /// Warnings about potential data loss or manual steps required.
    pub warnings: Vec<String>,
}

impl MigrationCql {
    /// Returns true if both up and down are empty.
    pub fn is_empty(&self) -> bool {
        self.up.trim().is_empty() && self.down.trim().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_migration_cql_default_is_empty() {
        let migration = MigrationCql::default();
        assert!(migration.is_empty());
        assert!(migration.warnings.is_empty());
    }

    #[test]
    fn test_migration_cql_with_up_is_not_empty() {
        let migration = MigrationCql {
            up: "CREATE TABLE foo (id uuid PRIMARY KEY);".to_string(),
            ..Default::default()
        };
        assert!(!migration.is_empty());
    }

    #[test]
    fn test_migration_cql_whitespace_only_is_empty() {
        let migration = MigrationCql {
            up: "   \n\t  ".to_string(),
            down: "   ".to_string(),
            warnings: vec![],
        };
        assert!(migration.is_empty());
    }
}
