//! CQL schema diff types.

/// A diff describing CQL schema changes to apply.
#[derive(Debug, Default, Clone)]
pub struct CqlSchemaDiff {
    /// Keyspace to create (runs first in up).
    pub create_keyspace: Option<KeyspaceConfig>,
    /// Keyspace to drop (runs last in up).
    pub drop_keyspace: Option<String>,

    /// User-Defined Types to create (before tables that reference them).
    pub create_udts: Vec<UdtDiff>,
    /// UDT names to drop.
    pub drop_udts: Vec<String>,
    /// UDT alterations.
    pub alter_udts: Vec<UdtAlterDiff>,

    /// Tables to create.
    pub create_tables: Vec<CqlTableDiff>,
    /// Table names to drop.
    pub drop_tables: Vec<String>,
    /// Table alterations.
    pub alter_tables: Vec<CqlTableAlterDiff>,

    /// Materialized views to create.
    pub create_materialized_views: Vec<MaterializedViewDiff>,
    /// Materialized view names to drop.
    pub drop_materialized_views: Vec<String>,

    /// Secondary indexes to create.
    pub create_indexes: Vec<CqlIndexDiff>,
    /// Index names to drop.
    pub drop_indexes: Vec<String>,

    /// Keyspace to use when qualifying object names. If None, generator emits
    /// unqualified names (useful for testing).
    pub keyspace_context: Option<String>,
}

/// Keyspace configuration for CREATE KEYSPACE.
#[derive(Debug, Clone)]
pub struct KeyspaceConfig {
    pub name: String,
    pub replication: ReplicationStrategy,
    pub durable_writes: bool,
}

/// Replication strategy for a keyspace.
#[derive(Debug, Clone)]
pub enum ReplicationStrategy {
    Simple { factor: u32 },
    NetworkTopology { dc_factors: Vec<(String, u32)> },
}

/// User-Defined Type diff (creation).
#[derive(Debug, Clone)]
pub struct UdtDiff {
    pub name: String,
    pub fields: Vec<UdtField>,
}

/// A field inside a UDT.
#[derive(Debug, Clone)]
pub struct UdtField {
    pub name: String,
    pub cql_type: String,
}

/// UDT alteration (add fields, rename fields).
#[derive(Debug, Clone)]
pub struct UdtAlterDiff {
    pub name: String,
    pub add_fields: Vec<UdtField>,
    pub rename_fields: Vec<(String, String)>,
}

/// Table creation diff.
#[derive(Debug, Clone)]
pub struct CqlTableDiff {
    pub name: String,
    pub fields: Vec<CqlFieldDiff>,
    pub partition_keys: Vec<String>,
    pub clustering_keys: Vec<ClusteringKey>,
    pub compaction: Option<CompactionStrategy>,
    pub default_ttl: Option<u32>,
}

/// A column in a CQL table.
#[derive(Debug, Clone)]
pub struct CqlFieldDiff {
    pub name: String,
    pub cql_type: String,
    pub is_static: bool,
}

/// A clustering key with explicit ordering.
#[derive(Debug, Clone)]
pub struct ClusteringKey {
    pub name: String,
    pub order: ClusteringOrder,
}

/// Clustering order direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClusteringOrder {
    Asc,
    Desc,
}

/// Compaction strategy for a table.
#[derive(Debug, Clone)]
pub enum CompactionStrategy {
    SizeTiered,
    Leveled,
    TimeWindow {
        window_unit: String,
        window_size: u32,
    },
}

/// Table alteration (add/drop/alter columns).
#[derive(Debug, Clone, Default)]
pub struct CqlTableAlterDiff {
    pub name: String,
    pub add_fields: Vec<CqlFieldDiff>,
    pub drop_fields: Vec<String>,
    pub alter_fields: Vec<CqlFieldAlterDiff>,
    /// Partition key change detected (generates warning, no SQL).
    pub partition_key_changed: bool,
    /// Clustering key change detected (generates warning, no SQL).
    pub clustering_key_changed: bool,
}

/// Column alteration within a table.
#[derive(Debug, Clone)]
pub struct CqlFieldAlterDiff {
    pub name: String,
    pub old_type: Option<String>,
    pub new_type: Option<String>,
}

/// Materialized view diff.
#[derive(Debug, Clone)]
pub struct MaterializedViewDiff {
    pub name: String,
    pub base_table: String,
    pub select_columns: Vec<String>,
    pub where_clause: String,
    pub partition_keys: Vec<String>,
    pub clustering_keys: Vec<ClusteringKey>,
}

/// Secondary index diff.
#[derive(Debug, Clone)]
pub struct CqlIndexDiff {
    pub name: String,
    pub table_name: String,
    pub column: String,
    pub index_type: CqlIndexType,
}

/// Secondary index type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CqlIndexType {
    Secondary,
    SasiPrefixed,
    Custom(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cql_schema_diff_default_is_empty() {
        let diff = CqlSchemaDiff::default();
        assert!(diff.create_keyspace.is_none());
        assert!(diff.create_tables.is_empty());
        assert!(diff.create_udts.is_empty());
        assert!(diff.keyspace_context.is_none());
    }

    #[test]
    fn test_replication_strategy_variants() {
        let simple = ReplicationStrategy::Simple { factor: 3 };
        let nt = ReplicationStrategy::NetworkTopology {
            dc_factors: vec![("us-east".to_string(), 3), ("us-west".to_string(), 2)],
        };
        match simple {
            ReplicationStrategy::Simple { factor } => assert_eq!(factor, 3),
            _ => panic!("wrong variant"),
        }
        match nt {
            ReplicationStrategy::NetworkTopology { dc_factors } => {
                assert_eq!(dc_factors.len(), 2);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_clustering_order_equality() {
        assert_eq!(ClusteringOrder::Asc, ClusteringOrder::Asc);
        assert_ne!(ClusteringOrder::Asc, ClusteringOrder::Desc);
    }
}
