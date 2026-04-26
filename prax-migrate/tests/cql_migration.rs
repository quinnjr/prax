//! Integration tests for CQL migration support.

use prax_migrate::{
    ClusteringKey, ClusteringOrder, CompactionStrategy, CqlDialect, CqlFieldDiff, CqlIndexDiff,
    CqlIndexType, CqlMigrationGenerator, CqlSchemaDiff, CqlTableDiff, KeyspaceConfig,
    MaterializedViewDiff, MigrationDialect, ReplicationStrategy, UdtDiff, UdtField,
};

fn field(name: &str, cql_type: &str) -> CqlFieldDiff {
    CqlFieldDiff {
        name: name.into(),
        cql_type: cql_type.into(),
        is_static: false,
    }
}

#[test]
fn test_cql_full_workflow_keyspace_udt_table_index_view() {
    let mut diff = CqlSchemaDiff {
        keyspace_context: Some("myapp".into()),
        ..Default::default()
    };

    diff.create_keyspace = Some(KeyspaceConfig {
        name: "myapp".into(),
        replication: ReplicationStrategy::Simple { factor: 3 },
        durable_writes: true,
    });

    diff.create_udts.push(UdtDiff {
        name: "order_status".into(),
        fields: vec![UdtField {
            name: "value".into(),
            cql_type: "text".into(),
        }],
    });

    diff.create_tables.push(CqlTableDiff {
        name: "events".into(),
        fields: vec![
            field("tenant_id", "uuid"),
            field("event_time", "timestamp"),
            field("event_id", "uuid"),
            field("payload", "text"),
            field("status", "frozen<\"order_status\">"),
        ],
        partition_keys: vec!["tenant_id".into()],
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
        compaction: Some(CompactionStrategy::TimeWindow {
            window_unit: "DAYS".into(),
            window_size: 1,
        }),
        default_ttl: Some(2592000),
    });

    diff.create_indexes.push(CqlIndexDiff {
        name: "events_status_idx".into(),
        table_name: "events".into(),
        column: "status".into(),
        index_type: CqlIndexType::Secondary,
    });

    diff.create_materialized_views.push(MaterializedViewDiff {
        name: "events_by_status".into(),
        base_table: "events".into(),
        select_columns: vec!["*".into()],
        where_clause: "status IS NOT NULL AND tenant_id IS NOT NULL AND event_time IS NOT NULL AND event_id IS NOT NULL".into(),
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
            ClusteringKey {
                name: "event_id".into(),
                order: ClusteringOrder::Asc,
            },
        ],
    });

    let migration = CqlMigrationGenerator::new().generate(&diff);

    // Ordering check: keyspace first, then UDT, then table, then index, then view
    let ks_pos = migration
        .up
        .find("CREATE KEYSPACE")
        .expect("keyspace missing");
    let udt_pos = migration.up.find("CREATE TYPE").expect("UDT missing");
    let tbl_pos = migration.up.find("CREATE TABLE").expect("table missing");
    let idx_pos = migration.up.find("CREATE INDEX").expect("index missing");
    let mv_pos = migration
        .up
        .find("CREATE MATERIALIZED VIEW")
        .expect("MV missing");

    assert!(ks_pos < udt_pos);
    assert!(udt_pos < tbl_pos);
    assert!(tbl_pos < idx_pos);
    assert!(idx_pos < mv_pos);

    // Keyspace context applied to names
    assert!(migration.up.contains("CREATE TABLE \"myapp\".\"events\""));
    assert!(
        migration
            .up
            .contains("CREATE TYPE \"myapp\".\"order_status\"")
    );

    // No warnings for pure creation
    assert!(migration.warnings.is_empty());
}

#[test]
fn test_cql_down_reverses_dependency_order() {
    let mut diff = CqlSchemaDiff::default();
    diff.create_keyspace = Some(KeyspaceConfig {
        name: "myapp".into(),
        replication: ReplicationStrategy::Simple { factor: 1 },
        durable_writes: true,
    });
    diff.create_udts.push(UdtDiff {
        name: "my_type".into(),
        fields: vec![UdtField {
            name: "v".into(),
            cql_type: "int".into(),
        }],
    });
    diff.create_tables.push(CqlTableDiff {
        name: "t".into(),
        fields: vec![field("id", "uuid")],
        partition_keys: vec!["id".into()],
        clustering_keys: vec![],
        compaction: None,
        default_ttl: None,
    });

    let migration = CqlMigrationGenerator::new().generate(&diff);

    // Down order should be: DROP TABLE, DROP TYPE, DROP KEYSPACE
    let drop_tbl = migration
        .down
        .find("DROP TABLE")
        .expect("DROP TABLE missing");
    let drop_type = migration.down.find("DROP TYPE").expect("DROP TYPE missing");
    let drop_ks = migration
        .down
        .find("DROP KEYSPACE")
        .expect("DROP KEYSPACE missing");
    assert!(drop_tbl < drop_type);
    assert!(drop_type < drop_ks);
}

#[test]
fn test_cql_dialect_trait_produces_same_output_as_generator() {
    let mut diff = CqlSchemaDiff::default();
    diff.create_tables.push(CqlTableDiff {
        name: "t".into(),
        fields: vec![field("id", "uuid")],
        partition_keys: vec!["id".into()],
        clustering_keys: vec![],
        compaction: None,
        default_ttl: None,
    });

    let via_trait = CqlDialect::generate(&diff);
    let via_direct = CqlMigrationGenerator::new().generate(&diff);

    assert_eq!(via_trait.up, via_direct.up);
    assert_eq!(via_trait.down, via_direct.down);
    assert_eq!(via_trait.warnings, via_direct.warnings);
}

#[test]
fn test_cql_destructive_operations_generate_warnings() {
    let mut diff = CqlSchemaDiff::default();
    diff.drop_tables.push("legacy".into());
    diff.drop_udts.push("old_type".into());
    diff.drop_keyspace = Some("old_app".into());

    let migration = CqlMigrationGenerator::new().generate(&diff);

    assert_eq!(migration.warnings.len(), 3);
    assert!(migration.warnings.iter().any(|w| w.contains("legacy")));
    assert!(migration.warnings.iter().any(|w| w.contains("old_type")));
    assert!(migration.warnings.iter().any(|w| w.contains("old_app")));
}
