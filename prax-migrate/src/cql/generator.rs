//! CQL migration SQL generator for ScyllaDB.

use crate::cql::diff::{
    ClusteringKey, ClusteringOrder, CompactionStrategy, CqlFieldDiff, CqlSchemaDiff, CqlTableDiff,
    KeyspaceConfig, ReplicationStrategy, UdtAlterDiff, UdtDiff,
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

        for table in &diff.create_tables {
            up.push(self.create_table_statement(table, ks_context));
            down.push(self.drop_table_statement(&table.name, ks_context));
        }

        for name in &diff.drop_tables {
            up.push(self.drop_table_statement(name, ks_context));
            warnings.push(format!(
                "Dropping table '{}' - all rows will be lost",
                name
            ));
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

    fn create_table_statement(&self, table: &CqlTableDiff, keyspace_context: Option<&str>) -> String {
        let qualified = self.qualify(&table.name, keyspace_context);
        let columns = table
            .fields
            .iter()
            .map(|f| self.format_column(f))
            .collect::<Vec<_>>()
            .join(",\n    ");
        let pk = self.format_primary_key(&table.partition_keys, &table.clustering_keys);

        let mut stmt = format!(
            "CREATE TABLE {} (\n    {},\n    {}\n)",
            qualified, columns, pk
        );

        let mut options: Vec<String> = Vec::new();
        if !table.clustering_keys.is_empty() {
            options.push(self.format_clustering_order(&table.clustering_keys));
        }
        if let Some(compaction) = &table.compaction {
            options.push(format!("compaction = {}", self.format_compaction(compaction)));
        }
        if let Some(ttl) = table.default_ttl {
            options.push(format!("default_time_to_live = {}", ttl));
        }

        if !options.is_empty() {
            stmt.push_str(" WITH ");
            stmt.push_str(&options.join("\n  AND "));
        }

        stmt.push(';');
        stmt
    }

    fn drop_table_statement(&self, name: &str, keyspace_context: Option<&str>) -> String {
        format!("DROP TABLE IF EXISTS {};", self.qualify(name, keyspace_context))
    }

    fn format_column(&self, field: &CqlFieldDiff) -> String {
        let mut col = format!("{} {}", field.name, field.cql_type);
        if field.is_static {
            col.push_str(" STATIC");
        }
        col
    }

    fn format_primary_key(
        &self,
        partition_keys: &[String],
        clustering_keys: &[ClusteringKey],
    ) -> String {
        let partition = if partition_keys.len() == 1 {
            format!("({})", partition_keys[0])
        } else {
            format!("({})", partition_keys.join(", "))
        };

        if clustering_keys.is_empty() {
            format!("PRIMARY KEY ({})", partition)
        } else {
            let clustering = clustering_keys
                .iter()
                .map(|ck| ck.name.clone())
                .collect::<Vec<_>>()
                .join(", ");
            format!("PRIMARY KEY ({}, {})", partition, clustering)
        }
    }

    fn format_clustering_order(&self, clustering_keys: &[ClusteringKey]) -> String {
        let order = clustering_keys
            .iter()
            .map(|ck| {
                let dir = match ck.order {
                    ClusteringOrder::Asc => "ASC",
                    ClusteringOrder::Desc => "DESC",
                };
                format!("{} {}", ck.name, dir)
            })
            .collect::<Vec<_>>()
            .join(", ");
        format!("CLUSTERING ORDER BY ({})", order)
    }

    fn format_compaction(&self, strategy: &CompactionStrategy) -> String {
        match strategy {
            CompactionStrategy::SizeTiered => {
                "{'class': 'SizeTieredCompactionStrategy'}".to_string()
            }
            CompactionStrategy::Leveled => {
                "{'class': 'LeveledCompactionStrategy'}".to_string()
            }
            CompactionStrategy::TimeWindow {
                window_unit,
                window_size,
            } => format!(
                "{{'class': 'TimeWindowCompactionStrategy', 'compaction_window_unit': '{}', 'compaction_window_size': {}}}",
                window_unit, window_size
            ),
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
    use crate::cql::diff::{
        ClusteringKey, ClusteringOrder, CompactionStrategy, CqlFieldDiff, CqlTableDiff,
        KeyspaceConfig, ReplicationStrategy, UdtAlterDiff, UdtDiff, UdtField,
    };

    fn simple_field(name: &str, cql_type: &str) -> CqlFieldDiff {
        CqlFieldDiff {
            name: name.into(),
            cql_type: cql_type.into(),
            is_static: false,
        }
    }

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

    #[test]
    fn test_create_table_with_simple_partition_key() {
        let generator = CqlMigrationGenerator::new();
        let mut diff = CqlSchemaDiff::default();
        diff.create_tables.push(CqlTableDiff {
            name: "users".into(),
            fields: vec![
                simple_field("id", "uuid"),
                simple_field("name", "text"),
            ],
            partition_keys: vec!["id".into()],
            clustering_keys: vec![],
            compaction: None,
            default_ttl: None,
        });

        let migration = generator.generate(&diff);
        assert!(migration.up.contains("CREATE TABLE \"users\" ("));
        assert!(migration.up.contains("id uuid"));
        assert!(migration.up.contains("name text"));
        assert!(migration.up.contains("PRIMARY KEY ((id))"));
        assert!(migration.down.contains("DROP TABLE IF EXISTS \"users\""));
    }

    #[test]
    fn test_create_table_with_compound_partition_and_clustering() {
        let generator = CqlMigrationGenerator::new();
        let mut diff = CqlSchemaDiff::default();
        diff.create_tables.push(CqlTableDiff {
            name: "events".into(),
            fields: vec![
                simple_field("tenant_id", "uuid"),
                simple_field("region", "text"),
                simple_field("event_time", "timestamp"),
                simple_field("event_id", "uuid"),
                simple_field("payload", "text"),
            ],
            partition_keys: vec!["tenant_id".into(), "region".into()],
            clustering_keys: vec![
                ClusteringKey {
                    name: "event_time".into(),
                    order: ClusteringOrder::Desc,
                },
                ClusteringKey {
                    name: "event_id".into(),
                    order: ClusteringOrder::Asc,
                },
            ],
            compaction: None,
            default_ttl: None,
        });

        let migration = generator.generate(&diff);
        assert!(migration.up.contains("PRIMARY KEY ((tenant_id, region), event_time, event_id)"));
        assert!(migration.up.contains("CLUSTERING ORDER BY (event_time DESC, event_id ASC)"));
    }

    #[test]
    fn test_create_table_with_compaction_and_ttl() {
        let generator = CqlMigrationGenerator::new();
        let mut diff = CqlSchemaDiff::default();
        diff.create_tables.push(CqlTableDiff {
            name: "metrics".into(),
            fields: vec![simple_field("id", "uuid"), simple_field("value", "double")],
            partition_keys: vec!["id".into()],
            clustering_keys: vec![],
            compaction: Some(CompactionStrategy::TimeWindow {
                window_unit: "DAYS".into(),
                window_size: 1,
            }),
            default_ttl: Some(2592000),
        });

        let migration = generator.generate(&diff);
        assert!(migration.up.contains("'class': 'TimeWindowCompactionStrategy'"));
        assert!(migration.up.contains("'compaction_window_unit': 'DAYS'"));
        assert!(migration.up.contains("'compaction_window_size': 1"));
        assert!(migration.up.contains("default_time_to_live = 2592000"));
    }

    #[test]
    fn test_create_table_with_static_column() {
        let generator = CqlMigrationGenerator::new();
        let mut diff = CqlSchemaDiff::default();
        let mut static_field = simple_field("tenant_name", "text");
        static_field.is_static = true;
        diff.create_tables.push(CqlTableDiff {
            name: "tenant_data".into(),
            fields: vec![
                simple_field("tenant_id", "uuid"),
                simple_field("event_id", "uuid"),
                static_field,
            ],
            partition_keys: vec!["tenant_id".into()],
            clustering_keys: vec![ClusteringKey {
                name: "event_id".into(),
                order: ClusteringOrder::Asc,
            }],
            compaction: None,
            default_ttl: None,
        });

        let migration = generator.generate(&diff);
        assert!(migration.up.contains("tenant_name text STATIC"));
    }

    #[test]
    fn test_drop_table_generates_warning() {
        let generator = CqlMigrationGenerator::new();
        let mut diff = CqlSchemaDiff::default();
        diff.drop_tables.push("legacy_table".into());

        let migration = generator.generate(&diff);
        assert!(migration.up.contains("DROP TABLE IF EXISTS \"legacy_table\""));
        assert!(
            migration.warnings.iter().any(|w| w.contains("legacy_table") && w.contains("all rows")),
            "expected drop-table warning"
        );
    }
}
