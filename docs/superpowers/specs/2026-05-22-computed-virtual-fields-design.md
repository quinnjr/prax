# Computed and Virtual Fields — Phase 5.5 Design

> Companion to `2026-05-18-typed-query-traits-design.md` §9. This spec
> covers the full phase 5.5 surface in a single PR.

## 1. Goals

- Support three classes of "fields without a write-side payload":
  1. **DB-generated columns** — `@generated("expr") @stored|@virtual`
  2. **Relation aggregate virtuals** — `@count(rel)`, `@sum/@avg/@min/@max(rel.field)`
  3. **Pure-Rust computed methods** — `impl Model { fn … }` (doc-only)
- Schema-level aggregates become real scalar fields on the result struct.
- Ad-hoc `select: { _count: { rel: true } }` accessor produces a per-model
  `<Model>Count` substruct, available even when no schema-level `@count` exists.
- Capability gates keep MongoDB and CQL engines off the new surface at compile time.
- Single PR for the whole phase. The internal task split lives in the implementation plan.

## 2. Non-goals

- MongoDB `$lookup` lowering for relation aggregates — separate follow-up.
- Lateral-join optimization for scalar subqueries — baseline is correlated subqueries.
- Aggregate filters on `@generated` of `@generated` (chained generated columns) — DB engines reject this; we don't go out of our way.
- Cross-engine dialect translation of `@generated` expression text — see §4.4.

## 3. Schema AST and parsing

### 3.1 New `FieldKind` variants (`prax-schema/src/ast.rs`)

```rust
pub enum FieldKind {
    // ... existing variants ...
    DbGenerated {
        expr: String,
        stored: bool,
    },
    Aggregate {
        kind: AggregateKind,
        relation: String,
        field: Option<String>,
    },
}

pub enum AggregateKind {
    Count,
    Sum,
    Avg,
    Min,
    Max,
}
```

Validation rules (enforced at parse time):
- `field` must be `None` when `kind == Count`, and `Some` otherwise.
- `relation` must resolve to an outgoing relation on the current model.
- `field` (when present) must resolve to a scalar field on the target model.
- Aggregate fields cannot be `@id`, `@unique`, `@relation`, or `@generated` simultaneously.
- `@generated` fields cannot be `@id` or `@auto`.

### 3.2 `.prax` grammar

```text
model User {
    id         Int      @id @auto
    email      String   @unique
    firstName  String
    lastName   String
    posts      Post[]

    fullName   String   @generated("first_name || ' ' || last_name") @stored
    searchKey  String   @generated("LOWER(email)") @virtual

    postCount  Int      @count(posts)
    totalViews Int      @sum(posts.views)
    lastPostAt DateTime? @max(posts.created_at)
}
```

Parser extensions in `prax-schema/src/parser.rs`:
- `@generated(<string-literal>)` recognized as a directive returning `Directive::Generated`.
- `@stored` / `@virtual` accepted as paired modifiers. Default is `@stored` if neither is given.
- `@count(<ident>)`, `@sum(<ident>.<ident>)`, etc. recognized as aggregate directives.
- Helpful errors when:
  - `@count(posts.views)` (Count must not include a field).
  - `@sum(posts)` (non-Count must include a field).
  - `@generated` is paired with `@id`/`@auto`.

### 3.3 `#[derive(Model)]` attribute syntax (`prax-codegen/src/derive.rs`)

```rust
#[derive(Model)]
#[prax(table = "users")]
struct User {
    #[prax(id, auto)]
    id: i32,

    first_name: String,
    last_name: String,

    #[prax(generated = "first_name || ' ' || last_name", stored)]
    full_name: String,

    #[prax(count(posts))]
    post_count: i64,

    #[prax(sum(posts.views))]
    total_views: Option<i32>,
}
```

The derive parser produces the same `FieldKind::DbGenerated` / `FieldKind::Aggregate` AST nodes, going through the same validation as the `.prax` parser.

## 4. Migration DDL (`prax-migrate`)

### 4.1 Capability marker

```rust
// prax-migrate/src/dialect.rs
pub trait SupportsGeneratedColumns {}
```

Impl'd by `PostgresSqlGenerator`, `MySqlGenerator`, `SqliteGenerator`,
`MssqlGenerator`, `DuckDbSqlGenerator`. Not impl'd by `CqlGenerator`
(both ScyllaDB and Cassandra).

`CqlGenerator::generate_create_table` rejects schemas containing
`FieldKind::DbGenerated` with
`MigrationError::unsupported("@generated columns not supported on CQL engines")`.

### 4.2 Per-dialect DDL fragments

| Dialect | Stored | Virtual |
|---------|--------|---------|
| Postgres | `"col" TYPE GENERATED ALWAYS AS (expr) STORED` | rejected — PG only supports STORED (see §4.3) |
| MySQL | `col TYPE AS (expr) STORED` | `col TYPE AS (expr) VIRTUAL` |
| SQLite | `col TYPE GENERATED ALWAYS AS (expr) STORED` | `col TYPE GENERATED ALWAYS AS (expr) VIRTUAL` |
| MSSQL | `col AS (expr) PERSISTED` | `col AS (expr)` (TYPE elided — MSSQL infers) |
| DuckDB | `col TYPE GENERATED ALWAYS AS (expr) STORED` | `col TYPE GENERATED ALWAYS AS (expr) VIRTUAL` |

### 4.3 Postgres `@virtual` rejection

Postgres does not support virtual generated columns prior to PG 17.
`PostgresSqlGenerator` rejects `DbGenerated { stored: false, .. }` with
a clear error. We do not try to detect PG version at migrate time.

### 4.4 Expression-text translation

The `expr` string is passed through verbatim to the DDL. We do *not*
translate dialect-specific functions. Users targeting multiple engines
must write portable SQL or use vendor-specific schemas. This matches
Prisma's behavior.

### 4.5 Aggregate fields

`FieldKind::Aggregate` produces **no DDL** — they are query-time
projections. The migration diff skips them entirely; renaming or
removing them is a code-only change.

## 5. Query IR (`prax-query`)

### 5.1 `Filter::ScalarSubquery` (already exists)

`Filter::ScalarSubquery { sql: Cow<'static, str>, params: Vec<FilterValue> }`
already exists in `prax-query/src/filter.rs` from phase 1. The `{N}`
placeholder substitution is handled by `Filter::to_sql` at SqlBuilder
time. Codegen for relation-aggregate `where:` clauses constructs the
full boolean predicate as the `sql` field.

Example: `where: { post_count: { gt: 5 } }` lowers to
```rust
Filter::ScalarSubquery {
    sql: Cow::Borrowed(
        "(SELECT COUNT(*) FROM \"posts\" WHERE \"posts\".\"author_id\" = \"users\".\"id\") > {0}"
    ),
    params: vec![FilterValue::Int(5)],
}
```

### 5.2 New `ScalarProjection` runtime type

```rust
// prax-query/src/projection.rs (new module)
pub struct ScalarProjection {
    /// SQL fragment with `{N}` placeholders, identical conventions to
    /// `Filter::ScalarSubquery::sql`.
    pub sql: Cow<'static, str>,
    pub params: Vec<FilterValue>,
    pub alias: &'static str,
}
```

`Operation` (the runtime query builder) gains:
```rust
pub struct Operation<'a> {
    // ... existing fields ...
    pub extra_projections: Vec<ScalarProjection>,
}
```

`SqlBuilder` emits `, (...) AS "alias"` after the regular column list,
renumbering `{N}` placeholders into dialect placeholders in the same
pass that handles `Filter::ScalarSubquery`.

### 5.3 Capability marker

```rust
// prax-query/src/capabilities.rs
pub trait SupportsScalarSubqueryInSelect {}
```

Impl'd by the same five SQL engines (`prax-postgres`, `prax-mysql`,
`prax-sqlite`, `prax-mssql`, `prax-duckdb`). Not by `prax-mongodb`
(until follow-up `$lookup` lowering ships) or by CQL engines.

`Operation` calls that attach a `ScalarProjection` are gated:
```rust
impl<'a> Operation<'a> {
    pub fn with_scalar_projection(self, proj: ScalarProjection) -> Self
    where Self: SupportsScalarSubqueryInSelect,
    { /* ... */ }
}
```

The actual gate sits on the engine the operation is bound to — same
pattern as `SupportsNestedWrites` for `with(NestedWriteOp)`.

### 5.4 OrderBy lowering

`<Model>OrderByInput` for aggregate fields lowers to ordering on the
scalar-subquery expression. The SqlBuilder renders this as
`ORDER BY (SELECT COUNT(*) FROM ...) ASC|DESC` and renumbers
placeholders consistently with WHERE and SELECT.

## 6. Codegen (`prax-codegen`)

### 6.1 Result struct fields

| Field kind | Result struct type |
|-----------|-------------------|
| `DbGenerated { .. }` | Same as the field's declared type. Real column → no special handling. |
| `Aggregate { kind: Count, .. }` | `i64` — `COUNT(*)` never returns NULL. |
| `Aggregate { kind: Sum, .. }` | `Option<T>` matching the underlying column. Empty sum → NULL. |
| `Aggregate { kind: Avg, .. }` | `Option<f64>` always — averages widen to float. |
| `Aggregate { kind: Min \| Max, .. }` | `Option<T>` matching the underlying column. |

### 6.2 Input-struct membership

| Input | Includes `DbGenerated` | Includes `Aggregate` |
|-------|-----------------------|---------------------|
| `WhereInput` | yes | yes (via `Filter::ScalarSubquery`) |
| `SelectInput` | yes | yes (via `ScalarProjection`) |
| `OrderByInput` | yes | yes |
| `CreateInput` | no | no |
| `UpdateInput` | no | no |

### 6.3 `<Model>FullColumns` set

`<Model>::FULL_COLUMNS` and `<Model>::COLUMN_LIST` include `DbGenerated`
(they have an underlying column) but exclude `Aggregate` (they don't).

### 6.4 `<Model>Count` synthetic struct

For every model with at least one outgoing relation, codegen emits:

```rust
#[derive(Debug, Clone, Default)]
pub struct UserCount {
    pub posts: Option<i64>,
    pub comments: Option<i64>,
    // one Option<i64> field per outgoing relation
}
```

The result struct gains:

```rust
pub struct User {
    // ... real columns + @generated + schema-level @count fields ...
    pub _count: Option<UserCount>,
}
```

`FromRow` for `User` defaults `_count` to `None` unless the row contains
any of the `_count_<rel>` columns, in which case it populates the
substruct with `Some(_)`.

### 6.5 `_count` accessor in `select:`

The `select:` brace block accepts a new key `_count` whose value is a
brace block of `<rel>: true` entries:

```rust
prax::find_many!(client.user, {
    select: { id, email, _count: { posts: true, comments: true } }
});
```

Lowering:
1. The lowered `SelectInput` adds two `ScalarProjection`s, one per
   relation: `(SELECT COUNT(*) FROM "posts" WHERE "posts"."author_id" = "users"."id") AS "_count_posts"`.
2. The lowered `Operation` carries them through `extra_projections`.
3. `FromRow` reads `_count_<rel>` columns into a `UserCount` value and
   stores `Some(_)` in `result._count`.

### 6.6 `include:` is not extended for `_count`

The spec mentions `include` as an alternative entry point but for
brevity and to keep this PR focused, only `select: { _count: ... }`
is supported here. `include: { _count: ... }` is a follow-up.

### 6.7 Macro diagnostics

- Unknown relation in `_count: { foo: true }` → did-you-mean across the
  model's outgoing relations.
- Aggregate field used in `data:` block (`create!`/`update!`) → "field
  `post_count` is a computed virtual and cannot be assigned".
- `@generated` field used in `data:` → same diagnostic.
- Aggregate filter used against an engine without
  `SupportsScalarSubqueryInSelect` → standard missing-impl diagnostic
  ("the trait `SupportsScalarSubqueryInSelect` is not implemented for `MongoEngine`").

## 7. Engine capability impls

```rust
// prax-postgres/src/capabilities.rs
impl SupportsGeneratedColumns for PostgresEngine {}
impl SupportsScalarSubqueryInSelect for PostgresEngine {}

// prax-mysql, prax-sqlite, prax-mssql, prax-duckdb: same pair.

// prax-mongodb, prax-scylladb, prax-cassandra: neither.
```

Migration-side `SupportsGeneratedColumns` lives on the *generator*
(`PostgresSqlGenerator`), not the engine; query-side
`SupportsScalarSubqueryInSelect` lives on the engine type.

## 8. Pure-Rust computed methods (class 3)

No code or DSL changes. The spec entry exists so the docs can describe
the recommended pattern (regular `impl Model { fn … }`) for derived
values that depend only on already-loaded fields. The implementation
plan includes a rustdoc paragraph but no code.

## 9. Testing strategy

| Layer | Location | What it catches |
|-------|----------|-----------------|
| Schema AST + parser | `prax-schema/tests/` | `@generated`/`@count`/`@sum` round-trip; validation error messages |
| Migration DDL | `prax-migrate/tests/generated_columns.rs` | Per-dialect SQL snapshots for `@generated`; CQL rejection |
| `Filter::ScalarSubquery` | `prax-query/src/filter.rs::tests` | Placeholder substitution covers `{0}`, `{1}`, repeated indices, mixed-order |
| `ScalarProjection` SQL emit | `prax-query/tests/projection.rs` | New module: alias quoting, placeholder offset alignment with WHERE params, multi-projection ordering |
| Codegen UI | `prax-codegen/tests/ui/computed_*.rs` | trybuild fixtures for `@generated`/`@count`/`_count` good + diagnostic cases |
| e2e via RecordingEngine | `tests/computed_fields_e2e.rs` (new) | Mock-engine assertions on the SQL emitted for filter-by-aggregate, select-with-`_count`, order-by-aggregate, `@generated` excluded from CreateInput |
| Live Postgres | `prax-postgres/tests/computed_fields.rs` | One container-backed test with both a `@generated` column and a `@count` virtual |

A separate compile_fail.rs case asserts that an aggregate filter
against `MongoEngine` fails to compile with the
`SupportsScalarSubqueryInSelect` diagnostic.

## 10. Risks and mitigations

| Risk | Probability | Mitigation |
|------|-------------|-----------|
| Placeholder collisions when stitching `ScalarProjection`+`ScalarSubquery`+regular params in one query | Medium | Centralize the `{N}` rewrite pass in SqlBuilder; unit-test multi-source placeholder rewriting |
| `_count` alias clashes with a real column named `_count_posts` | Low | Document that `_count_*` is reserved; runtime guard rejects schema fields starting with `_count_` |
| PG version detection for `@virtual` | Low | Reject at migrate time; users on PG 17+ override with raw migrations until we add a version probe |
| User-supplied `expr` text contains a placeholder collision | Low | We do not parse `expr` — it's emitted verbatim. Document that `{N}` patterns inside expression strings are a footgun (mitigation: the `{N}` substitution only runs on `Filter::ScalarSubquery::sql` / `ScalarProjection::sql`, never on `DbGenerated::expr` which is DDL text only) |

## 11. Compatibility

- New `FieldKind` variants — additive; `FieldKind` is `#[non_exhaustive]`.
- New `Operation::extra_projections` field — additive; constructors default to `Vec::new()`.
- New marker traits — additive.
- Existing schemas keep parsing unchanged.

## 12. Open follow-ups (deferred)

- MongoDB `$lookup` lowering for aggregates.
- `include: { _count: … }` form (mirror of `select: { _count: … }`).
- Lateral-join optimization for scalar subqueries.
- Postgres ≥ 17 `@virtual` generated columns (rejected today).
- Cross-dialect `@generated` expression translator.
- `_count: { posts: { where: { … } } }` form (filtered counts).
