# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.9.4] - 2026-04-30

### Added

- **`prax-cli generate` — emit `query_raw` / `execute_raw` / `engine()`
  on the generated `PraxClient<E>`.** The schema-generated top-level
  client previously exposed only per-model accessors, so consumers that
  needed to reach beyond the fluent builder (JOINs, subqueries, CTEs,
  multi-table aggregates, pgvector operators, window functions, vendor
  extensions) had to reach around the generated client entirely. The
  derive-path `prax_orm::PraxClient` already had these three methods;
  now the generated version matches. `query_raw<T>` routes rows
  through the same `FromRow` bridge the per-model `find_many` uses so
  raw queries still return typed records.

### Motivation

Surfaced porting lexmata-admin-backend's user_view service under
LX-33: queries like "user profile with firm summary + aggregate
document/demand-letter counts per case" use JOINs and subqueries
the fluent builder doesn't model, and the generated client was a
dead end. With this release they compile through
`client.query_raw::<Case>(Sql::new("SELECT …"))` — the same shape
the derive-path client already supported.

## [0.9.3] - 2026-04-30

### Added

- **`prax-query` — blanket `FromColumn` / `ToFilterValue` for
  `Option<T>`.** Every `T: FromColumn` now satisfies
  `Option<T>: FromColumn` via a single blanket impl that probes
  nullability with the new `RowRef::is_null` method. Unblocks
  schema-generated clients from hitting the orphan rule when a
  Prisma column is an `Enum?`: `Option<MyEnum>: FromColumn` now
  works out of the box without the consumer crate having to write
  its own (which orphan rules would reject). Replaced the dozen
  concrete `Option<primitive>` impls — behavior-preserving since
  every driver backend either honored the existing `_opt` path or
  falls back to the new `is_null` + inner decode.
- **`prax-query` — `FromColumn` / `ToFilterValue` for `Vec<f32>`.**
  Pgvector-typed columns in the schema emit `Vec<f32>` on the
  generated struct; those now decode via `RowRef::get_vector` (new
  base method, drivers implement) and encode as
  `FilterValue::List(Float, Float, …)`.
- **`prax-cli generate` — enum round-trip impls.** Every enum
  emitted by `prax generate` now carries `FromStr`, `FromColumn`,
  and `ToFilterValue` impls alongside the existing `Display` /
  `Default`. Schema-generated structs with enum fields
  now compile without needing handwritten decoder code per enum.

### Fixed

- **`prax-cli generate` — escape Rust reserved keywords.** Columns
  named `type`, `match`, `use`, `loop`, `move`, etc. (common —
  `documents.type`, `notifications.type`, `email_verification.type`
  all exist in Prisma schemas) were emitted as plain field
  identifiers, producing output that fails to parse with
  `expected identifier, found keyword \`type\``. Snake-cased field
  names whose result is a Rust keyword are now prefixed with `r#`.
  The four keywords Rust refuses as raw identifiers (`crate`,
  `self`, `Self`, `super`) stay un-escaped — a column literally
  named `self` still fails to compile, which is correct behavior.
  SQL column-name strings (serde rename values,
  `FromColumn::from_column(row, "col")` literals) use plain
  snake_case because they're opaque text, not identifiers.
- **`prax-cli generate` — qualify `VectorFilter` path; replace
  unshipped filter types with `ScalarFilter<T>`.** The bare
  `VectorFilter` reference only compiled if the consumer's
  `filters.rs` happened to have the right import, and
  `SparseVectorFilter` / `BitFilter` were invented names that don't
  exist anywhere in `prax-pgvector`. Fully qualified the vector
  filter as `prax_pgvector::filter::VectorFilter`, and swapped
  sparse + bit to `ScalarFilter<Vec<(u32, f32)>>` /
  `ScalarFilter<Vec<u8>>` until dedicated filters ship.

### Motivation

Ports the `prax generate` runtime-client output from "compiles on
toy examples" to "compiles on real Prisma schemas." Surfaced by the
LX-33 migration of lexmata-admin-backend's 71-model shared schema
(33 enums, 1149 fields, pgvector + nullable-enum + reserved-keyword
columns all represented). Every fix is covered by a regression test.

## [0.9.2] - 2026-04-30

### Fixed

- **`prax-codegen` — `snake_ident` escapes Rust reserved keywords.**
  Columns named `type`, `match`, `use`, `loop`, `move`, `where`, and
  similar (common in Prisma schemas) previously emitted verbatim as
  field and variable identifiers, producing output that fails to parse
  with `expected identifier, found keyword \`type\``. `snake_ident`
  now prefixes matches with `r#` so `pub r#type: …` round-trips
  through `rustc`. Four keywords Rust forbids as raw identifiers
  (`crate`, `self`, `Self`, `super`) are intentionally not escaped;
  a column literally named `self` would still fail to compile, which
  is the correct behavior (the schema should be fixed).
- **`prax-cli generate` — qualify `VectorFilter` path and drop
  references to unshipped filter types.** `field_to_filter_type`
  emitted bare `"VectorFilter"` (only compiled if the consumer's
  `filters.rs` happened to have the right import) and referenced
  `SparseVectorFilter` / `BitFilter` types that do not exist in
  `prax-pgvector`. Fully qualified the vector filter path as
  `prax_pgvector::filter::VectorFilter`, and fell back to
  `ScalarFilter<Vec<(u32, f32)>>` / `ScalarFilter<Vec<u8>>` for
  sparse and bit vectors until dedicated filter types ship.

Both fixes surfaced porting lexmata-admin-backend's 71-model shared
schema to the generated Prax client under LX-33. Each is covered by
regression tests.

## [0.9.1] - 2026-04-30

Forward-ports three correctness fixes that shipped in 0.8.2 on the
`release/0.8.1` branch but never landed on `develop` before 0.9.0
cut. All three were required to import the Lexmata application schema
(71 models, 33 enums) round-trippably through `prax import --from
prisma`; each is covered by a regression test.

### Fixed

- **`prax-import` (Prisma) — `@default` value round-trip.** String
  literal defaults no longer double-quote
  (`@default("standard")` ↛ `@default(""standard"")`); bare-identifier
  defaults on enum-typed fields map to `AttributeValue::Ident`
  rather than `AttributeValue::String` so the emitter doesn't
  render them as quoted strings; `dbgenerated("…")` arguments unwrap
  their Prisma source quotes uniformly.
- **`prax-import` (Prisma) — pgvector `Unsupported(…)`.** Prisma's
  `Unsupported("vector(N)")` / `halfvec(N)` / `sparsevec(N)` / `bit(N)`
  escape hatch now maps to the matching `ScalarType::Vector(…)` /
  `HalfVector(…)` / `SparseVector(…)` / `Bit(…)` variants. The CLI
  emitter prints the dimension via the `@dim(N)` attribute that the
  schema parser already accepts.
- **`prax validate` — diagnostic rendering.** Schema errors now render
  via `miette::Report` with the source attached, so the
  `prax::schema::invalid_field` / `unknown_type` / etc. diagnostic
  text and location are visible. Previously every parse or validation
  failure surfaced as a bare "syntax error in schema" string, hiding
  the actionable detail.
- **`prax-schema` validator — `Json` default values.** Accept
  `String`, `Array`, `Boolean`, `Int`, and `Float` payloads as the
  `@default` of a `Json`-typed field. Prisma encodes JSON defaults as
  quoted text literals (`@default("[]")`, `@default("{}")`), which
  are valid because Postgres parses the text into `jsonb` at insert
  time — the old validator only accepted `String` defaults on
  `String`-typed fields, rejecting every JSON default outright.

### Housekeeping

- `.gitignore` excludes `docs/superpowers/` and
  `tests/qualified_test.rs` (local scratch test, broken compile) so
  `cargo publish` doesn't require `--allow-dirty` for these
  pre-existing artifacts.

## [0.9.0] - 2026-04-30

### Added

- **`prax generate` now emits a runtime-ready client.** Generated
  model modules carry the trait impls the runtime needs to actually
  run queries, matching the surface produced by `#[derive(Model)]`:
  - `impl prax_query::row::FromRow` decodes scalar columns via
    `FromColumn` and default-initializes relation fields, so
    `find_many` and friends round-trip rows back into the generated
    structs at runtime.
  - `impl prax_query::traits::ModelWithPk` exposes `pk_value()` and
    `get_column_value()` for nested writes, upsert, and composite
    primary keys.
  - The per-model operations struct is named `Client<E>` (was
    `{Name}Operations<E>`), so `prax_orm::client!(User, Post, ...)`
    can resolve `<snake_name>::Client<E>` by path the same way it
    does for the derive path. The full CRUD surface — `find_many`,
    `find_unique`, `find_first`, `create`, `create_many`, `update`,
    `update_many`, `upsert`, `delete`, `delete_many`, `count` — is
    emitted on `Client<E>`.
  - Non-list relation fields are emitted as `Option<T>` (or
    `Option<Box<T>>` when boxing is needed to break a cycle)
    regardless of the schema modifier, so the FromRow default-init
    has a `None` to write into. The relation executor populates
    `Some(T)` on the `.include` path.

### Fixed

- **`prax-migrate` — CREATE TABLE emission respects FK dependencies.**
  Before, `SchemaDiffer` populated `create_models` by iterating a
  HashMap, leaving the resulting CREATE TABLE order
  non-deterministic. A schema where `tracks` and `playlists`
  reference `sync_sources` could emit `sync_sources` last; SQLite
  tolerated it because FK targets are resolved at row-write time,
  but strict engines (Postgres, MySQL with FK enforcement, MSSQL)
  and any deferred-constraint bootstrap would fail to apply the
  migration. `SchemaDiff::ordered_create_models` now does Kahn's
  algorithm topo sort over the FK graph: referenced tables emit
  before their dependents, self-references and FKs that point at
  out-of-batch tables don't constrain ordering, and cycles fall
  back to original order. All five SQL generators (Postgres,
  MySQL, SQLite, MSSQL, DuckDB) route through it and emit drops
  in the reverse direction so rollbacks drop dependents before
  parents.

### Changed

- **Workspace clippy gate is back online.** The husky pre-commit
  hook had silently been bypassed in environments that override
  `core.hookspath` with no project-local `pre-commit`; develop had
  accumulated 200+ clippy errors under `-D warnings`. Cleared every
  diagnostic so
  `cargo clippy --workspace --all-targets --all-features -- -D warnings`
  passes again. API-shape lints (`result_large_err`,
  `new_ret_no_self`, `should_implement_trait`, pedantic noise in
  `prax-scylladb`) are suppressed at crate level with rationale;
  bug-shaped lints (`manual_checked_ops`, `manual_strip`,
  `manual_clamp`, `manual_sort_by_key`, `&PathBuf` → `&Path`,
  `mixed_attributes_style`, `unnecessary_unwrap`) are fixed in
  place.
- **`prax-sqlite` — vector tests skip cleanly when the loader is
  unconfigured.** The two unit tests in `vector/register.rs` and
  the three integration tests in `tests/vector_integration.rs` no
  longer fail under `cargo test -- --include-ignored` (used by CI)
  in environments that haven't provisioned the sqlite-vector-rs
  cdylib. They detect the missing library at runtime via
  `SQLITE_VECTOR_RS_LIB` and bail out with a "skipping" message
  instead of panicking on the loader error.

## [0.8.0] - 2026-04-30

The headline of this release is the new executable **client API** —
Prisma-style `PraxClient<E>` with per-model accessors that run
(`client.user().find_many()...`) through a typed `QueryEngine`
instead of returning inert SQL strings. The driver layer was rewritten
from scratch to back it: typed row decoding via `FromRow`/`RowRef`
bridges on all four SQL drivers, a `SqlDialect` abstraction so filter
SQL emits the right placeholder/quoting/upsert syntax per backend,
real transactions, aggregate/group_by execution, cross-dialect
upsert, and a typed `query_raw`/`execute_raw` escape hatch on
`PraxClient`.

### Added

- **`PraxClient<E>` and `prax::client!(Model, ...)` macro** — top-level
  client grouping per-model accessors. The macro emits a sealed
  `PraxClientExt` trait and implements it for `PraxClient<E>` so
  callers write `client.user()` / `client.post()` without inherent
  `impl` blocks on a foreign type.
- **Per-model `Client<E>` emitted by `#[derive(Model)]` and by
  `prax_schema!`** — exposes `find_many`, `find_unique`, `find_first`,
  `create`, `create_many`, `update`, `update_many`, `upsert`, `delete`,
  `delete_many`, `count`, `aggregate`, `group_by`. Each accessor clones
  the engine and hands it to the matching operation builder.
- **`prax-query::dialect::SqlDialect` trait** — new module with
  `Postgres` / `Sqlite` / `Mysql` / `Mssql` / `NotSql` implementations.
  Attached to `QueryEngine::dialect()`. Each dialect drives placeholder
  syntax (`$1` / `?` / `?N` / `@P1`), `RETURNING` vs `OUTPUT INSERTED`,
  upsert clause shape, transaction statements, and identifier quoting.
  Marked `#[non_exhaustive]` so additional dialects can be added
  without a breaking release.
- **`ToFilterValue` trait + `ModelWithPk`** — reverse of `FromColumn`
  used by the relation executor and by upsert to extract PK/FK values.
- **`RelationMeta` + per-relation codegen modules**
  (`user::posts::fetch()`, `user::posts::Relation`) — declarative
  relation metadata emitted from
  `#[prax(relation(target = ..., foreign_key = ...))]`.
- **`.include(spec)` on `find_many` / `find_unique` / `find_first`** —
  eager-loads BelongsTo / HasOne / HasMany relations with one
  follow-up `IN (…)` query per relation.
- **Real transactions on all four SQL drivers**:
  `PraxClient::transaction(|tx| async { ... }).await` commits on `Ok`
  and rolls back on `Err`. Nested `transaction()` on the same engine
  currently returns `QueryError::internal(...)` until dialect-aware
  SAVEPOINT support lands.
- **Cross-dialect upsert**: `ON CONFLICT ... DO UPDATE`
  (Postgres / SQLite) / `ON DUPLICATE KEY UPDATE` (MySQL). Routed
  through the engine with the dialect's conflict clause spliced in
  by the builder.
- **Cross-dialect aggregate + group_by execution** via
  `QueryEngine::aggregate_query`.
- **Nested writes**: `.create().with(user::posts::create(vec![...]))`
  issues child inserts inside an implicit transaction.
- **Typed raw SQL escape hatch**: `PraxClient::query_raw<T>(Sql)` and
  `PraxClient::execute_raw(Sql)`. Rows route through the same
  `FromRow` bridge the derived models use, so the result stays typed.
- **`prax-query::row::FromRow` + `RowRef`** — expanded with
  default-erroring getters for `chrono::DateTime<Utc>`,
  `chrono::NaiveDateTime`, `chrono::NaiveDate`, `chrono::NaiveTime`,
  `uuid::Uuid`, `rust_decimal::Decimal`, `serde_json::Value` and their
  `Option<T>` variants. Drivers override the ones they support
  natively.
- **`prax-query::row::into_row_error`** — helper for driver `RowRef`
  bridges that maps any `Display` error into a
  `RowError::TypeConversion`.
- **`prax-{postgres,sqlite,mysql,mssql}` row_ref modules** — typed row
  bridges (`PgRow`, `SqliteRowRef`, `MysqlRowRef`, `MssqlRowRef`).
- **`prax-{postgres,sqlite,mysql,mssql}::*Engine`** — implement
  `QueryEngine` trait with typed row decoding via `FromRow`.
- **`#[derive(Model)]`** — emits `impl prax_query::traits::Model` and
  `impl prax_query::row::FromRow` alongside the legacy `PraxModel`
  marker. Also emits per-field filter operator constructors
  (`user::age::gt(18)`, etc.) that classify field types into
  Numeric / String / Boolean / Other buckets.
- **`FilterValue` `From` impls** — signed and unsigned integer widths,
  `f32`, `chrono::DateTime<Utc>`, `chrono::NaiveDateTime`,
  `chrono::NaiveDate`, `chrono::NaiveTime`, `uuid::Uuid`,
  `rust_decimal::Decimal`, `serde_json::Value`.
- **Integration tests against live Postgres, MySQL, SQLite, and MSSQL
  containers**, gated on `PRAX_E2E=1` + `#[ignore]` so the default
  `cargo test` run stays fast. Covers CRUD, upsert, aggregate,
  transaction commit/rollback, and select projection.
- **`examples/client_crud_postgres.rs`** — runnable end-to-end demo
  that walks the full CRUD cycle against docker-compose Postgres.
- **TypeScript Generator** (`prax-typegen` v0.1.0) — standalone crate
  for generating TypeScript from Prax schemas.
  - TypeScript interface generation for models, enums, composite
    types, and views.
  - Zod schema generation with runtime validation and inferred types.
  - `CreateInput` and `UpdateInput` variants for each model.
  - Lazy `z.lazy()` references for relation fields.
  - CLI binary installable via `cargo install prax-typegen`.
- **Schema Generator Blocks** (`prax-schema`) — first-class `generator`
  block support in `.prax` files.
  - `generate = env("VAR")` toggle: enable/disable generators via
    environment variables.
  - `generate = true/false` literal toggle.
  - Parsed into `Generator` AST with `provider`, `output`, `generate`,
    and arbitrary properties.
  - `Schema::enabled_generators()` for runtime filtering.

### Changed (BREAKING)

- **`prax-query::traits::QueryEngine`** — row-returning methods now
  require `T: FromRow`. Add `#[derive(Model)]` (which emits `FromRow`)
  or a hand-written `impl FromRow for MyModel`. Every operation
  builder propagates the bound. Driver impls route rows through the
  `RowRef` bridge instead of JSON.
- **`prax-query::traits::QueryEngine`** — new `dialect()` method on the
  trait. Has a default returning the inert `NotSql` dialect, so
  existing implementors continue to compile — but every SQL-backed
  engine must override it or SQL building will panic at runtime.
- **`prax-query::filter::Filter::to_sql`** — signature gained a
  `dialect: &dyn SqlDialect` parameter. Callers must pass their
  engine's dialect (or a literal `&prax_query::dialect::Postgres` if
  wedded to that backend).
- **`prax-query::filter::Filter::to_sql`** — column names are now
  quoted through `dialect.quote_ident` before being interpolated into
  SQL (SQL-injection fix). Generated SQL now reads `"col" = $1` on
  Postgres (was `col = $1`), `` `col` = ? `` on MySQL, `[col] = @P1`
  on MSSQL. Tests that matched the unquoted form need updating.
- **`prax-mysql` / `prax-sqlite` engines** — rewritten to return typed
  rows (`T: FromRow`) instead of JSON blobs. The legacy JSON surface
  moved to `prax_mysql::raw::MysqlRawEngine` +
  `prax_mysql::raw::MysqlJsonRow` (and the equivalent for SQLite).
  Callers that wanted JSON: `use prax_{mysql,sqlite}::raw::{MysqlRawEngine, MysqlJsonRow}`.
- **`prax-mysql::MysqlEngine` inherent methods removed** — the old
  `query(sql, params) -> Vec<RowData>`,
  `query_one(sql, params) -> RowData`,
  `query_opt(sql, params) -> Option<RowData>` no longer exist. They
  are replaced by the `QueryEngine` trait methods `query_many::<T>`,
  `query_one::<T>`, `query_optional::<T>`, each of which requires
  `T: Model + FromRow`. Callers consuming raw `RowData` /
  `serde_json::Value` must either migrate to a typed model via
  `#[derive(Model)]`, bridge through `prax_mysql::row_ref::MysqlRowRef`
  in a hand-written `FromRow`, or switch to
  `prax_mysql::raw::MysqlRawEngine` for the legacy JSON API.
  Side-effecting SQL that returns no rows should call
  `QueryEngine::execute_raw`.
- **`prax-sqlite::SqliteEngine` inherent methods removed** — same
  breakage as `MysqlEngine`. The old `query` / `query_one` /
  `query_opt` are gone; use `query_many::<T>` / `query_one::<T>` /
  `query_optional::<T>` with `T: Model + FromRow`, bridge via
  `prax_sqlite::row_ref::SqliteRowRef::from_rusqlite` for ad-hoc typed
  rows, or fall back to `prax_sqlite::raw::SqliteRawEngine` for the
  JSON API.
- **`prax-mysql::MysqlQueryResult` / `prax-sqlite::SqliteQueryResult`**
  — types removed from public re-exports. Renamed to
  `prax_{mysql,sqlite}::raw::{MysqlJsonRow, SqliteJsonRow}`.
- **`#[derive(Model)]` now emits `FromRow` in addition to `Model`** —
  the derive expands to *both* `impl prax_query::traits::Model for …`
  and `impl prax_query::row::FromRow for …`. If you had a
  hand-written `impl Model for …` or `impl FromRow for …` for a type
  that also carries the derive, the two impls will conflict (`E0119`).
  Delete the hand-written impl and rely on the derive, or drop the
  derive and keep the hand-written impls.
- **`#[derive(Model)]` now emits a lowercase-struct module** —
  alongside the per-field filter constructors, the derive emits
  `mod <lowercase_struct_name> { pub mod <field> { fn equals, gt, lt, … } }`.
  Crates that already define a module named the same as the lowercase
  form of a derived struct (e.g., a struct `User` plus a local
  `mod user { … }`) will see an `E0428` duplicate-definition error.
  Rename one of them.
- **`FilterValue::from::<u64>`** — values greater than `i64::MAX` now
  panic instead of silently clamping (previously an auth-bypass
  footgun). Callers that pass untrusted `u64` inputs must validate
  the range before conversion, or switch to
  `FilterValue::Int(value as i64)` with their own clamp policy.
- **Postgres driver integer width narrowing** — `FilterValue::Int` is
  narrowed to the target column width at bind time (INT2 / INT4 /
  INT8). Eliminates `WrongType { postgres: Int4, rust: "i64" }`
  errors when filtering on integer PKs.
- **MSSQL `OUTPUT INSERTED.*` clause order** — rearranged into the
  correct T-SQL position (between `(cols)` and `VALUES` on
  `INSERT`; between `SET` and `WHERE` on `UPDATE`).
- **MySQL stopped emitting `RETURNING`** — MySQL 8.0 doesn't support
  it (that's a MariaDB extension). The engine now re-`SELECT`s after
  `INSERT` via `LAST_INSERT_ID()`.

### Removed

- **Legacy `Actions` / `Query` inert helpers** emitted by the codegen
  — they returned SQL strings without an attached engine and are
  fully subsumed by the new executable `Client<E>`.
- **`#[derive(Model)]` phantom `increment` / `decrement` helpers** —
  the derive no longer emits helpers that called a non-existent
  `super::<field>::get_current_value()` function.

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

`find_many().select([...])` (and `find_first` / `find_unique`) now narrows
the emitted SQL column list instead of always sending `SELECT *`. The
returned rows are still decoded as whole `T` structs, so every
non-`Option` field on `T` must appear in the SELECT list — otherwise
you'll see `RowError::ColumnNotFound` (or a driver-level "column does not
exist" surfaced through `RowError::TypeConversion`) when `FromRow` tries
to read the missing column. Proper partial hydration (per-field
`Option<T>` decoding that treats absent columns as `None`) is a
follow-up; this change gets the easy 50% (narrower bandwidth) with no
partial-struct complexity. Leave `.select(...)` unset to keep the old
`SELECT *` behavior.

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

