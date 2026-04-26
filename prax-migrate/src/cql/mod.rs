//! CQL (Cassandra Query Language) migration support for ScyllaDB.

pub mod diff;
pub mod migration;

pub use diff::{
    ClusteringKey, ClusteringOrder, CompactionStrategy, CqlFieldAlterDiff, CqlFieldDiff,
    CqlIndexDiff, CqlIndexType, CqlSchemaDiff, CqlTableAlterDiff, CqlTableDiff, KeyspaceConfig,
    MaterializedViewDiff, ReplicationStrategy, UdtAlterDiff, UdtDiff, UdtField,
};
pub use migration::MigrationCql;
