# ScyllaDB Migration Support Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add ScyllaDB/CQL migration support to prax-migrate via a `MigrationDialect` trait that abstracts migration output over SQL and CQL dialects.

**Architecture:** Introduce `MigrationDialect` trait in `prax-migrate/src/dialect.rs`. Implement `SqlDialect` wrapping existing PostgresSqlGenerator behavior. Build new CQL module (`prax-migrate/src/cql/`) with `CqlSchemaDiff`, `MigrationCql`, and `CqlMigrationGenerator`. Keep the existing SQL engine unchanged; introduce a parallel CQL migration flow. Full genericization of `MigrationEngine` can follow in a later PR.

**Tech Stack:** Rust, CQL (Cassandra Query Language), existing prax-migrate event sourcing

---

## File Structure

**New files:**
- `prax-migrate/src/dialect.rs` - MigrationDialect trait + SqlDialect impl
- `prax-migrate/src/cql/mod.rs` - CQL module entry, CqlDialect impl, re-exports
- `prax-migrate/src/cql/diff.rs` - CqlSchemaDiff and related types
- `prax-migrate/src/cql/migration.rs` - MigrationCql output type
- `prax-migrate/src/cql/generator.rs` - CqlMigrationGenerator
- `prax-migrate/tests/cql_migration.rs` - integration tests
- `prax-migrate/examples/cql_migration.rs` - runnable example

**Modified files:**
- `prax-migrate/src/lib.rs` - export new modules and types

---

### Task 1: Introduce MigrationDialect trait with SqlDialect

**Files:**
- Create: `prax-migrate/src/dialect.rs`
- Modify: `prax-migrate/src/lib.rs`

- [ ] **Step 1: Write failing test**

Add to `prax-migrate/src/dialect.rs` (at the end of the file, creating it):

```rust
//! Migration dialect trait for abstracting over SQL and CQL backends.

use crate::diff::SchemaDiff;
use crate::sql::{MigrationSql, PostgresSqlGenerator};

/// A migration dialect abstracts the schema diff type, migration output type,
/// and generator for a specific database backend.
pub trait MigrationDialect {
    /// The schema diff type for this dialect.
    type Diff: Default + Send + Sync;

    /// The migration output type for this dialect.
    type Migration: Send + Sync;

    /// Human-readable dialect name (e.g., "sql", "cql").
    fn name() -> &'static str;

    /// Generate a migration from a schema diff.
    fn generate(diff: &Self::Diff) -> Self::Migration;

    /// Event log table name used by this dialect.
    fn event_log_table() -> &'static str;
}

/// The SQL dialect (PostgreSQL, MySQL, SQLite, MSSQL, DuckDB share this).
pub struct SqlDialect;

impl MigrationDialect for SqlDialect {
    type Diff = SchemaDiff;
    type Migration = MigrationSql;

    fn name() -> &'static str {
        "sql"
    }

    fn generate(diff: &SchemaDiff) -> MigrationSql {
        PostgresSqlGenerator.generate(diff)
    }

    fn event_log_table() -> &'static str {
        "_prax_migrations"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sql_dialect_name() {
        assert_eq!(SqlDialect::name(), "sql");
    }

    #[test]
    fn test_sql_dialect_event_log_table() {
        assert_eq!(SqlDialect::event_log_table(), "_prax_migrations");
    }

    #[test]
    fn test_sql_dialect_generates_empty_migration_from_empty_diff() {
        let diff = SchemaDiff::default();
        let migration = SqlDialect::generate(&diff);
        assert!(migration.is_empty());
    }
}
```

- [ ] **Step 2: Register the module**

Add to `prax-migrate/src/lib.rs` in the module declarations section (near other `pub mod` lines):

```rust
pub mod dialect;
```

And in the re-exports section, add:

```rust
pub use dialect::{MigrationDialect, SqlDialect};
```

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test -p prax-migrate --lib dialect::tests`
Expected: 3 tests pass.

- [ ] **Step 4: Commit**

```bash
git add prax-migrate/src/dialect.rs prax-migrate/src/lib.rs
git commit -m "feat(migrate): add MigrationDialect trait with SqlDialect

Introduce a trait that abstracts migration operations over database
dialects. SqlDialect wraps the existing PostgresSqlGenerator behavior
and uses the _prax_migrations event log table. CqlDialect will follow
for ScyllaDB support."
```

---

### Task 2: Create MigrationCql output type

**Files:**
- Create: `prax-migrate/src/cql/mod.rs`
- Create: `prax-migrate/src/cql/migration.rs`

- [ ] **Step 1: Create the CQL module directory and placeholder**

Create `prax-migrate/src/cql/mod.rs`:

```rust
//! CQL (Cassandra Query Language) migration support for ScyllaDB.

pub mod migration;

pub use migration::MigrationCql;
```

- [ ] **Step 2: Register cql module in lib.rs**

Add to `prax-migrate/src/lib.rs` in the module declarations section:

```rust
pub mod cql;
```

And in the re-exports section:

```rust
pub use cql::MigrationCql;
```

- [ ] **Step 3: Write failing tests for MigrationCql**

Create `prax-migrate/src/cql/migration.rs`:

```rust
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
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p prax-migrate --lib cql::migration`
Expected: 3 tests pass.

- [ ] **Step 5: Commit**

```bash
git add prax-migrate/src/cql/mod.rs prax-migrate/src/cql/migration.rs prax-migrate/src/lib.rs
git commit -m "feat(migrate): add MigrationCql output type

Mirrors MigrationSql but tailored for CQL: up/down strings with
warnings vector. Provides is_empty() for test assertions."
```

---

### Task 3: Create CqlSchemaDiff and supporting types

**Files:**
- Create: `prax-migrate/src/cql/diff.rs`
- Modify: `prax-migrate/src/cql/mod.rs`

- [ ] **Step 1: Write the CqlSchemaDiff types**

Create `prax-migrate/src/cql/diff.rs`:

```rust
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
    TimeWindow { window_unit: String, window_size: u32 },
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
```

- [ ] **Step 2: Re-export from cql module**

Edit `prax-migrate/src/cql/mod.rs` to add:

```rust
//! CQL (Cassandra Query Language) migration support for ScyllaDB.

pub mod diff;
pub mod migration;

pub use diff::{
    ClusteringKey, ClusteringOrder, CompactionStrategy, CqlFieldAlterDiff, CqlFieldDiff,
    CqlIndexDiff, CqlIndexType, CqlSchemaDiff, CqlTableAlterDiff, CqlTableDiff, KeyspaceConfig,
    MaterializedViewDiff, ReplicationStrategy, UdtAlterDiff, UdtDiff, UdtField,
};
pub use migration::MigrationCql;
```

- [ ] **Step 3: Re-export from lib.rs**

Add to `prax-migrate/src/lib.rs` re-exports section:

```rust
pub use cql::{
    ClusteringKey, ClusteringOrder, CompactionStrategy, CqlFieldAlterDiff, CqlFieldDiff,
    CqlIndexDiff, CqlIndexType, CqlSchemaDiff, CqlTableAlterDiff, CqlTableDiff, KeyspaceConfig,
    MaterializedViewDiff, MigrationCql, ReplicationStrategy, UdtAlterDiff, UdtDiff, UdtField,
};
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p prax-migrate --lib cql::diff`
Expected: 3 tests pass.

- [ ] **Step 5: Commit**

```bash
git add prax-migrate/src/cql/diff.rs prax-migrate/src/cql/mod.rs prax-migrate/src/lib.rs
git commit -m "feat(migrate): add CqlSchemaDiff and supporting types

Define the CQL-specific schema diff model: keyspaces, UDTs, tables
with partition/clustering keys, materialized views, secondary indexes.
Uses ReplicationStrategy enum for Simple vs NetworkTopology strategies
and CompactionStrategy for SizeTiered/Leveled/TimeWindow."
```

---

### Task 4: CqlMigrationGenerator skeleton

**Files:**
- Create: `prax-migrate/src/cql/generator.rs`
- Modify: `prax-migrate/src/cql/mod.rs`

- [ ] **Step 1: Create the generator skeleton**

Create `prax-migrate/src/cql/generator.rs`:

```rust
//! CQL migration SQL generator for ScyllaDB.

use crate::cql::diff::CqlSchemaDiff;
use crate::cql::migration::MigrationCql;

/// Generates CQL migration scripts from a CqlSchemaDiff.
pub struct CqlMigrationGenerator;

impl CqlMigrationGenerator {
    /// Create a new generator.
    pub fn new() -> Self {
        Self
    }

    /// Generate a CQL migration from a schema diff.
    pub fn generate(&self, _diff: &CqlSchemaDiff) -> MigrationCql {
        MigrationCql::default()
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

    #[test]
    fn test_empty_diff_produces_empty_migration() {
        let generator = CqlMigrationGenerator::new();
        let diff = CqlSchemaDiff::default();
        let migration = generator.generate(&diff);
        assert!(migration.is_empty());
        assert!(migration.warnings.is_empty());
    }
}
```

- [ ] **Step 2: Register module and re-export**

Edit `prax-migrate/src/cql/mod.rs`:

```rust
//! CQL (Cassandra Query Language) migration support for ScyllaDB.

pub mod diff;
pub mod generator;
pub mod migration;

pub use diff::{
    ClusteringKey, ClusteringOrder, CompactionStrategy, CqlFieldAlterDiff, CqlFieldDiff,
    CqlIndexDiff, CqlIndexType, CqlSchemaDiff, CqlTableAlterDiff, CqlTableDiff, KeyspaceConfig,
    MaterializedViewDiff, ReplicationStrategy, UdtAlterDiff, UdtDiff, UdtField,
};
pub use generator::CqlMigrationGenerator;
pub use migration::MigrationCql;
```

Add to `prax-migrate/src/lib.rs` re-exports:

```rust
pub use cql::CqlMigrationGenerator;
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p prax-migrate --lib cql::generator`
Expected: 1 test passes.

- [ ] **Step 4: Commit**

```bash
git add prax-migrate/src/cql/generator.rs prax-migrate/src/cql/mod.rs prax-migrate/src/lib.rs
git commit -m "feat(migrate): add CqlMigrationGenerator skeleton

Empty implementation that returns an empty MigrationCql. Following
tasks implement keyspace, UDT, table, index, and materialized view
generation."
```

---

### Task 5: CqlDialect implementation

**Files:**
- Modify: `prax-migrate/src/cql/mod.rs`
- Modify: `prax-migrate/src/dialect.rs`

- [ ] **Step 1: Add CqlDialect to cql/mod.rs**

Add at the top of `prax-migrate/src/cql/mod.rs` (below the module declarations):

```rust
use crate::dialect::MigrationDialect;

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
```

- [ ] **Step 2: Add test in dialect.rs**

Add to the `#[cfg(test)] mod tests` block in `prax-migrate/src/dialect.rs`:

```rust
    #[test]
    fn test_cql_dialect_name() {
        use crate::cql::CqlDialect;
        assert_eq!(CqlDialect::name(), "cql");
    }

    #[test]
    fn test_cql_dialect_event_log_table() {
        use crate::cql::CqlDialect;
        assert_eq!(CqlDialect::event_log_table(), "_prax_cql_migrations");
    }

    #[test]
    fn test_cql_dialect_generates_empty_migration_from_empty_diff() {
        use crate::cql::{CqlDialect, CqlSchemaDiff};
        let diff = CqlSchemaDiff::default();
        let migration = CqlDialect::generate(&diff);
        assert!(migration.is_empty());
    }
```

- [ ] **Step 3: Re-export CqlDialect**

Add to `prax-migrate/src/lib.rs` re-exports:

```rust
pub use cql::CqlDialect;
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p prax-migrate --lib dialect::tests`
Expected: 6 tests pass (3 SQL + 3 CQL).

- [ ] **Step 5: Commit**

```bash
git add prax-migrate/src/cql/mod.rs prax-migrate/src/dialect.rs prax-migrate/src/lib.rs
git commit -m "feat(migrate): implement CqlDialect

CqlDialect implements MigrationDialect for ScyllaDB:
- Diff type: CqlSchemaDiff
- Migration type: MigrationCql
- Event log table: _prax_cql_migrations
- Generator: CqlMigrationGenerator"
```

---

### Task 6: Keyspace generation

**Files:**
- Modify: `prax-migrate/src/cql/generator.rs`

- [ ] **Step 1: Write failing tests**

Add to the `#[cfg(test)] mod tests` block in `prax-migrate/src/cql/generator.rs`:

```rust
    use crate::cql::diff::{KeyspaceConfig, ReplicationStrategy};

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
```

- [ ] **Step 2: Implement keyspace generation**

Replace the contents of `prax-migrate/src/cql/generator.rs` (keep the existing test module structure, replace the impl):

```rust
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
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p prax-migrate --lib cql::generator`
Expected: 4 tests pass.

- [ ] **Step 4: Commit**

```bash
git add prax-migrate/src/cql/generator.rs
git commit -m "feat(migrate): generate CQL CREATE/DROP KEYSPACE statements

Support both SimpleStrategy and NetworkTopologyStrategy. Drop keyspace
produces a loud warning since it wipes all data in the keyspace."
```

---

### Task 7: UDT generation (create, drop, alter)

**Files:**
- Modify: `prax-migrate/src/cql/generator.rs`

- [ ] **Step 1: Write failing tests**

Add to the test module in `prax-migrate/src/cql/generator.rs`:

```rust
    use crate::cql::diff::{UdtAlterDiff, UdtDiff, UdtField};

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
```

- [ ] **Step 2: Implement UDT generation**

Update `prax-migrate/src/cql/generator.rs` - add imports at the top:

```rust
use crate::cql::diff::{
    CqlSchemaDiff, KeyspaceConfig, ReplicationStrategy, UdtAlterDiff, UdtDiff,
};
```

Replace the `generate` method with:

```rust
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
```

Add the UDT helper methods to the impl block:

```rust
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
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p prax-migrate --lib cql::generator`
Expected: 9 tests pass.

- [ ] **Step 4: Commit**

```bash
git add prax-migrate/src/cql/generator.rs
git commit -m "feat(migrate): generate CQL UDT statements

CREATE/DROP TYPE and ALTER TYPE (ADD field, RENAME field). UDTs are
qualified with keyspace_context when set. DROP TYPE generates a
warning to check for referencing tables."
```

---

### Task 8: Table creation

**Files:**
- Modify: `prax-migrate/src/cql/generator.rs`

- [ ] **Step 1: Write failing tests**

Add to the test module:

```rust
    use crate::cql::diff::{
        ClusteringKey, ClusteringOrder, CompactionStrategy, CqlFieldDiff, CqlTableDiff,
    };

    fn simple_field(name: &str, cql_type: &str) -> CqlFieldDiff {
        CqlFieldDiff {
            name: name.into(),
            cql_type: cql_type.into(),
            is_static: false,
        }
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
```

- [ ] **Step 2: Implement table generation**

In `prax-migrate/src/cql/generator.rs`, update the imports at the top:

```rust
use crate::cql::diff::{
    ClusteringKey, ClusteringOrder, CompactionStrategy, CqlFieldDiff, CqlSchemaDiff, CqlTableDiff,
    KeyspaceConfig, ReplicationStrategy, UdtAlterDiff, UdtDiff,
};
```

Update the `generate` method - insert table handling after UDT creation and before drop_udts:

```rust
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
```

Add table helper methods to the impl block:

```rust
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
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p prax-migrate --lib cql::generator`
Expected: 14 tests pass.

- [ ] **Step 4: Commit**

```bash
git add prax-migrate/src/cql/generator.rs
git commit -m "feat(migrate): generate CQL CREATE TABLE statements

Support single and compound partition keys, clustering keys with ASC/DESC
order, static columns, compaction strategies (SizeTiered, Leveled,
TimeWindow), and default TTL. DROP TABLE generates a data-loss warning."
```

---

### Task 9: ALTER TABLE operations with warnings

**Files:**
- Modify: `prax-migrate/src/cql/generator.rs`

- [ ] **Step 1: Write failing tests**

Add to the test module:

```rust
    use crate::cql::diff::{CqlFieldAlterDiff, CqlTableAlterDiff};

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
        assert!(migration.up.contains("ALTER TABLE \"users\" ADD email text"));
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
        assert!(migration.up.contains("ALTER TABLE \"users\" DROP legacy_field"));
        assert!(
            migration.warnings.iter().any(|w| w.contains("legacy_field") && w.contains("users")),
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
        assert!(migration.up.contains("ALTER TABLE \"users\" ALTER age TYPE bigint"));
        assert!(
            migration.warnings.iter().any(|w| w.contains("age") && w.contains("data is incompatible")),
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
            migration.warnings.iter().any(|w| w.contains("partition key") && w.contains("users")),
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
            migration.warnings.iter().any(|w| w.contains("clustering key") && w.contains("events")),
            "expected clustering-key-change warning"
        );
    }
```

- [ ] **Step 2: Implement ALTER TABLE generation**

In `prax-migrate/src/cql/generator.rs`, update imports:

```rust
use crate::cql::diff::{
    ClusteringKey, ClusteringOrder, CompactionStrategy, CqlFieldAlterDiff, CqlFieldDiff,
    CqlSchemaDiff, CqlTableAlterDiff, CqlTableDiff, KeyspaceConfig, ReplicationStrategy,
    UdtAlterDiff, UdtDiff,
};
```

Update the `generate` method - insert alter_tables handling after create_tables and before drop_tables:

```rust
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

        for name in &diff.drop_tables {
```

Add the `alter_table_statements` helper method:

```rust
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
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p prax-migrate --lib cql::generator`
Expected: 19 tests pass.

- [ ] **Step 4: Commit**

```bash
git add prax-migrate/src/cql/generator.rs
git commit -m "feat(migrate): generate CQL ALTER TABLE with warnings

Support ADD column, DROP column, and ALTER column TYPE. DROP and type
changes generate data-loss warnings. Partition/clustering key changes
warn loudly and emit no ALTER (CQL forbids in-place changes)."
```

---

### Task 10: Secondary indexes

**Files:**
- Modify: `prax-migrate/src/cql/generator.rs`

- [ ] **Step 1: Write failing tests**

Add to the test module:

```rust
    use crate::cql::diff::{CqlIndexDiff, CqlIndexType};

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
        assert!(migration.up.contains("CREATE INDEX IF NOT EXISTS \"users_email_idx\""));
        assert!(migration.up.contains("ON \"users\" (email)"));
        assert!(migration.down.contains("DROP INDEX IF EXISTS \"users_email_idx\""));
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
        assert!(migration.up.contains("USING 'org.apache.cassandra.index.sasi.SASIIndex'"));
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
```

- [ ] **Step 2: Implement index generation**

In `prax-migrate/src/cql/generator.rs`, update imports:

```rust
use crate::cql::diff::{
    ClusteringKey, ClusteringOrder, CompactionStrategy, CqlFieldAlterDiff, CqlFieldDiff,
    CqlIndexDiff, CqlIndexType, CqlSchemaDiff, CqlTableAlterDiff, CqlTableDiff, KeyspaceConfig,
    ReplicationStrategy, UdtAlterDiff, UdtDiff,
};
```

Update the `generate` method - insert index handling after alter_tables and before drop_tables:

```rust
        for alter in &diff.alter_tables {
            // ... existing alter_tables code ...
            up.extend(self.alter_table_statements(alter, ks_context));
        }

        for index in &diff.create_indexes {
            up.push(self.create_index_statement(index, ks_context));
            down.push(self.drop_index_statement(&index.name, ks_context));
        }

        for name in &diff.drop_indexes {
            up.push(self.drop_index_statement(name, ks_context));
        }

        for name in &diff.drop_tables {
```

Add index helper methods:

```rust
    fn create_index_statement(&self, index: &CqlIndexDiff, keyspace_context: Option<&str>) -> String {
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
        format!("DROP INDEX IF EXISTS {};", self.qualify(name, keyspace_context))
    }
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p prax-migrate --lib cql::generator`
Expected: 23 tests pass.

- [ ] **Step 4: Commit**

```bash
git add prax-migrate/src/cql/generator.rs
git commit -m "feat(migrate): generate CQL secondary index statements

Support standard secondary indexes, SASI prefixed indexes, and custom
indexes via CREATE CUSTOM INDEX with a class path."
```

---

### Task 11: Materialized views

**Files:**
- Modify: `prax-migrate/src/cql/generator.rs`

- [ ] **Step 1: Write failing tests**

Add to the test module:

```rust
    use crate::cql::diff::MaterializedViewDiff;

    #[test]
    fn test_create_materialized_view() {
        let generator = CqlMigrationGenerator::new();
        let mut diff = CqlSchemaDiff::default();
        diff.create_materialized_views.push(MaterializedViewDiff {
            name: "events_by_status".into(),
            base_table: "events".into(),
            select_columns: vec!["*".into()],
            where_clause: "status IS NOT NULL AND tenant_id IS NOT NULL AND event_time IS NOT NULL".into(),
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
        assert!(migration.up.contains("CREATE MATERIALIZED VIEW \"events_by_status\""));
        assert!(migration.up.contains("FROM \"events\""));
        assert!(migration.up.contains("WHERE status IS NOT NULL"));
        assert!(migration.up.contains("PRIMARY KEY ((status), tenant_id, event_time)"));
        assert!(migration.up.contains("CLUSTERING ORDER BY (tenant_id ASC, event_time DESC)"));
        assert!(migration.down.contains("DROP MATERIALIZED VIEW IF EXISTS \"events_by_status\""));
    }

    #[test]
    fn test_drop_materialized_view() {
        let generator = CqlMigrationGenerator::new();
        let mut diff = CqlSchemaDiff::default();
        diff.drop_materialized_views.push("old_view".into());

        let migration = generator.generate(&diff);
        assert!(migration.up.contains("DROP MATERIALIZED VIEW IF EXISTS \"old_view\""));
    }
```

- [ ] **Step 2: Implement materialized view generation**

In `prax-migrate/src/cql/generator.rs`, update imports:

```rust
use crate::cql::diff::{
    ClusteringKey, ClusteringOrder, CompactionStrategy, CqlFieldAlterDiff, CqlFieldDiff,
    CqlIndexDiff, CqlIndexType, CqlSchemaDiff, CqlTableAlterDiff, CqlTableDiff, KeyspaceConfig,
    MaterializedViewDiff, ReplicationStrategy, UdtAlterDiff, UdtDiff,
};
```

Update the `generate` method - insert MV handling after indexes and before drop_tables:

```rust
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
```

Add MV helper methods:

```rust
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
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p prax-migrate --lib cql::generator`
Expected: 25 tests pass.

- [ ] **Step 4: Commit**

```bash
git add prax-migrate/src/cql/generator.rs
git commit -m "feat(migrate): generate CQL materialized view statements

CREATE MATERIALIZED VIEW with SELECT/FROM/WHERE/PRIMARY KEY and optional
CLUSTERING ORDER BY. DROP MATERIALIZED VIEW for teardown and explicit
drops."
```

---

### Task 12: Verify all unit tests pass and add snapshot test for SQL dialect

**Files:**
- Modify: `prax-migrate/src/dialect.rs`

- [ ] **Step 1: Add snapshot test ensuring SQL dialect output is unchanged**

Add to the `#[cfg(test)] mod tests` block in `prax-migrate/src/dialect.rs`:

```rust
    #[test]
    fn test_sql_dialect_matches_postgres_generator_directly() {
        use crate::diff::{ModelDiff, FieldDiff};

        let mut diff = SchemaDiff::default();
        diff.create_models.push(ModelDiff {
            name: "User".to_string(),
            table_name: "users".to_string(),
            fields: vec![FieldDiff {
                name: "id".to_string(),
                column_name: "id".to_string(),
                sql_type: "BIGINT".to_string(),
                nullable: false,
                default: None,
                is_primary_key: true,
                is_auto_increment: true,
                is_unique: false,
            }],
            primary_key: vec!["id".to_string()],
            indexes: Vec::new(),
            unique_constraints: Vec::new(),
            foreign_keys: Vec::new(),
        });

        let via_trait = SqlDialect::generate(&diff);
        let via_direct = PostgresSqlGenerator.generate(&diff);

        assert_eq!(via_trait.up, via_direct.up);
        assert_eq!(via_trait.down, via_direct.down);
        assert_eq!(via_trait.warnings, via_direct.warnings);
    }
```

- [ ] **Step 2: Run all prax-migrate tests**

Run: `cargo test -p prax-migrate --lib`
Expected: All tests pass (including 25 CQL tests, 7 dialect tests, and all existing tests unchanged).

- [ ] **Step 3: Commit**

```bash
git add prax-migrate/src/dialect.rs
git commit -m "test(migrate): verify SqlDialect output matches PostgresSqlGenerator

Snapshot-style assertion proving the trait wrapper is behaviorally
identical to the direct generator call. Guards against drift."
```

---

### Task 13: Integration tests for CQL migration workflow

**Files:**
- Create: `prax-migrate/tests/cql_migration.rs`

- [ ] **Step 1: Create integration test file**

Create `prax-migrate/tests/cql_migration.rs`:

```rust
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
    let ks_pos = migration.up.find("CREATE KEYSPACE").expect("keyspace missing");
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
    assert!(migration.up.contains("CREATE TYPE \"myapp\".\"order_status\""));

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
    let drop_tbl = migration.down.find("DROP TABLE").expect("DROP TABLE missing");
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
```

- [ ] **Step 2: Run integration tests**

Run: `cargo test -p prax-migrate --test cql_migration`
Expected: 4 integration tests pass.

- [ ] **Step 3: Commit**

```bash
git add prax-migrate/tests/cql_migration.rs
git commit -m "test(migrate): add CQL migration integration tests

Cover full workflow ordering (keyspace → UDT → table → index → MV),
down script reverse ordering, trait-vs-generator equivalence, and
destructive operation warnings."
```

---

### Task 14: Runnable example for CQL migrations

**Files:**
- Create: `prax-migrate/examples/cql_migration.rs`

- [ ] **Step 1: Create the example**

Create `prax-migrate/examples/cql_migration.rs`:

```rust
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
```

- [ ] **Step 2: Run the example**

Run: `cargo run --example cql_migration -p prax-migrate`
Expected: Prints a valid CQL migration with keyspace, UDT, table, index, and materialized view.

- [ ] **Step 3: Commit**

```bash
git add prax-migrate/examples/cql_migration.rs
git commit -m "docs(migrate): add CQL migration example

Runnable example that generates a full CQL migration with keyspace
(NetworkTopology), UDT, table (compound clustering keys + time-window
compaction + TTL), secondary index, and materialized view."
```

---

### Task 15: Update module documentation

**Files:**
- Modify: `prax-migrate/src/lib.rs`

- [ ] **Step 1: Update module doc comment**

In `prax-migrate/src/lib.rs`, find the module doc block that starts with `//! # prax-migrate` and update the bullet list. Locate the line:

```rust
//! - SQL migration generation for PostgreSQL, MySQL, SQLite, MSSQL, and DuckDB
```

Replace it with:

```rust
//! - SQL migration generation for PostgreSQL, MySQL, SQLite, MSSQL, and DuckDB
//! - CQL migration generation for ScyllaDB (via MigrationDialect trait)
```

- [ ] **Step 2: Run tests to make sure doc still compiles**

Run: `cargo test -p prax-migrate --lib`
Expected: All tests still pass.

- [ ] **Step 3: Commit**

```bash
git add prax-migrate/src/lib.rs
git commit -m "docs(migrate): document CQL/ScyllaDB support in module docs"
```

---

### Task 16: Final verification and workspace version bump

**Files:**
- Modify: `Cargo.toml` (workspace root)
- Modify: `prax-cli/tests/cli_tests.rs`

- [ ] **Step 1: Run full test suite**

Run: `cargo test -p prax-migrate`
Expected: All unit tests + integration tests pass.

- [ ] **Step 2: Run workspace check**

Run: `cargo check --workspace`
Expected: Clean build, no errors.

- [ ] **Step 3: Format code**

Run: `cargo fmt --all`

- [ ] **Step 4: Bump workspace version to 0.7.2**

In `Cargo.toml` (workspace root), change:

```toml
[workspace.package]
version = "0.7.1"
```

to:

```toml
[workspace.package]
version = "0.7.2"
```

And replace `version = "0.7.1"` with `version = "0.7.2"` in every `[workspace.dependencies]` internal crate entry (prax-schema through prax-pgvector).

- [ ] **Step 5: Update CLI test version assertions**

Run: `sed -i 's/0\.7\.1/0.7.2/g' prax-cli/tests/cli_tests.rs`

Verify the file now asserts `0.7.2`:

Run: `grep "0.7" prax-cli/tests/cli_tests.rs`
Expected: Two matches, both showing `0.7.2`.

- [ ] **Step 6: Update lockfile**

Run: `cargo update -p prax-migrate`

- [ ] **Step 7: Build workspace**

Run: `cargo build --workspace`
Expected: Clean build.

- [ ] **Step 8: Run CLI tests**

Run: `cargo test -p prax-orm-cli --test cli_tests`
Expected: All 17 tests pass.

- [ ] **Step 9: Commit version bump**

```bash
git add Cargo.toml Cargo.lock prax-cli/tests/cli_tests.rs
git commit -m "chore(release): bump version to 0.7.2 for ScyllaDB support

Adds ScyllaDB/CQL migration support to prax-migrate via the new
MigrationDialect trait:
- CqlDialect alongside SqlDialect
- CqlSchemaDiff with partition/clustering keys, UDTs, keyspaces,
  materialized views, and compaction/TTL settings
- CqlMigrationGenerator with full CQL statement generation
- Keyspace-aware qualified naming
- Loud warnings for unsupported in-place changes (partition key,
  clustering key, UDT field drop)

Existing SQL migration path is unchanged: SqlDialect wraps the
existing PostgresSqlGenerator behavior."
```

---

## Self-Review

**Spec coverage:**
- MigrationDialect trait ✓ (Task 1)
- SqlDialect impl ✓ (Task 1)
- CqlDialect impl ✓ (Task 5)
- Event log table name per dialect ✓ (Tasks 1, 5)
- CqlSchemaDiff with all spec types ✓ (Task 3)
- MigrationCql ✓ (Task 2)
- CqlMigrationGenerator ✓ (Task 4 skeleton, Tasks 6–11 flesh out)
- Keyspace creation (Simple + NetworkTopology) ✓ (Task 6)
- UDT (create, drop, alter add/rename) ✓ (Task 7)
- Table creation with partition/clustering/compaction/TTL ✓ (Task 8)
- ALTER TABLE with warnings ✓ (Task 9)
- Partition/clustering key change warnings ✓ (Task 9)
- Secondary index (Secondary, SASI, Custom) ✓ (Task 10)
- Materialized views ✓ (Task 11)
- Down-script reverse ordering ✓ (Task 6 introduces, Task 13 verifies)
- Data-loss warnings ✓ (Tasks 6–11)
- Backward-compatible SQL output (snapshot test) ✓ (Task 12)
- Integration tests ✓ (Task 13)
- Runnable example ✓ (Task 14)
- Module docs updated ✓ (Task 15)
- Version bump ✓ (Task 16)

**Gaps from spec:**
- Event sourcing integration: spec mentions `MigrationEventStore<D>` becoming generic and a `_prax_cql_migrations` table schema. The plan scopes this to the trait definition (event_log_table() is exposed on MigrationDialect) but does NOT refactor the existing MigrationEventStore trait to be generic, nor does it introduce a ScyllaDB-backed event store implementation. Reason: the event store refactor is a cross-cutting change affecting the engine and existing tests; keeping it out of this PR minimizes breakage. A future PR can generalize MigrationEventStore<D> and add a ScyllaEventStore implementation. The plan correctly positions the trait method `event_log_table()` to be consumed by that future work.

**Placeholders:** None. All code steps include full code blocks. All commit messages are concrete.

**Type consistency:** Verified — field names and method signatures across tasks match. ClusteringKey { name, order }, CqlFieldDiff { name, cql_type, is_static }, qualify() helper signature, etc. are all consistent.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-04-26-scylladb-migrations.md`.

Two execution options:

**1. Subagent-Driven (recommended)** - I dispatch a fresh subagent per task, review between tasks, fast iteration

**2. Inline Execution** - Execute tasks in this session using executing-plans, batch execution with checkpoints

Which approach?
