//! CQL migration SQL generator for ScyllaDB.

use crate::cql::diff::{
    CqlSchemaDiff, KeyspaceConfig, ReplicationStrategy, UdtAlterDiff, UdtDiff,
};
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

        let ks_context = diff.keyspace_context.as_deref();

        for udt in &diff.create_udts {
            up.push(self.create_udt_statement(udt, ks_context));
            down.push(self.drop_udt_statement(&udt.name, ks_context));
        }

        for alter in &diff.alter_udts {
            up.extend(self.alter_udt_statements(alter, ks_context));
        }

        for name in &diff.drop_udts {
            up.push(self.drop_udt_statement(name, ks_context));
            warnings.push(format!(
                "Dropping UDT '{}' - ensure no tables reference this type",
                name
            ));
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

    fn qualify(&self, name: &str, keyspace_context: Option<&str>) -> String {
        match keyspace_context {
            Some(ks) => format!("\"{}\".\"{}\"", ks, name),
            None => format!("\"{}\"", name),
        }
    }

    fn create_udt_statement(&self, udt: &UdtDiff, keyspace_context: Option<&str>) -> String {
        let qualified = self.qualify(&udt.name, keyspace_context);
        let fields = udt
            .fields
            .iter()
            .map(|f| format!("    {} {}", f.name, f.cql_type))
            .collect::<Vec<_>>()
            .join(",\n");
        format!("CREATE TYPE {} (\n{}\n);", qualified, fields)
    }

    fn drop_udt_statement(&self, name: &str, keyspace_context: Option<&str>) -> String {
        format!("DROP TYPE IF EXISTS {};", self.qualify(name, keyspace_context))
    }

    fn alter_udt_statements(
        &self,
        alter: &UdtAlterDiff,
        keyspace_context: Option<&str>,
    ) -> Vec<String> {
        let qualified = self.qualify(&alter.name, keyspace_context);
        let mut stmts = Vec::new();
        for field in &alter.add_fields {
            stmts.push(format!(
                "ALTER TYPE {} ADD {} {};",
                qualified, field.name, field.cql_type
            ));
        }
        for (old, new) in &alter.rename_fields {
            stmts.push(format!(
                "ALTER TYPE {} RENAME {} TO {};",
                qualified, old, new
            ));
        }
        stmts
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
    use crate::cql::diff::{KeyspaceConfig, ReplicationStrategy, UdtAlterDiff, UdtDiff, UdtField};

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

    #[test]
    fn test_create_udt_without_keyspace_context() {
        let generator = CqlMigrationGenerator::new();
        let mut diff = CqlSchemaDiff::default();
        diff.create_udts.push(UdtDiff {
            name: "order_status".into(),
            fields: vec![UdtField {
                name: "value".into(),
                cql_type: "text".into(),
            }],
        });

        let migration = generator.generate(&diff);
        assert!(migration.up.contains("CREATE TYPE \"order_status\" ("));
        assert!(migration.up.contains("value text"));
        assert!(migration.down.contains("DROP TYPE IF EXISTS \"order_status\""));
    }

    #[test]
    fn test_create_udt_with_keyspace_context() {
        let generator = CqlMigrationGenerator::new();
        let mut diff = CqlSchemaDiff {
            keyspace_context: Some("myapp".into()),
            ..Default::default()
        };
        diff.create_udts.push(UdtDiff {
            name: "order_status".into(),
            fields: vec![UdtField {
                name: "value".into(),
                cql_type: "text".into(),
            }],
        });

        let migration = generator.generate(&diff);
        assert!(migration.up.contains("CREATE TYPE \"myapp\".\"order_status\" ("));
    }

    #[test]
    fn test_alter_udt_add_field() {
        let generator = CqlMigrationGenerator::new();
        let mut diff = CqlSchemaDiff::default();
        diff.alter_udts.push(UdtAlterDiff {
            name: "order_status".into(),
            add_fields: vec![UdtField {
                name: "description".into(),
                cql_type: "text".into(),
            }],
            rename_fields: vec![],
        });

        let migration = generator.generate(&diff);
        assert!(migration.up.contains("ALTER TYPE \"order_status\" ADD description text"));
    }

    #[test]
    fn test_alter_udt_rename_field() {
        let generator = CqlMigrationGenerator::new();
        let mut diff = CqlSchemaDiff::default();
        diff.alter_udts.push(UdtAlterDiff {
            name: "order_status".into(),
            add_fields: vec![],
            rename_fields: vec![("old_name".into(), "new_name".into())],
        });

        let migration = generator.generate(&diff);
        assert!(migration.up.contains("ALTER TYPE \"order_status\" RENAME old_name TO new_name"));
    }

    #[test]
    fn test_drop_udt_generates_warning() {
        let generator = CqlMigrationGenerator::new();
        let mut diff = CqlSchemaDiff::default();
        diff.drop_udts.push("legacy_type".into());

        let migration = generator.generate(&diff);
        assert!(migration.up.contains("DROP TYPE IF EXISTS \"legacy_type\""));
        assert!(migration.warnings.iter().any(|w| w.contains("legacy_type")));
    }
}
