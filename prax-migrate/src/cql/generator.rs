//! CQL migration SQL generator for ScyllaDB.

use crate::cql::diff::{CqlSchemaDiff, KeyspaceConfig, ReplicationStrategy};
use crate::cql::migration::MigrationCql;

/// Generates CQL migration scripts from a CqlSchemaDiff.
pub struct CqlMigrationGenerator;

impl CqlMigrationGenerator {
    /// Create a new generator.
    pub fn new() -> Self {
        Self
    }

    /// Generate a CQL migration from a schema diff.
    pub fn generate(&self, diff: &CqlSchemaDiff) -> MigrationCql {
        let mut up: Vec<String> = Vec::new();
        let mut down: Vec<String> = Vec::new();
        let mut warnings: Vec<String> = Vec::new();

        if let Some(keyspace) = &diff.create_keyspace {
            up.push(self.create_keyspace_statement(keyspace));
            down.push(self.drop_keyspace_statement(&keyspace.name));
        }

        if let Some(name) = &diff.drop_keyspace {
            up.push(self.drop_keyspace_statement(name));
            warnings.push(format!(
                "Dropping keyspace '{}' - ALL data in ALL tables in the keyspace will be permanently lost",
                name
            ));
        }

        down.reverse();

        MigrationCql {
            up: up.join("\n\n"),
            down: down.join("\n\n"),
            warnings,
        }
    }

    fn create_keyspace_statement(&self, cfg: &KeyspaceConfig) -> String {
        let replication = self.format_replication(&cfg.replication);
        format!(
            "CREATE KEYSPACE IF NOT EXISTS \"{}\"\nWITH replication = {}\nAND durable_writes = {};",
            cfg.name, replication, cfg.durable_writes
        )
    }

    fn drop_keyspace_statement(&self, name: &str) -> String {
        format!("DROP KEYSPACE IF EXISTS \"{}\";", name)
    }

    fn format_replication(&self, strategy: &ReplicationStrategy) -> String {
        match strategy {
            ReplicationStrategy::Simple { factor } => {
                format!(
                    "{{'class': 'SimpleStrategy', 'replication_factor': {}}}",
                    factor
                )
            }
            ReplicationStrategy::NetworkTopology { dc_factors } => {
                let mut parts = vec!["'class': 'NetworkTopologyStrategy'".to_string()];
                for (dc, factor) in dc_factors {
                    parts.push(format!("'{}': {}", dc, factor));
                }
                format!("{{{}}}", parts.join(", "))
            }
        }
    }
}

impl Default for CqlMigrationGenerator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cql::diff::{KeyspaceConfig, ReplicationStrategy};

    #[test]
    fn test_empty_diff_produces_empty_migration() {
        let generator = CqlMigrationGenerator::new();
        let diff = CqlSchemaDiff::default();
        let migration = generator.generate(&diff);
        assert!(migration.is_empty());
        assert!(migration.warnings.is_empty());
    }

    #[test]
    fn test_create_keyspace_simple_strategy() {
        let generator = CqlMigrationGenerator::new();
        let mut diff = CqlSchemaDiff::default();
        diff.create_keyspace = Some(KeyspaceConfig {
            name: "myapp".into(),
            replication: ReplicationStrategy::Simple { factor: 3 },
            durable_writes: true,
        });

        let migration = generator.generate(&diff);
        assert!(migration.up.contains("CREATE KEYSPACE IF NOT EXISTS \"myapp\""));
        assert!(migration.up.contains("'class': 'SimpleStrategy'"));
        assert!(migration.up.contains("'replication_factor': 3"));
        assert!(migration.up.contains("durable_writes = true"));
        assert!(migration.down.contains("DROP KEYSPACE IF EXISTS \"myapp\""));
    }

    #[test]
    fn test_create_keyspace_network_topology() {
        let generator = CqlMigrationGenerator::new();
        let mut diff = CqlSchemaDiff::default();
        diff.create_keyspace = Some(KeyspaceConfig {
            name: "myapp".into(),
            replication: ReplicationStrategy::NetworkTopology {
                dc_factors: vec![("us-east".into(), 3), ("us-west".into(), 2)],
            },
            durable_writes: false,
        });

        let migration = generator.generate(&diff);
        assert!(migration.up.contains("'class': 'NetworkTopologyStrategy'"));
        assert!(migration.up.contains("'us-east': 3"));
        assert!(migration.up.contains("'us-west': 2"));
        assert!(migration.up.contains("durable_writes = false"));
    }

    #[test]
    fn test_drop_keyspace_generates_warning() {
        let generator = CqlMigrationGenerator::new();
        let mut diff = CqlSchemaDiff::default();
        diff.drop_keyspace = Some("legacy".into());

        let migration = generator.generate(&diff);
        assert!(migration.up.contains("DROP KEYSPACE IF EXISTS \"legacy\""));
        assert!(
            migration.warnings.iter().any(|w| w.contains("legacy") && w.contains("ALL data")),
            "expected drop-keyspace warning"
        );
    }
}
