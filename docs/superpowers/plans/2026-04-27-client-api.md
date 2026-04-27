# Prax Client API Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver a Prisma-parity fluent client API (`client.user().find_many().r#where(...).order_by(...).exec().await?`) that runs real SQL against PostgreSQL, MySQL, SQLite, and MSSQL — including relations (`include`/`select`), nested writes, upsert, aggregations, transactions, and typed row deserialization — all driven by `#[derive(Model)]` and the `prax_schema!` macro without any user-written SQL.

**Architecture:**
- `prax-query` owns the database-agnostic runtime: `QueryEngine` trait, `Model`/`FromRow`/`RowRef` traits, `FindManyOperation`/`CreateOperation`/etc., relation loader.
- Each driver crate (`prax-postgres`, `prax-mysql`, `prax-sqlite`, `prax-mssql`) implements `RowRef` for its native row type, wires up `QueryEngine::query_many`/`query_one`/`execute_insert`/etc. by calling `T::from_row` via the bridge, and exposes a `Transaction`-backed engine.
- `prax-codegen` (both `prax_schema!` and `#[derive(Model)]` paths) emits: the model struct, a `prax_query::Model` trait impl, a `prax_query::row::FromRow` impl, a full `WhereParam`/`SetParam`/`OrderByParam`/`Include`/`Select` surface, a per-model `Client<E>` accessor whose methods return live `FindManyOperation<E, M>` (not the inert `Query` helper in use today), plus relation helpers.
- `prax-orm` re-exports a generic `PraxClient<E>` plus a `prax::client! { User, Post, ... }` declarative macro so any combination of derive-generated models can be registered on a single client with one-line field accessors.
- MySQL's current JSON-first engine is replaced with a proper `QueryEngine` impl (the JSON helpers move to a `raw::` submodule for the escape hatch).

**Tech Stack:** Rust 2024, tokio-postgres, mysql_async, tokio-rusqlite/rusqlite, tiberius, syn 2.0 + quote (proc-macro), trybuild (macro tests), docker-compose (integration tests), tokio, serde/serde_json, chrono, uuid, rust_decimal.

---

## File Structure

### New Files

**Driver row bridges**
- `prax-postgres/src/row_ref.rs` — `impl prax_query::row::RowRef for tokio_postgres::Row`
- `prax-mysql/src/row_ref.rs` — `MysqlRowRef` newtype + `impl RowRef` (converts from `mysql_async::Row`/`Value`)
- `prax-sqlite/src/row_ref.rs` — `SqliteRowRef` borrowed-row wrapper + `impl RowRef`
- `prax-mssql/src/row_ref.rs` — `impl RowRef for tiberius::Row`

**Driver engine helpers**
- `prax-postgres/src/deserialize.rs` — generic helpers that decode `Vec<tokio_postgres::Row>` into `Vec<T: FromRow>` and map errors to `QueryError`
- `prax-mysql/src/deserialize.rs` — same shape
- `prax-sqlite/src/deserialize.rs` — same shape (blocking SQLite step wrapped through `tokio_rusqlite`'s `call` API)
- `prax-mssql/src/deserialize.rs` — same shape

**Transaction engines**
- `prax-postgres/src/tx.rs` — `PgTxEngine` that owns a `deadpool_postgres::Transaction` and implements `QueryEngine`
- `prax-mysql/src/tx.rs` — `MysqlTxEngine`
- `prax-sqlite/src/tx.rs` — `SqliteTxEngine`
- `prax-mssql/src/tx.rs` — `MssqlTxEngine`

**Codegen: derive path**
- `prax-codegen/src/generators/derive_model_trait.rs` — emit `impl prax_query::Model` for a parsed struct
- `prax-codegen/src/generators/derive_from_row.rs` — emit `impl prax_query::row::FromRow`
- `prax-codegen/src/generators/derive_client.rs` — emit per-model `Client<E>` accessor struct + CRUD methods wiring into `prax_query::*Operation`
- `prax-codegen/src/generators/relation_accessors.rs` — emit `post::author::fetch()` / `user::posts::fetch()` `IncludeSpec` helpers

**Codegen: schema macro path**
- `prax-codegen/src/generators/schema_model_trait.rs` — same as derive trait emitter, but consumes `prax_schema::ast::Model`
- `prax-codegen/src/generators/schema_from_row.rs` — same pattern
- `prax-codegen/src/generators/schema_client.rs` — same pattern; merges with existing `model.rs` generator

**Orm umbrella**
- `src/client.rs` — `PraxClient<E>` struct, `prax::client!` declarative macro, `pub use` re-exports

**Relation loading runtime**
- `prax-query/src/relations/executor.rs` — `RelationExecutor` that, given an `IncludeSpec` + parent rows + `QueryEngine`, issues follow-up queries and stitches children onto parents
- `prax-query/src/relations/meta.rs` — `RelationMeta` trait generated per relation (describes join columns, direction, target model)

**Dialect abstraction**
- `prax-query/src/dialect.rs` — `SqlDialect` trait + `Postgres`, `Mysql`, `Sqlite`, `Mssql` impls

**Integration tests**
- `tests/client_postgres.rs` — end-to-end against Dockerised Postgres
- `tests/client_mysql.rs` — end-to-end against Dockerised MySQL
- `tests/client_sqlite.rs` — end-to-end against in-memory SQLite
- `tests/client_mssql.rs` — end-to-end against Dockerised MSSQL
- `tests/relations_postgres.rs` — relation loading
- `tests/tx_postgres.rs` — transactions
- `tests/upsert_postgres.rs` — upsert
- `tests/aggregate_postgres.rs` — aggregate + group_by
- `tests/nested_write_postgres.rs` — nested writes
- `tests/select_postgres.rs` — projection
- `tests/raw_postgres.rs` — typed raw SQL
- `prax-orm/tests/derive_ui/*.rs` — trybuild compile-pass and compile-fail cases for `#[derive(Model)]`

**Examples**
- `examples/client_crud_postgres.rs` — demonstrates `client.user().find_many()...` end-to-end

### Modified Files

- `prax-query/src/traits.rs:78-158` — extend `QueryEngine` with a `dialect()` method, a `transaction()` default, and tighten row-returning bounds to require `FromRow`
- `prax-query/src/traits.rs:186-230` — replace `ModelAccessor` with a leaner version that matches the generated `Client<E>` shape
- `prax-query/src/row.rs:70-125` — add `RowRef::get_uuid`, `get_uuid_opt`, `get_json`, `get_json_opt`, `get_datetime_utc`, `get_decimal` methods (default `unsupported` impls to keep backwards compat)
- `prax-query/src/row.rs:293-369` — add `FromColumn` impls for `chrono::DateTime<Utc>`, `chrono::NaiveDateTime`, `chrono::NaiveDate`, `chrono::NaiveTime`, `uuid::Uuid`, `rust_decimal::Decimal`, `serde_json::Value`, `Vec<u8>` + `Option<..>` variants for each
- `prax-query/src/operations/*.rs` — propagate `T: FromRow` bound; call the new dialect for placeholder and RETURNING emission; add `include()`/`select()` where missing
- `prax-query/src/filter.rs` — add a `ToFilterValue` conversion trait for scalar types
- `prax-query/src/relations/mod.rs:35-43` — re-export new `executor` + `meta` modules
- `prax-query/src/lib.rs:174-198` — re-export new relation executor + dialect types
- `prax-postgres/src/engine.rs:45-285` — rewrite every method to deserialize rows for real
- `prax-postgres/src/lib.rs:32-57` — export new modules, add `pub use tx::PgTxEngine`
- `prax-mysql/src/engine.rs` — replace the JSON-returning module with a `QueryEngine` impl; move JSON helpers to new `prax-mysql/src/raw.rs`
- `prax-mysql/src/lib.rs` — export new surface
- `prax-sqlite/src/engine.rs` — same treatment as MySQL
- `prax-sqlite/src/lib.rs` — re-export
- `prax-mssql/src/engine.rs` — implement the deserialization path
- `prax-codegen/src/generators/mod.rs:1-50` — register new generator modules
- `prax-codegen/src/generators/model.rs:25-221` — call the new trait/row/client emitters
- `prax-codegen/src/generators/derive.rs:90-183` — replace the inert `Actions`/`Query` helper with emission of the `Client<E>` accessor + trait impls
- `prax-codegen/src/generators/fields.rs:12-179` — emit `WhereParam -> Filter` conversion (currently only `to_sql` string is generated)
- `src/lib.rs:625-659` — re-export `PraxClient`, `client!` macro, the operation types in the prelude
- `Cargo.toml` — enable `tokio-postgres` `with-rust_decimal-1` feature; add `paste` as a direct workspace dep
- `.github/workflows/ci.yml` — add service-container jobs for Postgres/MySQL/MSSQL integration tests

### Deleted / Replaced
- `prax-mysql/src/engine.rs::MysqlQueryResult` — JSON blob type moved to `raw::MysqlJsonRow`
- `prax-sqlite/src/engine.rs::SqliteQueryResult` — same
- Legacy `Actions` struct emission in `prax-codegen/src/generators/model.rs:417-446` (replaced by `Client<E>`)

---

## Task 1: Verify clean baseline

**Files:**
- None (verification only)

- [ ] **Step 1: Run full workspace check**

Run: `cargo check --workspace --all-features`
Expected: `Finished 'dev' profile ...` with zero compile errors.

- [ ] **Step 2: Run unit test suite (no DB required)**

Run: `cargo test --workspace --lib`
Expected: all tests pass. Do NOT use `--no-default-features` at the workspace level — `prax-sqlx`'s `SqlxRow` / `SqlxPool` / `SqlxConnection` / `SqlxTransaction` enums have every variant gated by a backend feature, so stripping defaults leaves them uninhabited and `match` on them triggers E0004. Any pre-existing failure MUST be fixed before continuing per the project rule "the PR that finds a failure is the PR that fixes it."

- [ ] **Step 3: Start Dockerised databases**

Run: `docker compose up -d postgres mysql mssql`
Expected: all three containers `healthy` within 60 seconds (`docker compose ps`).

- [ ] **Step 4: Verify Postgres connectivity**

Run: `docker exec prax-postgres psql -U prax -d prax_test -c 'SELECT 1'`
Expected: one row returned.

- [ ] **Step 5: No commit needed — Task 1 is verification only**

---

## Task 2: Extend `FromColumn` with temporal / UUID / JSON / decimal types

**Files:**
- Modify: `prax-query/src/row.rs:70-369`
- Modify: `prax-query/Cargo.toml`
- Test: `prax-query/src/row.rs` (same file)

Real schemas contain `timestamptz`, `uuid`, `jsonb`, `numeric`. Current `FromColumn` stops at primitives.

- [ ] **Step 1: Add default-erroring methods to `RowRef`**

In `prax-query/src/row.rs`, inside `pub trait RowRef`, append methods that default to `TypeConversion` errors so drivers can opt in incrementally:

```rust
    fn get_datetime_utc(&self, column: &str) -> Result<chrono::DateTime<chrono::Utc>, RowError> {
        Err(RowError::TypeConversion { column: column.into(), message: "datetime_utc not supported by this row type".into() })
    }
    fn get_datetime_utc_opt(&self, column: &str) -> Result<Option<chrono::DateTime<chrono::Utc>>, RowError> {
        Err(RowError::TypeConversion { column: column.into(), message: "datetime_utc_opt not supported by this row type".into() })
    }
    fn get_naive_datetime(&self, column: &str) -> Result<chrono::NaiveDateTime, RowError> {
        Err(RowError::TypeConversion { column: column.into(), message: "naive_datetime not supported".into() })
    }
    fn get_naive_datetime_opt(&self, column: &str) -> Result<Option<chrono::NaiveDateTime>, RowError> {
        Err(RowError::TypeConversion { column: column.into(), message: "naive_datetime_opt not supported".into() })
    }
    fn get_naive_date(&self, column: &str) -> Result<chrono::NaiveDate, RowError> {
        Err(RowError::TypeConversion { column: column.into(), message: "naive_date not supported".into() })
    }
    fn get_naive_date_opt(&self, column: &str) -> Result<Option<chrono::NaiveDate>, RowError> {
        Err(RowError::TypeConversion { column: column.into(), message: "naive_date_opt not supported".into() })
    }
    fn get_naive_time(&self, column: &str) -> Result<chrono::NaiveTime, RowError> {
        Err(RowError::TypeConversion { column: column.into(), message: "naive_time not supported".into() })
    }
    fn get_naive_time_opt(&self, column: &str) -> Result<Option<chrono::NaiveTime>, RowError> {
        Err(RowError::TypeConversion { column: column.into(), message: "naive_time_opt not supported".into() })
    }
    fn get_uuid(&self, column: &str) -> Result<uuid::Uuid, RowError> {
        Err(RowError::TypeConversion { column: column.into(), message: "uuid not supported".into() })
    }
    fn get_uuid_opt(&self, column: &str) -> Result<Option<uuid::Uuid>, RowError> {
        Err(RowError::TypeConversion { column: column.into(), message: "uuid_opt not supported".into() })
    }
    fn get_json(&self, column: &str) -> Result<serde_json::Value, RowError> {
        Err(RowError::TypeConversion { column: column.into(), message: "json not supported".into() })
    }
    fn get_json_opt(&self, column: &str) -> Result<Option<serde_json::Value>, RowError> {
        Err(RowError::TypeConversion { column: column.into(), message: "json_opt not supported".into() })
    }
    fn get_decimal(&self, column: &str) -> Result<rust_decimal::Decimal, RowError> {
        Err(RowError::TypeConversion { column: column.into(), message: "decimal not supported".into() })
    }
    fn get_decimal_opt(&self, column: &str) -> Result<Option<rust_decimal::Decimal>, RowError> {
        Err(RowError::TypeConversion { column: column.into(), message: "decimal_opt not supported".into() })
    }
```

- [ ] **Step 2: Promote `chrono`, `uuid`, `rust_decimal` to non-optional deps for `prax-query`**

Edit `prax-query/Cargo.toml`. Under `[dependencies]` add:

```toml
chrono = { workspace = true }
uuid = { workspace = true }
rust_decimal = { workspace = true }
```

- [ ] **Step 3: Append `FromColumn` impls for the new scalar types**

Append near the other `FromColumn` impls in `prax-query/src/row.rs`:

```rust
impl FromColumn for chrono::DateTime<chrono::Utc> {
    fn from_column(row: &impl RowRef, column: &str) -> Result<Self, RowError> { row.get_datetime_utc(column) }
}
impl FromColumn for Option<chrono::DateTime<chrono::Utc>> {
    fn from_column(row: &impl RowRef, column: &str) -> Result<Self, RowError> { row.get_datetime_utc_opt(column) }
}
impl FromColumn for chrono::NaiveDateTime {
    fn from_column(row: &impl RowRef, column: &str) -> Result<Self, RowError> { row.get_naive_datetime(column) }
}
impl FromColumn for Option<chrono::NaiveDateTime> {
    fn from_column(row: &impl RowRef, column: &str) -> Result<Self, RowError> { row.get_naive_datetime_opt(column) }
}
impl FromColumn for chrono::NaiveDate {
    fn from_column(row: &impl RowRef, column: &str) -> Result<Self, RowError> { row.get_naive_date(column) }
}
impl FromColumn for Option<chrono::NaiveDate> {
    fn from_column(row: &impl RowRef, column: &str) -> Result<Self, RowError> { row.get_naive_date_opt(column) }
}
impl FromColumn for chrono::NaiveTime {
    fn from_column(row: &impl RowRef, column: &str) -> Result<Self, RowError> { row.get_naive_time(column) }
}
impl FromColumn for Option<chrono::NaiveTime> {
    fn from_column(row: &impl RowRef, column: &str) -> Result<Self, RowError> { row.get_naive_time_opt(column) }
}
impl FromColumn for uuid::Uuid {
    fn from_column(row: &impl RowRef, column: &str) -> Result<Self, RowError> { row.get_uuid(column) }
}
impl FromColumn for Option<uuid::Uuid> {
    fn from_column(row: &impl RowRef, column: &str) -> Result<Self, RowError> { row.get_uuid_opt(column) }
}
impl FromColumn for serde_json::Value {
    fn from_column(row: &impl RowRef, column: &str) -> Result<Self, RowError> { row.get_json(column) }
}
impl FromColumn for Option<serde_json::Value> {
    fn from_column(row: &impl RowRef, column: &str) -> Result<Self, RowError> { row.get_json_opt(column) }
}
impl FromColumn for rust_decimal::Decimal {
    fn from_column(row: &impl RowRef, column: &str) -> Result<Self, RowError> { row.get_decimal(column) }
}
impl FromColumn for Option<rust_decimal::Decimal> {
    fn from_column(row: &impl RowRef, column: &str) -> Result<Self, RowError> { row.get_decimal_opt(column) }
}
```

- [ ] **Step 4: Add a unit test asserting default methods error**

Append inside `mod tests`:

```rust
    #[test]
    fn default_datetime_method_errors() {
        let mut data = std::collections::HashMap::new();
        data.insert("created_at".into(), "2026-04-27T00:00:00Z".into());
        let row = MockRow { data };
        assert!(matches!(row.get_datetime_utc("created_at"), Err(RowError::TypeConversion { .. })));
    }
```

Run: `cargo test -p prax-query row::tests`
Expected: all row tests pass.

- [ ] **Step 5: Commit**

```bash
git add prax-query/src/row.rs prax-query/Cargo.toml
git commit -m "feat(query): add temporal, uuid, json, and decimal FromColumn impls

Row deserialization previously stopped at primitives — real schemas
need chrono::DateTime<Utc>, uuid::Uuid, rust_decimal::Decimal, and
serde_json::Value to flow through FromRow. Drivers opt in by
overriding the default RowRef getters (which error by default)."
```

---

## Task 3: Implement `RowRef` for `tokio_postgres::Row`

**Files:**
- Create: `prax-postgres/src/row_ref.rs`
- Create: `prax-postgres/tests/row_ref.rs`
- Modify: `prax-postgres/src/lib.rs:32-47`
- Modify: `Cargo.toml` (tokio-postgres features)

- [ ] **Step 1: Write a failing integration test**

Create `prax-postgres/tests/row_ref.rs`:

```rust
use prax_postgres::{PgPool, PgPoolBuilder};
use prax_query::row::RowRef;

fn test_url() -> String {
    std::env::var("PRAX_POSTGRES_URL")
        .unwrap_or_else(|_| "postgres://prax:prax_test_password@localhost:5433/prax_test".into())
}

#[tokio::test]
async fn get_i32_and_string_from_row() {
    let pool: PgPool = PgPoolBuilder::new().url(test_url()).build().await.unwrap();
    let conn = pool.get().await.unwrap();
    let row = conn.query_one("SELECT 42::int4 AS n, 'hello'::text AS s", &[]).await.unwrap();
    assert_eq!(row.get_i32("n").unwrap(), 42);
    assert_eq!(row.get_str("s").unwrap(), "hello");
}
```

Run: `cargo test -p prax-postgres --test row_ref`
Expected: COMPILE FAIL — `RowRef` is not implemented for `tokio_postgres::Row`.

- [ ] **Step 2: Enable tokio-postgres decimal feature**

Edit `Cargo.toml` root workspace dep for `tokio-postgres`:

```toml
tokio-postgres = { version = "0.7", features = ["with-serde_json-1", "with-chrono-0_4", "with-uuid-1", "with-rust_decimal-1"] }
```

- [ ] **Step 3: Create `prax-postgres/src/row_ref.rs`**

```rust
//! Bridge between tokio_postgres::Row and prax_query::row::RowRef.

use prax_query::row::{RowError, RowRef};
use tokio_postgres::Row;

fn tc<T, E: std::fmt::Display>(column: &str, res: Result<T, E>) -> Result<T, RowError> {
    res.map_err(|e| RowError::TypeConversion { column: column.to_string(), message: e.to_string() })
}

impl RowRef for Row {
    fn get_i32(&self, c: &str) -> Result<i32, RowError> { tc(c, self.try_get::<_, i32>(c)) }
    fn get_i32_opt(&self, c: &str) -> Result<Option<i32>, RowError> { tc(c, self.try_get::<_, Option<i32>>(c)) }
    fn get_i64(&self, c: &str) -> Result<i64, RowError> { tc(c, self.try_get::<_, i64>(c)) }
    fn get_i64_opt(&self, c: &str) -> Result<Option<i64>, RowError> { tc(c, self.try_get::<_, Option<i64>>(c)) }
    fn get_f64(&self, c: &str) -> Result<f64, RowError> { tc(c, self.try_get::<_, f64>(c)) }
    fn get_f64_opt(&self, c: &str) -> Result<Option<f64>, RowError> { tc(c, self.try_get::<_, Option<f64>>(c)) }
    fn get_bool(&self, c: &str) -> Result<bool, RowError> { tc(c, self.try_get::<_, bool>(c)) }
    fn get_bool_opt(&self, c: &str) -> Result<Option<bool>, RowError> { tc(c, self.try_get::<_, Option<bool>>(c)) }
    fn get_str(&self, c: &str) -> Result<&str, RowError> { tc(c, self.try_get::<_, &str>(c)) }
    fn get_str_opt(&self, c: &str) -> Result<Option<&str>, RowError> { tc(c, self.try_get::<_, Option<&str>>(c)) }
    fn get_bytes(&self, c: &str) -> Result<&[u8], RowError> { tc(c, self.try_get::<_, &[u8]>(c)) }
    fn get_bytes_opt(&self, c: &str) -> Result<Option<&[u8]>, RowError> { tc(c, self.try_get::<_, Option<&[u8]>>(c)) }
    fn get_datetime_utc(&self, c: &str) -> Result<chrono::DateTime<chrono::Utc>, RowError> { tc(c, self.try_get::<_, chrono::DateTime<chrono::Utc>>(c)) }
    fn get_datetime_utc_opt(&self, c: &str) -> Result<Option<chrono::DateTime<chrono::Utc>>, RowError> { tc(c, self.try_get::<_, Option<chrono::DateTime<chrono::Utc>>>(c)) }
    fn get_naive_datetime(&self, c: &str) -> Result<chrono::NaiveDateTime, RowError> { tc(c, self.try_get::<_, chrono::NaiveDateTime>(c)) }
    fn get_naive_datetime_opt(&self, c: &str) -> Result<Option<chrono::NaiveDateTime>, RowError> { tc(c, self.try_get::<_, Option<chrono::NaiveDateTime>>(c)) }
    fn get_naive_date(&self, c: &str) -> Result<chrono::NaiveDate, RowError> { tc(c, self.try_get::<_, chrono::NaiveDate>(c)) }
    fn get_naive_date_opt(&self, c: &str) -> Result<Option<chrono::NaiveDate>, RowError> { tc(c, self.try_get::<_, Option<chrono::NaiveDate>>(c)) }
    fn get_naive_time(&self, c: &str) -> Result<chrono::NaiveTime, RowError> { tc(c, self.try_get::<_, chrono::NaiveTime>(c)) }
    fn get_naive_time_opt(&self, c: &str) -> Result<Option<chrono::NaiveTime>, RowError> { tc(c, self.try_get::<_, Option<chrono::NaiveTime>>(c)) }
    fn get_uuid(&self, c: &str) -> Result<uuid::Uuid, RowError> { tc(c, self.try_get::<_, uuid::Uuid>(c)) }
    fn get_uuid_opt(&self, c: &str) -> Result<Option<uuid::Uuid>, RowError> { tc(c, self.try_get::<_, Option<uuid::Uuid>>(c)) }
    fn get_json(&self, c: &str) -> Result<serde_json::Value, RowError> { tc(c, self.try_get::<_, serde_json::Value>(c)) }
    fn get_json_opt(&self, c: &str) -> Result<Option<serde_json::Value>, RowError> { tc(c, self.try_get::<_, Option<serde_json::Value>>(c)) }
    fn get_decimal(&self, c: &str) -> Result<rust_decimal::Decimal, RowError> { tc(c, self.try_get::<_, rust_decimal::Decimal>(c)) }
    fn get_decimal_opt(&self, c: &str) -> Result<Option<rust_decimal::Decimal>, RowError> { tc(c, self.try_get::<_, Option<rust_decimal::Decimal>>(c)) }
}
```

- [ ] **Step 4: Wire the module into the crate**

Edit `prax-postgres/src/lib.rs`. Add `pub mod row_ref;` next to the existing module declarations.

- [ ] **Step 5: Run the integration test**

Run: `cargo test -p prax-postgres --test row_ref`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add prax-postgres/src/row_ref.rs prax-postgres/src/lib.rs prax-postgres/tests/row_ref.rs Cargo.toml
git commit -m "feat(postgres): implement RowRef for tokio_postgres::Row

Bridges the prax-query FromRow/FromColumn machinery to
tokio_postgres::Row so engines can deserialize results into typed
models without the caller writing row extraction boilerplate."
```

---

## Task 4: Tighten `QueryEngine` trait — require `FromRow` on row-returning methods

**Files:**
- Modify: `prax-query/src/traits.rs:78-158`
- Modify: `prax-query/src/operations/find_many.rs`, `find_unique.rs`, `find_first.rs`, `create.rs`, `update.rs`, `upsert.rs`
- Modify: `prax-query/src/error.rs` (add `QueryError::deserialization`)

The current `QueryEngine` bounds row-returning methods with `T: Model + Send + 'static`. That is not enough to call `T::from_row(row)`. Add the `FromRow` bound and propagate.

- [ ] **Step 1: Add `QueryError::deserialization`**

Open `prax-query/src/error.rs`. Find the `QueryError` constructor section. If `ErrorCode::Serialization` does not exist, add it to the enum. Add a constructor:

```rust
    pub fn deserialization(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Serialization, message.into())
    }
```

- [ ] **Step 2: Write failing unit test asserting the bound is required**

Append to `prax-query/src/traits.rs` inside `mod tests`:

```rust
    // This should not compile without FromRow bound; we assert by ensuring
    // that the trait method signature requires FromRow.
    #[test]
    fn query_many_bound_surface_compiles() {
        fn takes<E: QueryEngine, T: Model + crate::row::FromRow + Send + 'static>() {}
        // If the bound changes, this signature fails to match the trait.
    }
```

Run: `cargo test -p prax-query traits::tests`
Expected: test compiles; if it ever fails to compile the bound regression is caught.

- [ ] **Step 3: Tighten the trait**

In `prax-query/src/traits.rs`, change every row-returning method's generic bound from `T: Model + Send + 'static` to `T: Model + crate::row::FromRow + Send + 'static`:

```rust
    fn query_many<T: Model + crate::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<crate::filter::FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Vec<T>>>;

    fn query_one<T: Model + crate::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<crate::filter::FilterValue>,
    ) -> BoxFuture<'_, QueryResult<T>>;

    fn query_optional<T: Model + crate::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<crate::filter::FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Option<T>>>;

    fn execute_insert<T: Model + crate::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<crate::filter::FilterValue>,
    ) -> BoxFuture<'_, QueryResult<T>>;

    fn execute_update<T: Model + crate::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<crate::filter::FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Vec<T>>>;
```

- [ ] **Step 4: Propagate the bound to every `M: Model` in `prax-query/src/operations/*.rs`**

For each operation that issues a row-returning call, find the struct definition and every `impl` block and add `FromRow`:

```bash
rg -n 'M: Model \+ Send \+' prax-query/src/operations/
```

For each match (e.g., `find_many.rs`, `find_unique.rs`, `find_first.rs`, `create.rs`, `update.rs`, `upsert.rs`), change:

```rust
impl<E: QueryEngine, M: Model> FindManyOperation<E, M> { ... }
```

to:

```rust
impl<E: QueryEngine, M: Model + crate::row::FromRow> FindManyOperation<E, M> { ... }
```

Apply the same to the `pub async fn exec(self) -> QueryResult<Vec<M>> where M: Send + 'static` bounds — add `FromRow + ` before `Send`.

Leave `delete.rs`, `count.rs`, `execute_raw` untouched — they do not deserialize rows.

- [ ] **Step 5: Give each test-module `TestModel` a `FromRow` stub**

For each operation test file, find `impl Model for TestModel` and add below it:

```rust
impl crate::row::FromRow for TestModel {
    fn from_row(_row: &impl crate::row::RowRef) -> Result<Self, crate::row::RowError> {
        Ok(TestModel)
    }
}
```

- [ ] **Step 6: Run unit tests**

Run: `cargo test -p prax-query`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add prax-query/src/traits.rs prax-query/src/error.rs prax-query/src/operations/
git commit -m "refactor(query): require FromRow on row-returning QueryEngine methods

QueryEngine::query_many et al previously could not call T::from_row
because the bound was only Model + Send + 'static. Tightens the
bound to Model + FromRow + Send + 'static and propagates to every
FindMany / FindUnique / FindFirst / Create / Update / Upsert
operation. Also adds QueryError::deserialization."
```

---

## Task 5: Create `deserialize.rs` helper and rewrite `PgEngine`

**Files:**
- Create: `prax-postgres/src/deserialize.rs`
- Modify: `prax-postgres/src/engine.rs:45-285`
- Modify: `prax-postgres/src/lib.rs`
- Test: `prax-postgres/tests/engine_crud.rs`

- [ ] **Step 1: Write failing end-to-end test**

Create `prax-postgres/tests/engine_crud.rs`:

```rust
use prax_postgres::{PgEngine, PgPool, PgPoolBuilder};
use prax_query::filter::FilterValue;
use prax_query::row::{FromRow, RowError, RowRef};
use prax_query::traits::{Model, QueryEngine};

fn test_url() -> String {
    std::env::var("PRAX_POSTGRES_URL")
        .unwrap_or_else(|_| "postgres://prax:prax_test_password@localhost:5433/prax_test".into())
}

#[derive(Debug, PartialEq)]
struct Person { id: i32, email: String }

impl Model for Person {
    const MODEL_NAME: &'static str = "Person";
    const TABLE_NAME: &'static str = "crud_people";
    const PRIMARY_KEY: &'static [&'static str] = &["id"];
    const COLUMNS: &'static [&'static str] = &["id", "email"];
}

impl FromRow for Person {
    fn from_row(row: &impl RowRef) -> Result<Self, RowError> {
        Ok(Person { id: row.get_i32("id")?, email: row.get_string("email")? })
    }
}

async fn setup(pool: &PgPool) {
    let conn = pool.get().await.unwrap();
    conn.batch_execute(
        "DROP TABLE IF EXISTS crud_people;
         CREATE TABLE crud_people (id SERIAL PRIMARY KEY, email TEXT NOT NULL);
         INSERT INTO crud_people (email) VALUES ('alice@example.com'), ('bob@example.com');",
    ).await.unwrap();
}

#[tokio::test]
async fn query_many_returns_typed_rows() {
    let pool: PgPool = PgPoolBuilder::new().url(test_url()).build().await.unwrap();
    setup(&pool).await;
    let engine = PgEngine::new(pool);
    let rows = engine
        .query_many::<Person>("SELECT id, email FROM crud_people ORDER BY id", Vec::<FilterValue>::new())
        .await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].email, "alice@example.com");
}
```

Run: `cargo test -p prax-postgres --test engine_crud`
Expected: assertion fail — engine still returns `Vec::new()`.

- [ ] **Step 2: Create `deserialize.rs`**

```rust
//! Row -> T: FromRow helpers for PgEngine.

use prax_query::error::{QueryError, QueryResult};
use prax_query::row::FromRow;
use tokio_postgres::Row;

pub fn rows_into<T: FromRow>(rows: Vec<Row>) -> QueryResult<Vec<T>> {
    rows.into_iter()
        .map(|r| T::from_row(&r).map_err(|e| QueryError::deserialization(e.to_string())))
        .collect()
}

pub fn row_into<T: FromRow>(row: Row) -> QueryResult<T> {
    T::from_row(&row).map_err(|e| QueryError::deserialization(e.to_string()))
}
```

- [ ] **Step 3: Rewrite the five broken methods in `engine.rs`**

Replace lines 45-207 of `prax-postgres/src/engine.rs`. For each of `query_many`, `query_one`, `query_optional`, `execute_insert`, `execute_update`: after the driver-level query call, pipe the rows through `crate::deserialize::rows_into::<T>(rows)` or `crate::deserialize::row_into::<T>(row)`. Add the `prax_query::row::FromRow` bound to every method's `T:` list to match the trait.

- [ ] **Step 4: Export the module**

Add `pub mod deserialize;` to `prax-postgres/src/lib.rs`.

- [ ] **Step 5: Run the integration test**

Run: `cargo test -p prax-postgres --test engine_crud`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add prax-postgres/ 
git commit -m "feat(postgres): deserialize rows into typed models in PgEngine

PgEngine previously threw away every row and returned
'deserialization not yet implemented' or Vec::new(). All four query
methods now decode rows via prax_query::row::FromRow."
```

---

## Task 6: Implement `RowRef` for rusqlite

**Files:**
- Create: `prax-sqlite/src/row_ref.rs`
- Modify: `prax-sqlite/src/lib.rs`
- Test: `prax-sqlite/tests/row_ref.rs`

- [ ] **Step 1: Write failing test**

Create `prax-sqlite/tests/row_ref.rs`:

```rust
use prax_query::row::RowRef;
use prax_sqlite::row_ref::SqliteRowRef;
use rusqlite::Connection;

#[test]
fn materializes_row_from_rusqlite() {
    let conn = Connection::open_in_memory().unwrap();
    let mut stmt = conn.prepare("SELECT 42 AS n, 'hello' AS s").unwrap();
    let mut rows = stmt.query([]).unwrap();
    let row = rows.next().unwrap().unwrap();
    let r = SqliteRowRef::from_rusqlite(row).unwrap();
    assert_eq!(r.get_i32("n").unwrap(), 42);
    assert_eq!(r.get_str("s").unwrap(), "hello");
}
```

Run: `cargo test -p prax-sqlite --test row_ref`
Expected: COMPILE FAIL.

- [ ] **Step 2: Create `prax-sqlite/src/row_ref.rs`**

rusqlite rows are statement-bound, so materialize each row into an owned `HashMap<String, Value>` and implement `RowRef` against that snapshot. Include `get_datetime_utc`/`get_uuid`/`get_json`/`get_decimal` by parsing from the stored text representation (SQLite stores these as strings).

```rust
//! Bridge between rusqlite rows and prax_query::row::RowRef.

use std::collections::HashMap;

use prax_query::row::{RowError, RowRef};
use rusqlite::types::{Value, ValueRef};
use rusqlite::Row;

pub struct SqliteRowRef {
    values: HashMap<String, Value>,
}

impl SqliteRowRef {
    pub fn from_rusqlite(row: &Row<'_>) -> Result<Self, RowError> {
        let stmt = row.as_ref();
        let mut values = HashMap::with_capacity(stmt.column_count());
        for i in 0..stmt.column_count() {
            let name = stmt.column_name(i)
                .map_err(|e| RowError::TypeConversion { column: i.to_string(), message: e.to_string() })?
                .to_string();
            let v: Value = match row.get_ref(i)
                .map_err(|e| RowError::TypeConversion { column: name.clone(), message: e.to_string() })?
            {
                ValueRef::Null => Value::Null,
                ValueRef::Integer(i) => Value::Integer(i),
                ValueRef::Real(f) => Value::Real(f),
                ValueRef::Text(b) => Value::Text(String::from_utf8_lossy(b).into_owned()),
                ValueRef::Blob(b) => Value::Blob(b.to_vec()),
            };
            values.insert(name, v);
        }
        Ok(Self { values })
    }

    fn tc(column: &str, msg: impl Into<String>) -> RowError {
        RowError::TypeConversion { column: column.into(), message: msg.into() }
    }
}

impl RowRef for SqliteRowRef {
    fn get_i32(&self, c: &str) -> Result<i32, RowError> {
        match self.values.get(c).ok_or_else(|| RowError::ColumnNotFound(c.into()))? {
            Value::Integer(i) => i32::try_from(*i).map_err(|_| Self::tc(c, "i64 overflow")),
            Value::Null => Err(RowError::UnexpectedNull(c.into())),
            _ => Err(Self::tc(c, "not an integer")),
        }
    }
    fn get_i32_opt(&self, c: &str) -> Result<Option<i32>, RowError> {
        match self.values.get(c) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Integer(i)) => i32::try_from(*i).map(Some).map_err(|_| Self::tc(c, "overflow")),
            Some(_) => Err(Self::tc(c, "not an integer")),
        }
    }
    fn get_i64(&self, c: &str) -> Result<i64, RowError> {
        match self.values.get(c).ok_or_else(|| RowError::ColumnNotFound(c.into()))? {
            Value::Integer(i) => Ok(*i),
            Value::Null => Err(RowError::UnexpectedNull(c.into())),
            _ => Err(Self::tc(c, "not an integer")),
        }
    }
    fn get_i64_opt(&self, c: &str) -> Result<Option<i64>, RowError> {
        match self.values.get(c) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Integer(i)) => Ok(Some(*i)),
            Some(_) => Err(Self::tc(c, "not an integer")),
        }
    }
    fn get_f64(&self, c: &str) -> Result<f64, RowError> {
        match self.values.get(c).ok_or_else(|| RowError::ColumnNotFound(c.into()))? {
            Value::Real(f) => Ok(*f),
            Value::Integer(i) => Ok(*i as f64),
            Value::Null => Err(RowError::UnexpectedNull(c.into())),
            _ => Err(Self::tc(c, "not a number")),
        }
    }
    fn get_f64_opt(&self, c: &str) -> Result<Option<f64>, RowError> {
        match self.values.get(c) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Real(f)) => Ok(Some(*f)),
            Some(Value::Integer(i)) => Ok(Some(*i as f64)),
            Some(_) => Err(Self::tc(c, "not a number")),
        }
    }
    fn get_bool(&self, c: &str) -> Result<bool, RowError> { self.get_i64(c).map(|i| i != 0) }
    fn get_bool_opt(&self, c: &str) -> Result<Option<bool>, RowError> { self.get_i64_opt(c).map(|o| o.map(|i| i != 0)) }
    fn get_str(&self, c: &str) -> Result<&str, RowError> {
        match self.values.get(c).ok_or_else(|| RowError::ColumnNotFound(c.into()))? {
            Value::Text(s) => Ok(s.as_str()),
            Value::Null => Err(RowError::UnexpectedNull(c.into())),
            _ => Err(Self::tc(c, "not text")),
        }
    }
    fn get_str_opt(&self, c: &str) -> Result<Option<&str>, RowError> {
        match self.values.get(c) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Text(s)) => Ok(Some(s.as_str())),
            Some(_) => Err(Self::tc(c, "not text")),
        }
    }
    fn get_bytes(&self, c: &str) -> Result<&[u8], RowError> {
        match self.values.get(c).ok_or_else(|| RowError::ColumnNotFound(c.into()))? {
            Value::Blob(b) => Ok(b.as_slice()),
            Value::Text(s) => Ok(s.as_bytes()),
            Value::Null => Err(RowError::UnexpectedNull(c.into())),
            _ => Err(Self::tc(c, "not blob")),
        }
    }
    fn get_bytes_opt(&self, c: &str) -> Result<Option<&[u8]>, RowError> {
        match self.values.get(c) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Blob(b)) => Ok(Some(b.as_slice())),
            Some(Value::Text(s)) => Ok(Some(s.as_bytes())),
            Some(_) => Err(Self::tc(c, "not blob")),
        }
    }
    fn get_datetime_utc(&self, c: &str) -> Result<chrono::DateTime<chrono::Utc>, RowError> {
        let s = self.get_str(c)?;
        chrono::DateTime::parse_from_rfc3339(s)
            .map(|d| d.with_timezone(&chrono::Utc))
            .map_err(|e| Self::tc(c, e.to_string()))
    }
    fn get_datetime_utc_opt(&self, c: &str) -> Result<Option<chrono::DateTime<chrono::Utc>>, RowError> {
        match self.get_str_opt(c)? { None => Ok(None),
            Some(s) => chrono::DateTime::parse_from_rfc3339(s)
                .map(|d| Some(d.with_timezone(&chrono::Utc)))
                .map_err(|e| Self::tc(c, e.to_string())) }
    }
    fn get_uuid(&self, c: &str) -> Result<uuid::Uuid, RowError> {
        uuid::Uuid::parse_str(self.get_str(c)?).map_err(|e| Self::tc(c, e.to_string()))
    }
    fn get_uuid_opt(&self, c: &str) -> Result<Option<uuid::Uuid>, RowError> {
        match self.get_str_opt(c)? { None => Ok(None),
            Some(s) => uuid::Uuid::parse_str(s).map(Some).map_err(|e| Self::tc(c, e.to_string())) }
    }
    fn get_json(&self, c: &str) -> Result<serde_json::Value, RowError> {
        serde_json::from_str(self.get_str(c)?).map_err(|e| Self::tc(c, e.to_string()))
    }
    fn get_json_opt(&self, c: &str) -> Result<Option<serde_json::Value>, RowError> {
        match self.get_str_opt(c)? { None => Ok(None),
            Some(s) => serde_json::from_str(s).map(Some).map_err(|e| Self::tc(c, e.to_string())) }
    }
    fn get_decimal(&self, c: &str) -> Result<rust_decimal::Decimal, RowError> {
        self.get_str(c)?.parse::<rust_decimal::Decimal>().map_err(|e| Self::tc(c, e.to_string()))
    }
    fn get_decimal_opt(&self, c: &str) -> Result<Option<rust_decimal::Decimal>, RowError> {
        match self.get_str_opt(c)? { None => Ok(None),
            Some(s) => s.parse::<rust_decimal::Decimal>().map(Some).map_err(|e| Self::tc(c, e.to_string())) }
    }
}
```

- [ ] **Step 3: Export module**

Add `pub mod row_ref;` to `prax-sqlite/src/lib.rs`.

- [ ] **Step 4: Run test**

Run: `cargo test -p prax-sqlite --test row_ref`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add prax-sqlite/src/row_ref.rs prax-sqlite/src/lib.rs prax-sqlite/tests/row_ref.rs
git commit -m "feat(sqlite): implement RowRef for rusqlite rows"
```

---

## Task 7: Introduce `SqlDialect` trait

**Files:**
- Create: `prax-query/src/dialect.rs`
- Modify: `prax-query/src/lib.rs`
- Modify: `prax-query/src/traits.rs` (add `dialect()` method to `QueryEngine`)
- Test: `prax-query/src/dialect.rs` (unit tests)

Hard-coded Postgres syntax (`$N`, `RETURNING *`) breaks MySQL (`?`), SQLite (`?N`), and MSSQL (`@PN`, `OUTPUT INSERTED`). Abstract via a dialect trait attached to each engine.

- [ ] **Step 1: Create `prax-query/src/dialect.rs`**

```rust
//! Abstraction over SQL dialect differences (placeholders, RETURNING, quoting, upsert).

pub trait SqlDialect: Send + Sync {
    fn placeholder(&self, i: usize) -> String;
    fn returning_clause(&self, cols: &str) -> String;
    fn quote_ident(&self, ident: &str) -> String;
    fn supports_distinct_on(&self) -> bool { false }
    fn insert_has_returning(&self) -> bool { true }
    fn upsert_clause(&self, conflict_cols: &[&str], update_set: &str) -> String;
    fn begin_sql(&self) -> &'static str { "BEGIN" }
    fn commit_sql(&self) -> &'static str { "COMMIT" }
    fn rollback_sql(&self) -> &'static str { "ROLLBACK" }
}

pub struct Postgres;
pub struct Sqlite;
pub struct Mysql;
pub struct Mssql;

impl SqlDialect for Postgres {
    fn placeholder(&self, i: usize) -> String { format!("${}", i) }
    fn returning_clause(&self, cols: &str) -> String { format!(" RETURNING {}", cols) }
    fn quote_ident(&self, i: &str) -> String { format!("\"{}\"", i.replace('"', "\"\"")) }
    fn supports_distinct_on(&self) -> bool { true }
    fn upsert_clause(&self, c: &[&str], s: &str) -> String {
        format!(" ON CONFLICT ({}) DO UPDATE SET {}", c.join(", "), s)
    }
}
impl SqlDialect for Sqlite {
    fn placeholder(&self, i: usize) -> String { format!("?{}", i) }
    fn returning_clause(&self, cols: &str) -> String { format!(" RETURNING {}", cols) }
    fn quote_ident(&self, i: &str) -> String { format!("\"{}\"", i.replace('"', "\"\"")) }
    fn upsert_clause(&self, c: &[&str], s: &str) -> String {
        format!(" ON CONFLICT ({}) DO UPDATE SET {}", c.join(", "), s)
    }
}
impl SqlDialect for Mysql {
    fn placeholder(&self, _i: usize) -> String { "?".into() }
    fn returning_clause(&self, cols: &str) -> String { format!(" RETURNING {}", cols) }
    fn quote_ident(&self, i: &str) -> String { format!("`{}`", i.replace('`', "``")) }
    fn upsert_clause(&self, _c: &[&str], s: &str) -> String {
        format!(" ON DUPLICATE KEY UPDATE {}", s)
    }
}
impl SqlDialect for Mssql {
    fn placeholder(&self, i: usize) -> String { format!("@P{}", i) }
    fn returning_clause(&self, cols: &str) -> String {
        format!(" OUTPUT INSERTED.{}", if cols == "*" { "*".into() } else { cols.to_string() })
    }
    fn quote_ident(&self, i: &str) -> String { format!("[{}]", i.replace(']', "]]")) }
    fn insert_has_returning(&self) -> bool { true }
    // SQL Server MERGE is complex; drivers may post-process upserts.
    fn upsert_clause(&self, _c: &[&str], _s: &str) -> String {
        String::new()
    }

    fn begin_sql(&self) -> &'static str { "BEGIN TRANSACTION" }
    fn commit_sql(&self) -> &'static str { "COMMIT TRANSACTION" }
    fn rollback_sql(&self) -> &'static str { "ROLLBACK TRANSACTION" }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn placeholders() {
        assert_eq!(Postgres.placeholder(3), "$3");
        assert_eq!(Sqlite.placeholder(3), "?3");
        assert_eq!(Mysql.placeholder(3), "?");
        assert_eq!(Mssql.placeholder(3), "@P3");
    }
    #[test]
    fn returning_mssql_is_output() {
        assert_eq!(Mssql.returning_clause("*"), " OUTPUT INSERTED.*");
    }
    #[test]
    fn upsert_mysql_duplicate_key() {
        assert_eq!(Mysql.upsert_clause(&[], "x = 1"), " ON DUPLICATE KEY UPDATE x = 1");
    }
}
```

- [ ] **Step 2: Add `dialect()` to `QueryEngine`**

In `prax-query/src/traits.rs`, add:

```rust
    fn dialect(&self) -> &dyn crate::dialect::SqlDialect;
```

- [ ] **Step 3: Re-export from `prax-query/src/lib.rs`**

Add `pub mod dialect;` near the top module declarations. Add `pub use dialect::{Mssql, Mysql, Postgres, Sqlite, SqlDialect};` to the re-exports.

- [ ] **Step 4: Implement `dialect()` on `PgEngine`**

Edit `prax-postgres/src/engine.rs`, in the `impl QueryEngine for PgEngine` block, add:

```rust
    fn dialect(&self) -> &dyn prax_query::dialect::SqlDialect { &prax_query::dialect::Postgres }
```

(SQLite/MySQL/MSSQL engines get the same treatment in their respective rewrite tasks below.)

- [ ] **Step 5: Run unit tests**

Run: `cargo test -p prax-query dialect::tests`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add prax-query/src/dialect.rs prax-query/src/lib.rs prax-query/src/traits.rs prax-postgres/src/engine.rs
git commit -m "feat(query): add SqlDialect trait attached to QueryEngine

Operations hard-coded Postgres syntax. SqlDialect abstracts
placeholders, RETURNING/OUTPUT, identifier quoting, upsert, and
BEGIN/COMMIT/ROLLBACK per database. PgEngine wires up Postgres
dialect; sibling drivers follow in later tasks."
```

---

## Task 8: Thread `SqlDialect` through every `build_sql`

**Files:**
- Modify: `prax-query/src/operations/find_many.rs`
- Modify: `prax-query/src/operations/find_unique.rs`, `find_first.rs`, `create.rs`, `update.rs`, `upsert.rs`, `delete.rs`, `count.rs`, `aggregate.rs`
- Modify: `prax-query/src/filter.rs` (Filter::to_sql takes dialect)

Every `build_sql(&self)` becomes `build_sql(&self, dialect: &dyn SqlDialect)` and every placeholder emission site switches from `format!("${}", i)` to `dialect.placeholder(i)`. `.exec()` passes `self.engine.dialect()`.

- [ ] **Step 1: Change `Filter::to_sql` signature**

Open `prax-query/src/filter.rs`. Locate `impl Filter { pub fn to_sql(&self, start_param_idx: usize) -> (String, Vec<FilterValue>) }`. Change to:

```rust
pub fn to_sql(
    &self,
    start_param_idx: usize,
    dialect: &dyn crate::dialect::SqlDialect,
) -> (String, Vec<FilterValue>) { /* ... */ }
```

Within, replace every `format!("${}", idx)` with `dialect.placeholder(idx)`. Same change in every recursion path.

- [ ] **Step 2: Update `FindManyOperation::build_sql` to take `dialect`**

```rust
pub fn build_sql(&self, dialect: &dyn crate::dialect::SqlDialect) -> (String, Vec<crate::filter::FilterValue>) {
    let (where_sql, params) = self.filter.to_sql(0, dialect);
    // ... rest unchanged ...
}
```

Update `exec` to pass `self.engine.dialect()`:

```rust
pub async fn exec(self) -> QueryResult<Vec<M>> where M: Send + 'static + FromRow {
    let dialect = self.engine.dialect();
    let (sql, params) = self.build_sql(dialect);
    self.engine.query_many::<M>(&sql, params).await
}
```

- [ ] **Step 3: Repeat for the other operations**

Apply the same `build_sql(dialect)` signature change to:
- `find_unique.rs`, `find_first.rs` (same shape as find_many)
- `create.rs`: the `RETURNING *` emission switches to `dialect.returning_clause("*")`
- `update.rs`: both `SET` placeholders and `WHERE` filter use `dialect.placeholder`
- `upsert.rs`: use `dialect.upsert_clause(conflict_cols, set_clause)`
- `delete.rs`: only `WHERE` placeholders need the change
- `count.rs`, `aggregate.rs`: same

The existing unit tests in each file use Postgres-flavored assertions — update them to inject `&prax_query::dialect::Postgres` and keep the same expected strings.

- [ ] **Step 4: Add a cross-dialect regression test**

Append to `prax-query/src/operations/find_many.rs` inside `mod tests`:

```rust
    #[test]
    fn builds_mysql_placeholders() {
        let op = FindManyOperation::<MockEngine, TestModel>::new(MockEngine)
            .r#where(Filter::Equals("name".into(), "a".into()));
        let (sql, _) = op.build_sql(&crate::dialect::Mysql);
        assert!(sql.contains("?") && !sql.contains("$1"));
    }
    #[test]
    fn builds_mssql_placeholders() {
        let op = FindManyOperation::<MockEngine, TestModel>::new(MockEngine)
            .r#where(Filter::Equals("name".into(), "a".into()));
        let (sql, _) = op.build_sql(&crate::dialect::Mssql);
        assert!(sql.contains("@P1"));
    }
```

- [ ] **Step 5: Extend `MockEngine` in each test file to implement `dialect()`**

Each test module's `MockEngine` needs to return a dialect. Add:

```rust
    fn dialect(&self) -> &dyn crate::dialect::SqlDialect { &crate::dialect::Postgres }
```

- [ ] **Step 6: Run unit tests**

Run: `cargo test -p prax-query operations::`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add prax-query/src/operations/ prax-query/src/filter.rs
git commit -m "refactor(query): thread SqlDialect through every build_sql

Every Operation now takes a dialect for placeholder and RETURNING
emission. Removes hard-coded Postgres syntax so the same build_sql
works against MySQL, SQLite, and MSSQL. Cross-dialect regression
tests locked in."
```

---

## Task 9: Replace SQLite engine with `QueryEngine` impl

**Files:**
- Modify: `prax-sqlite/src/engine.rs` (full rewrite)
- Create: `prax-sqlite/src/raw.rs` (old JSON API moves here)
- Modify: `prax-sqlite/src/lib.rs`
- Test: `prax-sqlite/tests/engine_crud.rs`

- [ ] **Step 1: Write failing CRUD test**

Create `prax-sqlite/tests/engine_crud.rs`:

```rust
use prax_query::filter::FilterValue;
use prax_query::row::{FromRow, RowError, RowRef};
use prax_query::traits::{Model, QueryEngine};
use prax_sqlite::{SqliteEngine, SqlitePool, SqlitePoolBuilder};

struct Item { id: i32, name: String }
impl Model for Item {
    const MODEL_NAME: &'static str = "Item";
    const TABLE_NAME: &'static str = "items";
    const PRIMARY_KEY: &'static [&'static str] = &["id"];
    const COLUMNS: &'static [&'static str] = &["id", "name"];
}
impl FromRow for Item {
    fn from_row(r: &impl RowRef) -> Result<Self, RowError> {
        Ok(Item { id: r.get_i32("id")?, name: r.get_string("name")? })
    }
}

#[tokio::test]
async fn sqlite_query_many() {
    let pool: SqlitePool = SqlitePoolBuilder::new().url(":memory:").build().await.unwrap();
    let engine = SqliteEngine::new(pool);
    engine.execute_raw("CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT NOT NULL)", vec![]).await.unwrap();
    engine.execute_raw("INSERT INTO items (id, name) VALUES (1, 'a'), (2, 'b')", vec![]).await.unwrap();
    let rows = engine.query_many::<Item>("SELECT id, name FROM items ORDER BY id", vec![]).await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].name, "a");
}
```

Run: `cargo test -p prax-sqlite --test engine_crud`
Expected: COMPILE FAIL — `SqliteEngine` doesn't implement `QueryEngine`.

- [ ] **Step 2: Move legacy JSON API to `raw.rs`**

Move the entire current `SqliteEngine` body into `prax-sqlite/src/raw.rs`, renaming the type to `SqliteRawEngine` and `SqliteQueryResult` to `SqliteJsonRow`. Keep all method signatures identical so existing callers switch from `SqliteEngine` to `raw::SqliteRawEngine`.

- [ ] **Step 3: Rewrite `engine.rs` with a `QueryEngine` impl**

Full replacement content:

```rust
//! SQLite query engine implementing prax_query::QueryEngine.

use prax_query::error::{QueryError, QueryResult};
use prax_query::filter::FilterValue;
use prax_query::row::FromRow;
use prax_query::traits::{BoxFuture, Model, QueryEngine};
use rusqlite::types::Value as SqlValue;
use tracing::debug;

use crate::pool::SqlitePool;
use crate::row_ref::SqliteRowRef;
use crate::types::filter_value_to_sqlite;

#[derive(Clone)]
pub struct SqliteEngine { pool: SqlitePool }

impl SqliteEngine {
    pub fn new(pool: SqlitePool) -> Self { Self { pool } }
    pub fn pool(&self) -> &SqlitePool { &self.pool }
    fn bind(params: &[FilterValue]) -> Vec<SqlValue> {
        params.iter().map(filter_value_to_sqlite).collect()
    }
}

impl QueryEngine for SqliteEngine {
    fn dialect(&self) -> &dyn prax_query::dialect::SqlDialect { &prax_query::dialect::Sqlite }

    fn query_many<T: Model + FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Vec<T>>> {
        let sql = sql.to_string();
        Box::pin(async move {
            debug!(sql = %sql, "sqlite query_many");
            let conn = self.pool.get().await
                .map_err(|e| QueryError::connection(e.to_string()))?;
            let bound = Self::bind(&params);
            let snapshots: Vec<SqliteRowRef> = conn.call(move |c| {
                let mut stmt = c.prepare(&sql)?;
                let refs: Vec<&dyn rusqlite::ToSql> =
                    bound.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
                let mut rows = stmt.query(refs.as_slice())?;
                let mut out = Vec::new();
                while let Some(row) = rows.next()? {
                    out.push(SqliteRowRef::from_rusqlite(row).map_err(|e| {
                        rusqlite::Error::ToSqlConversionFailure(
                            Box::new(std::io::Error::other(e.to_string())))
                    })?);
                }
                Ok(out)
            }).await.map_err(|e| QueryError::database(e.to_string()))?;

            snapshots.into_iter()
                .map(|r| T::from_row(&r).map_err(|e| QueryError::deserialization(e.to_string())))
                .collect()
        })
    }

    fn query_one<T: Model + FromRow + Send + 'static>(&self, sql: &str, params: Vec<FilterValue>) -> BoxFuture<'_, QueryResult<T>> {
        Box::pin(async move {
            let mut rows = self.query_many::<T>(sql, params).await?;
            rows.pop().ok_or_else(|| QueryError::not_found(T::MODEL_NAME))
        })
    }
    fn query_optional<T: Model + FromRow + Send + 'static>(&self, sql: &str, params: Vec<FilterValue>) -> BoxFuture<'_, QueryResult<Option<T>>> {
        Box::pin(async move {
            let mut rows = self.query_many::<T>(sql, params).await?;
            Ok(rows.pop())
        })
    }
    fn execute_insert<T: Model + FromRow + Send + 'static>(&self, sql: &str, params: Vec<FilterValue>) -> BoxFuture<'_, QueryResult<T>> {
        Box::pin(async move { self.query_one::<T>(sql, params).await })
    }
    fn execute_update<T: Model + FromRow + Send + 'static>(&self, sql: &str, params: Vec<FilterValue>) -> BoxFuture<'_, QueryResult<Vec<T>>> {
        Box::pin(async move { self.query_many::<T>(sql, params).await })
    }
    fn execute_delete(&self, sql: &str, params: Vec<FilterValue>) -> BoxFuture<'_, QueryResult<u64>> {
        let sql = sql.to_string();
        Box::pin(async move {
            let conn = self.pool.get().await.map_err(|e| QueryError::connection(e.to_string()))?;
            let bound = Self::bind(&params);
            let n = conn.call(move |c| {
                let refs: Vec<&dyn rusqlite::ToSql> =
                    bound.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
                Ok(c.execute(&sql, refs.as_slice())?)
            }).await.map_err(|e| QueryError::database(e.to_string()))?;
            Ok(n as u64)
        })
    }
    fn execute_raw(&self, sql: &str, params: Vec<FilterValue>) -> BoxFuture<'_, QueryResult<u64>> {
        self.execute_delete(sql, params)
    }
    fn count(&self, sql: &str, params: Vec<FilterValue>) -> BoxFuture<'_, QueryResult<u64>> {
        let sql = sql.to_string();
        Box::pin(async move {
            let conn = self.pool.get().await.map_err(|e| QueryError::connection(e.to_string()))?;
            let bound = Self::bind(&params);
            let n = conn.call(move |c| {
                let refs: Vec<&dyn rusqlite::ToSql> =
                    bound.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
                let mut stmt = c.prepare(&sql)?;
                let n: i64 = stmt.query_row(refs.as_slice(), |r| r.get(0))?;
                Ok(n)
            }).await.map_err(|e| QueryError::database(e.to_string()))?;
            Ok(n as u64)
        })
    }
}
```

- [ ] **Step 4: Update `prax-sqlite/src/lib.rs`**

```rust
pub mod config;
pub mod connection;
pub mod engine;
pub mod error;
pub mod pool;
pub mod raw;
pub mod row;
pub mod row_ref;
pub mod types;

pub use engine::SqliteEngine;
pub use pool::{SqlitePool, SqlitePoolBuilder};
pub use raw::{SqliteJsonRow, SqliteRawEngine};
```

- [ ] **Step 5: Verify all `FilterValue` variants are handled in `filter_value_to_sqlite`**

Run `rg 'FilterValue::' prax-query/src/filter.rs` to list variants. Extend `prax-sqlite/src/types.rs::filter_value_to_sqlite` to cover every variant the typed engine can emit.

- [ ] **Step 6: Run integration test**

Run: `cargo test -p prax-sqlite --test engine_crud`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add prax-sqlite/
git commit -m "refactor(sqlite): implement QueryEngine on SqliteEngine

SqliteEngine used to return JSON blobs with no path to typed
models. Rewrites on FromRow / SqliteRowRef. Legacy JSON API moves
to prax_sqlite::raw (SqliteRawEngine / SqliteJsonRow)."
```

---

## Task 10: Implement `RowRef` for MySQL

**Files:**
- Create: `prax-mysql/src/row_ref.rs`
- Modify: `prax-mysql/src/lib.rs`
- Test: `prax-mysql/tests/row_ref.rs`

- [ ] **Step 1: Write failing test (MySQL must be running)**

```rust
// prax-mysql/tests/row_ref.rs
use mysql_async::prelude::*;
use prax_mysql::row_ref::MysqlRowRef;
use prax_query::row::RowRef;

fn test_url() -> String {
    std::env::var("PRAX_MYSQL_URL")
        .unwrap_or_else(|_| "mysql://prax:prax_test_password@localhost:3307/prax_test".into())
}

#[tokio::test]
async fn get_i32_and_string_from_row() {
    let pool = mysql_async::Pool::new(test_url().as_str());
    let mut conn = pool.get_conn().await.unwrap();
    let rows: Vec<mysql_async::Row> = conn.query("SELECT 42 AS n, 'hello' AS s").await.unwrap();
    let r = MysqlRowRef::from_row(rows.into_iter().next().unwrap()).unwrap();
    assert_eq!(r.get_i32("n").unwrap(), 42);
    assert_eq!(r.get_str("s").unwrap(), "hello");
}
```

Run: `cargo test -p prax-mysql --test row_ref`
Expected: COMPILE FAIL.

- [ ] **Step 2: Create `prax-mysql/src/row_ref.rs`**

`mysql_async::Row` gives owned `Value`s. Materialize into a `HashMap<String, Value>` keyed on column name. String reads need a cache because `RowRef::get_str` returns `&str` — store decoded strings in a `RefCell<HashMap<String, String>>` so repeated reads are stable:

```rust
use std::cell::RefCell;
use std::collections::HashMap;

use mysql_async::{Row, Value};
use prax_query::row::{RowError, RowRef};

pub struct MysqlRowRef {
    values: HashMap<String, Value>,
    text_cache: RefCell<HashMap<String, String>>,
}

impl MysqlRowRef {
    pub fn from_row(row: Row) -> Result<Self, RowError> {
        let columns = row.columns_ref().to_vec();
        let mut values = HashMap::with_capacity(columns.len());
        for (i, col) in columns.iter().enumerate() {
            let name = col.name_str().to_string();
            let v: Option<Value> = row.get(i);
            values.insert(name, v.unwrap_or(Value::NULL));
        }
        Ok(Self { values, text_cache: RefCell::new(HashMap::new()) })
    }

    fn get(&self, c: &str) -> Result<&Value, RowError> {
        self.values.get(c).ok_or_else(|| RowError::ColumnNotFound(c.into()))
    }
    fn tc(c: &str, m: impl Into<String>) -> RowError {
        RowError::TypeConversion { column: c.into(), message: m.into() }
    }

    fn cache_text(&self, c: &str) -> Result<(), RowError> {
        if self.text_cache.borrow().contains_key(c) { return Ok(()); }
        let text = match self.get(c)? {
            Value::Bytes(b) => std::str::from_utf8(b)
                .map_err(|e| Self::tc(c, e.to_string()))?.to_string(),
            Value::NULL => return Err(RowError::UnexpectedNull(c.into())),
            Value::Int(i) => i.to_string(),
            Value::UInt(u) => u.to_string(),
            Value::Float(f) => f.to_string(),
            Value::Double(d) => d.to_string(),
            Value::Date(y, mo, d, h, mi, s, us) => format!(
                "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:06}Z", y, mo, d, h, mi, s, us
            ),
            Value::Time(neg, days, h, m, s, us) => {
                let sign = if *neg { "-" } else { "" };
                format!("{}{}:{:02}:{:02}.{:06}", sign, days * 24 + (*h as u32), m, s, us)
            }
        };
        self.text_cache.borrow_mut().insert(c.to_string(), text);
        Ok(())
    }
}
```

Then implement every `RowRef` method by matching on the stored `Value`. For `get_str`, call `self.cache_text(c)?` and return a `&str` borrowed from the cache — safe because `RefCell` owns the string and we only grow it. See the Postgres implementation for patterns; the same scalar methods apply. For UUID/JSON/decimal, parse from the text form (MySQL stores as VARCHAR/JSON/DECIMAL). For `get_datetime_utc`, read `Value::Date(...)` directly and construct a `chrono::DateTime<Utc>`.

Full file content is the natural extension of Step 2 of Task 6 (SQLite), adapted to MySQL's `Value` enum. Keep `get_bytes` returning `&[u8]` directly from `Value::Bytes`.

- [ ] **Step 3: Export module**

Add `pub mod row_ref;` to `prax-mysql/src/lib.rs`.

- [ ] **Step 4: Run test**

Run: `docker compose up -d mysql && cargo test -p prax-mysql --test row_ref`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add prax-mysql/src/row_ref.rs prax-mysql/src/lib.rs prax-mysql/tests/row_ref.rs
git commit -m "feat(mysql): implement RowRef for mysql_async::Row"
```

---

## Task 11: Replace MySQL engine with `QueryEngine` impl

**Files:**
- Modify: `prax-mysql/src/engine.rs` (full rewrite)
- Create: `prax-mysql/src/raw.rs` (move the existing JSON API here)
- Modify: `prax-mysql/src/lib.rs`
- Test: `prax-mysql/tests/engine_crud.rs`

- [ ] **Step 1: Write failing test**

```rust
// prax-mysql/tests/engine_crud.rs
use prax_mysql::{MysqlEngine, MysqlPool, MysqlPoolBuilder};
use prax_query::row::{FromRow, RowError, RowRef};
use prax_query::traits::{Model, QueryEngine};

struct Person { id: i32, email: String }
impl Model for Person {
    const MODEL_NAME: &'static str = "Person";
    const TABLE_NAME: &'static str = "crud_people_my";
    const PRIMARY_KEY: &'static [&'static str] = &["id"];
    const COLUMNS: &'static [&'static str] = &["id", "email"];
}
impl FromRow for Person {
    fn from_row(r: &impl RowRef) -> Result<Self, RowError> {
        Ok(Person { id: r.get_i32("id")?, email: r.get_string("email")? })
    }
}

fn url() -> String {
    std::env::var("PRAX_MYSQL_URL")
        .unwrap_or_else(|_| "mysql://prax:prax_test_password@localhost:3307/prax_test".into())
}

#[tokio::test]
async fn mysql_query_many() {
    let pool = MysqlPoolBuilder::new().url(url()).build().await.unwrap();
    let engine = MysqlEngine::new(pool);
    engine.execute_raw("DROP TABLE IF EXISTS crud_people_my", vec![]).await.unwrap();
    engine.execute_raw("CREATE TABLE crud_people_my (id INT AUTO_INCREMENT PRIMARY KEY, email VARCHAR(128))", vec![]).await.unwrap();
    engine.execute_raw("INSERT INTO crud_people_my (email) VALUES ('a@x.com'),('b@x.com')", vec![]).await.unwrap();
    let rows = engine.query_many::<Person>("SELECT id, email FROM crud_people_my ORDER BY id", vec![]).await.unwrap();
    assert_eq!(rows.len(), 2);
}
```

- [ ] **Step 2: Move legacy JSON API to `prax-mysql/src/raw.rs`**

Move the current `MysqlEngine`, `MysqlQueryResult`, `build_select`/`build_insert`/`build_update`/`build_delete`, and all JSON-returning methods into a new `raw.rs`. Rename:
- `MysqlEngine` → `MysqlRawEngine`
- `MysqlQueryResult` → `MysqlJsonRow`

Keep the public surface of `raw.rs` unchanged except for the renames so existing users migrate with a `use prax_mysql::raw::{MysqlRawEngine, MysqlJsonRow};` switch.

- [ ] **Step 3: Write the new `engine.rs`**

```rust
//! MySQL query engine implementing prax_query::QueryEngine.

use mysql_async::prelude::*;
use mysql_async::{Params, Row as MyRow, Value as MyValue};
use prax_query::error::{QueryError, QueryResult};
use prax_query::filter::FilterValue;
use prax_query::row::FromRow;
use prax_query::traits::{BoxFuture, Model, QueryEngine};
use tracing::debug;

use crate::pool::MysqlPool;
use crate::row_ref::MysqlRowRef;
use crate::types::filter_value_to_mysql;

#[derive(Clone)]
pub struct MysqlEngine { pool: MysqlPool }

impl MysqlEngine {
    pub fn new(pool: MysqlPool) -> Self { Self { pool } }
    pub fn pool(&self) -> &MysqlPool { &self.pool }
    fn bind(params: &[FilterValue]) -> Vec<MyValue> {
        params.iter().map(filter_value_to_mysql).collect()
    }
}

impl QueryEngine for MysqlEngine {
    fn dialect(&self) -> &dyn prax_query::dialect::SqlDialect { &prax_query::dialect::Mysql }

    fn query_many<T: Model + FromRow + Send + 'static>(
        &self, sql: &str, params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Vec<T>>> {
        let sql = sql.to_string();
        Box::pin(async move {
            debug!(sql = %sql, "mysql query_many");
            let mut conn = self.pool.get().await
                .map_err(|e| QueryError::connection(e.to_string()))?;
            let bound = Self::bind(&params);
            let rows: Vec<MyRow> = conn.inner_mut()
                .exec(sql.as_str(), Params::Positional(bound))
                .await.map_err(|e| QueryError::database(e.to_string()))?;
            rows.into_iter()
                .map(|r| MysqlRowRef::from_row(r)
                    .map_err(|e| QueryError::deserialization(e.to_string()))
                    .and_then(|r| T::from_row(&r).map_err(|e| QueryError::deserialization(e.to_string()))))
                .collect()
        })
    }

    fn query_one<T: Model + FromRow + Send + 'static>(
        &self, sql: &str, params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<T>> {
        Box::pin(async move {
            let mut rows = self.query_many::<T>(sql, params).await?;
            rows.pop().ok_or_else(|| QueryError::not_found(T::MODEL_NAME))
        })
    }

    fn query_optional<T: Model + FromRow + Send + 'static>(
        &self, sql: &str, params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Option<T>>> {
        Box::pin(async move {
            let mut rows = self.query_many::<T>(sql, params).await?;
            Ok(rows.pop())
        })
    }

    fn execute_insert<T: Model + FromRow + Send + 'static>(
        &self, sql: &str, params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<T>> {
        // MySQL 8.0.22+ supports INSERT ... RETURNING. Require that version.
        Box::pin(async move { self.query_one::<T>(sql, params).await })
    }

    fn execute_update<T: Model + FromRow + Send + 'static>(
        &self, sql: &str, params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Vec<T>>> {
        Box::pin(async move { self.query_many::<T>(sql, params).await })
    }

    fn execute_delete(&self, sql: &str, params: Vec<FilterValue>) -> BoxFuture<'_, QueryResult<u64>> {
        let sql = sql.to_string();
        Box::pin(async move {
            let mut conn = self.pool.get().await
                .map_err(|e| QueryError::connection(e.to_string()))?;
            let bound = Self::bind(&params);
            conn.inner_mut()
                .exec_drop(sql.as_str(), Params::Positional(bound))
                .await.map_err(|e| QueryError::database(e.to_string()))?;
            Ok(conn.inner().affected_rows())
        })
    }

    fn execute_raw(&self, sql: &str, params: Vec<FilterValue>) -> BoxFuture<'_, QueryResult<u64>> {
        self.execute_delete(sql, params)
    }

    fn count(&self, sql: &str, params: Vec<FilterValue>) -> BoxFuture<'_, QueryResult<u64>> {
        let sql = sql.to_string();
        Box::pin(async move {
            let mut conn = self.pool.get().await
                .map_err(|e| QueryError::connection(e.to_string()))?;
            let bound = Self::bind(&params);
            let row: Option<(i64,)> = conn.inner_mut()
                .exec_first(sql.as_str(), Params::Positional(bound))
                .await.map_err(|e| QueryError::database(e.to_string()))?;
            Ok(row.map(|(n,)| n as u64).unwrap_or(0))
        })
    }
}
```

Note: because Task 8 already swapped placeholders to `?` in every operation when Mysql dialect is used, no rebind is needed here.

- [ ] **Step 4: Update `prax-mysql/src/lib.rs`**

```rust
pub mod config;
pub mod connection;
pub mod engine;
pub mod error;
pub mod pool;
pub mod raw;
pub mod row;
pub mod row_ref;
pub mod types;

pub use engine::MysqlEngine;
pub use pool::{MysqlPool, MysqlPoolBuilder};
pub use raw::{MysqlJsonRow, MysqlRawEngine};
```

- [ ] **Step 5: Run integration test**

Run: `cargo test -p prax-mysql --test engine_crud`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add prax-mysql/
git commit -m "refactor(mysql): implement QueryEngine on MysqlEngine

MySQL used to return untyped JSON blobs. Rewrites the engine atop
MysqlRowRef / FromRow for typed models. Legacy JSON surface moves
to prax_mysql::raw (MysqlRawEngine, MysqlJsonRow) as an escape
hatch for callers that need untyped rows."
```

---

## Task 12: Implement `RowRef` + `QueryEngine` for MSSQL

**Files:**
- Create: `prax-mssql/src/row_ref.rs`
- Modify: `prax-mssql/src/engine.rs`
- Modify: `prax-mssql/src/lib.rs`
- Test: `prax-mssql/tests/engine_crud.rs`

Mirror Task 10 + Task 11 but for tiberius.

- [ ] **Step 1: Write failing test**

Create `prax-mssql/tests/engine_crud.rs` following Task 5's pattern. Use URL `server=tcp:localhost,1433;database=prax_test;user=sa;password=Prax_Test_Password123!;trustservercertificate=true`. Table fields use `INT IDENTITY(1,1)` for auto-increment.

- [ ] **Step 2: Create `prax-mssql/src/row_ref.rs`**

tiberius exposes `tiberius::Row::try_get::<T, &str>("col")`. Materialize into an owned `HashMap` keyed by column name — tiberius rows are cursor-bound, similar to rusqlite. Implement every `RowRef` method via the tiberius typed getter, mapping errors to `RowError::TypeConversion`.

- [ ] **Step 3: Rewrite `prax-mssql/src/engine.rs`**

Implement `QueryEngine`. `dialect()` returns `&prax_query::dialect::Mssql`. Because dialect emits `OUTPUT INSERTED.*` already, `execute_insert` just does a `conn.query(sql, &params).await?` and decodes the returned row.

Transactions are `BEGIN TRANSACTION` / `COMMIT TRANSACTION` / `ROLLBACK TRANSACTION` as the dialect provides. Pool: use `bb8-tiberius` (already in workspace deps).

- [ ] **Step 4: Export**

Add `pub mod row_ref;` to `prax-mssql/src/lib.rs`.

- [ ] **Step 5: Run tests**

```bash
docker compose up -d mssql
cargo test -p prax-mssql --test engine_crud
```
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add prax-mssql/
git commit -m "feat(mssql): implement RowRef and QueryEngine with OUTPUT INSERTED semantics"
```

---

## Task 13: Emit `Model` + `FromRow` impls from `#[derive(Model)]`

**Files:**
- Create: `prax-codegen/src/generators/derive_model_trait.rs`
- Create: `prax-codegen/src/generators/derive_from_row.rs`
- Modify: `prax-codegen/src/generators/derive.rs`
- Modify: `prax-codegen/src/generators/mod.rs`
- Test: `prax-orm/tests/derive_basic.rs`

- [ ] **Step 1: Write failing test**

Create `prax-orm/tests/derive_basic.rs`:

```rust
use prax::Model;

#[derive(Model)]
#[prax(table = "authors")]
struct Author {
    #[prax(id, auto)] id: i32,
    #[prax(unique)] email: String,
    name: Option<String>,
}

fn assert_impls<T: prax_query::row::FromRow + prax_query::traits::Model>() {}

#[test]
fn author_has_model_and_fromrow_impls() {
    assert_impls::<Author>();
    assert_eq!(Author::TABLE_NAME, "authors");
    assert_eq!(Author::PRIMARY_KEY, &["id"]);
}
```

Run: `cargo test -p prax-orm --test derive_basic`
Expected: COMPILE FAIL.

- [ ] **Step 2: Create `derive_model_trait.rs`**

```rust
use proc_macro2::TokenStream;
use quote::quote;
use syn::Ident;

pub fn emit(
    model_name: &Ident,
    model_name_str: &str,
    table_name: &str,
    pk_columns: &[String],
    all_columns: &[String],
) -> TokenStream {
    let pks = pk_columns.iter();
    let cols = all_columns.iter();
    quote! {
        impl prax_query::traits::Model for #model_name {
            const MODEL_NAME: &'static str = #model_name_str;
            const TABLE_NAME: &'static str = #table_name;
            const PRIMARY_KEY: &'static [&'static str] = &[#(#pks),*];
            const COLUMNS: &'static [&'static str] = &[#(#cols),*];
        }
    }
}
```

- [ ] **Step 3: Create `derive_from_row.rs`**

```rust
use proc_macro2::TokenStream;
use quote::quote;
use syn::{Ident, Type};

pub fn emit(model_name: &Ident, fields: &[(Ident, Type, String)]) -> TokenStream {
    let rows = fields.iter().map(|(field, ty, col)| {
        quote! {
            #field: <#ty as prax_query::row::FromColumn>::from_column(row, #col)?,
        }
    });
    quote! {
        impl prax_query::row::FromRow for #model_name {
            fn from_row(row: &impl prax_query::row::RowRef)
                -> Result<Self, prax_query::row::RowError>
            {
                Ok(Self { #(#rows)* })
            }
        }
    }
}
```

- [ ] **Step 4: Register modules**

Edit `prax-codegen/src/generators/mod.rs`:

```rust
pub mod derive_model_trait;
pub mod derive_from_row;
```

- [ ] **Step 5: Call them from `derive_model_impl`**

Edit `prax-codegen/src/generators/derive.rs`. Before the final `Ok(quote! { ... })`, compute:

```rust
let all_columns: Vec<String> = field_infos.iter()
    .filter(|f| !f.is_list) // relations skipped; handled later
    .map(|f| f.column_name.clone()).collect();
let pk_columns_owned: Vec<String> = field_infos.iter()
    .filter(|f| f.is_id).map(|f| f.column_name.clone()).collect();
let from_row_fields: Vec<(syn::Ident, syn::Type, String)> = field_infos.iter()
    .filter(|f| !f.is_list)
    .map(|f| (f.name.clone(), f.ty.clone(), f.column_name.clone()))
    .collect();

let model_trait = super::derive_model_trait::emit(
    name, &name.to_string(), &table_name, &pk_columns_owned, &all_columns,
);
let from_row = super::derive_from_row::emit(name, &from_row_fields);
```

Splice `#model_trait #from_row` into the output `quote!` block, alongside the existing `PraxModel` impl — keep both for backwards compat.

- [ ] **Step 6: Run the test**

Run: `cargo test -p prax-orm --test derive_basic`
Expected: PASS.

- [ ] **Step 7: Add trybuild UI tests**

Create `prax-orm/tests/derive_ui.rs`:

```rust
#[test]
fn derive_ui() {
    let t = trybuild::TestCases::new();
    t.pass("tests/derive_ui/pass_basic.rs");
    t.compile_fail("tests/derive_ui/fail_no_id.rs");
}
```

Create `prax-orm/tests/derive_ui/pass_basic.rs`:

```rust
use prax::Model;
#[derive(Model)]
#[prax(table = "x")]
struct X {
    #[prax(id)] id: i32,
}
fn main() {}
```

Create `prax-orm/tests/derive_ui/fail_no_id.rs`:

```rust
use prax::Model;
#[derive(Model)]
struct NoId { name: String }
fn main() {}
```

Create `prax-orm/tests/derive_ui/fail_no_id.stderr`:

```
error: Model must have at least one field marked with #[prax(id)]
```

(The exact wording may need adjustment after running — `trybuild` prints the canonical form.)

- [ ] **Step 8: Run trybuild**

Run: `cargo test -p prax-orm --test derive_ui`
Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add prax-codegen/src/generators/ prax-orm/tests/
git commit -m "feat(codegen): emit Model and FromRow impls from #[derive(Model)]

Generated code previously stopped at the legacy _prax_prelude
PraxModel marker trait — the real prax_query::Model and FromRow
impls required hand-writing. The derive now emits both, so
client.user().find_many()::<User> decodes end-to-end. Adds
trybuild UI tests for pass/fail derive cases."
```

---

## Task 14: Emit `WhereParam -> Filter` conversion

**Files:**
- Modify: `prax-codegen/src/generators/filters.rs:131-179`
- Modify: `prax-codegen/src/generators/model.rs` (schema path)
- Modify: `prax-codegen/src/generators/derive.rs` (derive path)
- Test: `prax-orm/tests/filter_conversion.rs`

Without this conversion, callers cannot write `FindManyOperation::r#where(user::email::equals("..."))` because `WhereParam` does not yet coerce to `Filter`.

- [ ] **Step 1: Write failing test**

```rust
// prax-orm/tests/filter_conversion.rs
use prax::Model;

#[derive(Model)]
#[prax(table = "users")]
struct User {
    #[prax(id)] id: i32,
    email: String,
}

#[test]
fn where_param_converts_to_filter() {
    let p: user::WhereParam = user::email::equals("a@b.c".to_string());
    let f: prax_query::filter::Filter = p.into();
    match f {
        prax_query::filter::Filter::Equals(c, _) => assert_eq!(c.as_ref(), "email"),
        _ => panic!("wrong filter"),
    }
}
```

Run: `cargo test -p prax-orm --test filter_conversion`
Expected: COMPILE FAIL — no `From<WhereParam> for Filter`.

- [ ] **Step 2: Emit `to_filter` on each generated `WhereOp`**

Edit `prax-codegen/src/generators/filters.rs` (around line 173, inside the emitted `impl WhereOp` block). Append:

```rust
pub fn to_filter(self, column: &'static str) -> prax_query::filter::Filter {
    use prax_query::filter::{Filter, FilterValue};
    use std::borrow::Cow;
    let col: Cow<'static, str> = Cow::Borrowed(column);
    match self {
        Self::Equals(v) => Filter::Equals(col, v.into()),
        Self::Not(v) => Filter::Ne(col, v.into()),
        Self::IsNull => Filter::IsNull(col),
        Self::IsNotNull => Filter::IsNotNull(col),
        Self::In(vs) => Filter::In(col, vs.into_iter().map(Into::into).collect()),
        Self::NotIn(vs) => Filter::NotIn(col, vs.into_iter().map(Into::into).collect()),
        Self::Gt(v) => Filter::Gt(col, v.into()),
        Self::Gte(v) => Filter::Gte(col, v.into()),
        Self::Lt(v) => Filter::Lt(col, v.into()),
        Self::Lte(v) => Filter::Lte(col, v.into()),
        Self::Contains(v) => Filter::Contains(col, FilterValue::String(v)),
        Self::StartsWith(v) => Filter::StartsWith(col, FilterValue::String(v)),
        Self::EndsWith(v) => Filter::EndsWith(col, FilterValue::String(v)),
    }
}
```

Verify every variant in `Filter` enum exists (`rg 'pub enum Filter' prax-query/src/filter.rs`). If `Ne`/`NotIn`/`Contains`/`StartsWith`/`EndsWith` are missing, add them and their `to_sql` emission first.

- [ ] **Step 3: Emit `impl From<WhereParam> for Filter` on each model**

In `prax-codegen/src/generators/model.rs::generate_where_param`, after the existing `impl WhereParam { ... }` block, append:

```rust
let from_arms: Vec<_> = model.fields.values().map(|field| {
    let name = pascal_ident(field.name());
    let field_mod = snake_ident(field.name());
    quote! { Self::#name(op) => op.to_filter(#field_mod::COLUMN), }
}).collect();

quote! {
    impl From<WhereParam> for prax_query::filter::Filter {
        fn from(p: WhereParam) -> Self {
            match p {
                #(#from_arms)*
                WhereParam::And(ps) => prax_query::filter::Filter::and(
                    ps.into_iter().map(Into::into).collect::<Vec<_>>()
                ),
                WhereParam::Or(ps) => prax_query::filter::Filter::or(
                    ps.into_iter().map(Into::into).collect::<Vec<_>>()
                ),
                WhereParam::Not(p) => prax_query::filter::Filter::Not(Box::new((*p).into())),
            }
        }
    }
}
```

Apply the same change in `prax-codegen/src/generators/derive.rs` where `WhereParam` is emitted.

- [ ] **Step 4: Run test**

Run: `cargo test -p prax-orm --test filter_conversion`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add prax-codegen/src/generators/ prax-orm/tests/filter_conversion.rs
git commit -m "feat(codegen): emit WhereParam -> Filter conversion

Previously WhereParam was only usable via its to_sql helper.
Callers couldn't feed it to FindManyOperation::r#where because no
From impl existed. Now every generated model emits the From impl
and each WhereOp variant has a to_filter method mapping it to the
runtime Filter enum."
```

---

## Task 15: Emit per-model `Client<E>` accessor from `#[derive(Model)]`

**Files:**
- Create: `prax-codegen/src/generators/derive_client.rs`
- Modify: `prax-codegen/src/generators/derive.rs`
- Modify: `prax-codegen/src/generators/mod.rs`
- Test: `prax-orm/tests/derive_client.rs`

- [ ] **Step 1: Write failing test**

```rust
// prax-orm/tests/derive_client.rs
use prax::Model;

#[derive(Model)]
#[prax(table = "posts")]
struct Post {
    #[prax(id, auto)] id: i32,
    title: String,
}

#[test]
fn post_client_is_generated() {
    fn _check<E: prax_query::traits::QueryEngine>(engine: E) {
        let _ = post::Client::new(engine).find_many();
    }
}
```

Run: `cargo test -p prax-orm --test derive_client`
Expected: COMPILE FAIL.

- [ ] **Step 2: Create `derive_client.rs`**

```rust
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::Ident;

pub fn emit(model_name: &Ident) -> TokenStream {
    let client_ident = format_ident!("Client");
    quote! {
        pub struct #client_ident<E: prax_query::traits::QueryEngine> {
            engine: E,
        }
        impl<E: prax_query::traits::QueryEngine> #client_ident<E> {
            pub fn new(engine: E) -> Self { Self { engine } }
            pub fn find_many(&self)
                -> prax_query::operations::FindManyOperation<E, super::#model_name>
            { prax_query::operations::FindManyOperation::new(self.engine.clone()) }
            pub fn find_unique(&self)
                -> prax_query::operations::FindUniqueOperation<E, super::#model_name>
            { prax_query::operations::FindUniqueOperation::new(self.engine.clone()) }
            pub fn find_first(&self)
                -> prax_query::operations::FindFirstOperation<E, super::#model_name>
            { prax_query::operations::FindFirstOperation::new(self.engine.clone()) }
            pub fn create(&self)
                -> prax_query::operations::CreateOperation<E, super::#model_name>
            { prax_query::operations::CreateOperation::new(self.engine.clone()) }
            pub fn create_many(&self)
                -> prax_query::operations::CreateManyOperation<E, super::#model_name>
            { prax_query::operations::CreateManyOperation::new(self.engine.clone()) }
            pub fn update(&self)
                -> prax_query::operations::UpdateOperation<E, super::#model_name>
            { prax_query::operations::UpdateOperation::new(self.engine.clone()) }
            pub fn update_many(&self)
                -> prax_query::operations::UpdateManyOperation<E, super::#model_name>
            { prax_query::operations::UpdateManyOperation::new(self.engine.clone()) }
            pub fn upsert(&self)
                -> prax_query::operations::UpsertOperation<E, super::#model_name>
            { prax_query::operations::UpsertOperation::new(self.engine.clone()) }
            pub fn delete(&self)
                -> prax_query::operations::DeleteOperation<E, super::#model_name>
            { prax_query::operations::DeleteOperation::new(self.engine.clone()) }
            pub fn delete_many(&self)
                -> prax_query::operations::DeleteManyOperation<E, super::#model_name>
            { prax_query::operations::DeleteManyOperation::new(self.engine.clone()) }
            pub fn count(&self)
                -> prax_query::operations::CountOperation<E, super::#model_name>
            { prax_query::operations::CountOperation::new(self.engine.clone()) }
            pub fn aggregate(&self)
                -> prax_query::operations::AggregateOperation<E, super::#model_name>
            { prax_query::operations::AggregateOperation::new(self.engine.clone()) }
            pub fn group_by(&self)
                -> prax_query::operations::GroupByOperation<E, super::#model_name>
            { prax_query::operations::GroupByOperation::new(self.engine.clone()) }
        }
    }
}
```

- [ ] **Step 3: Register module**

Add `pub mod derive_client;` to `prax-codegen/src/generators/mod.rs`.

- [ ] **Step 4: Splice into `derive.rs`**

Edit `prax-codegen/src/generators/derive.rs`. Inside the `pub mod #module_name { ... }` block, insert `#client` tokens where the legacy `Actions` struct was. Compute:

```rust
let client = super::derive_client::emit(name);
```

- [ ] **Step 5: Delete legacy `Actions` / `Query` emission**

Remove the `Query` struct and `Actions` struct emission from `derive.rs` (lines ~131-182). They are superseded by the runtime operations. Run tests to confirm no other code depends on them.

- [ ] **Step 6: Run test**

Run: `cargo test -p prax-orm --test derive_client`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add prax-codegen/src/generators/ prax-orm/tests/derive_client.rs
git commit -m "feat(codegen): emit per-model Client<E> accessor from derive

Each #[derive(Model)] now generates a Client<E> inside the
snake_case model module with one method per prax_query operation.
Removes the legacy inert Actions/Query helper that returned SQL
strings without an attached engine."
```

---

## Task 16: Port emission changes to `prax_schema!` macro

**Files:**
- Create: `prax-codegen/src/generators/schema_client.rs`
- Create: `prax-codegen/src/generators/schema_model_trait.rs`
- Create: `prax-codegen/src/generators/schema_from_row.rs`
- Modify: `prax-codegen/src/generators/model.rs:25-221`
- Modify: `prax-codegen/src/generators/mod.rs`
- Test: `prax-orm/tests/schema_macro.rs`

- [ ] **Step 1: Write failing test**

Create `prax-orm/tests/fixtures/basic.prax`:

```
model User {
    id    Int    @id @auto
    email String @unique
    name  String?
}
```

Create `prax-orm/tests/schema_macro.rs`:

```rust
prax::prax_schema!("tests/fixtures/basic.prax");

#[test]
fn schema_generates_client() {
    fn _check<E: prax_query::traits::QueryEngine>(e: E) {
        let _ = user::Client::new(e).find_many();
    }
}
```

Run: `cargo test -p prax-orm --test schema_macro`
Expected: COMPILE FAIL.

- [ ] **Step 2: Create wrappers**

`prax-codegen/src/generators/schema_client.rs`:

```rust
use proc_macro2::TokenStream;
use syn::Ident;
pub fn emit(model_name: &Ident) -> TokenStream {
    super::derive_client::emit(model_name)
}
```

`prax-codegen/src/generators/schema_model_trait.rs`: same pattern — delegates to `derive_model_trait::emit`.

`prax-codegen/src/generators/schema_from_row.rs`: same — delegates to `derive_from_row::emit`.

- [ ] **Step 3: Register modules**

Edit `prax-codegen/src/generators/mod.rs`:

```rust
pub mod schema_client;
pub mod schema_from_row;
pub mod schema_model_trait;
```

- [ ] **Step 4: Call them from `model.rs`**

Edit `prax-codegen/src/generators/model.rs::generate_model_module_with_style`. After computing `pk_fields`, `data_fields`, etc., compute:

```rust
let all_columns: Vec<String> = model.fields.values()
    .filter(|f| !matches!(f.field_type, prax_schema::ast::FieldType::Model(_)))
    .map(|f| f.attributes.iter().find(|a| a.name() == "map")
        .and_then(|a| a.first_arg()).and_then(|v| v.as_string())
        .map(|s| s.to_string())
        .unwrap_or_else(|| f.name().to_string()))
    .collect();

let pk_columns_owned: Vec<String> = pk_field_names.iter().map(|s| s.to_string()).collect();

let from_row_fields: Vec<(syn::Ident, syn::Type, String)> = model.fields.values()
    .filter(|f| !matches!(f.field_type, prax_schema::ast::FieldType::Model(_)))
    .map(|f| {
        let rust_field = snake_ident(f.name());
        let rust_ty = field_type_to_rust(&f.field_type, &f.modifier);
        let col = /* as above */;
        (rust_field, syn::parse2(rust_ty.into()).unwrap(), col)
    })
    .collect();

let model_trait_impl = super::schema_model_trait::emit(
    &model_name, model.name(), table_name_str, &pk_columns_owned, &all_columns,
);
let from_row_impl = super::schema_from_row::emit(&model_name, &from_row_fields);
let client_impl = super::schema_client::emit(&model_name);
```

Splice `#model_trait_impl #from_row_impl #client_impl` into the `pub mod #module_name { ... }` block output.

- [ ] **Step 5: Delete legacy inert `Query` and `Actions` generation**

Remove `generate_query_builder` and the `Actions` emission inside `model.rs::generate_query_builder`. They are superseded by the Client<E> accessor.

- [ ] **Step 6: Run test**

Run: `cargo test -p prax-orm --test schema_macro`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add prax-codegen/src/generators/ prax-orm/tests/schema_macro.rs prax-orm/tests/fixtures/
git commit -m "feat(codegen): emit Client<E>, Model, FromRow from prax_schema!

Brings the schema-file codegen path to feature parity with
#[derive(Model)]. Every model declared in a .prax file now emits
an executable Client<E> instead of the legacy inert Query/Actions
helper."
```

---

## Task 17: Add top-level `PraxClient<E>` and `prax::client!` macro

**Files:**
- Create: `src/client.rs`
- Modify: `src/lib.rs`
- Modify: `Cargo.toml` (add `paste` dep)
- Test: `tests/client_shape.rs`

- [ ] **Step 1: Write failing test**

```rust
// tests/client_shape.rs
use prax::{client, Model, PraxClient};

#[derive(Model)]
#[prax(table = "users")]
struct User { #[prax(id, auto)] id: i32, email: String }

#[derive(Model)]
#[prax(table = "posts")]
struct Post { #[prax(id, auto)] id: i32, title: String }

client!(User, Post);

#[test]
fn client_has_user_and_post_accessors() {
    fn _check<E: prax_query::traits::QueryEngine>(engine: E) {
        let client = PraxClient::new(engine);
        let _ = client.user().find_many();
        let _ = client.post().find_many();
    }
}
```

Run: `cargo test --test client_shape`
Expected: COMPILE FAIL.

- [ ] **Step 2: Add `paste` to `Cargo.toml`**

Edit `Cargo.toml` root workspace deps:

```toml
paste = "1.0"
```

Then in the root `[dependencies]`:

```toml
paste = { workspace = true }
```

- [ ] **Step 3: Create `src/client.rs`**

```rust
//! Top-level Prax client grouping per-model accessors.

use prax_query::traits::QueryEngine;

#[derive(Clone)]
pub struct PraxClient<E: QueryEngine> {
    engine: E,
}

impl<E: QueryEngine> PraxClient<E> {
    pub fn new(engine: E) -> Self { Self { engine } }
    pub fn engine(&self) -> &E { &self.engine }
}

/// Declarative macro attaching per-model accessors to PraxClient.
///
/// Each identifier must name a model declared via #[derive(Model)]
/// or prax_schema!. Emits `fn <model_snake_case>() -> <model>::Client<E>`.
#[macro_export]
macro_rules! client {
    ($($model:ident),+ $(,)?) => {
        #[allow(non_snake_case)]
        impl<E: $crate::__prelude::QueryEngine> $crate::PraxClient<E> {
            $( $crate::__client_accessor!($model); )+
        }
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __client_accessor {
    ($model:ident) => {
        $crate::__paste::paste! {
            pub fn [<$model:snake>](&self) -> [<$model:snake>]::Client<E> {
                [<$model:snake>]::Client::new(self.engine().clone())
            }
        }
    };
}

#[doc(hidden)]
pub use ::paste as __paste;

#[doc(hidden)]
pub mod __prelude {
    pub use prax_query::traits::QueryEngine;
}
```

- [ ] **Step 4: Re-export from `src/lib.rs`**

Edit `src/lib.rs`. After the existing `pub use prax_codegen::Model;` line, add:

```rust
pub mod client;
pub use client::PraxClient;
// client! macro is exported via #[macro_export] automatically.
```

Also add to `pub mod prelude`:

```rust
pub use crate::client::PraxClient;
```

- [ ] **Step 5: Run test**

Run: `cargo test --test client_shape`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/client.rs src/lib.rs Cargo.toml tests/client_shape.rs
git commit -m "feat(orm): add PraxClient<E> and prax::client! macro

Introduces the top-level entry point. Users declare
prax::client!(User, Post, ...) next to their #[derive(Model)]
structs and receive a PraxClient<E> with client.user() /
client.post() accessors returning the per-model Client<E>."
```

---

## Task 18: End-to-end Postgres CRUD integration test

**Files:**
- Create: `tests/client_postgres.rs`

- [ ] **Step 1: Write end-to-end test**

```rust
// tests/client_postgres.rs
use prax::{client, Model, PraxClient};
use prax_postgres::{PgEngine, PgPool, PgPoolBuilder};

#[derive(Model, Debug, PartialEq)]
#[prax(table = "users")]
struct User {
    #[prax(id, auto)] id: i32,
    #[prax(unique)] email: String,
    name: Option<String>,
}
client!(User);

fn url() -> String {
    std::env::var("PRAX_POSTGRES_URL")
        .unwrap_or_else(|_| "postgres://prax:prax_test_password@localhost:5433/prax_test".into())
}

async fn client() -> PraxClient<PgEngine> {
    let pool: PgPool = PgPoolBuilder::new().url(url()).build().await.unwrap();
    pool.get().await.unwrap()
        .batch_execute(
            "DROP TABLE IF EXISTS users;
             CREATE TABLE users (
                id SERIAL PRIMARY KEY,
                email TEXT UNIQUE NOT NULL,
                name TEXT
             );"
        ).await.unwrap();
    PraxClient::new(PgEngine::new(pool))
}

#[tokio::test]
async fn create_find_update_delete_cycle() {
    let c = client().await;

    let alice = c.user().create()
        .set("email", "alice@example.com")
        .set("name", "Alice")
        .exec().await.unwrap();
    assert_eq!(alice.email, "alice@example.com");

    let users = c.user().find_many().exec().await.unwrap();
    assert_eq!(users.len(), 1);

    let updated = c.user().update()
        .r#where(user::id::equals(alice.id))
        .set("name", "Alicia")
        .exec().await.unwrap();
    assert_eq!(updated[0].name.as_deref(), Some("Alicia"));

    let count = c.user().delete_many()
        .r#where(user::email::contains("@example.com".to_string()))
        .exec().await.unwrap();
    assert_eq!(count, 1);
}
```

- [ ] **Step 2: Run**

Run: `docker compose up -d postgres && cargo test --test client_postgres`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add tests/client_postgres.rs
git commit -m "test(orm): end-to-end Postgres CRUD integration test"
```

---

## Task 19: End-to-end MySQL, SQLite, and MSSQL integration tests

**Files:**
- Create: `tests/client_mysql.rs`
- Create: `tests/client_sqlite.rs`
- Create: `tests/client_mssql.rs`

Copy Task 18's shape three times, substituting the driver.

- [ ] **Step 1: MySQL variant**

Adapt setup SQL to MySQL syntax (`INT AUTO_INCREMENT PRIMARY KEY`). Wire with `prax_mysql::MysqlEngine` and `MysqlPoolBuilder`.

Run: `docker compose up -d mysql && cargo test --test client_mysql`
Expected: PASS.

- [ ] **Step 2: SQLite variant**

Use `:memory:` pool URL — no docker needed. Adapt setup SQL to SQLite (`INTEGER PRIMARY KEY AUTOINCREMENT`).

Run: `cargo test --test client_sqlite`
Expected: PASS.

- [ ] **Step 3: MSSQL variant**

Use `server=tcp:localhost,1433;database=prax_test;user=sa;password=Prax_Test_Password123!;trustservercertificate=true`. Adapt setup SQL to SQL Server (`INT IDENTITY(1,1) PRIMARY KEY`, `NVARCHAR(128)`, etc.).

Run: `docker compose up -d mssql && cargo test --test client_mssql`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add tests/client_mysql.rs tests/client_sqlite.rs tests/client_mssql.rs
git commit -m "test(orm): end-to-end CRUD integration tests for MySQL, SQLite, MSSQL"
```

---

## Task 20: Add `ToFilterValue` trait + `ModelWithPk` for relation support

**Files:**
- Modify: `prax-query/src/filter.rs`
- Modify: `prax-query/src/traits.rs`
- Modify: `prax-codegen/src/generators/derive_model_trait.rs`
- Modify: `prax-codegen/src/generators/schema_model_trait.rs`
- Test: `prax-query/src/filter.rs`

Relation loading needs to extract the primary-key value from a fetched parent row so follow-up child queries can bucket by FK. This requires a reverse of `FromColumn` — `ToFilterValue`.

- [ ] **Step 1: Add `ToFilterValue` trait**

In `prax-query/src/filter.rs`:

```rust
pub trait ToFilterValue {
    fn to_filter_value(&self) -> FilterValue;
}

impl ToFilterValue for i32 {
    fn to_filter_value(&self) -> FilterValue { FilterValue::Int(*self as i64) }
}
impl ToFilterValue for i64 {
    fn to_filter_value(&self) -> FilterValue { FilterValue::Int(*self) }
}
impl ToFilterValue for String {
    fn to_filter_value(&self) -> FilterValue { FilterValue::String(self.clone()) }
}
impl ToFilterValue for &str {
    fn to_filter_value(&self) -> FilterValue { FilterValue::String((*self).to_string()) }
}
impl ToFilterValue for bool {
    fn to_filter_value(&self) -> FilterValue { FilterValue::Bool(*self) }
}
impl ToFilterValue for f64 {
    fn to_filter_value(&self) -> FilterValue { FilterValue::Float(*self) }
}
impl<T: ToFilterValue> ToFilterValue for Option<T> {
    fn to_filter_value(&self) -> FilterValue {
        self.as_ref().map(|v| v.to_filter_value()).unwrap_or(FilterValue::Null)
    }
}
impl ToFilterValue for uuid::Uuid {
    fn to_filter_value(&self) -> FilterValue { FilterValue::String(self.to_string()) }
}
impl ToFilterValue for chrono::DateTime<chrono::Utc> {
    fn to_filter_value(&self) -> FilterValue { FilterValue::String(self.to_rfc3339()) }
}
impl ToFilterValue for rust_decimal::Decimal {
    fn to_filter_value(&self) -> FilterValue { FilterValue::String(self.to_string()) }
}
impl ToFilterValue for serde_json::Value {
    fn to_filter_value(&self) -> FilterValue { FilterValue::Json(self.clone()) }
}
```

Add a unit test:

```rust
#[test]
fn to_filter_value_option_some() {
    let v: Option<i32> = Some(42);
    assert_eq!(v.to_filter_value(), FilterValue::Int(42));
}
#[test]
fn to_filter_value_option_none() {
    let v: Option<i32> = None;
    assert_eq!(v.to_filter_value(), FilterValue::Null);
}
```

- [ ] **Step 2: Add `ModelWithPk` trait**

In `prax-query/src/traits.rs`:

```rust
pub trait ModelWithPk: Model {
    /// Return the primary-key value. For composite keys, returns a List.
    fn pk_value(&self) -> crate::filter::FilterValue;

    /// Return the value of a named column, if this model has that column.
    fn get_column_value(&self, column: &str) -> Option<crate::filter::FilterValue>;
}
```

- [ ] **Step 3: Emit `ModelWithPk` from `derive_model_trait.rs`**

Update `emit` to take `fields: &[(Ident, Type, String, bool)]` (the fourth element marks `is_id`), and emit:

```rust
let id_fields: Vec<_> = fields.iter().filter(|(_, _, _, is_id)| *is_id).collect();
let pk_expr = if id_fields.len() == 1 {
    let (field, ty, _col, _) = id_fields[0];
    quote! {
        <#ty as prax_query::filter::ToFilterValue>::to_filter_value(&self.#field)
    }
} else {
    let items = id_fields.iter().map(|(field, ty, _, _)| quote! {
        <#ty as prax_query::filter::ToFilterValue>::to_filter_value(&self.#field)
    });
    quote! { prax_query::filter::FilterValue::List(vec![ #(#items),* ]) }
};

let col_arms = fields.iter().map(|(field, ty, col, _)| quote! {
    #col => Some(<#ty as prax_query::filter::ToFilterValue>::to_filter_value(&self.#field)),
});

// ... append to quote! output:
impl prax_query::traits::ModelWithPk for #model_name {
    fn pk_value(&self) -> prax_query::filter::FilterValue { #pk_expr }
    fn get_column_value(&self, column: &str) -> Option<prax_query::filter::FilterValue> {
        match column {
            #(#col_arms)*
            _ => None,
        }
    }
}
```

Thread the `is_id` flag through `derive.rs` (already parsed into `FieldInfo`). Update the `schema_model_trait.rs` wrapper analogously — the `prax-schema::ast::Field` exposes `is_id()`.

- [ ] **Step 4: Run tests**

Run: `cargo test -p prax-query filter::tests`
Run: `cargo test -p prax-orm --test derive_basic`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add prax-query/src/filter.rs prax-query/src/traits.rs prax-codegen/src/generators/
git commit -m "feat(query): ToFilterValue and ModelWithPk for relation bucketing

RelationExecutor (coming next) needs to extract primary-key
values from fetched parent rows and look up arbitrary column
values from children to bucket them by FK. Adds ToFilterValue as
the reverse of FromColumn and ModelWithPk::pk_value /
get_column_value emitted by the codegen."
```

---

## Task 21: `RelationMeta` trait and per-relation codegen accessors

**Files:**
- Create: `prax-query/src/relations/meta.rs`
- Modify: `prax-query/src/relations/mod.rs`
- Create: `prax-codegen/src/generators/relation_accessors.rs`
- Modify: `prax-codegen/src/generators/model.rs` (schema path)
- Modify: `prax-codegen/src/generators/derive.rs` (derive path)
- Test: `prax-orm/tests/relation_meta.rs`

- [ ] **Step 1: Create `meta.rs`**

```rust
//! Metadata describing a relation between two models.

use crate::traits::Model;

pub enum RelationKind {
    BelongsTo,
    HasMany,
    HasOne,
    ManyToMany { join_table: &'static str },
}

pub trait RelationMeta {
    type Owner: Model;
    type Target: Model;
    const NAME: &'static str;
    const KIND: RelationKind;
    /// Column on Owner that references Target (for BelongsTo/HasOne).
    const LOCAL_KEY: &'static str;
    /// Column on Target that references Owner's PK (for HasMany/HasOne).
    const FOREIGN_KEY: &'static str;
}
```

- [ ] **Step 2: Re-export from `relations/mod.rs`**

Edit `prax-query/src/relations/mod.rs`:

```rust
pub mod meta;
pub use meta::{RelationKind, RelationMeta};
```

- [ ] **Step 3: Write failing codegen test**

Create `prax-orm/tests/relation_meta.rs`:

```rust
use prax::Model;
use prax_query::relations::{RelationKind, RelationMeta};

#[derive(Model)]
#[prax(table = "users")]
struct User {
    #[prax(id, auto)] id: i32,
    #[prax(relation(target = "Post", foreign_key = "author_id"))]
    posts: Vec<Post>,
}

#[derive(Model)]
#[prax(table = "posts")]
struct Post {
    #[prax(id, auto)] id: i32,
    title: String,
    author_id: i32,
}

#[test]
fn user_posts_relation_meta() {
    assert_eq!(user::posts::Relation::NAME, "posts");
    assert_eq!(user::posts::Relation::FOREIGN_KEY, "author_id");
    assert!(matches!(user::posts::Relation::KIND, RelationKind::HasMany));
}
```

Run: `cargo test -p prax-orm --test relation_meta`
Expected: COMPILE FAIL.

- [ ] **Step 4: Create `relation_accessors.rs`**

```rust
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::Ident;

pub struct RelationSpec<'a> {
    pub field_name: &'a Ident,
    pub owner: &'a Ident,
    pub target: &'a Ident,
    pub kind: RelationKindTokens,
    pub local_key: &'a str,
    pub foreign_key: &'a str,
}

pub enum RelationKindTokens {
    BelongsTo,
    HasMany,
    HasOne,
}

pub fn emit(spec: RelationSpec<'_>) -> TokenStream {
    let field_mod = spec.field_name;
    let field_name_str = spec.field_name.to_string();
    let owner = spec.owner;
    let target = spec.target;
    let local = spec.local_key;
    let foreign = spec.foreign_key;
    let kind = match spec.kind {
        RelationKindTokens::BelongsTo => quote! { prax_query::relations::RelationKind::BelongsTo },
        RelationKindTokens::HasMany   => quote! { prax_query::relations::RelationKind::HasMany },
        RelationKindTokens::HasOne    => quote! { prax_query::relations::RelationKind::HasOne },
    };
    quote! {
        pub mod #field_mod {
            use super::*;
            pub fn fetch() -> prax_query::relations::IncludeSpec {
                prax_query::relations::IncludeSpec::new(#field_name_str)
            }
            pub struct Relation;
            impl prax_query::relations::RelationMeta for Relation {
                type Owner = super::#owner;
                type Target = super::super::#target;
                const NAME: &'static str = #field_name_str;
                const KIND: prax_query::relations::RelationKind = #kind;
                const LOCAL_KEY: &'static str = #local;
                const FOREIGN_KEY: &'static str = #foreign;
            }
        }
    }
}
```

- [ ] **Step 5: Parse `#[prax(relation(...))]` in `derive.rs`**

Extend `FieldInfo` to include `relation: Option<RelationAttr>` with fields `target: Ident`, `foreign_key: String`, `local_key: String` (default `"id"`). Parse inside `parse_field`:

```rust
} else if meta.path.is_ident("relation") {
    meta.parse_nested_meta(|inner| {
        if inner.path.is_ident("target") {
            let s: syn::LitStr = inner.value()?.parse()?;
            rel_target = Some(format_ident!("{}", s.value()));
        } else if inner.path.is_ident("foreign_key") {
            let s: syn::LitStr = inner.value()?.parse()?;
            rel_fk = Some(s.value());
        } else if inner.path.is_ident("local_key") {
            let s: syn::LitStr = inner.value()?.parse()?;
            rel_lk = Some(s.value());
        }
        Ok(())
    })?;
}
```

Then determine kind: `Vec<T>` → `HasMany`, plain `T` → `BelongsTo`, `Option<T>` → `HasOne`.

- [ ] **Step 6: Emit relation accessors inside the model module**

In `derive.rs`, after building the existing field modules, collect relation specs and splice into the output:

```rust
let relation_mods: Vec<_> = field_infos.iter()
    .filter(|f| f.relation.is_some())
    .map(|f| super::relation_accessors::emit(
        super::relation_accessors::RelationSpec {
            field_name: &f.name,
            owner: name,
            target: &f.relation.as_ref().unwrap().target,
            kind: /* decide from is_list/is_optional */,
            local_key: f.relation.as_ref().unwrap().local_key.as_str(),
            foreign_key: f.relation.as_ref().unwrap().foreign_key.as_str(),
        }
    ))
    .collect();
```

Add `#(#relation_mods)*` to the `pub mod #module_name { ... }` output.

- [ ] **Step 7: Mirror in schema path (`model.rs`)**

In `prax-codegen/src/generators/model.rs`, reuse the `field.field_type` (`FieldType::Model(..)`) and `field.attributes` (`@relation` with `fields:`/`references:`) to compute the same `RelationSpec` and emit. See `prax-schema/src/ast.rs::Field::relation_info` (if present) for the helper; otherwise walk the `Attribute::args`.

- [ ] **Step 8: Run test**

Run: `cargo test -p prax-orm --test relation_meta`
Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add prax-query/src/relations/ prax-codegen/src/generators/ prax-orm/tests/relation_meta.rs
git commit -m "feat(codegen): emit RelationMeta and fetch() accessors per relation

Introduces RelationMeta trait with BelongsTo/HasMany/HasOne
variants. Codegen emits <model>::<relation>::Relation (with all
join metadata) and <model>::<relation>::fetch() -> IncludeSpec
for both derive and schema paths."
```

---

## Task 22: Relation executor + `.include(...)` in `FindManyOperation`

**Files:**
- Create: `prax-query/src/relations/executor.rs`
- Modify: `prax-query/src/operations/find_many.rs`
- Modify: `prax-query/src/operations/find_unique.rs`
- Modify: `prax-query/src/operations/find_first.rs`
- Modify: `prax-codegen/src/generators/derive.rs` + `model.rs` (emit `set_relation` helper)
- Test: `tests/relations_postgres.rs`

- [ ] **Step 1: Write failing end-to-end test**

Create `tests/relations_postgres.rs`:

```rust
use prax::{client, Model, PraxClient};
use prax_postgres::{PgEngine, PgPool, PgPoolBuilder};

#[derive(Model, Debug, PartialEq)]
#[prax(table = "users")]
struct User {
    #[prax(id, auto)] id: i32,
    email: String,
    #[prax(relation(target = "Post", foreign_key = "author_id"))]
    posts: Vec<Post>,
}

#[derive(Model, Debug, PartialEq)]
#[prax(table = "posts")]
struct Post {
    #[prax(id, auto)] id: i32,
    title: String,
    author_id: i32,
}

client!(User, Post);

fn url() -> String { /* as in Task 18 */ unimplemented!() }

async fn setup() -> PraxClient<PgEngine> {
    let pool: PgPool = PgPoolBuilder::new().url(url()).build().await.unwrap();
    pool.get().await.unwrap().batch_execute(
        "DROP TABLE IF EXISTS posts; DROP TABLE IF EXISTS users;
         CREATE TABLE users (id SERIAL PRIMARY KEY, email TEXT UNIQUE NOT NULL);
         CREATE TABLE posts (
            id SERIAL PRIMARY KEY,
            title TEXT NOT NULL,
            author_id INT NOT NULL REFERENCES users(id)
         );
         INSERT INTO users (email) VALUES ('a@x.com');
         INSERT INTO posts (title, author_id) VALUES ('p1', 1), ('p2', 1), ('p3', 1);"
    ).await.unwrap();
    PraxClient::new(PgEngine::new(pool))
}

#[tokio::test]
async fn find_many_with_include_fetches_posts() {
    let c = setup().await;
    let users = c.user()
        .find_many()
        .include(user::posts::fetch())
        .exec().await.unwrap();
    assert_eq!(users.len(), 1);
    assert_eq!(users[0].posts.len(), 3);
}
```

- [ ] **Step 2: Create the executor**

Create `prax-query/src/relations/executor.rs`:

```rust
//! Runtime for relation loading: fetch children and bucket by FK.

use std::collections::HashMap;

use crate::error::{QueryError, QueryResult};
use crate::filter::{Filter, FilterValue};
use crate::relations::{IncludeSpec, RelationKind, RelationMeta};
use crate::row::FromRow;
use crate::traits::{Model, ModelWithPk, QueryEngine};

pub async fn load_has_many<E, P, C, R>(engine: &E, parents: &[P])
    -> QueryResult<HashMap<String, Vec<C>>>
where
    E: QueryEngine,
    P: Model + ModelWithPk,
    C: Model + ModelWithPk + FromRow + Send + 'static,
    R: RelationMeta<Owner = P, Target = C>,
{
    let pk_values: Vec<FilterValue> = parents.iter().map(|p| p.pk_value()).collect();
    if pk_values.is_empty() { return Ok(HashMap::new()); }

    let filter = Filter::In(R::FOREIGN_KEY.into(), pk_values.clone());
    let dialect = engine.dialect();
    let (where_sql, params) = filter.to_sql(0, dialect);

    let sql = format!("SELECT * FROM {} WHERE {}", C::TABLE_NAME, where_sql);
    let children: Vec<C> = engine.query_many::<C>(&sql, params).await?;

    let mut out: HashMap<String, Vec<C>> = HashMap::new();
    for child in children {
        let fk = child.get_column_value(R::FOREIGN_KEY)
            .ok_or_else(|| QueryError::internal(format!(
                "relation {}: target model missing column {}", R::NAME, R::FOREIGN_KEY
            )))?;
        let key = filter_value_key(&fk);
        out.entry(key).or_default().push(child);
    }
    Ok(out)
}

fn filter_value_key(v: &FilterValue) -> String {
    match v {
        FilterValue::Int(i) => i.to_string(),
        FilterValue::String(s) => s.clone(),
        FilterValue::Bool(b) => b.to_string(),
        FilterValue::Float(f) => f.to_string(),
        FilterValue::Null => "<null>".into(),
        FilterValue::Json(v) => v.to_string(),
        FilterValue::List(_) => panic!("composite FK not yet supported"),
    }
}
```

Add similar `load_belongs_to` and `load_has_one` variants (single-child bucketing).

- [ ] **Step 3: Add `include()` method to `FindManyOperation`**

Open `prax-query/src/operations/find_many.rs`. Add to the struct:

```rust
includes: Vec<crate::relations::IncludeSpec>,
include_loaders: Vec<Box<dyn FnOnce(&E, &mut Vec<M>) -> crate::traits::BoxFuture<'_, QueryResult<()>> + Send>>,
```

The dyn-loader approach lets each generated relation accessor register a loader. Simpler: store only `IncludeSpec`, and expect the generated `Client<E>::find_many()` to dispatch via a model-specific helper.

Alternative cleaner design: add a `RelationLoader` trait emitted per model:

```rust
pub trait ModelRelationLoader<E: QueryEngine>: Sized {
    fn load_relation(
        engine: &E, parents: &mut [Self], spec: &crate::relations::IncludeSpec,
    ) -> crate::traits::BoxFuture<'_, QueryResult<()>>;
}
```

Each `#[derive(Model)]` that declares relations emits:

```rust
impl<E: prax_query::traits::QueryEngine> prax_query::traits::ModelRelationLoader<E> for User {
    fn load_relation(engine: &E, parents: &mut [User], spec: &prax_query::relations::IncludeSpec)
        -> prax_query::traits::BoxFuture<'_, prax_query::QueryResult<()>>
    {
        Box::pin(async move {
            match spec.relation_name.as_str() {
                "posts" => {
                    let bucketed = prax_query::relations::executor::load_has_many::
                        <E, User, Post, user::posts::Relation>(engine, parents).await?;
                    for p in parents.iter_mut() {
                        let key = match p.pk_value() {
                            prax_query::filter::FilterValue::Int(i) => i.to_string(),
                            _ => continue,
                        };
                        if let Some(children) = bucketed.get(&key) {
                            p.posts = children.clone();
                        }
                    }
                    Ok(())
                }
                _ => Err(prax_query::QueryError::internal(
                    format!("unknown relation: {}", spec.relation_name)
                )),
            }
        })
    }
}
```

- [ ] **Step 4: Update `FindManyOperation::exec`**

Add `include(spec)` builder method. In `exec`:

```rust
pub async fn exec(self) -> QueryResult<Vec<M>>
where
    M: Send + 'static + FromRow + crate::traits::ModelRelationLoader<E> + Clone,
{
    let (sql, params) = self.build_sql(self.engine.dialect());
    let mut parents: Vec<M> = self.engine.query_many::<M>(&sql, params).await?;
    for spec in self.includes {
        <M as crate::traits::ModelRelationLoader<E>>::load_relation(&self.engine, &mut parents, &spec).await?;
    }
    Ok(parents)
}
```

For models without relations, codegen emits the default impl:

```rust
impl<E: prax_query::traits::QueryEngine> prax_query::traits::ModelRelationLoader<E> for NoRelationsModel {
    fn load_relation(_e: &E, _p: &mut [Self], _s: &prax_query::relations::IncludeSpec)
        -> prax_query::traits::BoxFuture<'_, prax_query::QueryResult<()>>
    {
        Box::pin(async { Ok(()) })
    }
}
```

- [ ] **Step 5: Mirror in `FindUniqueOperation` / `FindFirstOperation`**

Add the same `.include(...)` method + loader dispatch.

- [ ] **Step 6: Run integration test**

Run: `docker compose up -d postgres && cargo test --test relations_postgres`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add prax-query/src/relations/ prax-query/src/operations/ prax-query/src/traits.rs prax-codegen/src/generators/ tests/relations_postgres.rs
git commit -m "feat(query): eager-load relations via .include() on find operations

Adds RelationExecutor (HasMany/BelongsTo/HasOne bucketing) and a
ModelRelationLoader trait that codegen emits per model. Every
FindMany/FindUnique/FindFirst operation gains .include(spec) that
issues follow-up queries inside exec and stitches children onto
parents."
```

---

## Task 23: Real transactions on PgEngine

**Files:**
- Create: `prax-postgres/src/tx.rs`
- Modify: `prax-postgres/src/engine.rs`
- Modify: `prax-postgres/src/lib.rs`
- Modify: `prax-query/src/traits.rs` (add `transaction()` default)
- Modify: `src/client.rs` (expose `transaction()` on `PraxClient`)
- Test: `tests/tx_postgres.rs`

- [ ] **Step 1: Add default `transaction` to `QueryEngine`**

In `prax-query/src/traits.rs`:

```rust
    /// Run a closure inside a transaction. Default runs without
    /// transactional semantics (no BEGIN/COMMIT), driver overrides it.
    fn transaction<'a, R, Fut, F>(&'a self, f: F) -> BoxFuture<'a, QueryResult<R>>
    where
        F: FnOnce(Self) -> Fut + Send + 'a,
        Fut: std::future::Future<Output = QueryResult<R>> + Send + 'a,
        R: Send + 'a,
        Self: Clone,
    {
        let me = self.clone();
        Box::pin(async move { f(me).await })
    }
```

- [ ] **Step 2: Refactor `PgEngine` to route through a connection holder**

Change `PgEngine`'s inner representation to:

```rust
#[derive(Clone)]
pub struct PgEngine {
    inner: PgInner,
}

#[derive(Clone)]
enum PgInner {
    Pool(PgPool),
    Tx(std::sync::Arc<parking_lot::Mutex<Option<TxState>>>),
}

struct TxState {
    // owns a deadpool_postgres::Client + a live tokio_postgres Transaction
    conn: deadpool_postgres::Object,
    tx: Option<tokio_postgres::Transaction<'static>>, // borrowed from conn via unsafe transmute
}
```

Each `QueryEngine` method dispatches:

```rust
fn query_many<T: ...>(&self, sql: &str, params: Vec<FilterValue>) -> BoxFuture<'_, ...> {
    let sql = sql.to_string();
    Box::pin(async move {
        match &self.inner {
            PgInner::Pool(pool) => { /* as today */ }
            PgInner::Tx(state) => {
                let guard = state.lock();
                let tx = guard.as_ref().unwrap().tx.as_ref().unwrap();
                let rows = tx.query(&sql, &params_as_refs).await?;
                crate::deserialize::rows_into::<T>(rows)
            }
        }
    })
}
```

- [ ] **Step 3: Override `transaction` on `PgEngine`**

```rust
impl QueryEngine for PgEngine {
    fn transaction<'a, R, Fut, F>(&'a self, f: F) -> BoxFuture<'a, QueryResult<R>>
    where
        F: FnOnce(Self) -> Fut + Send + 'a,
        Fut: std::future::Future<Output = QueryResult<R>> + Send + 'a,
        R: Send + 'a,
        Self: Clone,
    {
        Box::pin(async move {
            let pool = match &self.inner {
                PgInner::Pool(p) => p.clone(),
                PgInner::Tx(_) => return Err(QueryError::internal("nested transactions via savepoints")),
            };
            // Acquire + start tx; wrap in TxState
            let mut conn = pool.get().await
                .map_err(|e| QueryError::connection(e.to_string()))?;
            let tx = conn.transaction().await
                .map_err(|e| QueryError::database(e.to_string()))?;
            // SAFETY: TxState owns conn; tx is tied to &'a conn where &'a lasts
            // as long as TxState. We transmute the lifetime. The transaction is
            // dropped before conn by explicit commit/rollback below.
            let tx_static: tokio_postgres::Transaction<'static> =
                unsafe { std::mem::transmute(tx) };
            let state = std::sync::Arc::new(parking_lot::Mutex::new(Some(TxState {
                conn, tx: Some(tx_static),
            })));
            let tx_engine = PgEngine { inner: PgInner::Tx(state.clone()) };

            let result = f(tx_engine).await;
            let mut guard = state.lock();
            let TxState { conn: _, tx } = guard.take().unwrap();
            let tx = tx.unwrap();
            match result {
                Ok(v) => {
                    tx.commit().await.map_err(|e| QueryError::database(e.to_string()))?;
                    Ok(v)
                }
                Err(e) => {
                    let _ = tx.rollback().await;
                    Err(e)
                }
            }
        })
    }
}
```

The `mem::transmute` lifetime extension is the conventional pattern for "async transaction + connection bundled into a single heap cell." Keep it narrow: the `'static` fiction is only valid while `TxState` holds both `conn` and `tx`.

- [ ] **Step 4: Expose `transaction` on `PraxClient`**

Edit `src/client.rs`:

```rust
impl<E: QueryEngine> PraxClient<E> {
    pub async fn transaction<R, Fut, F>(&self, f: F) -> prax_query::QueryResult<R>
    where
        F: FnOnce(PraxClient<E>) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = prax_query::QueryResult<R>> + Send + 'static,
        R: Send + 'static,
    {
        self.engine.transaction(|tx_engine| async move {
            f(PraxClient::new(tx_engine)).await
        }).await
    }
}
```

- [ ] **Step 5: Write integration test**

Create `tests/tx_postgres.rs`:

```rust
use prax::{client, Model, PraxClient};
use prax_postgres::{PgEngine, PgPool, PgPoolBuilder};

#[derive(Model, Debug)]
#[prax(table = "users")]
struct User { #[prax(id, auto)] id: i32, #[prax(unique)] email: String }
client!(User);

async fn setup() -> PraxClient<PgEngine> { /* as in Task 18 */ unimplemented!() }

#[tokio::test]
async fn transaction_rolls_back_on_error() {
    let c = setup().await;
    let res: prax_query::QueryResult<()> = c.transaction(|tx| async move {
        tx.user().create().set("email", "a@b.c").exec().await?;
        Err(prax_query::QueryError::internal("boom"))
    }).await;
    assert!(res.is_err());
    let count = c.user().count().exec().await.unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn transaction_commits_on_ok() {
    let c = setup().await;
    let res: prax_query::QueryResult<()> = c.transaction(|tx| async move {
        tx.user().create().set("email", "x@y.z").exec().await?;
        Ok(())
    }).await;
    assert!(res.is_ok());
    let count = c.user().count().exec().await.unwrap();
    assert_eq!(count, 1);
}
```

- [ ] **Step 6: Run test**

Run: `cargo test --test tx_postgres`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add prax-postgres/ prax-query/src/traits.rs src/client.rs tests/tx_postgres.rs
git commit -m "feat(postgres): real transactions via PgEngine::transaction

PgEngine gains an internal enum (Pool or Tx) so every QueryEngine
method routes through either the pool or an active transaction.
PraxClient::transaction(f) hands the closure a PraxClient backed
by the tx engine and commits on Ok / rolls back on Err. Closes the
gap between Transaction scaffolding in prax-query and real SQL
transactional semantics."
```

---

## Task 24: Real transactions on MySQL, SQLite, MSSQL

**Files:**
- Create: `prax-mysql/src/tx.rs`
- Create: `prax-sqlite/src/tx.rs`
- Create: `prax-mssql/src/tx.rs`
- Modify: each driver's `engine.rs` (inner enum + `transaction` override)
- Test: `tests/tx_mysql.rs`, `tests/tx_sqlite.rs`, `tests/tx_mssql.rs`

Apply the same `Pool | Tx` inner-enum pattern from Task 23 to each driver. SQLite uses `BEGIN;`/`COMMIT;`/`ROLLBACK;` string statements via `tokio_rusqlite::Connection`. MySQL uses `mysql_async`'s `Transaction` type with the same lifetime-extension trick. MSSQL uses `tiberius::Transaction` or manual `BEGIN TRANSACTION` statements if `bb8-tiberius` lacks transaction support (verify during Task 12 work).

- [ ] **Step 1-3 (per driver): mirror Task 23**, write integration test, run, commit.

- [ ] **Step 4: Final commit set**

```bash
git commit -m "feat(sqlite): real transactions via SqliteEngine::transaction"
git commit -m "feat(mysql): real transactions via MysqlEngine::transaction"
git commit -m "feat(mssql): real transactions via MssqlEngine::transaction"
```

---

## Task 25: Upsert execution with dialect-aware conflict clause

**Files:**
- Modify: `prax-query/src/operations/upsert.rs`
- Test: `tests/upsert_postgres.rs`, `tests/upsert_mysql.rs`

- [ ] **Step 1: Write failing test**

```rust
// tests/upsert_postgres.rs
#[tokio::test]
async fn upsert_inserts_then_updates_same_row() {
    let c = setup().await;
    let u1 = c.user().upsert()
        .r#where(user::email::equals("same@x.com".into()))
        .create(|c| c.set("email", "same@x.com").set("name", "A"))
        .update(|u| u.set("name", "B"))
        .exec().await.unwrap();

    let u2 = c.user().upsert()
        .r#where(user::email::equals("same@x.com".into()))
        .create(|c| c.set("email", "same@x.com").set("name", "A"))
        .update(|u| u.set("name", "C"))
        .exec().await.unwrap();

    assert_eq!(u1.id, u2.id);
    assert_eq!(u2.name.as_deref(), Some("C"));
}
```

- [ ] **Step 2: Rewrite `UpsertOperation::build_sql` to use the dialect**

Read `prax-query/src/operations/upsert.rs`. The existing `build_sql` likely hard-codes Postgres. Replace conflict-clause emission with `dialect.upsert_clause(&conflict_cols, &set_clause)`. For MSSQL (empty clause), implement as a two-step `MERGE` in the engine's `execute_insert`.

- [ ] **Step 3: Implement `exec` to run the INSERT with RETURNING**

```rust
pub async fn exec(self) -> QueryResult<M>
where M: Send + 'static + FromRow,
{
    let dialect = self.engine.dialect();
    let (sql, params) = self.build_sql(dialect);
    self.engine.execute_insert::<M>(&sql, params).await
}
```

- [ ] **Step 4: Run test**

Run: `cargo test --test upsert_postgres`
Expected: PASS.

- [ ] **Step 5: MySQL variant**

Create `tests/upsert_mysql.rs`. Works identically thanks to `ON DUPLICATE KEY UPDATE` emission from the Mysql dialect.

Run: `cargo test --test upsert_mysql`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add prax-query/src/operations/upsert.rs tests/upsert_postgres.rs tests/upsert_mysql.rs
git commit -m "feat(query): dialect-aware upsert with ON CONFLICT / ON DUPLICATE KEY"
```

---

## Task 26: Aggregate and group_by execution

**Files:**
- Modify: `prax-query/src/operations/aggregate.rs`
- Modify: `prax-query/src/traits.rs` (add `aggregate_query`)
- Modify: `prax-postgres/src/engine.rs`, `prax-mysql/src/engine.rs`, `prax-sqlite/src/engine.rs`, `prax-mssql/src/engine.rs`
- Test: `tests/aggregate_postgres.rs`

- [ ] **Step 1: Add `aggregate_query` to `QueryEngine`**

```rust
    /// Execute an aggregate/group_by query and return raw column maps per row.
    fn aggregate_query(
        &self,
        sql: &str,
        params: Vec<crate::filter::FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Vec<std::collections::HashMap<String, crate::filter::FilterValue>>>>;
```

- [ ] **Step 2: Implement `aggregate_query` on each driver**

For each driver, iterate over the returned rows, walk `columns_ref`, and convert each column to a `FilterValue` (Int/Float/String/Bool/Null). Build a `HashMap<String, FilterValue>` per row. See `prax-mysql/src/raw.rs::row_to_json` for the existing pattern — convert to `FilterValue` instead of `serde_json::Value`.

- [ ] **Step 3: Wire `AggregateOperation::exec` + `GroupByOperation::exec`**

Both call `engine.aggregate_query(sql, params)` and parse into `AggregateResult` / `GroupByResult` respectively.

- [ ] **Step 4: Integration test**

```rust
// tests/aggregate_postgres.rs
#[tokio::test]
async fn aggregate_avg_sum_min_max() {
    let c = setup().await;
    // seed a few posts with views
    for v in [1, 5, 7, 3] {
        c.post().create().set("title", "p").set("views", v).exec().await.unwrap();
    }
    let stats = c.post().aggregate()
        .avg("views")
        .sum("views")
        .min("views")
        .max("views")
        .exec().await.unwrap();
    assert_eq!(stats.sum("views"), Some(16.0));
    assert_eq!(stats.min("views"), Some(1.0));
    assert_eq!(stats.max("views"), Some(7.0));
}
```

Run: `cargo test --test aggregate_postgres`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add prax-query/src/operations/aggregate.rs prax-query/src/traits.rs prax-postgres/ prax-mysql/ prax-sqlite/ prax-mssql/ tests/aggregate_postgres.rs
git commit -m "feat(query): aggregate + group_by execution across all four drivers"
```

---

## Task 27: Nested writes (create with connect / create relation)

**Files:**
- Modify: `prax-query/src/operations/create.rs`
- Modify: `prax-query/src/nested.rs`
- Modify: `prax-codegen/src/generators/` (emit `<model>::<relation>::connect(id)` / `create(children)`)
- Test: `tests/nested_write_postgres.rs`

- [ ] **Step 1: Extend `CreateOperation` with `with` method**

```rust
pub struct CreateOperation<E, M> { /* existing fields */ nested: Vec<crate::nested::NestedWrite> }

impl<E: QueryEngine, M: Model + FromRow> CreateOperation<E, M> {
    pub fn with(mut self, nw: crate::nested::NestedWrite) -> Self {
        self.nested.push(nw); self
    }
}
```

- [ ] **Step 2: Emit `<model>::<relation>::connect(id)` / `create(children)` helpers**

Extend `relation_accessors::emit` to also emit:

```rust
pub fn connect(id: i32) -> prax_query::nested::NestedWrite {
    prax_query::nested::NestedWrite::Connect {
        relation: #field_name_str.into(), pk: prax_query::filter::FilterValue::Int(id as i64),
    }
}
pub fn create(children: Vec<super::super::#target_module::CreateInput>)
    -> prax_query::nested::NestedWrite
{
    prax_query::nested::NestedWrite::Create {
        relation: #field_name_str.into(),
        foreign_key: #foreign_key.into(),
        payload: children.into_iter().map(|c| c.into_fields()).collect(),
    }
}
```

(`CreateInput::into_fields` already exists in generated code; if not, emit it.)

- [ ] **Step 3: Wire `CreateOperation::exec` to run nested writes in a transaction**

```rust
pub async fn exec(self) -> QueryResult<M>
where M: Send + 'static + FromRow + ModelWithPk, E: Clone,
{
    self.engine.transaction(|tx| async move {
        let dialect = tx.dialect();
        let (sql, params) = self.build_sql(dialect);
        let parent: M = tx.execute_insert::<M>(&sql, params).await?;
        let parent_pk = parent.pk_value();
        for nw in self.nested {
            nw.execute(&tx, &parent_pk).await?;
        }
        Ok(parent)
    }).await
}
```

Implement `NestedWrite::execute` in `prax-query/src/nested.rs` — each variant issues one or more follow-up SQL statements using the same engine clone inside the transaction.

- [ ] **Step 4: Integration test**

```rust
#[tokio::test]
async fn create_user_with_nested_posts() {
    let c = setup().await;
    let u = c.user().create()
        .set("email", "nw@x.com")
        .with(user::posts::create(vec![
            post::CreateInput { title: "p1".into(), ..Default::default() },
            post::CreateInput { title: "p2".into(), ..Default::default() },
        ]))
        .exec().await.unwrap();
    let posts = c.post().find_many()
        .r#where(post::author_id::equals(u.id))
        .exec().await.unwrap();
    assert_eq!(posts.len(), 2);
}
```

Run: `cargo test --test nested_write_postgres`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add prax-query/src/operations/create.rs prax-query/src/nested.rs prax-codegen/src/generators/ tests/nested_write_postgres.rs
git commit -m "feat(query): nested writes via CreateOperation.with(relation)"
```

---

## Task 28: `select` and `include` pruning projection

**Files:**
- Modify: `prax-query/src/types.rs::Select`
- Modify: `prax-query/src/operations/find_many.rs` + siblings
- Modify: `prax-codegen/src/generators/fields.rs` + `model.rs` + `derive.rs` (emit `select!` macro)
- Test: `tests/select_postgres.rs`

- [ ] **Step 1: Emit a per-model `select!` macro**

For each generated model module, emit:

```rust
#[macro_export]
macro_rules! user_select {
    ($($field:ident),+ $(,)?) => {
        vec![ $( $crate::user::select::$field() ),+ ]
    };
}
pub mod select {
    pub fn id() -> super::SelectParam { super::SelectParam::Id }
    pub fn email() -> super::SelectParam { super::SelectParam::Email }
    // ...
}
```

- [ ] **Step 2: Update `FindManyOperation::build_sql` to honor the select list**

If `self.select` is a `Select::Fields(names)`, emit `SELECT col1, col2, ...` instead of `SELECT *`. Already implemented in `find_many.rs::build_sql` via `self.select.to_sql()` — verify and extend if only `*` path is wired.

- [ ] **Step 3: Test**

```rust
#[tokio::test]
async fn select_returns_only_requested_fields() {
    let c = setup().await;
    let users = c.user().find_many()
        .select(prax_query::Select::fields(["id", "email"]))
        .exec().await.unwrap();
    assert!(!users.is_empty());
    // All fields populated, but name is guaranteed None because it wasn't SELECTed.
}
```

Note: `FromRow` will fail if a field is not in the row — so selective `select()` forces user to provide a partial model. For Phase 1 parity, skip partial hydration and just test that the SQL column list narrows. Full partial-projection is a follow-up.

- [ ] **Step 4: Commit**

```bash
git add prax-query/ prax-codegen/ tests/select_postgres.rs
git commit -m "feat(query): honor select/include projection in SQL emission"
```

---

## Task 29: Typed raw SQL escape hatch on PraxClient

**Files:**
- Modify: `src/client.rs`
- Test: `tests/raw_postgres.rs`

- [ ] **Step 1: Add methods to `PraxClient`**

```rust
impl<E: QueryEngine> PraxClient<E> {
    pub async fn query_raw<T>(&self, sql: prax_query::raw::Sql) -> prax_query::QueryResult<Vec<T>>
    where T: prax_query::traits::Model + prax_query::row::FromRow + Send + 'static,
    {
        let (s, p) = sql.build();
        self.engine.query_many::<T>(&s, p).await
    }

    pub async fn execute_raw(&self, sql: prax_query::raw::Sql) -> prax_query::QueryResult<u64> {
        let (s, p) = sql.build();
        self.engine.execute_raw(&s, p).await
    }
}
```

- [ ] **Step 2: Test**

```rust
// tests/raw_postgres.rs
use prax_query::raw::Sql;

#[tokio::test]
async fn query_raw_decodes_rows() {
    let c = setup().await;
    c.user().create().set("email", "raw@x.com").exec().await.unwrap();
    let users: Vec<User> = c.query_raw(
        Sql::new("SELECT id, email, name FROM users WHERE email = ").bind("raw@x.com"),
    ).await.unwrap();
    assert_eq!(users.len(), 1);
}
```

Run: `cargo test --test raw_postgres`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add src/client.rs tests/raw_postgres.rs
git commit -m "feat(orm): typed raw SQL via PraxClient::query_raw/execute_raw"
```

---

## Task 30: Runnable example + README refresh + CHANGELOG

**Files:**
- Create: `examples/client_crud_postgres.rs`
- Modify: `README.md`
- Modify: `CHANGELOG.md`

- [ ] **Step 1: Write the example**

Create `examples/client_crud_postgres.rs`:

```rust
//! End-to-end Prax client example against PostgreSQL.
//!
//! Run: `docker compose up -d postgres && cargo run --example client_crud_postgres`

use prax::{client, Model, PraxClient};
use prax_postgres::{PgEngine, PgPool, PgPoolBuilder};

#[derive(Model, Debug)]
#[prax(table = "users")]
struct User {
    #[prax(id, auto)] id: i32,
    #[prax(unique)] email: String,
    name: Option<String>,
}

client!(User);

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url = std::env::var("PRAX_POSTGRES_URL")
        .unwrap_or_else(|_| "postgres://prax:prax_test_password@localhost:5433/prax_test".into());
    let pool: PgPool = PgPoolBuilder::new().url(url).build().await?;
    pool.get().await?.batch_execute(
        "DROP TABLE IF EXISTS users;
         CREATE TABLE users (
            id SERIAL PRIMARY KEY,
            email TEXT UNIQUE NOT NULL,
            name TEXT
         );"
    ).await?;

    let client = PraxClient::new(PgEngine::new(pool));

    let alice = client.user().create()
        .set("email", "alice@example.com")
        .set("name", "Alice")
        .exec().await?;
    println!("Created: {:?}", alice);

    let users = client.user().find_many().exec().await?;
    println!("All users: {:?}", users);

    let updated = client.user().update()
        .r#where(user::id::equals(alice.id))
        .set("name", "Alicia")
        .exec().await?;
    println!("Updated: {:?}", updated);

    let count = client.user().delete_many()
        .r#where(user::email::contains("@example.com".into()))
        .exec().await?;
    println!("Deleted {} rows", count);

    Ok(())
}
```

- [ ] **Step 2: Run the example**

Run: `docker compose up -d postgres && cargo run --example client_crud_postgres`
Expected: prints Created, All users, Updated, Deleted lines and exits 0.

- [ ] **Step 3: Update README Quick Start**

Replace speculative code in `README.md` with a verified snippet derived from the example. Fix the field-name casing: change `user::createdAt::desc()` (camelCase, doesn't compile) to `user::created_at::desc()`.

- [ ] **Step 4: Add CHANGELOG entry**

Append to `CHANGELOG.md` under a new `## [Unreleased]` heading:

```markdown
## [Unreleased]

### Added
- `PraxClient<E>` and `prax::client!` macro — Prisma-style client API
  with per-model accessors (`client.user().find_many().exec()...`).
- `SqlDialect` abstraction — Postgres / MySQL / SQLite / MSSQL each
  emit correct placeholders, `RETURNING` / `OUTPUT INSERTED`, and
  upsert clauses.
- `#[derive(Model)]` emits `Model`, `FromRow`, `ModelWithPk`,
  `WhereParam -> Filter`, `ModelRelationLoader`, and a `Client<E>`
  accessor.
- `prax_schema!` codegen emits the same surface from `.prax` files.
- Relation loading via `.include(user::posts::fetch())` with
  BelongsTo / HasOne / HasMany resolution.
- Real transactions on all four SQL drivers.
- Cross-dialect aggregate and group_by execution.
- Nested writes (`create().with(user::posts::create(...))`) inside an
  implicit transaction.
- Typed raw SQL on `PraxClient::query_raw` / `execute_raw`.
- Integration tests against live Postgres, MySQL, SQLite, MSSQL.

### Changed
- `QueryEngine` trait now requires `FromRow` for row-returning
  methods; every `Operation` propagates the bound.
- `prax-mysql` engine returns typed rows; the legacy JSON surface
  moves to `prax_mysql::raw::MysqlRawEngine`.
- `prax-sqlite` engine same change; legacy JSON in
  `prax_sqlite::raw::SqliteRawEngine`.

### Removed
- Legacy `Actions` / `Query` helpers emitted by the codegen — they
  returned SQL strings without an attached engine and are subsumed
  by the new live `Client<E>`.
```

- [ ] **Step 5: Commit**

```bash
git add examples/client_crud_postgres.rs README.md CHANGELOG.md
git commit -m "docs(orm): document the PraxClient API with a runnable example"
```

---

## Task 31: CI — spin up databases for integration tests

**Files:**
- Modify: `.github/workflows/ci.yml`

- [ ] **Step 1: Add service containers**

In `.github/workflows/ci.yml`, add to the test job:

```yaml
    services:
      postgres:
        image: pgvector/pgvector:pg16
        env:
          POSTGRES_USER: prax
          POSTGRES_PASSWORD: prax_test_password
          POSTGRES_DB: prax_test
        options: >-
          --health-cmd pg_isready
          --health-interval 5s
          --health-timeout 5s
          --health-retries 10
        ports:
          - 5433:5432
      mysql:
        image: mysql:8.0
        env:
          MYSQL_ROOT_PASSWORD: root_password
          MYSQL_USER: prax
          MYSQL_PASSWORD: prax_test_password
          MYSQL_DATABASE: prax_test
        ports:
          - 3307:3307
        options: --health-cmd "mysqladmin ping"
      mssql:
        image: mcr.microsoft.com/mssql/server:2022-latest
        env:
          ACCEPT_EULA: Y
          MSSQL_SA_PASSWORD: Prax_Test_Password123!
        ports:
          - 1433:1433
    env:
      PRAX_POSTGRES_URL: postgres://prax:prax_test_password@localhost:5433/prax_test
      PRAX_MYSQL_URL: mysql://prax:prax_test_password@localhost:3307/prax_test
      PRAX_MSSQL_URL: 'server=tcp:localhost,1433;database=prax_test;user=sa;password=Prax_Test_Password123!;trustservercertificate=true'
    steps:
      - uses: actions/checkout@v5
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo test --workspace --all-features
```

- [ ] **Step 2: Push branch and verify CI goes green**

```bash
git push -u origin feature/client-api
```

Wait for CI (use `gh pr checks`). Fix any CI-only failures (port conflicts, health-check race conditions) before marking done.

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: run integration tests against Postgres, MySQL, MSSQL service containers"
```

---

## Task 32: Final workspace verification

**Files:**
- None (verification only)

- [ ] **Step 1: Clean build**

Run: `cargo clean && cargo check --workspace --all-features`
Expected: zero errors, zero warnings treated as errors.

- [ ] **Step 2: Clippy**

Run: `cargo clippy --workspace --all-features -- -D warnings`
Expected: zero warnings.

- [ ] **Step 3: Unit tests**

Run: `cargo test --workspace --lib --bins`
Expected: all pass.

- [ ] **Step 4: Integration tests (all four DBs running)**

```bash
docker compose up -d postgres mysql mssql
cargo test --workspace --all-features
```
Expected: all pass.

- [ ] **Step 5: Doc build**

Run: `cargo doc --workspace --all-features --no-deps`
Expected: no broken intra-doc links. `#![deny(rustdoc::broken_intra_doc_links)]` in `src/lib.rs` must still hold.

- [ ] **Step 6: Verify no stub implementations**

Run: `rg 'todo!\(\)|unimplemented!\(\)|panic!\("not implemented' --glob '!tests/**' --glob '!**/*.md'`
Expected: zero hits in production code. If any surface, fix before merging — this violates the project "No stub implementations" rule.

- [ ] **Step 7: Open PR**

```bash
gh pr create --title "feat(client): full Prisma-parity fluent client API" \
  --body "$(cat <<'EOF'
## Summary
- PraxClient<E> + prax::client! macro: Prisma-style client API.
- QueryEngine + SqlDialect: Postgres / MySQL / SQLite / MSSQL row
  deserialization, placeholders, RETURNING / OUTPUT INSERTED,
  upsert clauses.
- codegen emits Model, FromRow, ModelWithPk, Client<E>, relation
  accessors, and WhereParam -> Filter conversion from both
  #[derive(Model)] and prax_schema!.
- Relations via .include(user::posts::fetch()) for BelongsTo /
  HasOne / HasMany.
- Real transactions, upsert, aggregate/group_by, nested writes,
  typed raw SQL.
- Integration tests against live Postgres, MySQL, SQLite, MSSQL.

## Test plan
- [x] cargo test --workspace --all-features (with live DBs)
- [x] cargo clippy --workspace --all-features -- -D warnings
- [x] cargo doc --workspace --all-features --no-deps
- [x] cargo run --example client_crud_postgres
- [x] No todo!()/unimplemented!() in production code
EOF
)"
```

---

## Self-Review Notes

**Spec coverage.** Tasks cover every user-requested parity item — `find_many`, `find_unique`, `find_first`, `create`, `create_many`, `update`, `update_many`, `upsert`, `delete`, `delete_many`, `count`, `aggregate`, `group_by`, `include`, `select`, `r#where`, `order_by`, `skip`, `take`, transactions, typed raw SQL, `#[derive(Model)]`, `prax_schema!`, MySQL refactor — across Postgres, MySQL, SQLite, and MSSQL.

**Driver parity.** Postgres, MySQL, SQLite, MSSQL all receive the same feature set via `SqlDialect`. MongoDB is deliberately out of scope; its document model warrants a separate plan.

**Task dependencies.**
- Task 7 (dialect) is a prerequisite for Tasks 9 / 11 / 12 / 25 (MySQL / SQLite / MSSQL / upsert SQL differs per dialect).
- Task 13 (derive traits) and Task 15 (derive client) feed Task 17 (`PraxClient` macro relies on the emitted `user::Client<E>`).
- Task 20 (`ToFilterValue` / `ModelWithPk`) gates Task 22 (relation executor needs to extract PK/FK values).
- Task 21 (RelationMeta + accessors) + Task 22 (executor + include) together deliver the include-loading feature.
- Task 23 (Pg transactions) precedes Task 27 (nested writes rely on implicit transactions).
- Task 8 (thread dialect through build_sql) must land before any driver's CRUD integration test can pass.

The serial order respects these dependencies.

**Type consistency.** `Model`, `FromRow`, `ModelWithPk`, `QueryEngine`, `SqlDialect`, `WhereParam`, `WhereOp`, `FilterValue`, `IncludeSpec`, `RelationMeta`, `ModelRelationLoader`, and the generated `Client<E>` / `select!` / `<model>::<relation>::Relation` are defined before referenced.

**Risks to flag during execution.**
- The `transmute` for the `'static` transaction lifetime is the conventional workaround but must be kept narrow. If it proves too fragile, swap to a separate `TxEngine` type that is explicitly non-`'static` and threaded through a trait-object `&dyn QueryEngine` instead — at the cost of one v-table hop per call.
- `FilterValue` already has `From<...>` impls in many directions — double-check `ToFilterValue` does not introduce conflicting blanket impls. Prefer using the existing `From` impls from codegen via `FilterValue::from(v)` if `ToFilterValue` produces conflicts.
- Composite primary keys are handled via `FilterValue::List(...)` in `ModelWithPk::pk_value`. The relation executor's `filter_value_key` must be extended to recognize lists (currently panics); add composite support before merging if any fixture uses it.
- MySQL 8.0 supports `INSERT ... RETURNING`, but the Docker image version must be confirmed at Task 1. If the image is < 8.0.22, switch `execute_insert` to `LAST_INSERT_ID() + SELECT` fallback.
- MSSQL upsert via `MERGE` is complex; Task 25's dialect emitter returns an empty string and the engine layer must post-process. Flag this during Task 12 so the engine knows to emit `MERGE INTO ... USING ...` when given an upsert intent (e.g., via a new `SqlDialect::emit_upsert_statement` hook).
- SQLite's `blocking` model inside `tokio_rusqlite::Connection::call` means every query hops through a thread pool. If perf shows regressions in the SQLite CRUD test, audit for unnecessary clones of `Vec<SqlValue>`.
