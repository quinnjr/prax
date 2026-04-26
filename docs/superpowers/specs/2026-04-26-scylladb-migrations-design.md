# ScyllaDB Migration Support Design

**Date:** 2026-04-26  
**Status:** Approved  
**Type:** Architecture Refactor + Feature Addition

## Overview

Add ScyllaDB migration support to prax-migrate by introducing a `MigrationDialect` trait that abstracts the migration system over schema diff types, migration output types, and generators. The existing SQL migration path becomes one dialect (`SqlDialect`); ScyllaDB introduces a second (`CqlDialect`).

This refactor enables future migration backends without further structural changes, while giving ScyllaDB its own schema diff model (`CqlSchemaDiff`) and migration output type (`MigrationCql`) — necessary because CQL differs fundamentally from SQL in its primary-key model, user-defined types, keyspaces, and ALTER semantics.

## Goals

1. **Introduce `MigrationDialect` trait** that abstracts migration operations over any database dialect
2. **Preserve existing SQL behavior** — current consumers see minimal breaking changes (type alias for `SqlMigrationEngine`)
3. **Native CQL schema diff** via `CqlSchemaDiff` — partition keys, clustering keys, UDTs, keyspaces, materialized views
4. **Auto-create keyspaces** from schema config (replication strategy, factor)
5. **UDT-based enum mapping** — Prax enums compile to CQL user-defined types
6. **Safe ALTER generation** — generate valid CQL ALTER statements, warn loudly where drop-and-recreate is needed
7. **Event sourcing integration** — reuse event model, new `_prax_cql_migrations` event log table stored in ScyllaDB keyspace

## Non-Goals

- SQL ↔ CQL cross-dialect migration (independent schemas)
- Automatic migration of unsupported ALTER operations (e.g., partition key changes) — user must write manual migrations
- Transactional guarantees — CQL migrations are not atomic across statements (inherent CQL limitation)
- Real ScyllaDB instance in CI tests (unit/integration tests verify string generation only)

## Background

ScyllaDB is a Cassandra-compatible columnar database. It uses CQL (Cassandra Query Language), which is superficially similar to SQL but diverges in fundamental ways:

- **No foreign keys** — denormalized schemas are required
- **Primary key = partition key + clustering keys** — determines data layout and query patterns
- **Keyspaces** replace databases/schemas and carry replication configuration
- **User-Defined Types (UDTs)** replace enums and structs
- **Materialized views** are a first-class schema object (auto-maintained denormalized views)
- **Limited ALTER support** — no RENAME COLUMN in general, no type changes for incompatible types, no partition/clustering key changes
- **Compaction strategies and TTL** are per-table schema concerns
- **No transactions** — only lightweight transactions (LWT) on single partitions

These differences make SQL-centric abstractions leaky if reused for CQL. The trait-based approach keeps each dialect's schema model and generation logic native to its database.

## Architecture

### MigrationDialect Trait

```rust
pub trait MigrationDialect {
    /// The schema diff type for this dialect.
    type Diff: Default + Send + Sync;
    
    /// The migration output type for this dialect.
    type Migration: Send + Sync;
    
    /// Human-readable dialect name (e.g., "sql", "cql").
    fn name() -> &'static str;
    
    /// Generate migration from schema diff.
    fn generate(diff: &Self::Diff) -> Self::Migration;
    
    /// Event log table name for this dialect.
    fn event_log_table() -> &'static str;
}
```

Two implementations:

**SqlDialect:**
```rust
pub struct SqlDialect;

impl MigrationDialect for SqlDialect {
    type Diff = SchemaDiff;
    type Migration = MigrationSql;
    
    fn name() -> &'static str { "sql" }
    fn generate(diff: &SchemaDiff) -> MigrationSql { /* uses existing generators */ }
    fn event_log_table() -> &'static str { "_prax_migrations" }
}
```

**CqlDialect:**
```rust
pub struct CqlDialect;

impl MigrationDialect for CqlDialect {
    type Diff = CqlSchemaDiff;
    type Migration = MigrationCql;
    
    fn name() -> &'static str { "cql" }
    fn generate(diff: &CqlSchemaDiff) -> MigrationCql {
        CqlMigrationGenerator::new().generate(diff)
    }
    fn event_log_table() -> &'static str { "_prax_cql_migrations" }
}
```

### Generic Migration Engine

```rust
pub struct MigrationEngine<D: MigrationDialect> {
    config: MigrationConfig,
    history: Arc<dyn MigrationHistoryRepository>,
    event_store: Arc<dyn MigrationEventStore<D>>,
    _dialect: PhantomData<D>,
}

pub type SqlMigrationEngine = MigrationEngine<SqlDialect>;
pub type CqlMigrationEngine = MigrationEngine<CqlDialect>;
```

### New Files

- `prax-migrate/src/dialect.rs` — `MigrationDialect` trait + `SqlDialect` impl
- `prax-migrate/src/cql/mod.rs` — CQL module entry + `CqlDialect` impl
- `prax-migrate/src/cql/diff.rs` — `CqlSchemaDiff` and related types
- `prax-migrate/src/cql/generator.rs` — `CqlMigrationGenerator`
- `prax-migrate/src/cql/migration.rs` — `MigrationCql`
- `prax-migrate/tests/cql_migration.rs` — integration tests
- `prax-migrate/examples/cql_migration.rs` — runnable example

### Modified Files

- `prax-migrate/src/lib.rs` — export dialect trait, CqlDialect, CqlSchemaDiff, etc.
- `prax-migrate/src/engine.rs` — make `MigrationEngine` generic over dialect
- `prax-migrate/src/event_store.rs` — make trait generic over dialect

## Components

### CqlSchemaDiff

```rust
#[derive(Debug, Default, Clone)]
pub struct CqlSchemaDiff {
    /// Keyspace to create (generated first, runs once).
    pub create_keyspace: Option<KeyspaceConfig>,
    /// Keyspace to drop (generated last in up, or avoided entirely).
    pub drop_keyspace: Option<String>,
    
    /// User-Defined Types (created before tables that reference them).
    pub create_udts: Vec<UdtDiff>,
    pub drop_udts: Vec<String>,
    pub alter_udts: Vec<UdtAlterDiff>,
    
    /// Tables.
    pub create_tables: Vec<CqlTableDiff>,
    pub drop_tables: Vec<String>,
    pub alter_tables: Vec<CqlTableAlterDiff>,
    
    /// Materialized views (reference a base table).
    pub create_materialized_views: Vec<MaterializedViewDiff>,
    pub drop_materialized_views: Vec<String>,
    
    /// Secondary indexes.
    pub create_indexes: Vec<CqlIndexDiff>,
    pub drop_indexes: Vec<String>,
}

pub struct KeyspaceConfig {
    pub name: String,
    pub replication: ReplicationStrategy,
    pub durable_writes: bool,
}

pub enum ReplicationStrategy {
    Simple { factor: u32 },
    NetworkTopology { dc_factors: Vec<(String, u32)> },
}

pub struct UdtDiff {
    pub name: String,
    pub fields: Vec<UdtField>,
}

pub struct UdtField {
    pub name: String,
    pub cql_type: String,
}

pub struct UdtAlterDiff {
    pub name: String,
    pub add_fields: Vec<UdtField>,
    pub rename_fields: Vec<(String, String)>,
}

pub struct CqlTableDiff {
    pub name: String,
    pub fields: Vec<CqlFieldDiff>,
    pub partition_keys: Vec<String>,       // at least one required
    pub clustering_keys: Vec<ClusteringKey>,
    pub compaction: Option<CompactionStrategy>,
    pub default_ttl: Option<u32>,
}

pub struct CqlFieldDiff {
    pub name: String,
    pub cql_type: String,  // e.g., "text", "uuid", "frozen<order_status>"
    pub is_static: bool,
}

pub struct ClusteringKey {
    pub name: String,
    pub order: ClusteringOrder,
}

pub enum ClusteringOrder {
    Asc,
    Desc,
}

pub enum CompactionStrategy {
    SizeTiered,
    Leveled,
    TimeWindow { window_unit: String, window_size: u32 },
}

pub struct CqlTableAlterDiff {
    pub name: String,
    pub add_fields: Vec<CqlFieldDiff>,
    pub drop_fields: Vec<String>,
    pub alter_fields: Vec<CqlFieldAlterDiff>,
}

pub struct CqlFieldAlterDiff {
    pub name: String,
    pub old_type: Option<String>,
    pub new_type: Option<String>,
}

pub struct MaterializedViewDiff {
    pub name: String,
    pub base_table: String,
    pub select_columns: Vec<String>,
    pub where_clause: String,
    pub partition_keys: Vec<String>,
    pub clustering_keys: Vec<ClusteringKey>,
}

pub struct CqlIndexDiff {
    pub name: String,
    pub table_name: String,
    pub column: String,
    pub index_type: CqlIndexType,
}

pub enum CqlIndexType {
    Secondary,
    SasiPrefixed,  // SASI index (Cassandra-specific)
    Custom(String),
}
```

### MigrationCql

```rust
pub struct MigrationCql {
    pub up: String,
    pub down: String,
    pub warnings: Vec<String>,
}

impl MigrationCql {
    pub fn is_empty(&self) -> bool {
        self.up.trim().is_empty() && self.down.trim().is_empty()
    }
}
```

### CqlMigrationGenerator

Generates CQL statements in dependency order:

**Up script order:**
1. `CREATE KEYSPACE` (if present)
2. `CREATE TYPE` for UDTs
3. `CREATE TABLE` (with partition/clustering keys, compaction, TTL)
4. `ALTER TABLE` for existing table changes
5. `CREATE INDEX` for secondary indexes
6. `CREATE MATERIALIZED VIEW` (depends on tables)

**Down script order (reverse):**
1. `DROP MATERIALIZED VIEW`
2. `DROP INDEX`
3. `DROP TABLE`
4. `DROP TYPE`
5. `DROP KEYSPACE` (if it was created by up)

### CQL Output Examples

**Keyspace creation:**
```cql
CREATE KEYSPACE IF NOT EXISTS "myapp"
WITH replication = {'class': 'SimpleStrategy', 'replication_factor': 3}
AND durable_writes = true;
```

**NetworkTopologyStrategy:**
```cql
CREATE KEYSPACE IF NOT EXISTS "myapp"
WITH replication = {'class': 'NetworkTopologyStrategy', 'us-east': 3, 'us-west': 2}
AND durable_writes = true;
```

**UDT for enum:**
```cql
CREATE TYPE "myapp"."order_status" (
    value text
);
```

Used in table: `status frozen<"order_status">`

**Table with compound partition key + clustering:**
```cql
CREATE TABLE "myapp"."events" (
    tenant_id uuid,
    event_time timestamp,
    event_id uuid,
    payload text,
    status frozen<"order_status">,
    PRIMARY KEY ((tenant_id), event_time, event_id)
) WITH CLUSTERING ORDER BY (event_time DESC, event_id ASC)
  AND compaction = {'class': 'TimeWindowCompactionStrategy', 'compaction_window_unit': 'DAYS', 'compaction_window_size': 1}
  AND default_time_to_live = 2592000;
```

**Materialized view:**
```cql
CREATE MATERIALIZED VIEW "myapp"."events_by_status" AS
    SELECT * FROM "myapp"."events"
    WHERE status IS NOT NULL AND tenant_id IS NOT NULL AND event_time IS NOT NULL AND event_id IS NOT NULL
    PRIMARY KEY ((status), tenant_id, event_time, event_id)
    WITH CLUSTERING ORDER BY (tenant_id ASC, event_time DESC, event_id ASC);
```

**Secondary index:**
```cql
CREATE INDEX IF NOT EXISTS "events_status_idx" ON "myapp"."events" (status);
```

## ALTER Semantics

Per design decisions (Q4 → option A):

| Operation | Support | Behavior |
|-----------|---------|----------|
| `ALTER TABLE ADD column` | ✅ | Safe, no warning |
| `ALTER TABLE DROP column` | ✅ | Generates statement, warns: data lost |
| `ALTER TABLE ALTER column TYPE` | ⚠️ | Only for compatible types (text↔varchar, int→bigint); warn otherwise |
| `ALTER TABLE RENAME column` | ⚠️ | Only for partition/clustering keys in CQL; warn for regular columns |
| Partition key change | ❌ | Not generated; requires manual drop/recreate migration |
| Clustering key change | ❌ | Not generated; requires manual drop/recreate migration |
| UDT add field | ✅ | Safe, no warning |
| UDT rename field | ✅ | CQL supports this for UDTs |
| UDT drop field | ❌ | Cassandra doesn't support dropping UDT fields; warn |
| Materialized view alter | ❌ | CQL requires DROP + CREATE; generator produces both with warning |

## Data-Loss Warnings

```
Dropping keyspace '<name>' — ALL data in ALL tables in the keyspace will be permanently lost
Dropping table '<keyspace>.<name>' — all rows will be lost
Dropping column '<name>' from '<keyspace>.<table>' — data in this column will be lost
Dropping UDT '<name>' — requires no referencing tables; ensure safety manually
Dropping materialized view '<name>' — the view data will be lost, but the base table is unaffected
Partition key change detected for '<table>' — not supported in-place; requires manual DROP/CREATE migration
Clustering key change detected for '<table>' — not supported in-place; requires manual DROP/CREATE migration
Type change from '<old>' to '<new>' for column '<name>' — CQL may reject this if data is incompatible
CQL migrations are not atomic across statements; partial failures require manual remediation
```

## Event Sourcing Integration

### Event Log Table

Created automatically on engine initialization in the user's keyspace:

```cql
CREATE TABLE IF NOT EXISTS "<user_keyspace>"."_prax_cql_migrations" (
    migration_id text,
    event_time timeuuid,
    event_type text,
    event_data text,
    created_at timestamp,
    PRIMARY KEY (migration_id, event_time)
) WITH CLUSTERING ORDER BY (event_time DESC);
```

- **Partition by `migration_id`** — all events for a migration are colocated for efficient replay
- **Clustering by `event_time`** (TIMEUUID) — events ordered chronologically per migration
- **`event_data` stored as JSON text** — CQL doesn't have JSONB; text is fine for event payloads

### Event Model

Reuses the existing `MigrationEvent` structure (event types: `Applied`, `RolledBack`, `Failed`, `Resolved`). Serialization to CQL uses JSON for `event_data`.

### MigrationEventStore trait

Becomes generic over dialect:

```rust
#[async_trait]
pub trait MigrationEventStore<D: MigrationDialect>: Send + Sync {
    async fn append_event(&self, event: MigrationEvent) -> Result<(), MigrationError>;
    async fn get_events(&self, migration_id: &str) -> Result<Vec<MigrationEvent>, MigrationError>;
    async fn get_all_events(&self) -> Result<Vec<MigrationEvent>, MigrationError>;
    async fn initialize(&self) -> Result<(), MigrationError>;
    async fn is_initialized(&self) -> Result<bool, MigrationError>;
    async fn get_latest_event(&self, migration_id: &str) -> Result<Option<MigrationEvent>, MigrationError>;
    async fn count_events(&self) -> Result<usize, MigrationError>;
}
```

An `InMemoryEventStore<D>` is provided for testing any dialect. Production usage expects dialect-specific implementations backed by the target database.

## Migration Path for Existing Users

**Breaking changes:**
- `MigrationEngine` becomes `MigrationEngine<SqlDialect>`. Type alias `SqlMigrationEngine` provided.
- `MigrationEventStore` trait becomes `MigrationEventStore<SqlDialect>`. Type alias `SqlMigrationEventStore` provided.

**Non-breaking:**
- `SchemaDiff`, `MigrationSql`, all SQL generators (PostgresSqlGenerator etc.) unchanged.
- Existing migration files and event log tables continue to work.

**Upgrade steps:**
1. Update any `MigrationEngine::new(...)` to use `SqlMigrationEngine::new(...)` — or rely on type inference.
2. Update any explicit `Arc<dyn MigrationEventStore>` to `Arc<dyn SqlMigrationEventStore>`.

## Testing Strategy

### Unit Tests

Location: `prax-migrate/src/cql/generator.rs` (inline)

**Coverage:**
- Keyspace creation (SimpleStrategy and NetworkTopologyStrategy)
- UDT creation, alter (add field, rename field), drop warnings
- Table creation with all partition/clustering/compaction/TTL combinations
- ALTER TABLE add/drop column with warnings
- Type change warnings
- Materialized view creation and drop-recreate for alter
- Secondary index creation
- Down-script ordering (reverse of up)

### Integration Tests

Location: `prax-migrate/tests/cql_migration.rs`

**Coverage:**
- Full workflow: diff → generate → verify output
- Keyspace + UDT + table + index + materialized view in correct order
- Drop operations produce correct warnings
- Partition key change produces no ALTER + loud warning

### Property-Based Tests

- Random `CqlSchemaDiff` → generator output respects dependency order
- Any diff with drops → at least one warning
- Down script drops in reverse order of up creates

### Trait Refactor Verification

- Snapshot tests: existing SQL generator output is unchanged after introducing `SqlDialect`
- Compile-time check: `MigrationEngine<CqlDialect>` compiles and accepts `CqlSchemaDiff`
- `event_log_table()` returns distinct strings per dialect

## Example Output

**Input CqlSchemaDiff:**
```rust
CqlSchemaDiff {
    create_keyspace: Some(KeyspaceConfig {
        name: "myapp".into(),
        replication: ReplicationStrategy::Simple { factor: 3 },
        durable_writes: true,
    }),
    create_udts: vec![UdtDiff {
        name: "order_status".into(),
        fields: vec![UdtField { name: "value".into(), cql_type: "text".into() }],
    }],
    create_tables: vec![CqlTableDiff {
        name: "events".into(),
        fields: vec![
            CqlFieldDiff { name: "tenant_id".into(), cql_type: "uuid".into(), is_static: false },
            CqlFieldDiff { name: "event_time".into(), cql_type: "timestamp".into(), is_static: false },
            CqlFieldDiff { name: "payload".into(), cql_type: "text".into(), is_static: false },
        ],
        partition_keys: vec!["tenant_id".into()],
        clustering_keys: vec![ClusteringKey {
            name: "event_time".into(),
            order: ClusteringOrder::Desc,
        }],
        compaction: None,
        default_ttl: None,
    }],
    ..Default::default()
}
```

**Generated up.cql:**
```cql
CREATE KEYSPACE IF NOT EXISTS "myapp"
WITH replication = {'class': 'SimpleStrategy', 'replication_factor': 3}
AND durable_writes = true;

CREATE TYPE "myapp"."order_status" (
    value text
);

CREATE TABLE "myapp"."events" (
    tenant_id uuid,
    event_time timestamp,
    payload text,
    PRIMARY KEY ((tenant_id), event_time)
) WITH CLUSTERING ORDER BY (event_time DESC);
```

**Generated down.cql:**
```cql
DROP TABLE IF EXISTS "myapp"."events";

DROP TYPE IF EXISTS "myapp"."order_status";

DROP KEYSPACE IF EXISTS "myapp";
```

**Warnings:**
```
(none for pure creation)
```

## Success Criteria

**Functional:**
- `MigrationDialect` trait abstracts migration over SQL and CQL dialects
- `CqlMigrationGenerator` produces valid CQL for all supported operations
- Keyspaces, UDTs, tables, materialized views, indexes all generate correctly
- Data-loss warnings emit for all destructive operations
- ALTER warnings for unsupported operations (partition/clustering key changes)

**Backward compatibility:**
- All existing `prax-migrate` tests still pass (SQL generation unchanged)
- Existing consumers compile with at most a type alias substitution

**Testing:**
- Unit tests for CQL generator
- Integration tests for full CQL migration workflow
- Snapshot tests verify SQL output identical to pre-refactor
- Property tests verify invariants

**Documentation:**
- CqlMigrationGenerator has doc comments
- Example in `examples/cql_migration.rs`
- Module doc updated in `lib.rs`

## Future Work

- Live ScyllaDB integration tests (behind feature flag)
- SASI index creation (currently returns plain secondary index)
- User-defined functions (UDFs) and aggregates (UDAs)
- Automatic partition key change via shadow table + data copy (complex, manual today)
- CQL-backed `CqlEventStore` implementation using scylla driver
