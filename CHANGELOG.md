# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed (BREAKING)

- **`prax-query::traits::QueryEngine`**: row-returning methods now require
  `T: FromRow`. Add `#[derive(Model)]` (which emits `FromRow`) or a
  hand-written `impl FromRow for MyModel`.
- **`prax-query::traits::QueryEngine`**: new `dialect()` method on the
  trait. Has a default returning the inert `NotSql` dialect, so existing
  implementors continue to compile — but every SQL-backed engine must
  override it or SQL building will panic at runtime.
- **`prax-query::filter::Filter::to_sql`**: signature gained a
  `dialect: &dyn SqlDialect` parameter. Callers must pass their engine's
  dialect (or a literal `&prax_query::dialect::Postgres` if wedded to
  that backend).
- **`prax-query::filter::Filter::to_sql`**: column names are now quoted
  through `dialect.quote_ident` before being interpolated into SQL.
  Generated SQL now reads `"col" = $1` on Postgres (was `col = $1`),
  `` `col` = ? `` on MySQL, `[col] = @P1` on MSSQL. Tests that matched
  the unquoted form need updating.
- **`prax-mysql::MysqlEngine` / `prax-sqlite::SqliteEngine`**: rewritten
  to return typed rows (`T: FromRow`) instead of JSON blobs. The legacy
  JSON API moved to `prax_mysql::raw::MysqlRawEngine` +
  `prax_mysql::raw::MysqlJsonRow` (and the equivalent for SQLite).
  Callers that wanted JSON: `use prax_{mysql,sqlite}::raw::{MysqlRawEngine, MysqlJsonRow}`.
- **`prax-mysql::MysqlEngine` inherent methods removed**: the old
  inherent methods `query(sql, params) -> Vec<RowData>`,
  `query_one(sql, params) -> RowData`, `query_opt(sql, params) -> Option<RowData>`
  no longer exist. They are replaced by the `QueryEngine` trait methods
  `query_many::<T>`, `query_one::<T>`, `query_optional::<T>`, each of
  which requires `T: Model + FromRow`. Callers consuming raw `RowData`
  / `serde_json::Value` must either migrate to a typed model via
  `#[derive(Model)]`, bridge through `prax_mysql::row_ref::MysqlRowRef`
  in a hand-written `FromRow`, or switch to `prax_mysql::raw::MysqlRawEngine`
  for the legacy JSON API. Side-effecting SQL that returns no rows
  should call `QueryEngine::execute_raw`.
- **`prax-sqlite::SqliteEngine` inherent methods removed**: same
  breakage as `MysqlEngine`. The old `query` / `query_one` / `query_opt`
  are gone; use `query_many::<T>` / `query_one::<T>` /
  `query_optional::<T>` with `T: Model + FromRow`, bridge via
  `prax_sqlite::row_ref::SqliteRowRef::from_rusqlite` for ad-hoc typed
  rows, or fall back to `prax_sqlite::raw::SqliteRawEngine` for the
  JSON API.
- **`prax-mysql::MysqlQueryResult` / `prax-sqlite::SqliteQueryResult`**:
  types removed from public re-exports. Renamed to
  `prax_{mysql,sqlite}::raw::{MysqlJsonRow, SqliteJsonRow}`.
- **`#[derive(Model)]` now emits `FromRow` in addition to `Model`**:
  the derive expands to *both* `impl prax_query::traits::Model for …`
  and `impl prax_query::row::FromRow for …`. If you had a hand-written
  `impl Model for …` or `impl FromRow for …` for a type that also
  carries the derive, the two impls will conflict (`E0119`). Delete
  the hand-written impl and rely on the derive, or drop the derive
  and keep the hand-written impls.
- **`#[derive(Model)]` now emits a lowercase-struct module**: alongside
  the per-field filter constructors, the derive emits
  `mod <lowercase_struct_name> { pub mod <field> { fn equals, gt, lt, … } }`.
  Crates that already define a module named the same as the lowercase
  form of a derived struct (e.g., a struct `User` plus a local
  `mod user { … }`) will see an `E0428` duplicate-definition error.
  Rename one of them.
- **`FilterValue::from::<u64>`**: values greater than `i64::MAX` now
  panic instead of silently clamping. Callers that pass untrusted
  `u64` inputs must validate the range before conversion, or switch
  to `FilterValue::Int(value as i64)` with their own clamp policy.

### Added

- **`prax-query::dialect`**: new module with `SqlDialect` trait and
  `Postgres`/`Sqlite`/`Mysql`/`Mssql`/`NotSql` implementations. Attached
  to `QueryEngine::dialect()`.
- **`prax-query::row::FromRow` + `RowRef`**: expanded with default-
  erroring getters for `chrono::DateTime<Utc>`, `chrono::NaiveDateTime`,
  `chrono::NaiveDate`, `chrono::NaiveTime`, `uuid::Uuid`,
  `rust_decimal::Decimal`, `serde_json::Value` and their `Option<T>`
  variants. Drivers override the ones they support natively.
- **`prax-query::row::into_row_error`**: helper for driver `RowRef`
  bridges that maps any `Display` error into a `RowError::TypeConversion`.
- **`prax-{postgres,sqlite,mysql,mssql}` row_ref modules**: typed row
  bridges (`PgRow`, `SqliteRowRef`, `MysqlRowRef`, `MssqlRowRef`).
- **`prax-{postgres,sqlite,mysql,mssql}::*Engine`**: implement
  `QueryEngine` trait with typed row decoding via `FromRow`.
- **`#[derive(Model)]`**: emits `impl prax_query::traits::Model` and
  `impl prax_query::row::FromRow` alongside the legacy `PraxModel`
  marker. Also emits per-field filter operator constructors
  (`user::age::gt(18)`, etc.) that classify field types into
  Numeric / String / Boolean / Other buckets.
- **`FilterValue`**: `From` impls for signed and unsigned integer
  widths, `f32`, `chrono::DateTime<Utc>`, `chrono::NaiveDateTime`,
  `chrono::NaiveDate`, `chrono::NaiveTime`, `uuid::Uuid`,
  `rust_decimal::Decimal`, `serde_json::Value`.

### Migration Guide

If you implement `QueryEngine` for a custom SQL backend:
1. Add `fn dialect(&self) -> &dyn SqlDialect { &prax_query::dialect::Postgres }` (or the dialect you target).
2. Ensure every type passed to `query_many::<T>`, `query_one::<T>`, etc. implements `FromRow`. Use `#[derive(Model)]`.

If you use `prax-mysql` or `prax-sqlite`:
- For typed rows (new default): no change — your `find_many::<User>()` etc. now return typed models.
- For JSON blobs (legacy): import `MysqlRawEngine` / `SqliteRawEngine` from the `raw` module.

If you call `Filter::to_sql` directly:
- Update to `filter.to_sql(offset, &prax_query::dialect::Postgres)` (or your dialect).

If you called `MysqlEngine`/`SqliteEngine` inherent methods directly:

```rust
// BEFORE (0.6)
let rows: Vec<RowData> = engine.query("SELECT * FROM users", vec![]).await?;

// AFTER (0.7) — with #[derive(Model)]
#[derive(prax_orm::Model)]
#[prax(table = "users")]
struct User {
    #[prax(id)]
    id: i32,
    email: String,
}

let rows: Vec<User> = engine
    .query_many::<User>("SELECT id, email FROM users", vec![])
    .await?;

// AFTER (0.7) — ad-hoc typed row without the Model derive
use prax_mysql::row_ref::MysqlRowRef;
use prax_query::row::{FromRow, RowError, RowRef};
use prax_query::traits::Model;

struct UserSummary { id: i32, email: String }

impl Model for UserSummary {
    const MODEL_NAME: &'static str = "UserSummary";
    const TABLE_NAME: &'static str = "users";
    // … fill in the remaining associated items per the trait …
}

impl FromRow for UserSummary {
    fn from_row(row: &dyn RowRef) -> Result<Self, RowError> {
        Ok(Self {
            id: row.get_i32("id")?,
            email: row.get_string("email")?,
        })
    }
}

let rows: Vec<UserSummary> = engine
    .query_many::<UserSummary>("SELECT id, email FROM users", vec![])
    .await?;
```

The SQLite bridge is identical apart from the row-ref import:
`use prax_sqlite::row_ref::SqliteRowRef;` and, inside a raw-row
callback, build the ref via `SqliteRowRef::from_rusqlite(&row)`.

If you need the old untyped JSON-blob behavior, switch to
`prax_mysql::raw::MysqlRawEngine` / `prax_sqlite::raw::SqliteRawEngine`;
those retain the legacy API.

`QueryEngine::query_one` behavior when the SQL returns 2+ rows is driver-dependent: Postgres errors (strict), while MySQL/SQLite/MSSQL silently return the first row. Callers that require "exactly one row or error" should add `LIMIT 2` (or `TOP 2` on MSSQL) and check the row count themselves, or use `count`/`query_many` + assert `len() == 1`.

- **TypeScript Generator** (`prax-typegen` v0.1.0) - Standalone crate for generating TypeScript from Prax schemas
  - TypeScript interface generation for models, enums, composite types, and views
  - Zod schema generation with runtime validation and inferred types
  - `CreateInput` and `UpdateInput` variants for each model
  - Lazy `z.lazy()` references for relation fields
  - CLI binary installable via `cargo install prax-typegen`
- **Schema Generator Blocks** (`prax-schema`) - First-class `generator` block support in `.prax` files
  - `generate = env("VAR")` toggle: enable/disable generators via environment variables
  - `generate = true/false` literal toggle
  - Parsed into `Generator` AST with `provider`, `output`, `generate`, and arbitrary properties
  - `Schema::enabled_generators()` for runtime filtering

## [0.6.0] - 2026-02-13

### Added

- **pgvector Support** (`prax-pgvector`) - New crate for vector similarity search
  - Dense vector embeddings via `Embedding` type wrapping `pgvector::Vector`
  - Sparse vector support via `SparseEmbedding` wrapping `pgvector::SparseVector`
  - Binary vector support via `BinaryVector` wrapping `pgvector::Bit`
  - Half-precision vectors via `HalfEmbedding` (feature-gated `halfvec`)
  - Distance metrics: L2, inner product, cosine, L1, Hamming, Jaccard
  - IVFFlat and HNSW index management with tuning parameters
  - Fluent `VectorSearchBuilder` for nearest-neighbor queries
  - `HybridSearchBuilder` for combined vector + full-text search (RRF scoring)
  - Vector filter integration for prax-query WHERE/ORDER BY clauses
  - Extension management SQL helpers (CREATE/DROP/CHECK pgvector)
  - Client-side vector math: L2 norm, normalization, dot product, cosine similarity
  - 99 unit tests + 10 doc tests + 36 integration tests

## [0.5.0] - 2026-01-07

### Added

- **Schema Import from Prisma, Diesel, and SeaORM** (`prax-import`)
  - Parse Prisma schema files (`.prisma`) and convert to Prax
  - Parse Diesel schema files (`table!` macros) and convert to Prax
  - Parse SeaORM entity files (`DeriveEntityModel`) and convert to Prax
  - Automatic type mapping between ORM schemas
  - Relation preservation and foreign key conversion
  - Model attribute conversion (@@map, @@index, @@unique)
  - Field attribute conversion (@id, @unique, @default, @relation)
  - Enum definition conversion
  - CLI integration via `prax import --from <prisma|diesel|sea-orm>`
  - Comprehensive test coverage for all import paths (13 tests)
  - Performance benchmarks with Criterion.rs

### Performance

- **Import Performance Optimization** (`prax-import`)
  - Regex compilation caching using `once_cell::sync::Lazy`
  - 42-57% faster Prisma imports (2.31x speedup on small schemas)
  - 15-45% faster Diesel imports (1.80x speedup on small schemas)
  - Throughput: ~7,675 Prisma schemas/sec, ~8,135 Diesel schemas/sec, ~7,799 SeaORM schemas/sec
  - Comprehensive benchmark suite with small/medium/large test cases

## [0.4.0] - 2025-12-28

### Added

- **ScyllaDB Support** (`prax-scylladb`)
  - High-performance Cassandra-compatible database driver
  - Built on the official `scylla` async driver
  - Connection pooling with automatic reconnection
  - Prepared statement caching
  - Lightweight Transactions (LWT) support for conditional updates
  - Batch operations (logged, unlogged, counter)
  - Full CQL type mapping to Rust types
  - URL-based configuration parsing

- **DuckDB Support** (`prax-duckdb`)
  - Analytical database driver optimized for OLAP workloads
  - In-process database with no server required
  - Parquet, CSV, JSON file reading/writing
  - Window functions, aggregations, analytical queries
  - Connection pooling with semaphore-based limiting

- **Multi-Tenancy Support** (`prax-query/src/tenant/`)
  - Zero-allocation task-local tenant context
  - PostgreSQL Row-Level Security (RLS) integration
  - LRU tenant cache with TTL and sharded cache for high concurrency
  - Per-tenant connection pools and statement caching

- **Data Caching Layer** (`prax-query/src/data_cache/`)
  - In-memory LRU cache with TTL
  - Redis distributed cache with connection pooling
  - Tiered L1 (memory) + L2 (Redis) caching
  - Pattern-based and tag-based cache invalidation

- **Async Optimizations** (`prax-query/src/async_optimize/`)
  - `ConcurrentExecutor` for parallel task execution
  - `ConcurrentIntrospector` for parallel database schema introspection
  - Bulk insert/update pipelines for batched operations

- **Memory Optimizations** (`prax-query/src/mem_optimize/`)
  - Global and scoped string interning
  - Arena allocation for query builders
  - Lazy schema parsing for on-demand introspection

- **Memory Profiling** (`prax-query/src/profiling/`)
  - Allocation tracking with size histograms
  - Memory snapshots and diff analysis
  - Leak detection with severity classification

- **New Benchmarks**
  - `async_bench`, `mem_optimize_bench`, `database_bench`
  - `throughput_bench`, `memory_profile_bench`
  - `duckdb_operations`, `scylladb_operations`

- **CI Workflows**
  - `.github/workflows/benchmarks.yml` - Regression detection
  - `.github/workflows/memory-check.yml` - Valgrind leak detection

- **Cursor Development Rules**
  - SQL safety, benchmarking, error handling, performance
  - Multi-tenancy, caching, profiling guidelines

### Changed

- Renamed project from `prax` to `prax-orm`
- Renamed CLI from `prax-cli` to `prax-orm-cli`
- Cleaned up TODO.md to concise feature reference (~200 lines)
- Updated all documentation URLs to `prax-orm`

### Fixed

- **ScyllaDB** - Resolved API compatibility issues with scylla driver v0.14
  - Fixed `Compression` enum usage (use `Option<Compression>`)
  - Fixed `ErrorCode` mapping to actual prax-query variants
  - Fixed `FilterValue` conversion for `Json` and `List` types
  - Fixed `Decimal` conversion using `mantissa()` and `scale()`
  - Added `BatchValues` trait bound for batch execution
  - Imported chrono `Datelike` and `Timelike` traits

## [0.3.3] - 2025-12-28

### Added

- **DuckDB Support** (`prax-duckdb`)
  - New analytical database driver optimized for OLAP workloads
  - In-process database with no server required
  - Parquet, CSV, JSON file reading/writing
  - Window functions, aggregations, analytical queries
  - Connection pooling with semaphore-based limiting
  - Async interface via `spawn_blocking`

- **Multi-Tenancy Support** (`prax-query/src/tenant/`)
  - Zero-allocation task-local tenant context (`task_local.rs`)
  - PostgreSQL Row-Level Security (RLS) integration (`rls.rs`)
  - LRU tenant cache with TTL and sharded cache for high concurrency (`cache.rs`)
  - Per-tenant connection pools (`pool.rs`)
  - Prepared statement caching (global and per-tenant) (`prepared.rs`)

- **Data Caching Layer** (`prax-query/src/data_cache/`)
  - In-memory LRU cache with TTL (`memory.rs`)
  - Redis distributed cache with connection pooling (`redis.rs`)
  - Tiered L1 (memory) + L2 (Redis) caching (`tiered.rs`)
  - Pattern-based and tag-based cache invalidation (`invalidation.rs`)
  - Cache metrics and hit rate tracking (`stats.rs`)

- **Async Optimizations** (`prax-query/src/async_optimize/`)
  - `ConcurrentExecutor` for parallel task execution with configurable limits
  - `ConcurrentIntrospector` for parallel database schema introspection
  - `QueryPipeline`, `BulkInsertPipeline`, `BulkUpdatePipeline` for batched operations

- **Memory Optimizations** (`prax-query/src/mem_optimize/`)
  - Global and scoped string interning (`GlobalInterner`, `ScopedInterner`)
  - Arena allocation for query builders (`QueryArena`, `ArenaScope`)
  - Lazy schema parsing for on-demand introspection (`LazySchema`, `LazyTable`)

- **Memory Profiling** (`prax-query/src/profiling/`)
  - Allocation tracking with size histograms
  - Memory snapshots and diff analysis
  - Leak detection with severity classification
  - Heap profiling integration
  - CI workflow for Valgrind and AddressSanitizer checks

- **New Benchmarks**
  - `async_bench` - Concurrent execution and pipeline performance
  - `mem_optimize_bench` - Interning, arena, lazy parsing benchmarks
  - `database_bench` - Database-specific SQL generation
  - `throughput_bench` - Queries-per-second measurements
  - `memory_profile_bench` - Memory profiling benchmarks
  - `duckdb_operations` - DuckDB analytical query benchmarks

- **CI Workflows**
  - `.github/workflows/benchmarks.yml` - Regression detection with baseline comparison
  - `.github/workflows/memory-check.yml` - Memory leak detection via Valgrind

- **Cursor Rules**
  - `sql-safety.mdc` - SQL injection prevention guidelines
  - `benchmarking.mdc` - Criterion.rs benchmarking standards
  - `error-handling.mdc` - Error handling best practices
  - `performance.mdc` - Performance optimization guidelines
  - `api-design.mdc` - API design principles
  - `multi-tenancy.mdc` - Multi-tenant application patterns
  - `caching.mdc` - Cache layer usage guidelines
  - `profiling.mdc` - Memory profiling documentation

### Changed

- Cleaned up TODO.md from 869 lines to ~150 lines (concise feature reference)
- Updated architecture to include `prax-duckdb`

## [0.3.2] - 2025-12-24

### Added

- **GraphQL Model Style Configuration** (`prax-codegen`, `prax-schema`)
  - New `model_style` option in `prax.toml`: `"standard"` (default) or `"graphql"`
  - When set to `"graphql"`, model structs generate with `#[derive(async_graphql::SimpleObject)]`
  - `CreateInput` and `UpdateInput` types generate with `#[derive(async_graphql::InputObject)]`
  - Auto-enables GraphQL plugins when `graphql` style is selected
  - Configuration example:
    ```toml
    [generator.client]
    model_style = "graphql"
    ```

## [0.3.1] - 2025-12-21

### Added

- **MySQL Execution Benchmarks** (`benches/database_execution.rs`)
  - Prax MySQL benchmarks with connection pooling
  - SQLx MySQL benchmarks for comparison
  - SELECT by ID, filtered SELECT, and COUNT operations

- **SQLite Execution Benchmarks** (`benches/database_execution.rs`)
  - Prax SQLite benchmarks with in-memory database seeding
  - SQLx SQLite benchmarks for comparison
  - Complete benchmark coverage across all three databases

### Fixed

- Resolved all clippy warnings across the codebase
- Renamed `from_str` methods to `parse` to avoid trait confusion
- Fixed `Include::add` → `Include::with` naming
- Fixed `PooledBuffer::as_mut` → `PooledBuffer::as_mut_str` naming
- Added proper allow attributes for API modules with intentionally unused code

### Changed

- Enabled sqlx `mysql` and `sqlite` features for benchmarks
- Added `prax-mysql`, `prax-sqlite`, `rusqlite` as dev-dependencies

## [0.3.0] - 2025-12-21

### Added

- **Zero-Copy Row Deserialization** (`prax-query`)
  - `RowRef` trait for borrowing string data directly from database rows
  - `FromRowRef<'a>` trait for zero-allocation struct deserialization
  - `FromRow` trait for traditional owning deserialization
  - `FromColumn` trait for type-specific column extraction
  - `RowData` enum for borrowed/owned string data (like `Cow`)
  - `impl_from_row!` macro for easy struct implementation

- **Batch & Pipeline Execution** (`prax-query`)
  - `Pipeline` and `PipelineBuilder` for grouping multiple queries
  - Execute multiple queries with minimal round-trips
  - `PipelineResult` with per-query status tracking
  - Enhanced `Batch::to_combined_sql()` for multi-row INSERT optimization

- **Query Plan Caching** (`prax-query`)
  - `ExecutionPlanCache` for caching query plans with metrics
  - `ExecutionPlan` with SQL, hints, and execution time tracking
  - `PlanHint` enum: `IndexScan`, `SeqScan`, `Parallel`, `Timeout`, etc.
  - `record_execution()` for automatic timing collection
  - `slowest_queries()` and `most_used()` for performance analysis

- **Type-Level Filter Optimizations** (`prax-query`)
  - `InI64Slice`, `InStrSlice` for zero-allocation IN filters
  - `NotInI64Slice`, `NotInStrSlice` for NOT IN filters
  - `And5` struct with `DirectSql` implementation
  - Pre-computed PostgreSQL IN patterns (`POSTGRES_IN_FROM_1`) for 1-32 elements

- **Documentation Website**
  - New "Advanced Performance" page with comprehensive examples
  - Updated Performance page with latest benchmark results
  - Added batch execution, zero-copy, and plan caching documentation

### Changed

- Optimized `write_postgres_in_pattern` for faster IN clause generation
- Updated benchmark results showing Prax matching Diesel for type-level filters
- Improved performance page with database execution benchmarks

### Performance

- Type-level `And5` filter: **~5.1ns** (matches Diesel!)
- `IN(10)` SQL generation: **~3.8ns** (5.8x faster with pre-computed patterns)
- `IN(32)` SQL generation: **~5.0ns** (uses pre-computed pattern lookup)
- Database SELECT by ID: **193µs** (30% faster than SQLx)

## [0.2.0] - 2025-12-20

### Added

- Initial project structure and configuration
- Dual MIT/Apache-2.0 licensing
- Project README with API examples and documentation
- Implementation roadmap (TODO.md)
- Git hooks via cargo-husky:
  - Pre-commit hook for formatting and linting
  - Pre-push hook for test suite validation
  - Commit-msg hook for Conventional Commits enforcement
- Contributing guidelines (CONTRIBUTING.md)
- Schema definition language (SDL) parser (`prax-schema`)
  - Custom `.prax` schema files with Prisma-like syntax
  - AST types for models, fields, relations, enums, views
  - Schema validation and semantic analysis
  - Documentation comments with validation directives (`@validate`)
  - Field metadata and visibility controls (`@hidden`, `@deprecated`, etc.)
  - GraphQL and async-graphql support with federation
- Proc-macro code generation (`prax-codegen`)
  - `#[derive(Model)]` and `prax_schema!` macros
  - Plugin system for extensible code generation
  - Built-in plugins: debug, JSON Schema, GraphQL, serde, validator
- Type-safe query builder (`prax-query`)
  - Fluent API: `findMany`, `findUnique`, `findFirst`, `create`, `update`, `delete`, `upsert`, `count`
  - Filtering system with WHERE clauses, AND/OR/NOT combinators
  - Scalar filters: equals, in, contains, startsWith, endsWith, lt, gt, etc.
  - Sorting with `orderBy`, pagination with `skip`/`take` and cursor-based
  - Aggregation queries: `count`, `sum`, `avg`, `min`, `max`, `groupBy` with `HAVING`
  - Raw SQL escape hatch with type interpolation via `Sql` builder
  - Ergonomic create API with `data!` macro and builder pattern
  - Middleware/hooks system for query interception (logging, metrics, timing, retry)
  - Connection string parsing and multi-database configuration
  - Comprehensive error types with error codes, suggestions, and colored output
  - Multi-tenant support (row-level, schema-based, database-based isolation)
- Async query engines
  - PostgreSQL via `tokio-postgres` with `deadpool-postgres` connection pool (`prax-postgres`)
  - MySQL via `mysql_async` driver (`prax-mysql`)
  - SQLite via `tokio-rusqlite` (`prax-sqlite`)
  - SQLx alternative backend with compile-time checked queries (`prax-sqlx`)
- Relation loading (eager/lazy)
  - `include` and `select` operations for related data
  - Nested writes: create/connect/disconnect/set relations
- Transaction API with async closures, savepoints, isolation levels
- Migration engine (`prax-migrate`)
  - Schema diffing and SQL generation
  - Migration history tracking
  - Database introspection (reverse engineer existing databases)
  - Shadow database support for safe migration testing
  - View migration support (CREATE/DROP/ALTER VIEW, materialized views)
  - Migration resolution system (checksum handling, skip, baseline)
- CLI tool (`prax-cli`)
  - Commands: `init`, `generate`, `migrate`, `db`, `validate`, `format`
  - User-friendly colored output and error handling
- Documentation website with Angular
- Docker setup for testing with real databases
- Benchmarking suite with Criterion
- Profiling support (CPU, memory, tracing)
- Fuzzing infrastructure

### Planned

- Framework integrations (Armature, Axum, Actix-web)
- Integration test suite expansion

---

## Release History

<!--
## [0.1.0] - YYYY-MM-DD

### Added
- Initial release
- Core query builder functionality
- PostgreSQL support via tokio-postgres

### Changed
- N/A

### Deprecated
- N/A

### Removed
- N/A

### Fixed
- N/A

### Security
- N/A
-->

[Unreleased]: https://github.com/pegasusheavy/prax-orm/compare/v0.6.0...HEAD
[0.6.0]: https://github.com/pegasusheavy/prax-orm/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/pegasusheavy/prax-orm/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/pegasusheavy/prax-orm/compare/v0.3.3...v0.4.0
[0.3.3]: https://github.com/pegasusheavy/prax-orm/compare/v0.3.2...v0.3.3
[0.3.2]: https://github.com/pegasusheavy/prax-orm/compare/v0.3.1...v0.3.2
[0.3.1]: https://github.com/pegasusheavy/prax-orm/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/pegasusheavy/prax-orm/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/pegasusheavy/prax-orm/releases/tag/v0.2.0

