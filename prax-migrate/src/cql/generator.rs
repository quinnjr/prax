//! CQL migration SQL generator for ScyllaDB.

use crate::cql::diff::{
    ClusteringKey, ClusteringOrder, CompactionStrategy, CqlFieldDiff, CqlIndexDiff, CqlIndexType,
    CqlSchemaDiff, CqlTableAlterDiff, CqlTableDiff, KeyspaceConfig, MaterializedViewDiff,
    ReplicationStrategy, UdtAlterDiff, UdtDiff,
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

        for alter in &diff.alter_tables {
            if alter.partition_key_changed {
                warnings.push(format!(
                    "Partition key change detected for table '{}' - not supported in-place; requires manual DROP/CREATE migration",
                    alter.name
                ));
            }
            if alter.clustering_key_changed {
                warnings.push(format!(
                    "Clustering key change detected for table '{}' - not supported in-place; requires manual DROP/CREATE migration",
                    alter.name
                ));
            }

            for field_name in &alter.drop_fields {
                warnings.push(format!(
                    "Dropping column '{}' from table '{}' - data in this column will be lost",
                    field_name, alter.name
                ));
            }

            for field in &alter.alter_fields {
                if field.new_type.is_some() && field.old_type.is_some() {
                    warnings.push(format!(
                        "Type change from '{}' to '{}' for column '{}' on table '{}' - CQL may reject this if data is incompatible",
                        field.old_type.as_deref().unwrap_or(""),
                        field.new_type.as_deref().unwrap_or(""),
                        field.name,
                        alter.name
                    ));
                }
            }

            up.extend(self.alter_table_statements(alter, ks_context));
        }

        for index in &diff.create_indexes {
            up.push(self.create_index_statement(index, ks_context));
            down.push(self.drop_index_statement(&index.name, ks_context));
        }

        for name in &diff.drop_indexes {
            up.push(self.drop_index_statement(name, ks_context));
        }

        for view in &diff.create_materialized_views {
            up.push(self.create_materialized_view_statement(view, ks_context));
            down.push(self.drop_materialized_view_statement(&view.name, ks_context));
        }

        for name in &diff.drop_materialized_views {
            up.push(self.drop_materialized_view_statement(name, ks_context));
        }

        for name in &diff.drop_tables {
            up.push(self.drop_table_statement(name, ks_context));
            warnings.push(format!("Dropping table '{}' - all rows will be lost", name));
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
        format!(
            "DROP TYPE IF EXISTS {};",
            self.qualify(name, keyspace_context)
        )
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

    fn create_table_statement(
        &self,
        table: &CqlTableDiff,
        keyspace_context: Option<&str>,
    ) -> String {
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
            options.push(format!(
                "compaction = {}",
                self.format_compaction(compaction)
            ));
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
        format!(
            "DROP TABLE IF EXISTS {};",
            self.qualify(name, keyspace_context)
        )
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
            CompactionStrategy::Leveled => "{'class': 'LeveledCompactionStrategy'}".to_string(),
            CompactionStrategy::TimeWindow {
                window_unit,
                window_size,
            } => format!(
                "{{'class': 'TimeWindowCompactionStrategy', 'compaction_window_unit': '{}', 'compaction_window_size': {}}}",
                window_unit, window_size
            ),
        }
    }

    fn alter_table_statements(
        &self,
        alter: &CqlTableAlterDiff,
        keyspace_context: Option<&str>,
    ) -> Vec<String> {
        let qualified = self.qualify(&alter.name, keyspace_context);
        let mut stmts = Vec::new();

        for field in &alter.add_fields {
            stmts.push(format!(
                "ALTER TABLE {} ADD {} {};",
                qualified, field.name, field.cql_type
            ));
        }

        for field_name in &alter.drop_fields {
            stmts.push(format!("ALTER TABLE {} DROP {};", qualified, field_name));
        }

        for field in &alter.alter_fields {
            if let Some(new_type) = &field.new_type {
                stmts.push(format!(
                    "ALTER TABLE {} ALTER {} TYPE {};",
                    qualified, field.name, new_type
                ));
            }
        }

        stmts
    }

    fn create_index_statement(
        &self,
        index: &CqlIndexDiff,
        keyspace_context: Option<&str>,
    ) -> String {
        let qualified_index = self.qualify(&index.name, keyspace_context);
        let qualified_table = self.qualify(&index.table_name, keyspace_context);

        match &index.index_type {
            CqlIndexType::Secondary => format!(
                "CREATE INDEX IF NOT EXISTS {} ON {} ({});",
                qualified_index, qualified_table, index.column
            ),
            CqlIndexType::SasiPrefixed => format!(
                "CREATE CUSTOM INDEX IF NOT EXISTS {} ON {} ({}) USING 'org.apache.cassandra.index.sasi.SASIIndex';",
                qualified_index, qualified_table, index.column
            ),
            CqlIndexType::Custom(class) => format!(
                "CREATE CUSTOM INDEX IF NOT EXISTS {} ON {} ({}) USING '{}';",
                qualified_index, qualified_table, index.column, class
            ),
        }
    }

    fn drop_index_statement(&self, name: &str, keyspace_context: Option<&str>) -> String {
        format!(
            "DROP INDEX IF EXISTS {};",
            self.qualify(name, keyspace_context)
        )
    }

    fn create_materialized_view_statement(
        &self,
        view: &MaterializedViewDiff,
        keyspace_context: Option<&str>,
    ) -> String {
        let qualified_view = self.qualify(&view.name, keyspace_context);
        let qualified_base = self.qualify(&view.base_table, keyspace_context);
        let columns = view.select_columns.join(", ");
        let pk = self.format_primary_key(&view.partition_keys, &view.clustering_keys);

        let mut stmt = format!(
            "CREATE MATERIALIZED VIEW {} AS\nSELECT {} FROM {}\nWHERE {}\n{}",
            qualified_view, columns, qualified_base, view.where_clause, pk
        );

        if !view.clustering_keys.is_empty() {
            stmt.push_str(&format!(
                " WITH {}",
                self.format_clustering_order(&view.clustering_keys)
            ));
        }

        stmt.push(';');
        stmt
    }

    fn drop_materialized_view_statement(
        &self,
        name: &str,
        keyspace_context: Option<&str>,
    ) -> String {
        format!(
            "DROP MATERIALIZED VIEW IF EXISTS {};",
            self.qualify(name, keyspace_context)
        )
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
        ClusteringKey, ClusteringOrder, CompactionStrategy, CqlFieldAlterDiff, CqlFieldDiff,
        CqlIndexDiff, CqlIndexType, CqlTableAlterDiff, CqlTableDiff, KeyspaceConfig,
        MaterializedViewDiff, ReplicationStrategy, UdtAlterDiff, UdtDiff, UdtField,
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
        assert!(
            migration
                .up
                .contains("CREATE KEYSPACE IF NOT EXISTS \"myapp\"")
        );
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
            migration
                .warnings
                .iter()
                .any(|w| w.contains("legacy") && w.contains("ALL data")),
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
        assert!(
            migration
                .down
                .contains("DROP TYPE IF EXISTS \"order_status\"")
        );
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
        assert!(
            migration
                .up
                .contains("CREATE TYPE \"myapp\".\"order_status\" (")
        );
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
        assert!(
            migration
                .up
                .contains("ALTER TYPE \"order_status\" ADD description text")
        );
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
        assert!(
            migration
                .up
                .contains("ALTER TYPE \"order_status\" RENAME old_name TO new_name")
        );
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
            fields: vec![simple_field("id", "uuid"), simple_field("name", "text")],
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
        assert!(
            migration
                .up
                .contains("PRIMARY KEY ((tenant_id, region), event_time, event_id)")
        );
        assert!(
            migration
                .up
                .contains("CLUSTERING ORDER BY (event_time DESC, event_id ASC)")
        );
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
        assert!(
            migration
                .up
                .contains("'class': 'TimeWindowCompactionStrategy'")
        );
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
        assert!(
            migration
                .up
                .contains("DROP TABLE IF EXISTS \"legacy_table\"")
        );
        assert!(
            migration
                .warnings
                .iter()
                .any(|w| w.contains("legacy_table") && w.contains("all rows")),
            "expected drop-table warning"
        );
    }

    #[test]
    fn test_alter_table_add_column() {
        let generator = CqlMigrationGenerator::new();
        let mut diff = CqlSchemaDiff::default();
        diff.alter_tables.push(CqlTableAlterDiff {
            name: "users".into(),
            add_fields: vec![simple_field("email", "text")],
            drop_fields: vec![],
            alter_fields: vec![],
            partition_key_changed: false,
            clustering_key_changed: false,
        });

        let migration = generator.generate(&diff);
        assert!(
            migration
                .up
                .contains("ALTER TABLE \"users\" ADD email text")
        );
    }

    #[test]
    fn test_alter_table_drop_column_generates_warning() {
        let generator = CqlMigrationGenerator::new();
        let mut diff = CqlSchemaDiff::default();
        diff.alter_tables.push(CqlTableAlterDiff {
            name: "users".into(),
            add_fields: vec![],
            drop_fields: vec!["legacy_field".into()],
            alter_fields: vec![],
            partition_key_changed: false,
            clustering_key_changed: false,
        });

        let migration = generator.generate(&diff);
        assert!(
            migration
                .up
                .contains("ALTER TABLE \"users\" DROP legacy_field")
        );
        assert!(
            migration
                .warnings
                .iter()
                .any(|w| w.contains("legacy_field") && w.contains("users")),
            "expected drop-column warning"
        );
    }

    #[test]
    fn test_alter_table_type_change_generates_warning() {
        let generator = CqlMigrationGenerator::new();
        let mut diff = CqlSchemaDiff::default();
        diff.alter_tables.push(CqlTableAlterDiff {
            name: "users".into(),
            add_fields: vec![],
            drop_fields: vec![],
            alter_fields: vec![CqlFieldAlterDiff {
                name: "age".into(),
                old_type: Some("int".into()),
                new_type: Some("bigint".into()),
            }],
            partition_key_changed: false,
            clustering_key_changed: false,
        });

        let migration = generator.generate(&diff);
        assert!(
            migration
                .up
                .contains("ALTER TABLE \"users\" ALTER age TYPE bigint")
        );
        assert!(
            migration
                .warnings
                .iter()
                .any(|w| w.contains("age") && w.contains("data is incompatible")),
            "expected type-change warning"
        );
    }

    #[test]
    fn test_partition_key_change_warns_without_alter() {
        let generator = CqlMigrationGenerator::new();
        let mut diff = CqlSchemaDiff::default();
        diff.alter_tables.push(CqlTableAlterDiff {
            name: "users".into(),
            add_fields: vec![],
            drop_fields: vec![],
            alter_fields: vec![],
            partition_key_changed: true,
            clustering_key_changed: false,
        });

        let migration = generator.generate(&diff);
        assert!(
            migration
                .warnings
                .iter()
                .any(|w| w.contains("Partition key") && w.contains("users")),
            "expected partition-key-change warning"
        );
        assert!(
            !migration.up.contains("ALTER TABLE"),
            "partition key change should not emit ALTER TABLE"
        );
    }

    #[test]
    fn test_clustering_key_change_warns_without_alter() {
        let generator = CqlMigrationGenerator::new();
        let mut diff = CqlSchemaDiff::default();
        diff.alter_tables.push(CqlTableAlterDiff {
            name: "events".into(),
            add_fields: vec![],
            drop_fields: vec![],
            alter_fields: vec![],
            partition_key_changed: false,
            clustering_key_changed: true,
        });

        let migration = generator.generate(&diff);
        assert!(
            migration
                .warnings
                .iter()
                .any(|w| w.contains("Clustering key") && w.contains("events")),
            "expected clustering-key-change warning"
        );
    }

    #[test]
    fn test_create_secondary_index() {
        let generator = CqlMigrationGenerator::new();
        let mut diff = CqlSchemaDiff::default();
        diff.create_indexes.push(CqlIndexDiff {
            name: "users_email_idx".into(),
            table_name: "users".into(),
            column: "email".into(),
            index_type: CqlIndexType::Secondary,
        });

        let migration = generator.generate(&diff);
        assert!(
            migration
                .up
                .contains("CREATE INDEX IF NOT EXISTS \"users_email_idx\"")
        );
        assert!(migration.up.contains("ON \"users\" (email)"));
        assert!(
            migration
                .down
                .contains("DROP INDEX IF EXISTS \"users_email_idx\"")
        );
    }

    #[test]
    fn test_create_sasi_index() {
        let generator = CqlMigrationGenerator::new();
        let mut diff = CqlSchemaDiff::default();
        diff.create_indexes.push(CqlIndexDiff {
            name: "users_name_sasi".into(),
            table_name: "users".into(),
            column: "name".into(),
            index_type: CqlIndexType::SasiPrefixed,
        });

        let migration = generator.generate(&diff);
        assert!(
            migration
                .up
                .contains("USING 'org.apache.cassandra.index.sasi.SASIIndex'")
        );
    }

    #[test]
    fn test_create_custom_index() {
        let generator = CqlMigrationGenerator::new();
        let mut diff = CqlSchemaDiff::default();
        diff.create_indexes.push(CqlIndexDiff {
            name: "custom_idx".into(),
            table_name: "users".into(),
            column: "data".into(),
            index_type: CqlIndexType::Custom("my.custom.IndexClass".into()),
        });

        let migration = generator.generate(&diff);
        assert!(migration.up.contains("USING 'my.custom.IndexClass'"));
    }

    #[test]
    fn test_drop_index() {
        let generator = CqlMigrationGenerator::new();
        let mut diff = CqlSchemaDiff::default();
        diff.drop_indexes.push("old_idx".into());

        let migration = generator.generate(&diff);
        assert!(migration.up.contains("DROP INDEX IF EXISTS \"old_idx\""));
    }

    #[test]
    fn test_create_materialized_view() {
        let generator = CqlMigrationGenerator::new();
        let mut diff = CqlSchemaDiff::default();
        diff.create_materialized_views.push(MaterializedViewDiff {
            name: "events_by_status".into(),
            base_table: "events".into(),
            select_columns: vec!["*".into()],
            where_clause: "status IS NOT NULL AND tenant_id IS NOT NULL AND event_time IS NOT NULL"
                .into(),
            partition_keys: vec!["status".into()],
            clustering_keys: vec![
                ClusteringKey {
                    name: "tenant_id".into(),
                    order: ClusteringOrder::Asc,
                },
                ClusteringKey {
                    name: "event_time".into(),
                    order: ClusteringOrder::Desc,
                },
            ],
        });

        let migration = generator.generate(&diff);
        assert!(
            migration
                .up
                .contains("CREATE MATERIALIZED VIEW \"events_by_status\"")
        );
        assert!(migration.up.contains("FROM \"events\""));
        assert!(migration.up.contains("WHERE status IS NOT NULL"));
        assert!(
            migration
                .up
                .contains("PRIMARY KEY ((status), tenant_id, event_time)")
        );
        assert!(
            migration
                .up
                .contains("CLUSTERING ORDER BY (tenant_id ASC, event_time DESC)")
        );
        assert!(
            migration
                .down
                .contains("DROP MATERIALIZED VIEW IF EXISTS \"events_by_status\"")
        );
    }

    #[test]
    fn test_drop_materialized_view() {
        let generator = CqlMigrationGenerator::new();
        let mut diff = CqlSchemaDiff::default();
        diff.drop_materialized_views.push("old_view".into());

        let migration = generator.generate(&diff);
        assert!(
            migration
                .up
                .contains("DROP MATERIALIZED VIEW IF EXISTS \"old_view\"")
        );
    }
}
