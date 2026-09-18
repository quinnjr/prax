# AGENTS.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

**Prax ORM** — a type-safe, async-first, Prisma-inspired ORM for Rust. Organized as a Cargo workspace of ~20 focused sub-crates that publish together to crates.io under a single workspace version (currently 0.11.0).

The top-level crate is `prax-orm` (`src/lib.rs` + root `Cargo.toml`); it re-exports the ecosystem. Sub-crates live at `prax-<name>/` and are all members of the workspace.

## Workspace layout

- **Core**: `prax-schema` (schema DSL parser + AST), `prax-query` (query builder), `prax-codegen` (proc-macros), `prax-migrate` (migration engine)
- **Database engines**: `prax-postgres`, `prax-mysql`, `prax-sqlite`, `prax-mssql`, `prax-mongodb`, `prax-duckdb`, `prax-scylladb`, `prax-cassandra`, `prax-sqlx`, `prax-pgvector`
- **Integration**: `prax-armature`, `prax-axum`, `prax-actix`, `prax-import` (Prisma/Diesel/SeaORM importers), `prax-typegen`
- **Tooling**: `prax-cli` (installs as `prax` binary; published as `prax-orm-cli`)

All internal crates use `version.workspace = true` and are referenced via `workspace = true` in their own `Cargo.toml` — the single source of truth is `[workspace.package].version` and `[workspace.dependencies]` in the root `Cargo.toml`.

## Architecture notes that span multiple files

- **Migration system uses event sourcing** (see `prax-migrate/src/event.rs`, `event_store.rs`, `state.rs`). All operations are appended as immutable events (`Applied`, `RolledBack`, `Failed`, `Resolved`) to a `_prax_migrations` table; current state is derived by replay. Rollbacks produce a new `RolledBack` event rather than mutating prior records.
- **Migration dialect abstraction** — `prax-migrate/src/dialect.rs` defines `MigrationDialect` with `SqlDialect` and `CqlDialect` impls. `SqlDialect` routes to the SQL generators (Postgres/MySQL/SQLite/MSSQL/DuckDB share one generator family in `prax-migrate/src/sql.rs`). `CqlDialect` routes to `prax-migrate/src/cql/generator.rs` for ScyllaDB/Cassandra (both share the same CQL dialect). Event-log table differs per dialect (`_prax_migrations` vs `_prax_cql_migrations`).
- **SQL generators are vendor-specific per dialect** but structurally similar — `PostgresSqlGenerator`, `MySqlGenerator`, `SqliteGenerator`, `MssqlGenerator`, `DuckDbSqlGenerator` all live in `prax-migrate/src/sql.rs` and share the same `SchemaDiff` → `MigrationSql` shape. Avoid cross-generator refactors without a plan — duplication between them is deliberate since each dialect has subtle differences.
- **Schema diff is SQL-centric**; CQL has its own `CqlSchemaDiff` with partition/clustering keys, UDTs, keyspaces, materialized views.
- **prax-cassandra and prax-scylladb are separate crates** despite sharing the CQL protocol — they use different Rust drivers (`cdrs-tokio` vs `scylla`) and expose different types. Don't try to unify them.

## Common commands

Build, lint, test:

```bash
cargo build --workspace
cargo test --workspace
cargo test -p prax-migrate              # single crate
cargo test -p prax-migrate --test duckdb_migration    # single test file
cargo test -p prax-migrate test_duckdb_generate_list_type  # single test
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all
```

Run a single doc example or binary example:

```bash
cargo run --example cql_migration -p prax-migrate
cargo run --example duckdb_migration -p prax-migrate
```

Integration tests gated behind features (don't run in default `cargo test`):

```bash
cargo test -p prax-cassandra --features cassandra-live    # requires live Cassandra at 127.0.0.1:9042
```

CLI:

```bash
cargo run --bin prax -- --help
cargo test -p prax-orm-cli --test cli_tests    # CLI integration tests
```

## Git hooks are installed and enforced

Hooks live in `.cargo-husky/hooks/`. Referenced in root `Cargo.toml` under `[package.metadata.husky.hooks]`, installed automatically via the `cargo-husky` dev-dependency on first build.

- **pre-commit**: runs `cargo fmt --all -- --check` and `cargo clippy --all-targets --all-features -- -D warnings`
- **commit-msg**: enforces `<type>(<scope>): <description>` with **scope REQUIRED**. Valid types: `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`, `build`, `ci`, `chore`, `revert`. Valid scopes include crate names (`query`, `postgres`, `mysql`, `sqlite`, `mssql`, `mongodb`, `duckdb`, `scylladb`, `cassandra`, `schema`, `codegen`, `migrate`, `cli`, `sqlx`, `armature`, `axum`, `actix`, `import`, `pgvector`, `typegen`) and special scopes (`deps`, `ci`, `docs`, `release`, `security`, `repo`, `workspace`).
- **pre-push**: runs the full test suite

**Never bypass hooks** with `--no-verify`, `-n`, `HUSKY=0`, or any equivalent. If a hook fails, fix the underlying issue — see `.cursor/rules/no-skip-hooks.mdc` for the specific remedies.

## Branching model (git-flow)

- **`main`** — production/release; never direct-commit; merged from `release/*` or `hotfix/*`.
- **`develop`** — integration branch; base for all `feature/*` and `bugfix/*`.
- **`feature/<scope>-<description>`**, **`bugfix/...`**, **`release/<semver>`**, **`hotfix/...`**.
- Feature work uses worktrees under `.worktrees/` (gitignored). Use `git worktree add .worktrees/<branch> -b feature/<branch>` rather than switching branches in the main checkout.

PRs target `develop`; `main` receives only release/hotfix merges. `feature/*` → `develop` uses squash merges; `release/*`/`hotfix/*` → `main` use merge commits.

## Release and publish

Scripts live in `scripts/`:

- **`scripts/release.sh <version> [--no-push]`** — bumps workspace version in `Cargo.toml` (workspace.package and all internal workspace.dependencies), updates CHANGELOG, runs checks. Example: `./scripts/release.sh 0.7.4 --no-push`.
- **`scripts/publish.sh [--dry-run | --allow-dirty | --version VER]`** — publishes all crates to crates.io in dependency order, waiting for the index between tiers, skipping already-published versions.

Publish order (enforced by the script): Tier 1 `prax-schema`, `prax-query` → Tier 2 `prax-codegen`, `prax-import`, `prax-migrate`, `prax-postgres`, `prax-mysql`, `prax-sqlite`, `prax-mssql`, `prax-mongodb`, `prax-duckdb`, `prax-scylladb`, `prax-cassandra`, `prax-sqlx`, `prax-typegen` → Tier 3 `prax-armature`, `prax-axum`, `prax-actix`, `prax-cli`, `prax-pgvector` → Tier 4 `prax-orm`.

The CLI test assertions hardcode the current version (`prax-cli/tests/cli_tests.rs`) — bump them when changing the workspace version.

## Feature-dev workflow conventions used here

Plans and specs live under `docs/superpowers/`:

- **`docs/superpowers/specs/YYYY-MM-DD-<topic>-design.md`** — design specs produced via the brainstorming skill
- **`docs/superpowers/plans/YYYY-MM-DD-<topic>.md`** — implementation plans produced via writing-plans, with checkbox tasks

Completed plans from recent work (event-sourced migrations, DuckDB migrations, ScyllaDB/CQL dialect, prax-cassandra) are committed under this path and serve as reference for how similar work gets scoped. When picking up a plan to execute, prefer the subagent-driven-development flow with a worktree per branch.

## SQL safety

See `.cursor/rules/sql-safety.mdc`. Never concatenate user input into SQL; always use parameterized queries (`Filter::to_sql` / `SqlBuilder::push_param`). Identifier names (tables, columns) can't be parameterized — whitelist them via enums or constant lists.

## External URLs and projects

- crates.io: https://crates.io/crates/prax-orm (and per-crate pages)
- docs.rs: https://docs.rs/prax-orm
- Repo: https://github.com/quinnjr/prax (canonical; migrated from the old `pegasusheavy/prax-orm` org — the npm package `@pegasusheavy/tailswatch` and `pegasusheavy.com` remain under the old name intentionally)

---

<!-- converted from Cursor rules -->

## Cursor rule: `.cursor/rules/api-design.mdc`

# API Design Guidelines

This project provides a public API for ORM operations. Follow these guidelines for consistent, ergonomic, and safe API design.

## Builder Pattern

### Use Builders for Complex Construction

```rust
// ✅ Good: Builder pattern for many options
let query = client
    .user()
    .find_many()
    .where_(user::status::equals("active"))
    .where_(user::age::gte(18))
    .order_by(user::created_at::desc())
    .take(10)
    .skip(20)
    .select(user::select!([id, email, name]))
    .exec()
    .await?;

// ✅ Good: Builder with required fields enforced by types
let config = DatabaseConfig::builder()
    .url("postgres://...")  // Required
    .max_connections(10)    // Optional
    .build()?;              // Validates required fields
```

### Return Self for Chaining

```rust
pub struct QueryBuilder {
    filter: Option<Filter>,
    order_by: Option<OrderBy>,
    take: Option<usize>,
}

impl QueryBuilder {
    // ✅ Good: Return Self for chaining
    pub fn where_(mut self, filter: Filter) -> Self {
        self.filter = Some(filter);
        self
    }

    pub fn order_by(mut self, order: OrderBy) -> Self {
        self.order_by = Some(order);
        self
    }

    pub fn take(mut self, n: usize) -> Self {
        self.take = Some(n);
        self
    }
}
```

### Use `#[must_use]` for Builders

```rust
#[must_use = "queries do nothing until .exec() is called"]
pub struct QueryBuilder { ... }

#[must_use = "this returns a new builder and does not modify self"]
pub fn where_(self, filter: Filter) -> Self { ... }
```

## Type-Safe APIs

### Use Enums Instead of Strings

```rust
// ✅ Good: Type-safe ordering
pub enum SortDirection {
    Asc,
    Desc,
}

pub fn order_by(column: &str, direction: SortDirection) -> OrderBy { ... }

// ❌ Bad: Stringly typed
pub fn order_by(column: &str, direction: &str) -> OrderBy { ... }
// Caller can pass "ascending", "ASC", "up", etc.
```

### Use Newtypes for Domain Concepts

```rust
// ✅ Good: Newtypes prevent mixing up arguments
#[derive(Debug, Clone, Copy)]
pub struct UserId(pub i64);

#[derive(Debug, Clone, Copy)]
pub struct PostId(pub i64);

pub fn get_posts_by_user(user_id: UserId) -> Vec<Post> { ... }

// ❌ Bad: Easy to mix up i64 arguments
pub fn get_posts_by_user(user_id: i64) -> Vec<Post> { ... }
// get_posts_by_user(post_id) compiles but is wrong!
```

### Use `Into` for Flexible Input

```rust
// ✅ Good: Accept multiple input types
pub fn where_<F: Into<Filter>>(self, filter: F) -> Self {
    self.filter = Some(filter.into());
    self
}

// Caller can use:
// .where_(Filter::Equals(...))
// .where_(user::id::equals(1))
// .where_("id = 1")  // if impl Into<Filter> for &str
```

## Error Handling in APIs

### Return Result for Fallible Operations

```rust
// ✅ Good: Result for operations that can fail
pub async fn exec(self) -> Result<Vec<User>, QueryError> { ... }

// ✅ Good: Option for "not found" scenarios
pub async fn find_unique(self) -> Result<Option<User>, QueryError> { ... }

// ❌ Bad: Panic on error
pub async fn exec(self) -> Vec<User> {
    self.try_exec().unwrap() // Don't panic in library code!
}
```

### Document Error Conditions

```rust
/// Execute the query and return results.
///
/// # Errors
///
/// Returns `QueryError::Connection` if the database is unreachable.
/// Returns `QueryError::Timeout` if the query exceeds the configured timeout.
/// Returns `QueryError::InvalidFilter` if the filter references unknown columns.
pub async fn exec(self) -> Result<Vec<User>, QueryError> { ... }
```

## Naming Conventions

### Methods

```rust
// Constructors: new, with_*, from_*
pub fn new() -> Self { ... }
pub fn with_capacity(n: usize) -> Self { ... }
pub fn from_row(row: Row) -> Self { ... }

// Queries: is_*, has_*, can_*
pub fn is_empty(&self) -> bool { ... }
pub fn has_filter(&self) -> bool { ... }

// Getters: no prefix, or get_* for clarity
pub fn len(&self) -> usize { ... }
pub fn get_column(&self, name: &str) -> Option<&Column> { ... }

// Setters: set_* or builder-style
pub fn set_filter(&mut self, filter: Filter) { ... }
pub fn filter(self, filter: Filter) -> Self { ... } // Builder

// Conversions: to_*, into_*, as_*
pub fn to_sql(&self) -> String { ... }           // Allocates
pub fn into_inner(self) -> T { ... }             // Consumes self
pub fn as_str(&self) -> &str { ... }             // Borrows
```

### Types

```rust
// Traits: Verb or -able/-ible
pub trait Execute { ... }
pub trait Filterable { ... }

// Builders: *Builder
pub struct QueryBuilder { ... }
pub struct ConfigBuilder { ... }

// Errors: *Error
pub enum QueryError { ... }
pub enum ParseError { ... }

// Results: Use type aliases
pub type QueryResult<T> = Result<T, QueryError>;
```

## Documentation

### Document Public Items

```rust
/// A type-safe query builder for database operations.
///
/// # Examples
///
/// ```rust
/// let users = client
///     .user()
///     .find_many()
///     .where_(user::active::equals(true))
///     .take(10)
///     .exec()
///     .await?;
/// ```
///
/// # Panics
///
/// This method never panics.
///
/// # Errors
///
/// Returns `QueryError` if the database query fails.
pub struct QueryBuilder { ... }
```

### Provide Examples in Docs

```rust
/// Parse a filter expression from a string.
///
/// # Examples
///
/// ```rust
/// use prax_query::Filter;
///
/// // Simple equality
/// let filter = Filter::parse("status = 'active'")?;
///
/// // Complex expression
/// let filter = Filter::parse("age > 18 AND (status = 'active' OR verified = true)")?;
/// ```
pub fn parse(expr: &str) -> Result<Filter, ParseError> { ... }
```

## Deprecation

### Use `#[deprecated]` Properly

```rust
#[deprecated(since = "0.3.0", note = "use `find_many()` instead")]
pub fn find_all(&self) -> Vec<T> { ... }

// Provide migration path
/// Use `find_many().exec().await` instead of `find_all()`.
#[deprecated(since = "0.3.0", note = "use find_many().exec().await")]
pub async fn find_all(&self) -> Result<Vec<T>> {
    self.find_many().exec().await
}
```

## Extensibility

### Use `#[non_exhaustive]` for Future Compatibility

```rust
// ✅ Good: Can add variants without breaking change
#[non_exhaustive]
pub enum FilterOp {
    Equals,
    NotEquals,
    Gt,
    Lt,
    // Can add more later
}

// ✅ Good: Can add fields without breaking change
#[non_exhaustive]
pub struct QueryOptions {
    pub timeout: Duration,
    pub max_rows: usize,
    // Can add more later
}
```

### Seal Traits That Shouldn't Be Implemented Externally

```rust
mod private {
    pub trait Sealed {}
}

/// This trait is sealed and cannot be implemented outside this crate.
pub trait DatabaseType: private::Sealed {
    fn name(&self) -> &'static str;
}

impl private::Sealed for PostgreSQL {}
impl DatabaseType for PostgreSQL {
    fn name(&self) -> &'static str { "postgresql" }
}
```

## Thread Safety

### Document Send/Sync Requirements

```rust
/// A thread-safe connection pool.
///
/// This type is `Send + Sync` and can be shared across threads.
/// Clone is cheap (Arc internally).
#[derive(Clone)]
pub struct Pool {
    inner: Arc<PoolInner>,
}

/// A query builder bound to a specific connection.
///
/// This type is `Send` but NOT `Sync` - don't share across threads.
/// Use `.clone()` to create independent builders.
pub struct QueryBuilder<'a> {
    conn: &'a Connection, // Not Sync
}
```

## Summary

1. **Use builders** for complex construction with chaining
2. **Prefer types over strings** - enums, newtypes
3. **Accept flexible input** with `Into<T>`
4. **Return Result** for fallible operations
5. **Document thoroughly** - examples, errors, panics
6. **Use `#[must_use]`** for builders and important returns
7. **Use `#[non_exhaustive]`** for future extensibility
8. **Follow naming conventions** consistently


## Cursor rule: `.cursor/rules/async-first.mdc`

# Async/Parallel-First Design

This project prioritizes **asynchronous and parallel execution** over synchronous code. All I/O operations, database queries, and potentially blocking operations must be async.

## Core Principles

### 1. Default to Async

Every function that performs I/O should be `async`:

```rust
// ✅ Good: Async by default
pub async fn find_user(id: i64) -> Result<User> {
    let row = client.query_one("SELECT * FROM users WHERE id = $1", &[&id]).await?;
    Ok(User::from_row(row))
}

// ❌ Bad: Synchronous I/O
pub fn find_user(id: i64) -> Result<User> {
    let row = client.query_one("SELECT * FROM users WHERE id = $1", &[&id])?;
    Ok(User::from_row(row))
}
```

### 2. Parallel by Default

When multiple independent operations exist, execute them in parallel:

```rust
// ✅ Good: Parallel execution
let (users, posts, comments) = tokio::try_join!(
    client.user().find_many().exec(),
    client.post().find_many().exec(),
    client.comment().find_many().exec(),
)?;

// ❌ Bad: Sequential when parallel is possible
let users = client.user().find_many().exec().await?;
let posts = client.post().find_many().exec().await?;
let comments = client.comment().find_many().exec().await?;
```

### 3. Use `tokio::spawn` for CPU-bound Work

Offload CPU-intensive tasks to avoid blocking the async runtime:

```rust
// ✅ Good: Spawn blocking work
let parsed = tokio::task::spawn_blocking(move || {
    expensive_parsing_operation(&data)
}).await?;

// ❌ Bad: Blocking in async context
let parsed = expensive_parsing_operation(&data); // blocks the runtime
```

### 4. Prefer `tokio::select!` for Racing

When you need the first result from multiple futures:

```rust
// ✅ Good: Race with select
tokio::select! {
    result = primary_db.query(&sql) => handle_result(result),
    result = replica_db.query(&sql) => handle_result(result),
    _ = tokio::time::sleep(timeout) => return Err(Error::Timeout),
}
```

## Required Patterns

### Connection Pools

Always use async connection pools:

```rust
// Use deadpool-postgres or bb8
use deadpool_postgres::{Config, Pool, Runtime};

pub struct DatabasePool {
    pool: Pool,
}

impl DatabasePool {
    pub async fn get(&self) -> Result<PooledConnection> {
        self.pool.get().await.map_err(Into::into)
    }
}
```

### Streaming Results

For large result sets, use async streams:

```rust
use futures::Stream;
use tokio_stream::StreamExt;

pub fn find_all(&self) -> impl Stream<Item = Result<User>> {
    // Return a stream instead of Vec for memory efficiency
    async_stream::try_stream! {
        let mut rows = client.query_raw(&sql, &[]).await?;
        while let Some(row) = rows.next().await {
            yield User::from_row(row?);
        }
    }
}
```

### Batch Operations

Batch multiple queries for efficiency:

```rust
// ✅ Good: Batch inserts
pub async fn create_many(users: Vec<CreateUser>) -> Result<Vec<User>> {
    let futures: Vec<_> = users
        .into_iter()
        .map(|u| self.create(u))
        .collect();

    futures::future::try_join_all(futures).await
}
```

### Timeouts

Always include timeouts for async operations:

```rust
use tokio::time::{timeout, Duration};

pub async fn query_with_timeout(&self, sql: &str) -> Result<Vec<Row>> {
    timeout(Duration::from_secs(30), self.client.query(sql, &[]))
        .await
        .map_err(|_| Error::Timeout)?
        .map_err(Into::into)
}
```

## Trait Definitions

All database traits must be async:

```rust
// ✅ Good: Async trait (Rust 2024 supports this natively)
pub trait Repository {
    async fn find(&self, id: i64) -> Result<Option<Model>>;
    async fn find_many(&self, filter: Filter) -> Result<Vec<Model>>;
    async fn create(&self, data: CreateInput) -> Result<Model>;
    async fn update(&self, id: i64, data: UpdateInput) -> Result<Model>;
    async fn delete(&self, id: i64) -> Result<()>;
}

// ❌ Bad: Sync trait
pub trait Repository {
    fn find(&self, id: i64) -> Result<Option<Model>>;
}
```

## Concurrency Primitives

### Use `tokio::sync` Not `std::sync`

```rust
// ✅ Good: Tokio's async-aware primitives
use tokio::sync::{RwLock, Mutex, Semaphore, broadcast, mpsc};

// ❌ Bad: std sync primitives block the runtime
use std::sync::{RwLock, Mutex};
```

### Prefer `RwLock` Over `Mutex`

When reads dominate writes:

```rust
// ✅ Good: RwLock for read-heavy cache
let cache: Arc<RwLock<HashMap<K, V>>> = Arc::new(RwLock::new(HashMap::new()));

// Read path (many concurrent readers)
let value = cache.read().await.get(&key).cloned();

// Write path (exclusive access)
cache.write().await.insert(key, value);
```

## Error Handling in Async

### Propagate Errors with `?`

```rust
pub async fn complex_operation(&self) -> Result<Output> {
    let a = self.step_a().await?;
    let b = self.step_b(&a).await?;
    let c = self.step_c(&b).await?;
    Ok(c)
}
```

### Use `try_join!` for Parallel Error Handling

```rust
// All succeed or first error is returned
let (a, b, c) = tokio::try_join!(
    self.fetch_a(),
    self.fetch_b(),
    self.fetch_c(),
)?;
```

## Cancellation Safety

Document cancellation behavior for all public async functions:

```rust
/// Executes a database query.
///
/// # Cancellation Safety
///
/// This function is cancellation safe. If the future is dropped before
/// completion, no partial writes will occur. The connection is returned
/// to the pool in a clean state.
pub async fn execute(&self, query: &str) -> Result<u64> {
    // ...
}
```

## Testing Async Code

Use `#[tokio::test]` for async tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_find_user() {
        let pool = setup_test_pool().await;
        let user = pool.user().find(1).await.unwrap();
        assert_eq!(user.id, 1);
    }

    #[tokio::test]
    async fn test_concurrent_queries() {
        let pool = setup_test_pool().await;

        let handles: Vec<_> = (0..10)
            .map(|i| {
                let pool = pool.clone();
                tokio::spawn(async move {
                    pool.user().find(i).await
                })
            })
            .collect();

        let results = futures::future::join_all(handles).await;
        assert!(results.iter().all(|r| r.is_ok()));
    }
}
```

## Never Block the Runtime

### Forbidden Patterns

```rust
// ❌ NEVER: std::thread::sleep in async
std::thread::sleep(Duration::from_secs(1));

// ❌ NEVER: Blocking file I/O in async
std::fs::read_to_string("file.txt");

// ❌ NEVER: Sync HTTP requests in async
reqwest::blocking::get("https://api.example.com");

// ❌ NEVER: CPU-bound loops without yielding
loop {
    heavy_computation();
}
```

### Correct Alternatives

```rust
// ✅ Use tokio::time::sleep
tokio::time::sleep(Duration::from_secs(1)).await;

// ✅ Use tokio::fs for file I/O
tokio::fs::read_to_string("file.txt").await;

// ✅ Use async HTTP client
reqwest::get("https://api.example.com").await;

// ✅ Yield in long loops or spawn_blocking
loop {
    heavy_computation();
    tokio::task::yield_now().await;
}
```

## Summary

1. **All I/O is async** - No blocking operations in async contexts
2. **Parallel when possible** - Use `join!`, `try_join!`, `join_all` for independent operations
3. **Stream large results** - Don't load everything into memory
4. **Use tokio primitives** - `tokio::sync`, `tokio::time`, `tokio::fs`
5. **Document cancellation** - Specify safety guarantees for public APIs
6. **Test concurrently** - Verify code works under concurrent load


## Cursor rule: `.cursor/rules/benchmarking.mdc`

# Benchmarking Guidelines

This project uses [Criterion.rs](https://github.com/bheisler/criterion.rs) for benchmarks. Follow these guidelines to write meaningful, reliable benchmarks.

## Benchmark File Organization

### Location

All benchmarks live in `prax-query/benches/`:

```
prax-query/benches/
├── operations_bench.rs      # Core filter and SQL builder
├── aggregation_bench.rs     # Aggregation and grouping
├── pagination_bench.rs      # Cursor and offset pagination
├── advanced_features_bench.rs # Window functions, CTEs
├── tenant_bench.rs          # Multi-tenancy overhead
├── async_bench.rs           # Concurrent execution
├── mem_optimize_bench.rs    # Memory optimizations
├── database_bench.rs        # Database-specific SQL
└── throughput_bench.rs      # Queries-per-second
```

### Registration

Add new benchmarks to `Cargo.toml`:

```toml
[[bench]]
name = "my_new_bench"
harness = false
```

## Writing Good Benchmarks

### Use `black_box` to Prevent Optimization

```rust
use criterion::black_box;

// ✅ Good: Result consumed by black_box
group.bench_function("filter_creation", |b| {
    b.iter(|| {
        let filter = Filter::Equals("id".into(), FilterValue::Int(1));
        black_box(filter)
    });
});

// ❌ Bad: Compiler may optimize away unused result
group.bench_function("filter_creation", |b| {
    b.iter(|| {
        let filter = Filter::Equals("id".into(), FilterValue::Int(1));
        // filter is dropped immediately, compiler may skip creation
    });
});
```

### Separate Setup from Measurement

```rust
// ✅ Good: Setup outside the measurement loop
group.bench_function("filter_to_sql", |b| {
    // Setup: Create filter once
    let filter = Filter::and(vec![
        Filter::Equals("status".into(), FilterValue::String("active".into())),
        Filter::Gt("age".into(), FilterValue::Int(18)),
    ]);

    // Measure: Only the to_sql() call
    b.iter(|| black_box(filter.to_sql(0)))
});

// ❌ Bad: Setup included in measurement
group.bench_function("filter_to_sql", |b| {
    b.iter(|| {
        let filter = Filter::and(vec![
            Filter::Equals("status".into(), FilterValue::String("active".into())),
            Filter::Gt("age".into(), FilterValue::Int(18)),
        ]);
        black_box(filter.to_sql(0))
    });
});
```

### Use Throughput for Batch Operations

```rust
// ✅ Good: Throughput shows operations/second
for batch_size in [10, 50, 100, 500] {
    group.throughput(Throughput::Elements(batch_size as u64));

    group.bench_function(BenchmarkId::new("batch_insert", batch_size), |b| {
        b.iter(|| {
            // Process batch_size items
        });
    });
}
```

### Group Related Benchmarks

```rust
fn bench_filter_creation(c: &mut Criterion) {
    let mut group = c.benchmark_group("filter_creation");

    group.bench_function("equals_int", |b| { /* ... */ });
    group.bench_function("equals_string", |b| { /* ... */ });
    group.bench_function("in_filter", |b| { /* ... */ });
    group.bench_function("complex_and_or", |b| { /* ... */ });

    group.finish();
}
```

## Benchmark Categories

### Micro-benchmarks

Test individual operations in isolation:

```rust
// Filter creation
group.bench_function("create_equals", |b| {
    b.iter(|| black_box(Filter::Equals("id".into(), FilterValue::Int(1))))
});

// SQL generation
group.bench_function("simple_to_sql", |b| {
    let filter = Filter::Equals("id".into(), FilterValue::Int(1));
    b.iter(|| black_box(filter.to_sql(0)))
});
```

### Throughput Benchmarks

Measure sustained operations per second:

```rust
group.throughput(Throughput::Elements(1000));
group.measurement_time(Duration::from_secs(10));

group.bench_function("1000_queries", |b| {
    b.iter(|| {
        for i in 0..1000 {
            let filter = Filter::Equals("id".into(), FilterValue::Int(i));
            black_box(filter.to_sql(0));
        }
    });
});
```

### Realistic Scenario Benchmarks

Simulate actual usage patterns:

```rust
group.bench_function("ecommerce_search", |b| {
    b.iter(|| {
        let filter = Filter::and(vec![
            Filter::Contains("name".into(), FilterValue::String("laptop".into())),
            Filter::Gte("price".into(), FilterValue::Float(500.0)),
            Filter::Lte("price".into(), FilterValue::Float(2000.0)),
            Filter::Equals("in_stock".into(), FilterValue::Bool(true)),
        ]);
        let (sql, params) = filter.to_sql(0);
        black_box((format!("SELECT * FROM products WHERE {} LIMIT 24", sql), params))
    });
});
```

### Comparative Benchmarks

Compare different approaches:

```rust
// Compare with and without optimization
group.bench_function("without_interning", |b| {
    b.iter(|| {
        let mut strings: Vec<String> = Vec::new();
        for i in 0..100 {
            strings.push(format!("field_{}", i % 10));
        }
        black_box(strings)
    });
});

group.bench_function("with_interning", |b| {
    let interner = GlobalInterner::get();
    b.iter(|| {
        let mut strings = Vec::new();
        for i in 0..100 {
            strings.push(interner.intern(&format!("field_{}", i % 10)));
        }
        black_box(strings)
    });
});
```

## Running Benchmarks

### Basic Commands

```bash
# Run all benchmarks
cargo bench --package prax-query

# Run specific benchmark file
cargo bench --package prax-query --bench operations_bench

# Run specific benchmark function
cargo bench --package prax-query -- filter_creation

# Quick run (fewer iterations)
cargo bench --package prax-query -- --quick
```

### Baseline Comparisons

```bash
# Save a baseline
cargo bench --package prax-query -- --save-baseline main

# Compare against baseline
cargo bench --package prax-query -- --load-baseline main

# Save new baseline and compare
cargo bench --package prax-query -- --save-baseline feature --load-baseline main
```

### CI Integration

The `.github/workflows/benchmarks.yml` runs on PRs:
- Compares against main branch baseline
- Reports regressions > 10%
- Posts results as PR comment

## Interpreting Results

### Time Measurements

```
filter_creation/equals_int
                        time:   [15.234 ns 15.456 ns 15.692 ns]
                        thrpt:  [63.724 Melem/s 64.698 Melem/s 65.641 Melem/s]
```

- **time**: [lower bound, estimate, upper bound] with 95% confidence
- **thrpt**: Throughput if `Throughput::Elements` was set

### Change Detection

```
filter_creation/equals_int
                        time:   [15.456 ns 15.692 ns 15.928 ns]
                        change: [-2.1234% -0.5678% +1.0234%] (p = 0.12 > 0.05)
                        No change in performance detected.
```

- **change**: Percentage change from baseline
- **p value**: Statistical significance (< 0.05 = significant)

### Regression Warnings

```
Performance has regressed.
filter_creation/complex_and_or
                        time:   [125.45 ns 128.92 ns 132.87 ns]
                        change: [+12.34% +15.67% +19.01%] (p = 0.00 < 0.05)
```

Investigate regressions > 5% before merging.

## Common Pitfalls

### Don't Benchmark Debug Builds

```bash
# ❌ Bad: Debug build
cargo bench

# ✅ Good: Release build (default for bench)
cargo bench --release
```

### Avoid External Variability

```rust
// ❌ Bad: Network I/O in benchmark
group.bench_function("real_db_query", |b| {
    b.iter(|| {
        let result = db.query("SELECT * FROM users").await; // Variable latency!
        black_box(result)
    });
});

// ✅ Good: Mock or in-memory for consistent results
group.bench_function("sql_generation", |b| {
    b.iter(|| {
        let sql = query_builder.build(); // Deterministic
        black_box(sql)
    });
});
```

### Warm Up Caches

```rust
// ✅ Good: Pre-warm any caches
group.bench_function("with_warm_cache", |b| {
    // Warm up
    let interner = GlobalInterner::get();
    for i in 0..100 {
        interner.intern(&format!("field_{}", i));
    }

    // Now measure cache hits
    b.iter(|| black_box(interner.intern("field_50")))
});
```

## Adding New Benchmarks Checklist

- [ ] File added to `benches/` directory
- [ ] Registered in `Cargo.toml` with `harness = false`
- [ ] Uses `black_box` for all measured results
- [ ] Setup separated from measurement
- [ ] Grouped logically with `benchmark_group`
- [ ] Uses `Throughput` for batch operations
- [ ] Documents what is being measured
- [ ] No external I/O or network calls
- [ ] Runs in reasonable time (< 60s total)


## Cursor rule: `.cursor/rules/caching.mdc`

# Data Caching Guidelines

This project provides a tiered caching layer with in-memory and Redis backends. Follow these guidelines for effective cache usage.

## Cache Architecture

### Tiered Cache (Recommended)

Use L1 (memory) + L2 (Redis) for best performance:

```rust
use prax_query::data_cache::{TieredCache, MemoryCache, RedisCache};

// L1: Fast in-memory cache
let memory = MemoryCache::builder()
    .max_capacity(10_000)
    .time_to_live(Duration::from_secs(60))
    .build();

// L2: Distributed Redis cache
let redis = RedisCache::builder()
    .url("redis://localhost:6379")
    .key_prefix("myapp:")
    .default_ttl(Duration::from_secs(300))
    .build()
    .await?;

// Tiered: Check L1 first, then L2
let cache = TieredCache::new(memory, redis);
```

### Memory-Only Cache

For single-instance deployments:

```rust
use prax_query::data_cache::MemoryCache;

let cache = MemoryCache::builder()
    .max_capacity(50_000)
    .time_to_live(Duration::from_secs(300))
    .time_to_idle(Duration::from_secs(60))
    .build();
```

### Redis-Only Cache

For distributed caching without local cache:

```rust
use prax_query::data_cache::RedisCache;

let cache = RedisCache::builder()
    .url("redis://localhost:6379")
    .pool_size(10)
    .key_prefix("myapp:")
    .default_ttl(Duration::from_secs(3600))
    .build()
    .await?;
```

## Cache Keys

### Use Structured Keys

```rust
use prax_query::data_cache::CacheKey;

// ✅ Good: Structured, predictable keys
let key = CacheKey::entity("User", 123);           // "User:123"
let key = CacheKey::query("users", &filter_hash);  // "query:users:{hash}"
let key = CacheKey::custom("feature_flags", "v1"); // "feature_flags:v1"

// ❌ Bad: Unstructured keys
let key = format!("user_{}", id);  // No namespace, hard to invalidate
```

### Include Tenant in Keys (Multi-Tenant)

```rust
// ✅ Good: Tenant-scoped keys
let key = CacheKey::tenant_entity(tenant_id, "User", user_id);
// "tenant:123:User:456"

// ✅ Good: Tenant prefix in Redis
let redis = RedisCache::builder()
    .key_prefix(format!("tenant:{}:", tenant_id))
    .build()
    .await?;

// ❌ DANGEROUS: Shared keys across tenants
let key = CacheKey::entity("User", user_id);
// Tenant A might see Tenant B's cached data!
```

## Cache Operations

### Basic Get/Set

```rust
// Get with type inference
let user: Option<User> = cache.get(&key).await?;

// Set with default TTL
cache.set(&key, &user).await?;

// Set with custom TTL
cache.set_with_ttl(&key, &user, Duration::from_secs(600)).await?;

// Get or compute
let user = cache.get_or_set(&key, || async {
    db.user().find_unique(user::id::equals(id)).exec().await
}).await?;
```

### Batch Operations

```rust
// Get multiple keys
let keys = vec![
    CacheKey::entity("User", 1),
    CacheKey::entity("User", 2),
    CacheKey::entity("User", 3),
];
let users: Vec<Option<User>> = cache.get_many(&keys).await?;

// Set multiple
cache.set_many(&[(key1, user1), (key2, user2)]).await?;
```

## Invalidation Strategies

### Entity-Based Invalidation

```rust
// Invalidate single entity
cache.invalidate(&CacheKey::entity("User", user_id)).await?;

// Invalidate all entities of a type
cache.invalidate_pattern("User:*").await?;

// Invalidate related entities
async fn update_user(id: i64, data: UpdateUser) -> Result<User> {
    let user = db.user().update(id, data).exec().await?;

    // Invalidate user cache
    cache.invalidate(&CacheKey::entity("User", id)).await?;

    // Invalidate related caches
    cache.invalidate(&CacheKey::entity("UserProfile", id)).await?;
    cache.invalidate_pattern(&format!("query:users:*")).await?;

    Ok(user)
}
```

### Tag-Based Invalidation

```rust
use prax_query::data_cache::EntityTag;

// Cache with tags
cache.set_with_tags(
    &key,
    &user,
    &[EntityTag::entity("User"), EntityTag::record("User", user_id)],
).await?;

// Invalidate by tag
cache.invalidate_tag(&EntityTag::entity("User")).await?;
// All User caches invalidated
```

### Write-Through Pattern

```rust
// Update database and cache atomically
async fn update_user(id: i64, data: UpdateUser) -> Result<User> {
    // Update DB
    let user = db.user().update(id, data).exec().await?;

    // Update cache (not invalidate)
    cache.set(&CacheKey::entity("User", id), &user).await?;

    Ok(user)
}
```

### Cache-Aside Pattern

```rust
// Read: Check cache first
async fn get_user(id: i64) -> Result<User> {
    let key = CacheKey::entity("User", id);

    // Try cache
    if let Some(user) = cache.get(&key).await? {
        return Ok(user);
    }

    // Cache miss: load from DB
    let user = db.user()
        .find_unique(user::id::equals(id))
        .exec()
        .await?
        .ok_or(Error::NotFound)?;

    // Populate cache
    cache.set(&key, &user).await?;

    Ok(user)
}
```

## TTL Configuration

### Choose Appropriate TTLs

```rust
// ✅ Good: Different TTLs for different data types

// Rarely changes, long TTL
let feature_flags = CachePolicy::new()
    .ttl(Duration::from_secs(3600))  // 1 hour
    .stale_while_revalidate(Duration::from_secs(300));

// User data, medium TTL
let user_data = CachePolicy::new()
    .ttl(Duration::from_secs(300))  // 5 minutes
    .stale_while_revalidate(Duration::from_secs(60));

// Real-time data, short TTL
let live_data = CachePolicy::new()
    .ttl(Duration::from_secs(10))  // 10 seconds
    .no_stale();

// Static reference data, very long TTL
let countries = CachePolicy::new()
    .ttl(Duration::from_secs(86400));  // 24 hours
```

### Use Presets

```rust
use prax_query::data_cache::CachePolicy;

// Built-in presets
let policy = CachePolicy::user_data();      // 5 min TTL
let policy = CachePolicy::reference_data(); // 1 hour TTL
let policy = CachePolicy::static_data();    // 24 hour TTL
let policy = CachePolicy::realtime();       // 10 sec TTL
```

## Cache Metrics

### Monitor Cache Health

```rust
let stats = cache.stats();

println!("Hits: {}", stats.hits);
println!("Misses: {}", stats.misses);
println!("Hit rate: {:.2}%", stats.hit_rate() * 100.0);
println!("Size: {} entries", stats.size);
println!("Memory: {} bytes", stats.memory_bytes);

// Alert on low hit rate
if stats.hit_rate() < 0.7 {
    warn!("Cache hit rate below 70%: {:.2}%", stats.hit_rate() * 100.0);
}
```

### Expose Metrics

```rust
// Prometheus metrics
cache.register_metrics(&prometheus_registry);

// Metrics: prax_cache_hits_total, prax_cache_misses_total, etc.
```

## Testing Cache

### Test Cache Hit/Miss

```rust
#[tokio::test]
async fn test_cache_hit() {
    let cache = MemoryCache::new(1000);
    let key = CacheKey::entity("User", 1);

    // Miss
    assert!(cache.get::<User>(&key).await?.is_none());

    // Set
    let user = User { id: 1, name: "Alice".into() };
    cache.set(&key, &user).await?;

    // Hit
    let cached = cache.get::<User>(&key).await?;
    assert_eq!(cached, Some(user));
}
```

### Test Invalidation

```rust
#[tokio::test]
async fn test_invalidation() {
    let cache = MemoryCache::new(1000);
    let key = CacheKey::entity("User", 1);

    cache.set(&key, &user).await?;
    assert!(cache.get::<User>(&key).await?.is_some());

    cache.invalidate(&key).await?;
    assert!(cache.get::<User>(&key).await?.is_none());
}
```

### Test TTL Expiration

```rust
#[tokio::test]
async fn test_ttl_expiration() {
    let cache = MemoryCache::builder()
        .time_to_live(Duration::from_millis(100))
        .build();

    cache.set(&key, &user).await?;
    assert!(cache.get::<User>(&key).await?.is_some());

    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(cache.get::<User>(&key).await?.is_none());
}
```

## Common Pitfalls

### Avoid Cache Stampede

```rust
// ❌ Bad: Many concurrent requests trigger DB load
async fn get_user(id: i64) -> Result<User> {
    if let Some(user) = cache.get(&key).await? {
        return Ok(user);
    }
    // 100 concurrent requests all miss and hit DB
    let user = db.user().find(id).await?;
    cache.set(&key, &user).await?;
    Ok(user)
}

// ✅ Good: Use locking or singleflight
async fn get_user(id: i64) -> Result<User> {
    cache.get_or_set_with_lock(&key, || async {
        // Only one request loads from DB
        db.user().find(id).await
    }).await
}
```

### Don't Cache Errors

```rust
// ❌ Bad: Caching error state
let result = db.user().find(id).await;
cache.set(&key, &result).await?; // Caches Err!

// ✅ Good: Only cache success
match db.user().find(id).await {
    Ok(user) => {
        cache.set(&key, &user).await?;
        Ok(user)
    }
    Err(e) => Err(e), // Don't cache
}
```

### Cache Serializable Data Only

```rust
// ✅ Good: Cache serializable types
#[derive(Serialize, Deserialize)]
struct CachedUser {
    id: i64,
    name: String,
}

// ❌ Bad: Cache types with connections, handles
struct User {
    id: i64,
    db_connection: Connection, // Not serializable!
}
```

## Summary

1. **Use tiered cache** (L1 memory + L2 Redis) for best performance
2. **Structure cache keys** with entity type and ID
3. **Include tenant ID** in keys for multi-tenant apps
4. **Choose appropriate TTLs** based on data volatility
5. **Invalidate on writes** - either entity or tag-based
6. **Monitor cache metrics** - alert on low hit rates
7. **Prevent cache stampede** with locking
8. **Test cache behavior** - hits, misses, TTL, invalidation


## Cursor rule: `.cursor/rules/changelog.mdc`

_Guidelines for maintaining CHANGELOG.md following Keep a Changelog format_

Applies to: `["CHANGELOG.md"]`

# Changelog Guidelines

This project follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) format and [Semantic Versioning](https://semver.org/).

## When to Update

Update `CHANGELOG.md` when:
- ✅ Adding a new feature (`feat` commits)
- ✅ Fixing a bug (`fix` commits)
- ✅ Making breaking changes
- ✅ Deprecating functionality
- ✅ Removing features
- ✅ Security fixes
- ✅ Performance improvements (`perf` commits)

Do NOT update for:
- ❌ Internal refactoring (no user-facing changes)
- ❌ Test additions/changes
- ❌ CI/CD changes
- ❌ Documentation-only changes (unless significant)
- ❌ Code style/formatting

## File Structure

```markdown
# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- New features here

### Changed
- Changes to existing functionality

### Deprecated
- Features that will be removed

### Removed
- Features that were removed

### Fixed
- Bug fixes

### Security
- Security-related fixes

## [0.1.0] - 2025-01-15

### Added
- Initial release features

[Unreleased]: https://github.com/quinnjr/prax/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/quinnjr/prax/releases/tag/v0.1.0
```

## Section Definitions

### Added
New features or capabilities added to the project.

```markdown
### Added
- Query builder now supports nested `where` clauses
- New `include()` method for eager loading relations
- PostgreSQL `JSONB` column type support
```

### Changed
Changes to existing functionality (non-breaking).

```markdown
### Changed
- `execute()` now returns `QueryResult<T>` instead of `Result<T, Error>`
- Connection pool default size increased from 5 to 10
- Improved error messages for invalid schema definitions
```

### Deprecated
Features that will be removed in future versions.

```markdown
### Deprecated
- `Client::query_raw()` - use `Client::raw_query()` instead
- The `sync` feature flag will be removed in v1.0.0
```

### Removed
Features that were removed in this release.

```markdown
### Removed
- Removed deprecated `Client::execute_sync()` method
- Dropped support for PostgreSQL versions below 12
```

### Fixed
Bug fixes.

```markdown
### Fixed
- Fixed connection leak when queries timeout
- Fixed panic when parsing schemas with circular relations
- Corrected SQL generation for `NOT IN` clauses
```

### Security
Security-related fixes (always include CVE if applicable).

```markdown
### Security
- Fixed SQL injection vulnerability in raw query interpolation (CVE-2025-XXXX)
- Updated `tokio` to address potential DoS vector
```

## Writing Good Entries

### DO ✅

```markdown
### Added
- Add `cursor()` method for cursor-based pagination (#123)
- Add support for `RETURNING` clause in insert queries

### Fixed
- Fix memory leak in connection pool under high load (#456)
- Fix incorrect SQL generation for nullable enum fields
```

### DON'T ❌

```markdown
### Added
- Added stuff
- New feature
- Implemented the thing from issue #123

### Fixed
- Fixed bug
- Fix
- Bugfix
```

## Guidelines

1. **Use imperative mood**: "Add feature" not "Added feature"
2. **Be specific**: Describe what changed, not just that something changed
3. **Reference issues/PRs**: Include `(#123)` when applicable
4. **Group related changes**: Don't repeat similar entries
5. **Order by importance**: Most significant changes first
6. **Keep entries concise**: One line per change when possible

## Mapping Commits to Sections

| Commit Type | Changelog Section |
|-------------|-------------------|
| `feat` | Added |
| `fix` | Fixed |
| `perf` | Changed |
| `refactor` | Changed (if user-facing) |
| `deprecate` | Deprecated |
| `security` | Security |
| `BREAKING CHANGE` | Changed (with note) |

## Breaking Changes

Always highlight breaking changes prominently:

```markdown
### Changed
- **BREAKING**: `QueryBuilder::new()` now requires a connection parameter
- **BREAKING**: Renamed `Client` to `PraxClient` for clarity
```

Or use a dedicated section:

```markdown
### ⚠️ Breaking Changes
- `QueryBuilder::new()` now requires a connection parameter
- Renamed `Client` to `PraxClient` for clarity
```

## Release Checklist

When preparing a release:

1. Move entries from `[Unreleased]` to new version section
2. Add release date: `## [0.2.0] - 2025-02-01`
3. Update comparison links at bottom of file
4. Remove empty sections
5. Ensure all breaking changes are clearly marked
6. Verify version matches `Cargo.toml`

```markdown
## [Unreleased]

## [0.2.0] - 2025-02-01

### Added
- (moved from Unreleased)

[Unreleased]: https://github.com/quinnjr/prax/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/quinnjr/prax/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/quinnjr/prax/releases/tag/v0.1.0
```

## Version Links

Always maintain comparison links at the bottom:

```markdown
[Unreleased]: https://github.com/quinnjr/prax/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/quinnjr/prax/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/quinnjr/prax/releases/tag/v0.1.0
```

## Examples

### Feature Addition
```markdown
### Added
- Add `find_first()` method to query builder for single result queries
- Add `#[prax(default = "...")]` attribute for default column values
- Add MySQL support via `mysql_async` driver (#89)
```

### Bug Fix
```markdown
### Fixed
- Fix `ORDER BY` clause being ignored when combined with `LIMIT` (#142)
- Fix transaction rollback not releasing connection back to pool
- Fix schema parser rejecting valid enum definitions with attributes
```

### Breaking Change
```markdown
### Changed
- **BREAKING**: `PraxClient::new()` is now async and returns `Result<Self, Error>`

  Before:
  ```rust
  let client = PraxClient::new("postgres://...");
  ```

  After:
  ```rust
  let client = PraxClient::new("postgres://...").await?;
  ```
```


## Cursor rule: `.cursor/rules/documentation.mdc`

_Guidelines for keeping TODO.md and CHANGELOG.md synchronized with code changes_

Applies to: `["**/*.rs", "**/*.toml", "**/*.yml", "**/*.yaml"]`

# Documentation Synchronization

**Always keep `TODO.md` and `CHANGELOG.md` synchronized when making code changes.**

## When to Update CHANGELOG.md

Update the `[Unreleased]` section for:
- ✅ New features (`feat` commits) → **Added**
- ✅ Bug fixes (`fix` commits) → **Fixed**
- ✅ Breaking changes → **Changed** with **BREAKING** prefix
- ✅ Deprecations → **Deprecated**
- ✅ Removed features → **Removed**
- ✅ Security fixes → **Security**
- ✅ Performance improvements (`perf` commits) → **Changed**

**Do NOT update for**: internal refactoring, tests, CI changes, documentation-only changes.

### CHANGELOG Entry Format

```markdown
### Added
- **Feature Name** (`crate-name`) - Brief description of what was added
```

Example:
```markdown
### Added
- **DuckDB Support** (`prax-duckdb`) - Analytical database driver with Parquet export
```

## When to Update TODO.md

Update when:
- ✅ Completing a feature listed in TODO.md → Mark as completed or remove
- ✅ Adding significant new features → Add to completed features table
- ✅ Changing architecture → Update architecture diagram
- ✅ Adding new crates → Add to crate list
- ✅ Adding new benchmarks → Add to benchmark list

### TODO.md Structure

Keep it concise:
- **Architecture section**: List all crates
- **Completed Features**: Tables organized by category
- **Benchmarks**: List available benchmark suites
- **Quick Start**: Minimal working example
- **References**: External documentation links

## Checklist Before Completing a Task

```
□ Code changes committed
□ CHANGELOG.md updated (if user-facing change)
□ TODO.md updated (if feature completed or added)
□ Tests pass
```

## Examples

### Adding a New Feature

When implementing multi-tenancy support:

1. **CHANGELOG.md** - Add to `[Unreleased]`:
   ```markdown
   ### Added
   - **Multi-Tenancy Support** (`prax-query/src/tenant/`)
     - Zero-allocation task-local tenant context
     - PostgreSQL RLS integration
   ```

2. **TODO.md** - Add to completed features:
   ```markdown
   ### Multi-Tenancy (`prax-query/src/tenant/`)
   | Feature | Module |
   |---------|--------|
   | Task-local context | `task_local.rs` |
   | RLS integration | `rls.rs` |
   ```

### Fixing a Bug

When fixing a query builder bug:

1. **CHANGELOG.md** only:
   ```markdown
   ### Fixed
   - Fix SQL injection vulnerability in raw query builder (#123)
   ```

2. **TODO.md** - No update needed for bug fixes

### Adding a New Crate

When adding `prax-duckdb`:

1. **CHANGELOG.md**:
   ```markdown
   ### Added
   - **DuckDB Support** (`prax-duckdb`) - Analytical database driver
   ```

2. **TODO.md** - Update architecture:
   ```
   prax/
   ├── prax-duckdb/         # DuckDB analytical driver  ← Add this
   ```

## Automation Hints

When the agent completes a significant task:
1. Review what was changed
2. Determine if it's user-facing
3. Update CHANGELOG.md with proper section
4. Update TODO.md if architecture changed or feature completed
5. Keep entries concise (one line when possible)


## Cursor rule: `.cursor/rules/error-handling.mdc`

# Error Handling Guidelines

This project uses `thiserror` for library errors and follows Rust error handling best practices.

## Error Types

### Use `thiserror` for Library Errors

```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum QueryError {
    #[error("connection failed: {0}")]
    Connection(#[source] tokio_postgres::Error),

    #[error("query timeout after {0:?}")]
    Timeout(std::time::Duration),

    #[error("invalid filter: {field} - {message}")]
    InvalidFilter { field: String, message: String },

    #[error("record not found: {entity} with id {id}")]
    NotFound { entity: &'static str, id: i64 },

    #[error("constraint violation: {0}")]
    Constraint(String),
}
```

### Error Hierarchy

```rust
// Top-level error type
#[derive(Error, Debug)]
pub enum PraxError {
    #[error("query error: {0}")]
    Query(#[from] QueryError),

    #[error("schema error: {0}")]
    Schema(#[from] SchemaError),

    #[error("migration error: {0}")]
    Migration(#[from] MigrationError),

    #[error("connection error: {0}")]
    Connection(#[from] ConnectionError),
}

// Domain-specific errors
#[derive(Error, Debug)]
pub enum SchemaError {
    #[error("parse error at line {line}: {message}")]
    Parse { line: usize, message: String },

    #[error("invalid model: {0}")]
    InvalidModel(String),

    #[error("unknown type: {0}")]
    UnknownType(String),
}
```

## Error Propagation

### Use `?` Operator

```rust
// ✅ Good: Clean error propagation
pub async fn find_user(id: i64) -> Result<User, QueryError> {
    let conn = self.pool.get().await?;
    let row = conn.query_one(&self.sql, &[&id]).await?;
    let user = User::from_row(row)?;
    Ok(user)
}

// ❌ Bad: Explicit match everywhere
pub async fn find_user(id: i64) -> Result<User, QueryError> {
    let conn = match self.pool.get().await {
        Ok(c) => c,
        Err(e) => return Err(e.into()),
    };
    // ... more matches ...
}
```

### Add Context with `map_err`

```rust
use std::path::Path;

pub fn read_schema(path: &Path) -> Result<Schema, SchemaError> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| SchemaError::Io {
            path: path.to_path_buf(),
            source: e,
        })?;

    parse_schema(&content)
        .map_err(|e| SchemaError::Parse {
            path: path.to_path_buf(),
            source: e,
        })
}
```

### Use `anyhow` for Context in Applications

```rust
// In CLI or application code (not library)
use anyhow::{Context, Result};

pub fn run_migration(path: &str) -> Result<()> {
    let schema = read_schema(path)
        .with_context(|| format!("failed to read schema from {}", path))?;

    let sql = generate_sql(&schema)
        .context("failed to generate SQL")?;

    execute_sql(&sql)
        .context("failed to execute migration")?;

    Ok(())
}
```

## Error Design Principles

### Make Errors Actionable

```rust
// ✅ Good: Error tells you what went wrong and how to fix it
#[error("field '{field}' requires type {expected}, got {actual}. Use @{expected} attribute or change the type.")]
InvalidFieldType {
    field: String,
    expected: &'static str,
    actual: String,
}

// ❌ Bad: Vague error
#[error("invalid field")]
InvalidField,
```

### Include Relevant Data

```rust
// ✅ Good: Error includes debugging information
#[error("query failed after {attempts} attempts (last error: {last_error})")]
RetryExhausted {
    attempts: u32,
    last_error: String,
    query: String, // Include the query for debugging
}

// ❌ Bad: No context
#[error("retry failed")]
RetryFailed,
```

### Preserve Error Chain

```rust
#[derive(Error, Debug)]
pub enum DatabaseError {
    // ✅ Good: Preserves source error
    #[error("connection pool exhausted")]
    PoolExhausted(#[source] deadpool::PoolError),

    // ✅ Good: #[from] for automatic conversion
    #[error("postgres error")]
    Postgres(#[from] tokio_postgres::Error),
}
```

## Handling Specific Error Cases

### Not Found vs Error

```rust
// Return Option for "not found" when that's a valid state
pub async fn find_by_id(id: i64) -> Result<Option<User>, QueryError> {
    match self.query_one(&sql, &[&id]).await {
        Ok(row) => Ok(Some(User::from_row(row)?)),
        Err(e) if e.is_no_rows() => Ok(None),
        Err(e) => Err(e.into()),
    }
}

// Return Error for "not found" when it indicates a problem
pub async fn get_by_id(id: i64) -> Result<User, QueryError> {
    self.find_by_id(id)
        .await?
        .ok_or(QueryError::NotFound { entity: "User", id })
}
```

### Validation Errors

```rust
#[derive(Error, Debug)]
pub enum ValidationError {
    #[error("validation failed")]
    Multiple(Vec<FieldError>),
}

#[derive(Debug)]
pub struct FieldError {
    pub field: String,
    pub message: String,
    pub code: &'static str,
}

impl ValidationError {
    pub fn single(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Multiple(vec![FieldError {
            field: field.into(),
            message: message.into(),
            code: "invalid",
        }])
    }

    pub fn builder() -> ValidationErrorBuilder {
        ValidationErrorBuilder::new()
    }
}

// Usage
let mut errors = ValidationError::builder();
if email.is_empty() {
    errors.add("email", "Email is required", "required");
}
if password.len() < 8 {
    errors.add("password", "Password must be at least 8 characters", "min_length");
}
errors.build()?; // Returns Ok(()) or Err(ValidationError)
```

### Database Constraint Errors

```rust
impl From<tokio_postgres::Error> for QueryError {
    fn from(e: tokio_postgres::Error) -> Self {
        // Parse PostgreSQL error codes
        if let Some(db_err) = e.as_db_error() {
            match db_err.code().code() {
                "23505" => return QueryError::UniqueViolation {
                    constraint: db_err.constraint().map(String::from),
                    detail: db_err.detail().map(String::from),
                },
                "23503" => return QueryError::ForeignKeyViolation {
                    constraint: db_err.constraint().map(String::from),
                },
                "23502" => return QueryError::NotNullViolation {
                    column: db_err.column().map(String::from),
                },
                _ => {}
            }
        }
        QueryError::Database(e)
    }
}
```

## Testing Error Handling

### Test Error Cases Explicitly

```rust
#[test]
fn test_invalid_filter_error() {
    let result = parse_filter("invalid syntax");

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(matches!(err, FilterError::Parse { .. }));
    assert!(err.to_string().contains("invalid syntax"));
}

#[test]
fn test_error_source_chain() {
    let inner = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
    let err = SchemaError::Io {
        path: "schema.prax".into(),
        source: inner,
    };

    // Verify source is accessible
    assert!(err.source().is_some());
    assert!(err.to_string().contains("schema.prax"));
}
```

### Test Error Recovery

```rust
#[tokio::test]
async fn test_retry_on_transient_error() {
    let mut attempts = 0;
    let result = retry_with_backoff(|| async {
        attempts += 1;
        if attempts < 3 {
            Err(QueryError::Connection("transient".into()))
        } else {
            Ok("success")
        }
    }).await;

    assert!(result.is_ok());
    assert_eq!(attempts, 3);
}
```

## Logging Errors

### Log at Appropriate Levels

```rust
use tracing::{error, warn, debug};

pub async fn execute(&self) -> Result<(), QueryError> {
    match self.try_execute().await {
        Ok(()) => Ok(()),
        Err(e) if e.is_transient() => {
            warn!(error = %e, "transient error, will retry");
            self.retry().await
        }
        Err(e) => {
            error!(error = %e, query = %self.sql, "query execution failed");
            Err(e)
        }
    }
}
```

### Include Structured Context

```rust
use tracing::{error, instrument};

#[instrument(skip(self), fields(user_id = %id))]
pub async fn find_user(&self, id: i64) -> Result<User, QueryError> {
    self.query_one(&sql, &[&id]).await.map_err(|e| {
        error!(error = %e, "failed to find user");
        e
    })
}
```

## Summary

1. **Use `thiserror`** for library error types
2. **Preserve error chains** with `#[source]` and `#[from]`
3. **Make errors actionable** with clear messages and context
4. **Use `?` operator** for clean propagation
5. **Add context** with `map_err` or `anyhow::Context`
6. **Test error cases** explicitly
7. **Log appropriately** with structured context


## Cursor rule: `.cursor/rules/git-flow.mdc`

_Git-flow branching strategy and workflow conventions for the Prax project_

Applies to: `["**/*"]`

# Git-Flow Workflow

This project follows the **Git-Flow** branching model for organized development and releases.

## Branch Structure

```
main (production)
  │
  └── develop (integration)
        │
        ├── feature/* (new features)
        ├── bugfix/* (non-critical fixes)
        ├── release/* (release preparation)
        └── hotfix/* (critical production fixes)
```

## Primary Branches

### `main`
- **Purpose**: Production-ready code only
- **Protection**: Protected, requires PR approval
- **Deploys to**: Production / crates.io releases
- **Never commit directly** - only merge from `release/*` or `hotfix/*`

### `develop`
- **Purpose**: Integration branch for features
- **Contains**: Latest delivered development changes
- **Base for**: All `feature/*` and `bugfix/*` branches
- **Merged to**: `release/*` branches

## Supporting Branches

### Feature Branches: `feature/<name>`

For new features and enhancements.

```bash
# Create feature branch from develop
git checkout develop
git pull origin develop
git checkout -b feature/query-builder

# Work on feature...
git add .
git commit -m "feat(query): implement basic query builder"

# Keep up to date with develop
git fetch origin develop
git rebase origin/develop

# Push and create PR to develop
git push -u origin feature/query-builder
```

**Naming convention**: `feature/<scope>-<description>`
- `feature/query-builder`
- `feature/postgres-connection-pool`
- `feature/schema-parser`

### Bugfix Branches: `bugfix/<name>`

For non-critical bug fixes during development.

```bash
git checkout develop
git checkout -b bugfix/connection-timeout

# Fix the bug...
git commit -m "fix(postgres): handle connection timeout gracefully"

git push -u origin bugfix/connection-timeout
```

**Naming convention**: `bugfix/<scope>-<description>`
- `bugfix/query-null-handling`
- `bugfix/migration-rollback`

### Release Branches: `release/<version>`

For preparing a new production release.

```bash
# Create release branch from develop
git checkout develop
git checkout -b release/0.1.0

# Update version in Cargo.toml
# Update CHANGELOG.md
# Final testing and bug fixes only

git commit -m "chore(release): prepare v0.1.0"

# Merge to main
git checkout main
git merge --no-ff release/0.1.0
git tag -a v0.1.0 -m "Release v0.1.0"

# Merge back to develop
git checkout develop
git merge --no-ff release/0.1.0

# Delete release branch
git branch -d release/0.1.0
```

**Naming convention**: `release/<semver>`
- `release/0.1.0`
- `release/1.0.0`
- `release/2.3.1`

### Hotfix Branches: `hotfix/<name>`

For critical production fixes that can't wait.

```bash
# Create hotfix from main
git checkout main
git checkout -b hotfix/security-vulnerability

# Fix the issue...
git commit -m "fix(security): patch SQL injection vulnerability"

# Merge to main
git checkout main
git merge --no-ff hotfix/security-vulnerability
git tag -a v0.1.1 -m "Hotfix v0.1.1"

# Merge to develop (or current release branch)
git checkout develop
git merge --no-ff hotfix/security-vulnerability

# Delete hotfix branch
git branch -d hotfix/security-vulnerability
```

**Naming convention**: `hotfix/<description>`
- `hotfix/security-patch`
- `hotfix/critical-query-fix`

## Workflow Diagrams

### Feature Development
```
develop ─────●─────────────●─────────────●───────
              \           /
feature/*      ●────●────●
               ↑    ↑    ↑
            commits on feature
```

### Release Process
```
main    ─────────────────────●────── (v0.1.0)
                            /
release/0.1.0    ●────●────●
                /
develop ───●───●─────────────●───────
```

### Hotfix Process
```
main    ────●─────────────●────── (v0.1.1)
             \           /
hotfix/*      ●────●────●
                        \
develop ─────────────────●───────
```

## Commit Message Format

All commits **MUST** follow [Conventional Commits](https://conventionalcommits.org/) with **REQUIRED scope**:

```
<type>(<scope>): <description>

[optional body]

[optional footer]
```

### ⚠️ Scope is REQUIRED

Every commit must include a scope. This is enforced by the `commit-msg` git hook.

**Valid scopes:**
- Crate names: `query`, `postgres`, `mysql`, `sqlite`, `mssql`, `mongodb`, `duckdb`, `schema`, `codegen`, `migrate`, `cli`
- Special: `deps`, `ci`, `docs`, `release`, `security`

### Valid Types

| Type | Description | Example |
|------|-------------|---------|
| `feat` | New feature | `feat(query): add nested filter support` |
| `fix` | Bug fix | `fix(postgres): handle connection timeout` |
| `docs` | Documentation | `docs(readme): add installation guide` |
| `style` | Formatting | `style(query): fix indentation` |
| `refactor` | Code refactoring | `refactor(schema): simplify parser logic` |
| `perf` | Performance | `perf(query): optimize SQL generation` |
| `test` | Tests | `test(postgres): add connection tests` |
| `build` | Build system | `build(deps): update tokio to 1.35` |
| `ci` | CI/CD | `ci(github): add benchmark workflow` |
| `chore` | Maintenance | `chore(release): bump version to 0.3.3` |
| `revert` | Revert commit | `revert(query): undo filter changes` |

### Breaking Changes

Add `!` before `:` for breaking changes:
```
feat(api)!: change query builder interface
```

### Types by Branch

| Branch Type | Common Commit Types |
|-------------|---------------------|
| `feature/*` | `feat`, `test`, `docs` |
| `bugfix/*` | `fix`, `test` |
| `release/*` | `chore`, `docs`, `fix` |
| `hotfix/*` | `fix`, `security` |

## Pull Request Guidelines

### PR Titles
Follow the same conventional commit format:
- `feat(query): add nested filter support`
- `fix(postgres): resolve connection leak`

### PR Checklist
- [ ] Branch is up to date with target branch
- [ ] All tests pass (`cargo test --all-features`)
- [ ] Code is formatted (`cargo fmt`)
- [ ] No clippy warnings (`cargo clippy`)
- [ ] CHANGELOG.md updated (for features/fixes)
- [ ] Documentation updated if needed

### Merge Strategy

| Target Branch | Merge Type | Reason |
|---------------|------------|--------|
| `develop` ← `feature/*` | Squash | Clean history |
| `develop` ← `bugfix/*` | Squash | Clean history |
| `main` ← `release/*` | Merge commit | Preserve release history |
| `main` ← `hotfix/*` | Merge commit | Preserve hotfix history |
| `develop` ← `release/*` | Merge commit | Sync changes |
| `develop` ← `hotfix/*` | Merge commit | Sync changes |

## Version Tagging

Tags are created on `main` branch only:

```bash
# Annotated tags for releases
git tag -a v0.1.0 -m "Release v0.1.0: Initial release with PostgreSQL support"

# Push tags
git push origin v0.1.0
# or push all tags
git push origin --tags
```

### Semantic Versioning

```
v<MAJOR>.<MINOR>.<PATCH>

MAJOR: Breaking API changes
MINOR: New features (backwards compatible)
PATCH: Bug fixes (backwards compatible)
```

**Pre-release versions**:
- `v0.1.0-alpha.1`
- `v0.1.0-beta.1`
- `v0.1.0-rc.1`

## Quick Reference

### Starting New Work

```bash
# Feature
git checkout develop && git pull
git checkout -b feature/my-feature

# Bugfix
git checkout develop && git pull
git checkout -b bugfix/my-fix

# Hotfix (critical)
git checkout main && git pull
git checkout -b hotfix/critical-fix
```

### Keeping Branch Updated

```bash
# Rebase feature on latest develop
git fetch origin develop
git rebase origin/develop

# Resolve conflicts if any, then:
git push --force-with-lease
```

### Finishing Work

```bash
# Push branch and create PR
git push -u origin <branch-name>
# Create PR via GitHub UI or CLI

# After PR merged, clean up
git checkout develop
git pull
git branch -d <branch-name>
```

## Branch Protection Rules

### `main`
- Require pull request reviews (1+ approval)
- Require status checks to pass
- Require linear history (no merge commits from features)
- Restrict who can push (maintainers only)

### `develop`
- Require pull request reviews
- Require status checks to pass
- Allow squash merging


## Cursor rule: `.cursor/rules/multi-tenancy.mdc`

# Multi-Tenancy Guidelines

This project provides multi-tenancy support with multiple isolation strategies. Follow these guidelines for secure and performant tenant isolation.

## Isolation Strategies

### Row-Level Security (RLS) - Recommended

Most efficient for shared-table tenancy:

```rust
use prax_query::tenant::{RlsManager, RlsConfig};

// Configure RLS
let rls = RlsManager::new(
    RlsConfig::new("tenant_id")
        .with_session_variable("app.current_tenant")
        .add_tables(["users", "orders", "products"])
);

// Apply to connection
rls.set_tenant(&conn, tenant_id).await?;

// All queries automatically filtered by tenant_id
let users = client.user().find_many().exec().await?;
// SQL: SELECT * FROM users WHERE tenant_id = current_setting('app.current_tenant')
```

### Schema-Based Isolation

For stronger isolation with separate schemas:

```rust
use prax_query::tenant::{TenantPoolManager, SchemaIsolation};

let manager = TenantPoolManager::new(base_pool)
    .with_isolation(SchemaIsolation::Schema);

// Get tenant-specific pool
let pool = manager.get_pool(tenant_id).await?;
// Queries run against schema_{tenant_id}.users
```

### Database-Based Isolation

Maximum isolation with separate databases:

```rust
let manager = TenantPoolManager::new(base_pool)
    .with_isolation(SchemaIsolation::Database);

// Each tenant has own database
let pool = manager.get_pool(tenant_id).await?;
// Connects to tenant_{tenant_id} database
```

## Context Propagation

### Use Task-Local Context (Zero Allocation)

```rust
use prax_query::tenant::task_local::with_tenant;

// Set tenant context for async block
with_tenant("tenant-123", async {
    // All queries in this block use tenant-123
    let users = client.user().find_many().exec().await?;
    let orders = client.order().find_many().exec().await?;
    Ok(())
}).await?;

// ❌ Bad: Manual tenant filter on each query
let users = client.user()
    .find_many()
    .where_(user::tenant_id::equals(tenant_id)) // Easy to forget!
    .exec()
    .await?;
```

### Thread-Local for Sync Code

```rust
use prax_query::tenant::thread_local::{set_tenant, get_tenant};

// In request handler
set_tenant(tenant_id);

// Deep in call stack
let current = get_tenant().expect("tenant not set");
```

## Security Rules

### Never Trust Client-Provided Tenant ID

```rust
// ✅ Good: Extract tenant from authenticated session
async fn handler(session: Session, req: Request) -> Response {
    let tenant_id = session.tenant_id(); // From verified JWT/session

    with_tenant(tenant_id, async {
        // Process request
    }).await
}

// ❌ DANGEROUS: Accept tenant from request
async fn handler(req: Request) -> Response {
    let tenant_id = req.header("X-Tenant-ID"); // Attacker can set this!
    // ...
}
```

### Validate Cross-Tenant References

```rust
// ✅ Good: Validate foreign key belongs to same tenant
async fn create_order(tenant_id: TenantId, user_id: UserId) -> Result<Order> {
    // Verify user belongs to tenant
    let user = client.user()
        .find_unique(user::id::equals(user_id))
        .exec()
        .await?
        .ok_or(Error::NotFound)?;

    if user.tenant_id != tenant_id {
        return Err(Error::CrossTenantAccess);
    }

    // Proceed with order creation
}
```

### Enable RLS at Database Level

```sql
-- PostgreSQL RLS setup
ALTER TABLE users ENABLE ROW LEVEL SECURITY;

CREATE POLICY tenant_isolation ON users
    USING (tenant_id = current_setting('app.current_tenant')::int);

-- Force RLS even for table owners
ALTER TABLE users FORCE ROW LEVEL SECURITY;
```

## Performance Patterns

### Use Statement Caching

```rust
use prax_query::tenant::cache::StatementCache;

// Global mode: Share prepared statements across tenants (with RLS)
let cache = StatementCache::global();

// Per-tenant mode: Separate caches for schema-based isolation
let cache = StatementCache::per_tenant(1000); // LRU size per tenant
```

### Use Tenant Cache with TTL

```rust
use prax_query::tenant::cache::ShardedTenantCache;

// High-concurrency tenant cache
let cache = ShardedTenantCache::high_concurrency(10_000);

// With TTL
cache.insert(tenant_id, config, Duration::from_secs(300));

// Get or load
let config = cache.get_or_insert(tenant_id, || {
    load_tenant_config(tenant_id)
}).await?;
```

### Warm Up Tenant Pools

```rust
// Pre-warm pools for known active tenants
let active_tenants = get_active_tenant_ids().await?;
manager.warmup(&active_tenants).await?;

// Lazy pools for less active tenants
// Created on first access, evicted after idle timeout
```

## Testing Multi-Tenancy

### Test Tenant Isolation

```rust
#[tokio::test]
async fn test_tenant_isolation() {
    let tenant_a = create_test_tenant().await;
    let tenant_b = create_test_tenant().await;

    // Create data for tenant A
    with_tenant(tenant_a.id, async {
        client.user().create(user::create! { name: "Alice" }).exec().await?;
        Ok::<_, Error>(())
    }).await?;

    // Verify tenant B cannot see tenant A's data
    with_tenant(tenant_b.id, async {
        let users = client.user().find_many().exec().await?;
        assert!(users.is_empty(), "Tenant B should not see Tenant A's users");
        Ok::<_, Error>(())
    }).await?;
}

#[tokio::test]
async fn test_cross_tenant_access_denied() {
    let tenant_a = create_test_tenant().await;
    let tenant_b = create_test_tenant().await;

    // Create user in tenant A
    let user = with_tenant(tenant_a.id, async {
        client.user().create(user::create! { name: "Alice" }).exec().await
    }).await?;

    // Try to access from tenant B
    let result = with_tenant(tenant_b.id, async {
        client.user().find_unique(user::id::equals(user.id)).exec().await
    }).await?;

    assert!(result.is_none(), "Should not find user from different tenant");
}
```

### Test Context Propagation

```rust
#[tokio::test]
async fn test_tenant_context_propagation() {
    with_tenant("test-tenant", async {
        // Spawn tasks
        let handles: Vec<_> = (0..10)
            .map(|_| tokio::spawn(async {
                // Context should be available in spawned tasks
                let tenant = get_current_tenant();
                assert_eq!(tenant, Some("test-tenant"));
            }))
            .collect();

        futures::future::join_all(handles).await;
        Ok::<_, Error>(())
    }).await?;
}
```

## Common Patterns

### Tenant Middleware

```rust
// Axum middleware example
async fn tenant_middleware(
    session: Session,
    mut request: Request,
    next: Next,
) -> Response {
    let tenant_id = session.tenant_id();

    // Set tenant context
    with_tenant(tenant_id, async {
        next.run(request).await
    }).await
}
```

### Multi-Tenant Migrations

```rust
// Run migration for all tenants
async fn migrate_all_tenants(migration: &Migration) -> Result<()> {
    let tenants = list_all_tenants().await?;

    for tenant in tenants {
        let pool = manager.get_pool(tenant.id).await?;
        migration.run(&pool).await?;
    }

    Ok(())
}
```

## Summary

1. **Use RLS** for shared-table multi-tenancy (most efficient)
2. **Use task-local context** for zero-allocation tenant propagation
3. **Never trust client-provided tenant IDs** - extract from authenticated session
4. **Validate cross-tenant references** before creating relationships
5. **Enable database-level RLS** as defense in depth
6. **Cache per-tenant data** with TTL for performance
7. **Test isolation thoroughly** - both positive and negative cases


## Cursor rule: `.cursor/rules/no-skip-hooks.mdc`

_Never bypass git hooks - fix the underlying issues instead_

Applies to: `["**/*"]`

# Never Skip Git Hooks

**CRITICAL**: Never use `--no-verify` or `-n` flags to bypass git hooks. Always fix the underlying issue.

## Forbidden Commands

```bash
# ❌ NEVER DO THIS
git commit --no-verify
git commit -n
git push --no-verify
git merge --no-verify

# ❌ ALSO NEVER DO THIS
HUSKY=0 git commit
HUSKY_SKIP_HOOKS=1 git commit
```

## Why This Matters

The git hooks enforce:
- **pre-commit**: Code formatting (`cargo fmt`) and linting (`cargo clippy`)
- **pre-push**: Full test suite passes
- **commit-msg**: Conventional commit format

Bypassing these hooks:
- Introduces unformatted code to the repository
- Allows linting errors into the codebase
- Breaks CI/CD pipelines
- Creates inconsistent commit history
- Causes problems for other contributors

## What to Do Instead

### Hook: pre-commit (format/lint issues)

```bash
# Fix formatting
cargo fmt --all

# Fix clippy warnings
cargo clippy --fix --allow-dirty --allow-staged

# Then commit normally
git commit -m "feat: your message"
```

### Hook: pre-push (test failures)

```bash
# Run tests and fix failures
cargo test --all-features

# Check specific failing test
cargo test test_name -- --nocapture

# Fix the issue, then push
git push
```

### Hook: commit-msg (message format)

```bash
# Use correct format: type(scope): description
git commit -m "feat(query): add nested filter support"
git commit -m "fix(postgres): handle connection timeout"
git commit -m "docs: update README examples"

# Valid types: feat, fix, docs, style, refactor, perf, test, build, ci, chore, revert
```

## Common Issues and Solutions

### "Formatting check failed"

```bash
# Problem: Code not formatted
cargo fmt --all
git add -u
git commit -m "your message"
```

### "Clippy found issues"

```bash
# Problem: Linting warnings
cargo clippy --all-targets --all-features
# Read the warnings and fix them, then:
git add -u
git commit -m "your message"
```

### "Tests failed"

```bash
# Problem: Tests don't pass
cargo test --all-features
# Fix failing tests, then:
git add -u
git commit -m "your message"
git push
```

### "Invalid commit message"

```bash
# Problem: Wrong format
# Instead of: "fixed the bug"
# Use: "fix(module): resolve specific issue"

git commit --amend -m "fix(postgres): handle null values in query results"
```

### "I need to commit work-in-progress"

```bash
# Use a WIP branch instead of bypassing hooks
git stash
# or
git checkout -b wip/my-feature
# Make a proper commit when ready
```

## Emergency Situations

If you genuinely believe you need to bypass hooks (you almost certainly don't):

1. **Stop and ask**: Is this really necessary?
2. **Document why**: Create an issue explaining the situation
3. **Get approval**: Discuss with the team first
4. **Fix immediately**: The next commit must fix what you bypassed

**There is virtually never a legitimate reason to skip hooks in this project.**

## For AI Assistants

When a user asks to bypass git hooks or encounters hook failures:

1. **Never suggest** `--no-verify`, `-n`, or similar flags
2. **Diagnose the actual problem** by reading the error message
3. **Provide the fix** for the underlying issue
4. **Explain why** hooks exist and why bypassing is harmful

Example response:
```
I see the pre-commit hook failed due to formatting issues.
Let me fix that:

cargo fmt --all

Now the commit should succeed. Never use --no-verify
as it bypasses important quality checks.
```


## Cursor rule: `.cursor/rules/performance.mdc`

# Performance Optimization Guidelines

This project prioritizes performance. Follow these guidelines to write efficient, allocation-conscious code.

## Memory Optimization

### Use String Interning

For repeated identifiers (column names, table names):

```rust
use prax_query::mem_optimize::interning::{GlobalInterner, ScopedInterner};

// ✅ Good: Intern repeated field names
let interner = GlobalInterner::get();
let field = interner.intern("user_id"); // Shared across all uses

// ✅ Good: Use scoped interner for request-local interning
let mut scoped = ScopedInterner::new();
for column in &columns {
    let interned = scoped.intern(column);
    // Memory freed when scoped is dropped
}

// ❌ Bad: Repeated String allocations
for _ in 0..1000 {
    let field = "user_id".to_string(); // 1000 allocations!
}
```

### Use Arena Allocation

For query building with many temporary allocations:

```rust
use prax_query::mem_optimize::arena::QueryArena;

// ✅ Good: Arena-based query building
let arena = QueryArena::new();
let sql = arena.scope(|scope| {
    let filter = scope.and(vec![
        scope.eq("active", true),
        scope.or(vec![
            scope.gt("age", 18),
            scope.is_not_null("email"),
        ]),
    ]);
    scope.build_select("users", filter)
});
// Arena memory freed, sql String is owned

// ❌ Bad: Many small heap allocations
let filter = Filter::and(vec![
    Filter::Equals("active".into(), FilterValue::Bool(true)),
    Filter::or(vec![
        Filter::Gt("age".into(), FilterValue::Int(18)),
        Filter::IsNotNull("email".into()),
    ]),
]);
```

### Use Lazy Parsing

For large schemas where not all data is needed:

```rust
use prax_query::mem_optimize::lazy::LazySchema;

// ✅ Good: Lazy schema - only parses what you access
let schema = LazySchema::from_json(large_json)?;

// Table names available immediately (no parsing)
for name in schema.table_names() {
    if name == "users" {
        // Only now parse the users table
        let table = schema.get_table(name)?;
        for col in table.columns() {
            // Columns parsed on first access
        }
    }
}

// ❌ Bad: Parse everything eagerly
let schema: DatabaseSchema = serde_json::from_str(large_json)?;
// All tables and columns parsed upfront
```

### Avoid Unnecessary Allocations

```rust
// ✅ Good: Use references and slices
fn process(data: &[u8]) -> &str { ... }

// ✅ Good: Use Cow for conditional ownership
use std::borrow::Cow;

fn normalize(s: &str) -> Cow<'_, str> {
    if needs_normalization(s) {
        Cow::Owned(s.to_lowercase())
    } else {
        Cow::Borrowed(s)
    }
}

// ✅ Good: Reuse buffers
let mut buf = String::with_capacity(1024);
for item in items {
    buf.clear();
    write!(&mut buf, "{}", item)?;
    process(&buf);
}

// ❌ Bad: Allocate in a loop
for item in items {
    let s = format!("{}", item); // Allocation each iteration
    process(&s);
}
```

### Use SmallVec for Small Collections

```rust
use smallvec::SmallVec;

// ✅ Good: Inline storage for common case
let columns: SmallVec<[&str; 8]> = SmallVec::new();
// No heap allocation until > 8 elements

// ❌ Bad: Always heap allocate
let columns: Vec<&str> = Vec::new();
```

## Query Performance

### Batch Operations

```rust
// ✅ Good: Single multi-row INSERT
let mut builder = SqlBuilder::postgres();
builder.push("INSERT INTO events (user_id, type) VALUES ");
for (i, event) in events.iter().enumerate() {
    if i > 0 { builder.push(", "); }
    builder.push("(");
    builder.push_param(FilterValue::Int(event.user_id));
    builder.push(", ");
    builder.push_param(FilterValue::String(event.event_type.clone()));
    builder.push(")");
}

// ❌ Bad: Multiple INSERT statements
for event in events {
    execute("INSERT INTO events (user_id, type) VALUES ($1, $2)", &[...]).await?;
}
```

### Use Prepared Statements

```rust
// ✅ Good: Prepare once, execute many
let stmt = client.prepare("SELECT * FROM users WHERE id = $1").await?;
for id in ids {
    let row = client.query_one(&stmt, &[&id]).await?;
}

// ❌ Bad: Re-parse query each time
for id in ids {
    let row = client.query_one("SELECT * FROM users WHERE id = $1", &[&id]).await?;
}
```

### Efficient IN Clauses

```rust
// ✅ Good: Use ANY with array parameter (PostgreSQL)
let sql = "SELECT * FROM users WHERE id = ANY($1)";
let ids: Vec<i64> = vec![1, 2, 3, 4, 5];
client.query(sql, &[&ids]).await?;

// ✅ Good: Generate IN clause efficiently
let placeholders = postgres_in_pattern(1, ids.len());
let sql = format!("SELECT * FROM users WHERE id IN ({})", placeholders);

// ❌ Bad: Dynamic SQL with many parameters
let sql = format!(
    "SELECT * FROM users WHERE id IN ({})",
    ids.iter().map(|_| "?").collect::<Vec<_>>().join(", ")
);
```

### Select Only Needed Columns

```rust
// ✅ Good: Select specific columns
let select = Select::fields(["id", "email", "name"]);

// ❌ Bad: Select all columns when only few needed
let select = Select::all(); // SELECT * includes unused columns
```

## Async Performance

### Use Concurrent Execution

```rust
use prax_query::async_optimize::ConcurrentExecutor;

// ✅ Good: Execute independent queries in parallel
let executor = ConcurrentExecutor::new(config);
let results = executor.execute_batch(vec![
    || async { fetch_users().await },
    || async { fetch_posts().await },
    || async { fetch_comments().await },
]).await;

// ❌ Bad: Sequential when parallel is possible
let users = fetch_users().await?;
let posts = fetch_posts().await?;
let comments = fetch_comments().await?;
```

### Use Connection Pooling

```rust
// ✅ Good: Pool connections
let pool = Pool::builder()
    .max_size(20)
    .build(manager)
    .await?;

// Get connection from pool
let conn = pool.get().await?;

// ❌ Bad: New connection per query
let conn = PgConnection::connect(&url).await?;
```

### Stream Large Results

```rust
use futures::StreamExt;

// ✅ Good: Stream rows without loading all into memory
let mut stream = client.query_raw(&sql, &[]).await?;
while let Some(row) = stream.next().await {
    process_row(row?);
}

// ❌ Bad: Load all rows into memory
let rows = client.query(&sql, &[]).await?; // All in memory
for row in rows {
    process_row(row);
}
```

## Profiling and Measurement

### Use Benchmarks

```bash
# Run benchmarks
cargo bench --package prax-query

# Compare against baseline
cargo bench -- --save-baseline main
cargo bench -- --load-baseline main
```

### Profile with flamegraph

```bash
# Install flamegraph
cargo install flamegraph

# Generate flamegraph
cargo flamegraph --bench throughput_bench -- --bench
```

### Measure Allocations

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn test_allocation_count() {
        // Use a counting allocator or DHAT
        let stats = arena.stats();
        assert!(stats.allocations < 10, "Too many allocations");
    }
}
```

## Hot Path Optimization

### Inline Small Functions

```rust
// ✅ Good: Inline hint for hot path
#[inline]
pub fn intern(&self, s: &str) -> InternedStr {
    // Fast path: check cache
    if let Some(existing) = self.cache.get(s) {
        return existing.clone();
    }
    // Slow path: allocate
    self.intern_slow(s)
}

#[inline(never)] // Don't inline slow path
fn intern_slow(&self, s: &str) -> InternedStr {
    // Complex allocation logic
}
```

### Avoid Dynamic Dispatch in Loops

```rust
// ✅ Good: Monomorphization
fn process_filters<F: FilterTrait>(filters: &[F]) {
    for f in filters {
        f.to_sql(); // Static dispatch
    }
}

// ❌ Bad in hot path: Dynamic dispatch
fn process_filters(filters: &[Box<dyn FilterTrait>]) {
    for f in filters {
        f.to_sql(); // Virtual call each iteration
    }
}
```

### Pre-allocate with Capacity

```rust
// ✅ Good: Pre-allocate when size known
let mut results = Vec::with_capacity(expected_count);
let mut sql = String::with_capacity(256);

// ❌ Bad: Grow incrementally
let mut results = Vec::new(); // Reallocates as it grows
```

## Summary Checklist

- [ ] Use string interning for repeated identifiers
- [ ] Use arena allocation for temporary query building
- [ ] Use lazy parsing for large schemas
- [ ] Batch database operations
- [ ] Use prepared statements
- [ ] Select only needed columns
- [ ] Use concurrent execution for independent operations
- [ ] Stream large result sets
- [ ] Profile before optimizing
- [ ] Benchmark changes for regression


## Cursor rule: `.cursor/rules/profiling.mdc`

# Memory Profiling Guidelines

This project includes comprehensive memory profiling tools for detecting leaks and analyzing memory usage patterns.

## Profiling Module Overview

The `prax_query::profiling` module provides:

- **Allocation Tracking**: Track every allocation/deallocation
- **Memory Snapshots**: Capture and compare memory state
- **Leak Detection**: Identify memory that wasn't freed
- **Heap Profiling**: System-level heap analysis

## Quick Start

```rust
use prax_query::profiling::{MemoryProfiler, with_profiling, enable_profiling};

// Option 1: Use with_profiling wrapper
let (result, leak_report) = with_profiling(|| {
    // Your code here
    perform_operations()
});

if leak_report.has_leaks() {
    eprintln!("⚠️  Potential leaks: {}", leak_report);
}

// Option 2: Use MemoryProfiler directly
let profiler = MemoryProfiler::new();
let before = profiler.snapshot();

// ... do work ...

let after = profiler.snapshot();
let diff = after.diff(&before);
println!("{}", diff.report());
```

## Enabling Profiling

Profiling has runtime overhead. Enable only when needed:

```rust
// Enable globally
prax_query::profiling::enable_profiling();

// Or use RAII guard
let detector = LeakDetector::new();
let _guard = detector.start(); // Enables profiling
// ... profiling active ...
// guard dropped - profiling disabled
```

## Leak Detection Patterns

### Detecting Repeated Allocations

```rust
use prax_query::profiling::{LeakDetector, LeakSeverity};
use std::time::Duration;

let detector = LeakDetector::with_threshold(Duration::from_secs(30));
let report = detector.analyze(&tracker);

for leak in &report.potential_leaks {
    match leak.severity {
        LeakSeverity::High => eprintln!("🔴 High severity: {}", leak.pattern.description()),
        LeakSeverity::Medium => eprintln!("🟡 Medium: {}", leak.pattern.description()),
        LeakSeverity::Low => eprintln!("🟢 Low: {}", leak.pattern.description()),
    }
}
```

### Memory Growth Analysis

```rust
use prax_query::profiling::snapshot::SnapshotSeries;

let mut series = SnapshotSeries::new(100);

// Periodically capture snapshots
for _ in 0..10 {
    series.add(profiler.snapshot());
    tokio::time::sleep(Duration::from_secs(1)).await;
}

if series.has_growth_trend() {
    eprintln!("⚠️  Memory growing at {:.2} bytes/sec", series.growth_rate());
}
```

## Testing for Leaks

### In Unit Tests

```rust
#[test]
fn test_no_memory_leak() {
    let (_, report) = prax_query::profiling::with_profiling(|| {
        // Create and drop resources
        let filter = Filter::and(vec![
            Filter::Equals("id".into(), FilterValue::Int(1)),
            Filter::Equals("status".into(), FilterValue::String("active".into())),
        ]);
        drop(filter);
    });

    assert!(!report.has_high_severity_leaks(), "Memory leak detected: {}", report);
}
```

### In Integration Tests

```rust
#[tokio::test]
async fn test_connection_pool_no_leak() {
    let profiler = MemoryProfiler::new();
    let before = profiler.snapshot();

    // Simulate many connections
    for _ in 0..100 {
        let conn = pool.get().await.unwrap();
        conn.query("SELECT 1").await.unwrap();
        drop(conn);
    }

    let after = profiler.snapshot();
    let diff = after.diff(&before);

    assert!(
        diff.bytes_delta < 10_000,  // Allow some variance
        "Excessive memory growth: {} bytes", diff.bytes_delta
    );
}
```

## Benchmark Memory Usage

```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn bench_memory_efficiency(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory");

    group.bench_function("interned_vs_string", |b| {
        let interner = GlobalInterner::get_instance();

        b.iter(|| {
            // Compare interned vs regular strings
            for _ in 0..100 {
                black_box(interner.intern("field_name"));
            }
        });
    });
}
```

## CI Integration

The `.github/workflows/memory-check.yml` workflow runs:

1. **Leak Detection Tests**: Run profiling module tests
2. **Valgrind Analysis**: Check for definite memory leaks
3. **AddressSanitizer**: Runtime memory error detection
4. **DHAT Profiling**: Heap allocation analysis

## Using TrackedAllocator

For comprehensive tracking, use the custom allocator:

```rust
// In main.rs or lib.rs (ONE location only)
use prax_query::profiling::TrackedAllocator;

#[global_allocator]
static ALLOC: TrackedAllocator = TrackedAllocator::new();

// Now all allocations are tracked automatically
fn main() {
    prax_query::profiling::enable_profiling();

    // ... your code ...

    let stats = prax_query::profiling::GLOBAL_TRACKER.stats();
    println!("Total allocations: {}", stats.total_allocations);
    println!("Current bytes: {}", stats.current_bytes);
    println!("Peak bytes: {}", stats.peak_bytes);
}
```

## Memory Optimization Tips

### Use String Interning

```rust
// ❌ Bad: Many allocations for repeated strings
for _ in 0..1000 {
    let field = "user_id".to_string();
}

// ✅ Good: Single allocation, shared reference
let interner = GlobalInterner::get_instance();
for _ in 0..1000 {
    let field = interner.intern("user_id");
}
```

### Use Arena Allocation

```rust
// ❌ Bad: Many small heap allocations
let filters: Vec<Filter> = items.iter()
    .map(|i| Filter::Equals("id".into(), FilterValue::Int(*i)))
    .collect();

// ✅ Good: Arena-allocated, freed together
let arena = QueryArena::new();
let filters = arena.scope(|s| {
    items.iter()
        .map(|i| s.eq("id", *i))
        .collect::<Vec<_>>()
});
```

### Use Buffer Pools

```rust
// ❌ Bad: Allocate new buffer each time
let mut sql = String::new();
write!(&mut sql, "SELECT * FROM {}", table)?;

// ✅ Good: Reuse pooled buffers
let mut buf = GLOBAL_BUFFER_POOL.get();
write!(&mut buf, "SELECT * FROM {}", table)?;
// Buffer returned to pool on drop
```

## Interpreting Reports

### Allocation Stats

```
Total allocations: 10,000
Current bytes: 50,000
Peak bytes: 100,000
Net allocations: 500  ← If positive, potential leak
```

### Leak Severity

- **High**: Many allocations of same size held long time
- **Medium**: Growing allocation count over time
- **Low**: Old allocations that might be intentional caching

### Heap Stats

```
RSS: 50 MB
Fragmentation: 15%  ← Over 30% is concerning
```

## Summary

1. **Enable profiling** only when debugging memory issues
2. **Use with_profiling** for scoped leak detection
3. **Compare snapshots** before/after operations
4. **Test for leaks** in unit and integration tests
5. **Use interning and arenas** to reduce allocations
6. **Monitor CI** for memory regressions


## Cursor rule: `.cursor/rules/readme.mdc`

_Guidelines for maintaining README.md documentation for the Prax ORM project_

Applies to: `["README.md"]`

# README Guidelines

This document provides guidelines for maintaining the project README to ensure clear, accurate, and helpful documentation.

## README Structure

The README should follow this structure:

```markdown
# Prax

<badges and shields>

<brief description>

## Features
## Installation
## Quick Start
## Query Operations
## Architecture
## CLI
## Comparison
## Contributing
## License
## Acknowledgments
```

## When to Update

### Always Update When:
- ✅ Adding new public API methods
- ✅ Changing installation requirements
- ✅ Adding new features mentioned in examples
- ✅ Changing CLI commands
- ✅ Adding new database backend support
- ✅ Adding new framework integrations
- ✅ Changing minimum Rust version

### Consider Updating When:
- 🤔 Fixing bugs that affect documented behavior
- 🤔 Improving performance significantly
- 🤔 Adding new optional features

### Don't Update For:
- ❌ Internal refactoring
- ❌ Test changes
- ❌ Minor bug fixes
- ❌ Dependency updates (unless breaking)

## Section Guidelines

### Header & Badges

```markdown
# Prax

<p align="center">
  <strong>A next-generation, type-safe ORM for Rust</strong>
</p>

<p align="center">
  <a href="https://crates.io/crates/prax"><img src="https://img.shields.io/crates/v/prax.svg" alt="crates.io"></a>
  <a href="https://docs.rs/prax"><img src="https://docs.rs/prax/badge.svg" alt="docs.rs"></a>
  <a href="https://github.com/quinnjr/prax/actions"><img src="https://github.com/quinnjr/prax/workflows/CI/badge.svg" alt="CI"></a>
  <img src="https://img.shields.io/badge/rust-1.85%2B-blue.svg" alt="Rust 1.85+">
  <a href="#license"><img src="https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg" alt="License"></a>
</p>
```

### Features Section

List features with emoji icons for scannability:

```markdown
## Features

- 🔒 **Type-Safe Queries** - Compile-time checked queries
- ⚡ **Async-First** - Built on Tokio
- 🎯 **Fluent API** - Intuitive query builder
- 🔗 **Relations** - Eager and lazy loading
- 📦 **Migrations** - Schema management
- 🛠️ **Code Generation** - Proc-macro models
- 🗄️ **Multi-Database** - PostgreSQL, MySQL, SQLite
- 🔌 **Framework Integration** - Armature, Axum, Actix-web
```

### Installation Section

Always show:
1. Basic installation
2. Feature flags for different backends
3. Minimum Rust version requirement

```markdown
## Installation

Add Prax to your `Cargo.toml`:

\`\`\`toml
[dependencies]
prax = "0.1"
\`\`\`

**Requires Rust 1.85+** (Edition 2024)

### Feature Flags

| Feature | Description |
|---------|-------------|
| `postgres` | PostgreSQL support (default) |
| `mysql` | MySQL support |
| `sqlite` | SQLite support |
| `runtime-tokio` | Tokio runtime (default) |
```

### Quick Start Section

Provide a complete, copy-pasteable example:

```markdown
## Quick Start

\`\`\`rust
use prax::prelude::*;

#[derive(Model)]
#[prax(table = "users")]
pub struct User {
    #[prax(id)]
    pub id: i32,
    pub email: String,
}

#[tokio::main]
async fn main() -> Result<(), prax::Error> {
    let client = PraxClient::new("postgresql://localhost/mydb").await?;

    let users = client.user().find_many().exec().await?;

    Ok(())
}
\`\`\`
```

### Code Examples

#### DO ✅

```rust
// Good: Complete, runnable example
use prax::prelude::*;

let users = client
    .user()
    .find_many()
    .where_(user::active::equals(true))
    .order_by(user::created_at::desc())
    .take(10)
    .exec()
    .await?;
```

#### DON'T ❌

```rust
// Bad: Incomplete, won't compile
let users = client.user().find_many()...
```

### API Documentation

When documenting query operations, use tables for clarity:

```markdown
## Query Operations

### Filtering

| Method | SQL Equivalent | Example |
|--------|---------------|---------|
| `equals(v)` | `= v` | `user::id::equals(1)` |
| `not_equals(v)` | `!= v` | `user::status::not_equals("banned")` |
| `contains(v)` | `LIKE %v%` | `user::name::contains("john")` |
| `gt(v)` | `> v` | `user::age::gt(18)` |
```

### Architecture Section

Keep the directory tree updated:

```markdown
## Architecture

\`\`\`
prax/
├── prax-core/           # Core types and traits
├── prax-schema/         # Schema parser
├── prax-codegen/        # Proc-macros
├── prax-query/          # Query builder
├── prax-postgres/       # PostgreSQL driver
├── prax-mysql/          # MySQL driver
├── prax-sqlite/         # SQLite driver
├── prax-migrate/        # Migrations
├── prax-cli/            # CLI tool
├── prax-armature/       # Armature integration
└── prax/                # Main crate
\`\`\`
```

### Comparison Table

Keep comparisons fair and up-to-date:

```markdown
## Comparison

| Feature | Prax | Diesel | SeaORM | SQLx |
|---------|------|--------|--------|------|
| Async | ✅ | ❌ | ✅ | ✅ |
| Type-Safe | ✅ | ✅ | ✅ | ✅ |
| Schema DSL | ✅ | ❌ | ❌ | ❌ |
| Migrations | ✅ | ✅ | ✅ | ✅ |
```

## Writing Style

### Tone
- Professional but approachable
- Confident but not arrogant
- Technical but accessible

### Formatting
- Use code blocks with language hints
- Use tables for structured data
- Use emoji sparingly for visual scanning
- Keep paragraphs short (2-3 sentences max)

### Links
- Link to detailed docs for complex topics
- Use relative links for repo files: `[CONTRIBUTING](./CONTRIBUTING.md)`
- Use absolute links for external resources

## Code Block Standards

### Always Specify Language

```markdown
\`\`\`rust
// Rust code
\`\`\`

\`\`\`toml
# TOML config
\`\`\`

\`\`\`bash
# Shell commands
\`\`\`

\`\`\`sql
-- SQL queries
\`\`\`
```

### Include Error Handling

Show realistic code with proper error handling:

```rust
// Good: Shows error handling
let user = client
    .user()
    .find_unique(user::id::equals(1))
    .exec()
    .await?
    .ok_or(Error::NotFound)?;

// Avoid: Hides complexity
let user = client.user().find_unique(...).exec().await.unwrap();
```

### Keep Examples Concise

```rust
// Good: Focused example
let users = client
    .user()
    .find_many()
    .where_(user::active::equals(true))
    .exec()
    .await?;

// Avoid: Too much going on
let users = client
    .user()
    .find_many()
    .where_(and![
        user::active::equals(true),
        user::role::equals("admin"),
        user::created_at::gt(DateTime::from(...)),
    ])
    .include(user::posts::fetch().include(post::comments::fetch()))
    .order_by(user::name::asc())
    .skip(page * 10)
    .take(10)
    .exec()
    .await?;
```

## Sync Checklist

When updating the README, verify:

- [ ] Code examples compile and run
- [ ] Version numbers match `Cargo.toml`
- [ ] Feature flags are accurate
- [ ] Links are not broken
- [ ] CLI commands are current
- [ ] Comparison table is fair/accurate
- [ ] Architecture tree matches actual structure
- [ ] Installation instructions work

## Common Updates

### New Feature Added

1. Add to Features list if significant
2. Add usage example in appropriate section
3. Update comparison table if relevant

### New Database Backend

1. Add to Features list
2. Add installation instructions with feature flag
3. Add to comparison table
4. Add backend-specific examples if needed

### New Framework Integration

1. Add to Features list
2. Add installation with integration crate
3. Add example in Quick Start or dedicated section

### API Change

1. Update affected code examples
2. Note breaking changes prominently
3. Show migration path if applicable

### Version Bump

1. Update badge URLs if needed
2. Update minimum Rust version if changed
3. Verify all version references are consistent


## Cursor rule: `.cursor/rules/rust-2024.mdc`

_Rust 2024 Edition best practices and code conventions for the Prax ORM project_

Applies to: `["**/*.rs"]`

# Rust 2024 Edition Guidelines

This project uses **Rust 2024 edition** (requires Rust 1.85+). Follow these guidelines for idiomatic, safe, and performant code.

## Edition 2024 Specifics

### RPIT Lifetime Capture Rules
- Rust 2024 changes `impl Trait` return type lifetime capture behavior
- Use explicit `+ use<'a>` syntax when you need specific lifetime capture:
  ```rust
  fn process<'a>(data: &'a str) -> impl Iterator<Item = &'a str> + use<'a> {
      data.split(',')
  }
  ```

### `gen` Blocks (Experimental)
- Use `gen` blocks for generator-based iterators when stabilized
- Prefer standard iterators for now unless generators provide clear benefits

### New Reserved Keywords
- `gen` is now a reserved keyword - avoid as identifier

## Code Style

### Formatting
- Always run `cargo fmt` before committing (enforced by pre-commit hook)
- Use default rustfmt settings
- Maximum line width: 100 characters

### Naming Conventions
```rust
// Types: PascalCase
struct QueryBuilder;
trait AsyncExecutor;
enum FilterOperator;

// Functions, methods, variables: snake_case
fn execute_query() {}
let user_count = 0;

// Constants: SCREAMING_SNAKE_CASE
const MAX_CONNECTIONS: usize = 100;

// Type parameters: single uppercase or descriptive PascalCase
fn process<T, Item>(data: T) {}

// Lifetimes: short lowercase, descriptive when needed
fn parse<'src>(input: &'src str) {}
```

### Imports
```rust
// Group imports: std, external crates, internal modules
use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;
use tracing::{debug, error, info};

use crate::query::QueryBuilder;
use crate::schema::Model;
```

## Async Code

### Use `async`/`await` Idiomatically
```rust
// Good: Use async blocks for lazy futures
let future = async {
    let result = fetch_data().await?;
    process(result).await
};

// Good: Avoid unnecessary async when not needed
fn sync_operation() -> Result<()> {
    // No await needed, don't make it async
}

// Good: Use async traits (stabilized in 2024)
trait Repository {
    async fn find(&self, id: i64) -> Result<Option<Model>>;
    async fn save(&self, model: &Model) -> Result<()>;
}
```

### Concurrency Patterns
```rust
// Good: Use tokio::spawn for concurrent tasks
let handles: Vec<_> = ids
    .into_iter()
    .map(|id| tokio::spawn(async move { fetch(id).await }))
    .collect();

let results = futures::future::join_all(handles).await;

// Good: Use select! for racing futures
tokio::select! {
    result = operation() => handle(result),
    _ = tokio::time::sleep(timeout) => return Err(Error::Timeout),
}

// Good: Prefer RwLock over Mutex when reads dominate
let cache: Arc<RwLock<HashMap<K, V>>> = Arc::new(RwLock::new(HashMap::new()));
```

### Cancellation Safety
```rust
// Document cancellation safety for public async functions
/// Fetches user data from the database.
///
/// # Cancellation Safety
///
/// This function is cancellation safe. If cancelled, no partial
/// writes will occur.
pub async fn fetch_user(id: i64) -> Result<User> {
    // ...
}
```

## Error Handling

### Use `thiserror` for Library Errors
```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum QueryError {
    #[error("connection failed: {0}")]
    Connection(#[from] tokio_postgres::Error),

    #[error("query timeout after {0:?}")]
    Timeout(std::time::Duration),

    #[error("invalid filter: {field} {message}")]
    InvalidFilter { field: String, message: String },
}
```

### Use `?` Operator
```rust
// Good: Propagate errors with ?
async fn execute(&self) -> Result<Vec<Row>, QueryError> {
    let conn = self.pool.get().await?;
    let rows = conn.query(&self.sql, &self.params).await?;
    Ok(rows)
}

// Avoid: Manual match for simple propagation
// let conn = match self.pool.get().await {
//     Ok(c) => c,
//     Err(e) => return Err(e.into()),
// };
```

### Provide Context
```rust
use anyhow::Context;

// Good: Add context to errors
let config = std::fs::read_to_string(path)
    .with_context(|| format!("failed to read config from {}", path.display()))?;
```

## Type System

### Use Type Aliases for Complex Types
```rust
// Good: Simplify complex types
pub type QueryResult<T> = Result<T, QueryError>;
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
```

### Leverage Associated Types
```rust
// Good: Use associated types in traits
trait Database {
    type Connection: Connection;
    type Error: std::error::Error;

    async fn connect(&self) -> Result<Self::Connection, Self::Error>;
}
```

### Generic Constraints
```rust
// Good: Use impl Trait in argument position for cleaner APIs
pub fn where_clause(filter: impl Into<Filter>) -> Self {
    // ...
}

// Good: Use where clauses for complex bounds
fn execute<T, E>(query: T) -> Result<E::Output>
where
    T: Query + Send + Sync,
    E: Executor<Query = T>,
{
    // ...
}
```

## Performance

### Avoid Unnecessary Allocations
```rust
// Good: Use references and slices
fn process(data: &[u8]) -> &str { ... }

// Good: Use Cow for conditional ownership
use std::borrow::Cow;
fn normalize(s: &str) -> Cow<'_, str> {
    if needs_normalization(s) {
        Cow::Owned(s.to_lowercase())
    } else {
        Cow::Borrowed(s)
    }
}

// Good: Reuse buffers
let mut buf = String::with_capacity(1024);
for item in items {
    buf.clear();
    write!(&mut buf, "{}", item)?;
    process(&buf);
}
```

### Use Iterators
```rust
// Good: Chain iterator methods
let active_users: Vec<_> = users
    .iter()
    .filter(|u| u.is_active)
    .map(|u| &u.name)
    .collect();

// Good: Use collect with type inference
let map: HashMap<_, _> = pairs.into_iter().collect();
```

### Smart Pointers
```rust
// Use Arc for shared ownership across threads
let shared: Arc<Config> = Arc::new(config);

// Use Box for heap allocation and trait objects
let handler: Box<dyn Handler> = Box::new(MyHandler);

// Avoid Rc in async code (not Send)
```

## Documentation

### Document Public APIs
```rust
/// A type-safe query builder for database operations.
///
/// # Examples
///
/// ```rust
/// let users = client
///     .user()
///     .find_many()
///     .where_(user::active::equals(true))
///     .exec()
///     .await?;
/// ```
///
/// # Panics
///
/// Panics if called without a valid connection.
///
/// # Errors
///
/// Returns `QueryError::Connection` if the database is unreachable.
pub struct QueryBuilder<T> { ... }
```

### Use `#[must_use]` for Important Return Values
```rust
#[must_use = "queries do nothing until executed"]
pub fn where_(self, filter: Filter) -> Self { ... }

#[must_use]
pub fn build(self) -> Query { ... }
```

## Testing

### Unit Tests
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_equals() {
        let filter = user::id::equals(1);
        assert_eq!(filter.to_sql(), "id = $1");
    }

    #[tokio::test]
    async fn test_async_operation() {
        let result = async_fn().await;
        assert!(result.is_ok());
    }
}
```

### Use `#[should_panic]` and `#[ignore]`
```rust
#[test]
#[should_panic(expected = "invalid input")]
fn test_panics_on_invalid_input() {
    parse("invalid");
}

#[test]
#[ignore = "requires database connection"]
fn test_integration() {
    // ...
}
```

## Safety

### Minimize `unsafe`
- Avoid `unsafe` unless absolutely necessary
- Document safety invariants for any `unsafe` code
- Isolate `unsafe` in small, well-tested modules

```rust
/// # Safety
///
/// The caller must ensure that `ptr` is valid and properly aligned.
unsafe fn raw_operation(ptr: *mut u8) {
    // ...
}
```

### Use `#[non_exhaustive]` for Extensibility
```rust
#[non_exhaustive]
pub enum FilterOp {
    Equals,
    NotEquals,
    Contains,
    // Can add variants without breaking changes
}
```

## Clippy

All code must pass `cargo clippy` with no warnings. Key lints enforced:

```rust
#![deny(
    clippy::all,
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
)]
#![allow(
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
)]
```

## Commit Messages

Follow Conventional Commits (enforced by commit-msg hook):
- `feat(query):` - New features
- `fix(postgres):` - Bug fixes
- `docs:` - Documentation
- `refactor:` - Code refactoring
- `test:` - Test changes
- `perf:` - Performance improvements


## Cursor rule: `.cursor/rules/sql-safety.mdc`

# SQL Safety & Security

This project handles user-provided data that becomes part of SQL queries. **Never allow SQL injection vulnerabilities.**

## Core Principles

### 1. Always Use Parameterized Queries

**NEVER concatenate user input directly into SQL strings.**

```rust
// ✅ Good: Parameterized query
let filter = Filter::Equals("email".into(), FilterValue::String(user_email.into()));
let (sql, params) = filter.to_sql(0);
// sql = "email = $1", params = [user_email]

// ✅ Good: Using SqlBuilder
let mut builder = SqlBuilder::postgres();
builder.push("SELECT * FROM users WHERE email = ");
builder.push_param(FilterValue::String(user_email.into()));

// ❌ DANGEROUS: String concatenation
let sql = format!("SELECT * FROM users WHERE email = '{}'", user_email);

// ❌ DANGEROUS: Direct interpolation
let sql = format!("SELECT * FROM users WHERE id = {}", user_id);
```

### 2. Validate Identifiers

Table names, column names, and other identifiers cannot be parameterized. **Always validate them.**

```rust
// ✅ Good: Whitelist allowed identifiers
const ALLOWED_COLUMNS: &[&str] = &["id", "name", "email", "created_at"];

pub fn sort_by(column: &str) -> Result<String> {
    if ALLOWED_COLUMNS.contains(&column) {
        Ok(format!("ORDER BY {}", column))
    } else {
        Err(Error::InvalidColumn(column.to_string()))
    }
}

// ✅ Good: Use enums for allowed values
pub enum SortColumn {
    Id,
    Name,
    Email,
    CreatedAt,
}

impl SortColumn {
    pub fn as_sql(&self) -> &'static str {
        match self {
            SortColumn::Id => "id",
            SortColumn::Name => "name",
            SortColumn::Email => "email",
            SortColumn::CreatedAt => "created_at",
        }
    }
}

// ❌ DANGEROUS: Accepting arbitrary identifiers
pub fn sort_by(column: &str) -> String {
    format!("ORDER BY {}", column) // SQL injection possible!
}
```

#### Trust boundary for schema-author identifiers

Identifiers that originate from `.prax` schema files (`@@map("table")`,
`@map("col")`, `@relation(fields: [...])`, model and field names) are
treated as **compile-time-trusted** — the schema author is the developer.
Codegen interpolates these into the generated SQL and into
`RelationFilterMeta` consts without runtime validation; if a malicious
actor controls a schema file at build time they can already do arbitrary
things to the produced binary.

This rule still applies at runtime: any identifier that flows from end-user
input (HTTP body, query string, CLI flag, etc.) into SQL **must** be
whitelisted or rejected. Schema-author strings and end-user strings are
distinct trust domains; never blur them.

### 3. Quote Identifiers When Dynamic

If you must use dynamic identifiers, properly quote them:

```rust
// ✅ Good: Quoted identifier (PostgreSQL style)
pub fn quote_identifier(name: &str) -> String {
    // Escape any embedded double quotes
    let escaped = name.replace('"', "\"\"");
    format!("\"{}\"", escaped)
}

// Usage
let safe_column = quote_identifier(user_provided_column);
let sql = format!("SELECT {} FROM users", safe_column);
```

## Filter Building Rules

### Use the Type System

The `Filter` and `FilterValue` types enforce parameterization:

```rust
// ✅ Good: Type-safe filter construction
let filter = Filter::and(vec![
    Filter::Equals("status".into(), FilterValue::String("active".into())),
    Filter::Gte("age".into(), FilterValue::Int(18)),
    Filter::Contains("email".into(), FilterValue::String(search_term.into())),
]);

// The to_sql() method ensures all values become parameters
let (where_clause, params) = filter.to_sql(0);
// where_clause = "(status = $1 AND age >= $2 AND email LIKE $3)"
// params = ["active", 18, "%search_term%"]
```

### Escape LIKE Patterns

When using `Contains`, `StartsWith`, `EndsWith`, the ORM handles escaping:

```rust
// ✅ Good: ORM escapes LIKE wildcards in user input
let filter = Filter::Contains("name".into(), FilterValue::String(user_search.into()));
// If user_search = "test%", it becomes "test\%" in the LIKE pattern

// ❌ Bad: Manual LIKE without escaping
let pattern = format!("%{}%", user_search); // user_search = "%" breaks query
```

## Raw SQL Safety

### Minimize Raw SQL

Prefer the query builder. When raw SQL is necessary:

```rust
// ✅ Good: Raw SQL with parameterized values
use prax_query::raw::Sql;

let sql = Sql::new("SELECT * FROM users WHERE email = $1 AND status = $2")
    .bind(FilterValue::String(email.into()))
    .bind(FilterValue::String("active".into()));

// ✅ Good: Raw SQL with safe interpolation for identifiers only
let table = validate_table_name(user_table)?; // whitelist check
let sql = Sql::new(&format!("SELECT * FROM {} WHERE id = $1", table))
    .bind(FilterValue::Int(id));

// ❌ DANGEROUS: Raw SQL with user values in string
let sql = Sql::new(&format!("SELECT * FROM users WHERE email = '{}'", email));
```

### Review All `format!` Calls in SQL Context

Every `format!` that produces SQL should be audited:

```rust
// Questions to ask:
// 1. Are all user-provided values going through push_param()?
// 2. Are identifiers validated against a whitelist?
// 3. Could a malicious input change the query structure?
```

## JSON/JSONB Safety

When working with JSON queries:

```rust
// ✅ Good: JSON value as parameter
builder.push("WHERE metadata @> ");
builder.push_param(FilterValue::Json(serde_json::json!({"role": role})));

// ✅ Good: JSON path with validated keys
const ALLOWED_KEYS: &[&str] = &["role", "status", "type"];
if ALLOWED_KEYS.contains(&key) {
    builder.push(&format!("WHERE metadata->>'{}' = ", key));
    builder.push_param(FilterValue::String(value.into()));
}

// ❌ DANGEROUS: Unvalidated JSON path
builder.push(&format!("WHERE metadata->>'{}' = ", user_key)); // injection risk
```

## Multi-Tenancy Security

### Always Include Tenant Filter

```rust
// ✅ Good: Tenant filter added at ORM level
let filter = Filter::and(vec![
    Filter::Equals("tenant_id".into(), FilterValue::Int(current_tenant)),
    user_provided_filter,
]);

// ✅ Good: PostgreSQL RLS handles it
// SET app.current_tenant = $1; -- set at connection level
// CREATE POLICY tenant_isolation ON users USING (tenant_id = current_setting('app.current_tenant')::int);

// ❌ DANGEROUS: Trusting user to provide tenant filter
let filter = user_provided_filter; // Could query other tenants!
```

## Testing for SQL Injection

### Include Injection Tests

```rust
#[test]
fn test_sql_injection_in_filter_value() {
    let malicious = "'; DROP TABLE users; --";
    let filter = Filter::Equals("name".into(), FilterValue::String(malicious.into()));
    let (sql, params) = filter.to_sql(0);

    // Value should be a parameter, not in SQL string
    assert_eq!(sql, "name = $1");
    assert!(!sql.contains("DROP"));
    assert!(matches!(&params[0], FilterValue::String(s) if s == malicious));
}

#[test]
fn test_sql_injection_in_like_pattern() {
    let malicious = "test%' OR '1'='1";
    let filter = Filter::Contains("name".into(), FilterValue::String(malicious.into()));
    let (sql, params) = filter.to_sql(0);

    assert_eq!(sql, "name LIKE $1");
    // Pattern should be escaped and parameterized
}

#[test]
fn test_identifier_validation() {
    let malicious = "id; DROP TABLE users; --";
    let result = validate_column_name(malicious);
    assert!(result.is_err());
}
```

## Code Review Checklist

When reviewing SQL-related code:

- [ ] All user values go through `FilterValue` or `push_param()`
- [ ] All dynamic identifiers are validated against whitelist
- [ ] No string concatenation for SQL with user input
- [ ] LIKE patterns are properly escaped
- [ ] Raw SQL is minimized and justified
- [ ] Multi-tenant queries always include tenant filter
- [ ] Tests include SQL injection attempts

## Summary

1. **Use parameterized queries** - Always use `FilterValue` and `push_param()`
2. **Validate identifiers** - Whitelist table/column names, or use enums
3. **Quote when necessary** - Properly escape dynamic identifiers
4. **Test for injection** - Include malicious input in test cases
5. **Review `format!`** - Audit all SQL string formatting
6. **Defense in depth** - Combine ORM safety with database-level controls (RLS)


## Cursor rule: `.cursor/rules/testing.mdc`

_Testing requirements and standards for the Prax ORM project_

Applies to: `["**/*.rs"]`

# Testing Standards

This project requires **90%+ code coverage**. All new code must include comprehensive tests.

## Coverage Requirements

- **Minimum coverage**: 90% line coverage
- **Target coverage**: 95%+ for critical paths (parser, query builder, migrations)
- Run coverage with: `cargo llvm-cov --all-features`

## Test Organization

### Unit Tests

Place unit tests in the same file as the code being tested:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_function_name() {
        // Arrange
        let input = ...;

        // Act
        let result = function_under_test(input);

        // Assert
        assert_eq!(result, expected);
    }
}
```

### Integration Tests

Place integration tests in `tests/` directory:

```
tests/
├── parser_integration.rs
├── config_integration.rs
└── common/
    └── mod.rs
```

### Test Naming Conventions

```rust
// Unit test: test_<function>_<scenario>_<expected_outcome>
#[test]
fn test_parse_model_with_relations_succeeds() { }

#[test]
fn test_parse_model_missing_id_returns_error() { }

// Parameterized tests with descriptive names
#[test]
fn test_scalar_type_int_parses_correctly() { }

#[test]
fn test_scalar_type_string_parses_correctly() { }
```

## What to Test

### Always Test

1. **Happy path** - Normal successful execution
2. **Edge cases** - Empty inputs, boundaries, limits
3. **Error cases** - Invalid inputs, missing required fields
4. **All branches** - Every if/else, match arm, Option/Result path

### Parser Tests Must Cover

- All scalar types (Int, String, Boolean, DateTime, etc.)
- All type modifiers (optional `?`, list `[]`)
- All field attributes (@id, @auto, @unique, @default, etc.)
- All model attributes (@@map, @@index, @@unique, etc.)
- Relation definitions with all referential actions
- Enum definitions with variants
- Composite types
- Views
- Documentation comments
- Error cases (syntax errors, invalid attributes)

### Config Tests Must Cover

- Default values
- Environment variable expansion
- All configuration sections
- Environment-specific overrides
- Invalid configuration handling

### AST Tests Must Cover

- All type constructors
- All accessor methods
- Serialization/deserialization (if serde enabled)
- Display implementations

## Test Utilities

### Use Test Fixtures

```rust
fn sample_schema() -> &'static str {
    r#"
    model User {
        id    Int    @id @auto
        email String @unique
    }
    "#
}

fn sample_config() -> &'static str {
    r#"
    [database]
    provider = "postgresql"
    url = "postgres://localhost/test"
    "#
}
```

### Use Snapshot Testing for Complex Output

```rust
use insta::assert_yaml_snapshot;

#[test]
fn test_parse_complex_schema() {
    let schema = parse_schema(COMPLEX_SCHEMA).unwrap();
    assert_yaml_snapshot!(schema);
}
```

### Use Property-Based Testing for Parsers

```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn test_identifier_roundtrip(name in "[a-zA-Z][a-zA-Z0-9_]*") {
        let parsed = parse_identifier(&name);
        assert!(parsed.is_ok());
    }
}
```

## Test Quality Checklist

Before submitting code, verify:

- [ ] All public functions have tests
- [ ] All error paths are tested
- [ ] Edge cases are covered
- [ ] Tests are deterministic (no random, no time-dependent)
- [ ] Tests are fast (mock external dependencies)
- [ ] Tests have descriptive names
- [ ] Tests use assertions with good error messages
- [ ] No `#[ignore]` without explanation

## Running Tests

```bash
# Run all tests
cargo test --all-features

# Run with coverage
cargo llvm-cov --all-features

# Run specific test
cargo test test_name

# Run tests for specific crate
cargo test -p prax-schema

# Run with output
cargo test -- --nocapture

# Run ignored tests
cargo test -- --ignored
```

## Mocking Guidelines

- Use traits for external dependencies
- Create mock implementations in test modules
- Prefer dependency injection over global state

```rust
// Production code
trait DatabaseConnection {
    async fn execute(&self, query: &str) -> Result<()>;
}

// Test code
struct MockConnection {
    queries: RefCell<Vec<String>>,
}

impl DatabaseConnection for MockConnection {
    async fn execute(&self, query: &str) -> Result<()> {
        self.queries.borrow_mut().push(query.to_string());
        Ok(())
    }
}
```

## Continuous Integration

Tests run on every PR:
- `cargo test --all-features`
- `cargo clippy -- -D warnings`
- `cargo fmt -- --check`
- Coverage must not decrease

## When to Skip Tests

Only skip tests with `#[ignore]` when:
- Test requires external service (database, network)
- Test is flaky and being investigated
- Test is for future functionality

Always document why:
```rust
#[test]
#[ignore = "requires PostgreSQL database connection"]
fn test_real_database_connection() { }
```

