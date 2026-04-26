//! CQL (Cassandra Query Language) migration support for ScyllaDB.

use crate::dialect::MigrationDialect;

pub mod diff;
pub mod generator;
pub mod migration;

/// The CQL dialect for ScyllaDB.
pub struct CqlDialect;

impl MigrationDialect for CqlDialect {
    type Diff = CqlSchemaDiff;
    type Migration = MigrationCql;

    fn name() -> &'static str {
        "cql"
    }

    fn generate(diff: &CqlSchemaDiff) -> MigrationCql {
        CqlMigrationGenerator::new().generate(diff)
    }

    fn event_log_table() -> &'static str {
        "_prax_cql_migrations"
    }
}

pub use diff::{
    ClusteringKey, ClusteringOrder, CompactionStrategy, CqlFieldAlterDiff, CqlFieldDiff,
    CqlIndexDiff, CqlIndexType, CqlSchemaDiff, CqlTableAlterDiff, CqlTableDiff, KeyspaceConfig,
    MaterializedViewDiff, ReplicationStrategy, UdtAlterDiff, UdtDiff, UdtField,
};
pub use generator::CqlMigrationGenerator;
pub use migration::MigrationCql;
