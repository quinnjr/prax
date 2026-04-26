//! Example: generate CQL migration scripts for a ScyllaDB schema.
//!
//! Run with: cargo run --example cql_migration -p prax-migrate

use prax_migrate::{
    ClusteringKey, ClusteringOrder, CompactionStrategy, CqlFieldDiff, CqlIndexDiff, CqlIndexType,
    CqlMigrationGenerator, CqlSchemaDiff, CqlTableDiff, KeyspaceConfig, MaterializedViewDiff,
    ReplicationStrategy, UdtDiff, UdtField,
};

fn field(name: &str, cql_type: &str) -> CqlFieldDiff {
    CqlFieldDiff {
        name: name.into(),
        cql_type: cql_type.into(),
        is_static: false,
    }
}

fn main() {
    let mut diff = CqlSchemaDiff {
        keyspace_context: Some("myapp".into()),
        ..Default::default()
    };

    diff.create_keyspace = Some(KeyspaceConfig {
        name: "myapp".into(),
        replication: ReplicationStrategy::NetworkTopology {
            dc_factors: vec![
                ("us-east".into(), 3),
                ("us-west".into(), 2),
            ],
        },
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
        default_ttl: Some(2_592_000),
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

    println!("=== up.cql ===");
    println!("{}", migration.up);
    println!();
    println!("=== down.cql ===");
    println!("{}", migration.down);

    if !migration.warnings.is_empty() {
        println!();
        println!("=== warnings ===");
        for w in &migration.warnings {
            println!("- {}", w);
        }
    }
}
