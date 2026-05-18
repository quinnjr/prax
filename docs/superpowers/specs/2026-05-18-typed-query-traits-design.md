# Typed Query Trait System and Prisma-Style Macro DSL

- **Status**: Draft
- **Date**: 2026-05-18
- **Author**: Joseph R. Quinn (with Claude)
- **Affects**: `prax-query`, `prax-codegen`, every engine crate, `prax-migrate`, `prax-schema`, the docs Angular site
- **Builds on**: existing `Filter` / `IncludeSpec` / `CreateData` / `UpdateData` / `QueryEngine` (unchanged)

## 1. Goals and non-goals

### Goals

- A declarative, Prisma-like macro DSL for the full operation surface of `prax-query` (`find_*`, `create`, `update`, `delete`, `upsert`, `count`, `aggregate`, `group_by`).
- A typed trait system that codegen emits per model so the macro is a thin layer on top of inspectable, documentable structs rather than ad-hoc proc-macro output.
- Compile-time validation of field names, operators, relation cardinality, and engine capability.
- Composition: spread (`..base`) and conditional (`#[if(...)]`) field syntax for building queries from runtime values.
- Full coexistence with the existing fluent builder API. Nothing breaks; nothing is deprecated in this design.
- First-class support for computed and virtual fields (DB-generated columns and relation aggregates).

### Non-goals

- Generic result types parameterized by `include` shape (defer; relations stay `Option<Vec<T>>` on the model).
- Polymorphic relations or single-table inheritance.
- Custom raw SQL fragments inside the DSL — `raw_query!` covers that.
- Pure-Rust computed accessors woven into WHERE clauses — out of scope; use `@generated` columns or `@aggregate` virtuals.
- CQL (ScyllaDB / Cassandra) nested writes — explicitly compile-time blocked.
- MongoDB-specific operators (`$exists`, `$type`, geo queries).
- MongoDB `$lookup` lowering for aggregate virtuals — follow-up plan.

## 2. Architecture — three layers

### Layer 1: Runtime IR (unchanged)

`prax-query`'s `Filter`, `FilterValue`, `IncludeSpec`, `OrderBy`, `Pagination`, `CreateData`, `UpdateData`, `QueryEngine`, and every operation in `prax_query::operations` stay exactly as they are. They are the canonical runtime IR consumed by the dialect/SQL builders. **Query execution does not change.**

One additive change to the IR: a new `Filter::ScalarSubquery { sql: Cow<'static, str>, params: Vec<FilterValue> }` variant to support relation-aggregate virtual fields (see §9). SQL builders splice the embedded SQL into the surrounding WHERE/SELECT. Marked `#[non_exhaustive]` so future additions remain non-breaking.

### Layer 2: Per-model typed inputs (new, codegen-emitted)

Codegen emits a per-model `inputs` submodule containing the following types and trait impls. For a model `User`:

| Generated type                | New trait                                | Lowers to (runtime IR)                |
|-------------------------------|------------------------------------------|---------------------------------------|
| `UserWhereInput`              | `WhereInput<Model = User>`               | `Filter`                              |
| `UserWhereUniqueInput`        | `WhereUniqueInput<Model = User>`         | `Filter` (unique-only invariant)      |
| `UserInclude`                 | `IncludeInput<Model = User>`             | `Include` (set of `IncludeSpec`)      |
| `UserSelect`                  | `SelectInput<Model = User>`              | `SelectionSet`                        |
| `UserOrderBy`                 | `OrderByInput<Model = User>`             | `OrderBy`                             |
| `UserCreateInput`             | `CreateInput<Model = User>`              | `NestedWritePlan`                     |
| `UserUpdateInput`             | `UpdateInput<Model = User>`              | `NestedWritePlan`                     |
| `UserCountSelect`             | `CountSelect<Model = User>`              | `_count` aggregate spec               |
| `UserAggregateInput`          | `AggregateInput<Model = User>`           | aggregate spec                        |
| `UserGroupByInput`            | `GroupByInput<Model = User>`             | group-by spec                         |

Each trait has the form:

```rust
pub trait WhereInput {
    type Model: crate::traits::Model;
    fn into_ir(self) -> Filter;
}
```

— one method, plus an associated `Model` type so bounds remain tight.

Shared scalar filter wrappers in `prax_query::inputs`:

```rust
#[derive(Debug, Clone, Default)]
pub struct StringFilter {
    pub equals:      Option<String>,
    pub not:         Option<Box<StringFilter>>,
    pub in_list:     Option<Vec<String>>,
    pub not_in:      Option<Vec<String>>,
    pub lt:          Option<String>,
    pub lte:         Option<String>,
    pub gt:          Option<String>,
    pub gte:         Option<String>,
    pub contains:    Option<String>,
    pub starts_with: Option<String>,
    pub ends_with:   Option<String>,
    pub mode:        Option<QueryMode>,
}

#[derive(Debug, Clone, Default)]
pub struct StringNullableFilter {
    // every StringFilter field, plus:
    pub is_null: Option<bool>,
}

// Plus: IntFilter, BigIntFilter, FloatFilter, DecimalFilter, BoolFilter,
//       BytesFilter, DateTimeFilter, DateFilter, TimeFilter, UuidFilter,
//       JsonFilter, EnumFilter<E>, and nullable variants.

#[derive(Debug, Clone, Default)]
pub struct ListRelationFilter<W> { pub some: Option<W>, pub every: Option<W>, pub none: Option<W> }

#[derive(Debug, Clone, Default)]
pub struct SingleRelationFilter<W> { pub is: Option<W>, pub is_not: Option<W> }
```

Each scalar filter has helper constructors (`StringFilter::contains("...")`, `IntFilter::gt(18)`) and `From<scalar>` impls so the macro can accept `email: "a@b.com"` as shorthand for `email: { equals: "a@b.com" }`.

Scalar field updates use dedicated wrappers for atomic operations:

```rust
#[derive(Debug, Clone, Default)]
pub struct IntFieldUpdate {
    pub set:       Option<i64>,
    pub increment: Option<i64>,
    pub decrement: Option<i64>,
    pub multiply:  Option<i64>,
    pub divide:    Option<i64>,
}

#[derive(Debug, Clone, Default)]
pub struct StringNullableFieldUpdate {
    pub set:   Option<String>,
    pub unset: Option<bool>,
}

impl From<i64>    for IntFieldUpdate              { /* { set: Some(v), .. } */ }
impl From<String> for StringNullableFieldUpdate   { /* { set: Some(v), .. } */ }
```

### Layer 3: Macros and capability gates (new)

One proc-macro per operation in `prax-codegen`:

```
prax::find_unique!   prax::find_first!     prax::find_many!
prax::create!        prax::create_many!
prax::update!        prax::update_many!
prax::delete!        prax::delete_many!
prax::upsert!
prax::count!         prax::aggregate!      prax::group_by!
```

Plus shape macros for composition:

```
prax::where!         prax::include!        prax::select!
prax::order_by!      prax::data!
```

All macros are schema-aware proc-macros that load the schema once per compile and emit token streams that construct the layer-2 input types.

Capability marker traits in `prax_query::capabilities`:

- `SupportsRelationFilter` — `some`/`every`/`none`/`is`/`is_not` available.
- `SupportsCorrelatedSubquery` — superset for nested EXISTS.
- `SupportsJsonPath` — JSON-path filter operators.
- `SupportsCaseInsensitiveMode` — Postgres ILIKE; others fall back to `LOWER(...)` and skip the gate.
- `SupportsFullTextSearch`, `SupportsArrayOps` — extensible.
- `SupportsGeneratedColumns` — DDL for `GENERATED ALWAYS AS (...)`.
- `SupportsScalarSubqueryInSelect` — relation-aggregate virtuals.
- `SupportsNestedWrites` — Prisma-style nested create/connect/disconnect.

Engine crates impl the marker traits they satisfy. Methods on input types and macro expansions that produce capability-dependent fragments carry `where E: SupportsX` bounds, so misuse fails at compile time with a `#[diagnostic::on_unimplemented]` message.

## 3. Generated input types — concrete shapes

For the example schema:

```text
model User {
    id        Int      @id @auto
    email     String   @unique
    name      String?
    age       Int?
    active    Boolean  @default(true)
    role      Role
    posts     Post[]
    profile   Profile?
    createdAt DateTime @default(now())
}
```

codegen emits, in module `inputs::user`:

```rust
#[derive(Debug, Clone, Default)]
pub struct UserWhereInput {
    pub id:         Option<IntFilter>,
    pub email:      Option<StringFilter>,
    pub name:       Option<StringNullableFilter>,
    pub age:        Option<IntNullableFilter>,
    pub active:     Option<BoolFilter>,
    pub role:       Option<EnumFilter<Role>>,
    pub created_at: Option<DateTimeFilter>,

    // Relation filters — only emitted when the engine impls SupportsRelationFilter.
    pub posts:   Option<ListRelationFilter<PostWhereInput>>,
    pub profile: Option<SingleRelationFilter<ProfileWhereInput>>,

    // Logical combinators.
    pub and: Option<Vec<UserWhereInput>>,
    pub or:  Option<Vec<UserWhereInput>>,
    pub not: Option<Box<UserWhereInput>>,
}

#[derive(Debug, Clone)]
pub enum UserWhereUniqueInput {
    Id(i64),
    Email(String),
    // Composite uniques generated as named variants.
}

#[derive(Debug, Clone, Default)]
pub struct UserInclude {
    pub posts:   Option<PostsIncludeArgs>,
    pub profile: Option<ProfileIncludeArgs>,
    pub _count:  Option<UserCountSelect>,
}
// Codegen emits `#[allow(non_snake_case)]` on UserInclude/UserSelect so the
// `_count` ident doesn't trigger lints in user crates. The leading underscore
// follows the Prisma convention and disambiguates from any user-defined `count`
// field on a model.

#[derive(Debug, Clone, Default)]
pub struct PostsIncludeArgs {
    pub r#where:  Option<PostWhereInput>,
    pub order_by: Option<Vec<PostOrderBy>>,
    pub skip:     Option<u64>,
    pub take:     Option<u64>,
    pub cursor:   Option<PostWhereUniqueInput>,
    pub include:  Option<PostInclude>,
    pub select:   Option<PostSelect>,
}

#[derive(Debug, Clone, Default)]
pub struct UserSelect {
    pub id:         Option<bool>,
    pub email:      Option<bool>,
    pub name:       Option<bool>,
    pub age:        Option<bool>,
    pub active:     Option<bool>,
    pub role:       Option<bool>,
    pub created_at: Option<bool>,
    pub posts:      Option<PostsSelectArgs>,
    pub profile:    Option<ProfileSelectArgs>,
    pub _count:     Option<UserCountSelect>,
}

#[derive(Debug, Clone)]
pub enum UserOrderBy {
    Id(SortOrder),
    Email(SortOrder),
    Name(NullableSortOrder),
    Age(NullableSortOrder),
    CreatedAt(SortOrder),
    Posts(PostsOrderByRelationAggregate),  // _count, _avg, _sum on the relation
    Profile(ProfileOrderBy),
}

#[derive(Debug, Clone, Default)]
pub struct UserCreateInput {
    pub email:   String,                          // required
    pub name:    Option<String>,
    pub age:     Option<i32>,
    pub active:  Option<bool>,
    pub role:    Role,
    pub posts:   Option<PostsCreateNestedInput>,
    pub profile: Option<ProfileCreateNestedInput>,
}

#[derive(Debug, Clone, Default)]
pub struct UserUpdateInput {
    pub email:   Option<StringFieldUpdate>,
    pub name:    Option<StringNullableFieldUpdate>,
    pub age:     Option<IntFieldUpdate>,
    pub active:  Option<BoolFieldUpdate>,
    pub role:    Option<EnumFieldUpdate<Role>>,
    pub posts:   Option<PostsUpdateNestedInput>,
    pub profile: Option<ProfileUpdateNestedInput>,
}
```

Notes:

- `Option<T>` for every field is what makes `..Default::default()` work and lets the macro spread/merge cleanly. Required `CreateInput` fields stay bare (no `Option`).
- `From<scalar>` impls on filters/updates allow `email: "x"` → `{ equals: "x" }` and `name: "Alice"` → `{ set: "Alice" }`.
- `NullableSortOrder` (Prisma's `{ sort: asc, nulls: first }`) is in scope: emitted on `UserOrderBy` variants for nullable fields.
- `_count` lives in `UserSelect` / `UserInclude` only (not as a top-level `_count` key on findMany).

## 4. Macro grammar and expansion

### Forms

```rust
// Form 1: accessor + brace block (recommended).
prax::find_many!(client.user, { where: { email: { contains: "@x.com" } }, take: 10 });

// Form 2: typed-accessor path (generic code).
prax::find_many!(User on &engine, { where: { ... } });

// Form 3: explicit `for` annotation (when accessor can't be resolved automatically).
prax::find_many!(get_client().user(), for User, { where: { ... } });
```

The grammar reflects all three forms:

```
operation_macro_args := accessor_expr ("," "for" model_ident)? "," operation_input
accessor_expr        := expr | model_ident "on" expr
```

The macro produces an unexec'd operation (`FindManyOperation<E, User>`). The caller invokes `.exec().await?` themselves. That preserves composition with transactions, middleware, tenant context, etc.

### Grammar (EBNF-ish)

```
operation_input  := "{" field_list "}"
field_list       := (field ",")* field?
field            := ident ":" value
                  | ".." expr                                  // spread
                  | "..move" expr                              // move-spread (no clone)
                  | "#[if(" expr ")]" ident ":" value
                  | "#[else_if(" expr ")]" ident ":" value
                  | "#[else]" ident ":" value
value            := literal | path | expr_in_parens
                  | "{" field_list "}"                         // nested input shape
                  | "[" expr_list "]"                          // list (in_list, AND, etc.)
                  | "true" | "false"                           // include/select shortcuts
                  | "@(" expr ")"                              // explicit Rust expr escape
                  | bare_ident                                 // role: Admin → role: { equals: Admin }
```

### Top-level keys per operation

| Macro              | Allowed top-level keys                                                            |
|--------------------|-----------------------------------------------------------------------------------|
| `find_unique!`     | `where` (must be unique), `include` xor `select`                                  |
| `find_first!`      | `where`, `order_by`, `cursor`, `skip`, `take`, `include` xor `select`             |
| `find_many!`       | `where`, `order_by`, `cursor`, `skip`, `take`, `distinct`, `include` xor `select` |
| `create!`          | `data`, `include` xor `select`                                                    |
| `create_many!`     | `data`, `skip_duplicates`                                                         |
| `update!`          | `where` (unique), `data`, `include` xor `select`                                  |
| `update_many!`     | `where`, `data`                                                                   |
| `upsert!`          | `where` (unique), `create`, `update`, `include` xor `select`                      |
| `delete!`          | `where` (unique), `include` xor `select`                                          |
| `delete_many!`     | `where`                                                                           |
| `count!`           | `where`, `order_by`, `cursor`, `skip`, `take`, `select`                           |
| `aggregate!`       | `where`, `order_by`, `cursor`, `skip`, `take`, `_count`, `_avg`, `_sum`, `_min`, `_max` |
| `group_by!`        | `by`, `where`, `having`, `order_by`, `skip`, `take`, `_count`, `_avg`, `_sum`, `_min`, `_max` |

Unknown keys at any level produce a "did you mean" diagnostic computed against the actual model.

### Operator naming

Snake_case Rust idiom: `gt`, `gte`, `lt`, `lte`, `equals`, `not`, `in_list`, `not_in`, `contains`, `starts_with`, `ends_with`, `mode: insensitive`, `some`, `every`, `none`, `is`, `is_not`, `is_null`, `set`, `increment`, `decrement`, `multiply`, `divide`, `unset`.

### Spread and conditional semantics

- `..expr` clones `expr` (which must be the same input type) and any subsequent `field: value` entries overwrite. Later wins, matching Rust struct-update syntax. The macro inserts `Clone::clone(&expr)` so the spread doesn't consume.
- `..move expr` is the same but moves (no clone) — opt-in for hot paths.
- `#[if(cond)] field: value` lowers to a `let __tmp = …; if cond { __w.field = Some(__tmp); }` block. Pairs with `#[else_if]` and `#[else]`.
- Bare identifiers as values (`role: Admin`) are resolved against the schema; if the field is an enum, becomes `EnumFilter::equals(Path::Admin)`.
- `@(expr)` escapes DSL parsing — treat the contents as a Rust expression.
- `order_by: { field: dir }` auto-wraps to `vec![{ field: dir }]`.

### Expansion example

```rust
prax::find_many!(client.user, {
    where: {
        email: { contains: "@x.com", mode: insensitive },
        age:   { gte: 18 },
        ..base_filter,
        posts: { some: { published: true } },
        or: [
            { role: Admin },
            { role: Moderator },
        ],
    },
    include: {
        posts: {
            where: { published: true },
            order_by: { created_at: desc },
            take: 5,
        },
        profile: true,
    },
    order_by: [{ created_at: desc }],
    take: 10,
});
```

expands (sketch) to:

```rust
{
    let __where = {
        let mut __w = ::prax::inputs::user::UserWhereInput {
            email: Some(::prax::inputs::StringFilter {
                contains: Some(("@x.com").into()),
                mode:     Some(::prax::inputs::QueryMode::Insensitive),
                ..::core::default::Default::default()
            }),
            age: Some(::prax::inputs::IntNullableFilter::gte(18)),
            ..::core::clone::Clone::clone(&(base_filter))
        };
        __w.posts = Some(::prax::inputs::ListRelationFilter {
            some: Some(::prax::inputs::post::PostWhereInput {
                published: Some(true.into()),
                ..::core::default::Default::default()
            }),
            ..::core::default::Default::default()
        });
        __w.or = Some(::std::vec![ /* ... */ ]);
        __w
    };
    let __include = ::prax::inputs::user::UserInclude { /* ... */ };
    ::prax::traits::ModelAccessor::find_many(&client.user())
        .with_where_input(__where)
        .with_include_input(__include)
        .with_order_by_input(::std::vec![/* ... */])
        .take(10)
}
```

The macro never invents new runtime types. `with_*_input` are extension methods on each `Operation` that call `into_ir()` and stash the resulting `Filter`/`Include`/`OrderBy`. The existing `.r#where(filter)` / `.include(spec)` builder methods stay untouched.

## 5. Schema-aware proc-macro implementation

### Schema discovery

```rust
fn resolve_schema() -> &'static Arc<Schema>;
```

Resolution order:

1. `PRAX_SCHEMA` env var (absolute or relative to `CARGO_MANIFEST_DIR`). Used directly if set; error if file missing.
2. Walk up from `CARGO_MANIFEST_DIR` looking for `prax.toml`. Read `[generator.client].schema` (defaults to `prax/schema.prax`). Resolve relative to the `prax.toml` location.
3. No `prax.toml` found → hard error: *"Could not find a `prax.toml` in any ancestor of $CARGO_MANIFEST_DIR. Set `PRAX_SCHEMA=path/to/schema.prax` or run `prax init`."*

Caching: `OnceLock<Mutex<HashMap<PathBuf, Arc<Schema>>>>` keyed by absolute schema path. First macro call parses; subsequent calls hit cache.

Dependency tracking: prefer `proc_macro::tracked_path::path(absolute_schema)` for re-trigger on schema change. Fallback for older toolchains: emit `const _: &[u8] = include_bytes!(absolute_schema_path);` inside a hidden module of the expansion so rustc's dep graph notices changes.

### Macro flow

```
parse TokenStream
  → resolve_schema() — &Schema
  → resolve model from accessor expr (`client.user` → "User")
  → parse DSL block into typed AST (WhereInputAst, IncludeAst, …)
  → validate AST against schema:
      - unknown field → "did you mean `{closest}`" (strsim::jaro_winkler ≥ 0.85)
      - wrong operator for scalar type → "field `age` (Int) does not support `contains`"
      - relation op on non-relation → "field `email` is not a relation"
      - `some`/`every`/`none` on to-one → "use `is`/`is_not` for to-one"
      - `where` not unique on find_unique/update/etc.
      - `select` xor `include`
  → lower AST → token stream constructing layer-2 input structs
  → emit
```

### Accessor resolution

- `client.MODEL` → snake_case the last path segment, match against `schema.models` (PascalCase).
- `MODEL on EXPR` → take the type-position `MODEL` ident directly.
- `expr(), for MODEL` → explicit annotation.

Failures here produce a clear error pointing to the accessor token.

### Codegen entry points

1. `prax generate` (CLI) — emits `inputs/` submodule alongside existing client `mod.rs`.
2. `#[derive(Model)]` — emits `<model_snake>_inputs` sibling module. Requires sibling models defined in the same module for relation-filter codegen to resolve cross-model types (default constraint; documented).
3. `prax_schema!` macro — same codegen as `prax generate`, inline.

### Compile-time cost budget

Per-model generated input footprint: ~200–400 lines (~10 types). For a 50-model schema with 8 fields average, ~16 kLOC generated — comparable to a moderately large `serde::Deserialize` derive footprint. Schema parse: ~5–10 ms once per crate per compile; subsequent macro calls in the same compile are <100 µs.

Feature-gated: `prax/inputs` Cargo feature on `prax-codegen` (default-on). Disabling skips input-type codegen for users who prefer the existing fluent API.

## 6. Write operations and nested-write plans

### Principles

1. **Lower to existing operations.** Nested writes compile to a sequence of `Create`/`Update`/`Upsert`/`Delete` runtime operations inside a single transaction. No new SQL.
2. **Reuse engine transactions.** The macro auto-wraps multi-op plans in `engine.transaction(...)`. Single-op plans skip the wrapper.
3. **Static dependency ordering.** Codegen knows relation directions; the plan emits ops in topological order. No runtime topo-sort.

### Nested input shapes

For `posts: Post[]` (FK on `Post.authorId`):

```rust
#[derive(Debug, Clone, Default)]
pub struct PostsCreateNestedInput {
    pub create:            Option<Vec<PostCreateWithoutAuthorInput>>,
    pub create_many:       Option<PostsCreateManyWithoutAuthorInput>,
    pub connect:           Option<Vec<PostWhereUniqueInput>>,
    pub connect_or_create: Option<Vec<PostConnectOrCreateWithoutAuthorInput>>,
}

#[derive(Debug, Clone, Default)]
pub struct PostsUpdateNestedInput {
    pub create:            Option<Vec<PostCreateWithoutAuthorInput>>,
    pub connect:           Option<Vec<PostWhereUniqueInput>>,
    pub disconnect:        Option<Vec<PostWhereUniqueInput>>,
    pub set:               Option<Vec<PostWhereUniqueInput>>,
    pub update:            Option<Vec<PostUpdateWithWhereUniqueWithoutAuthorInput>>,
    pub update_many:       Option<Vec<PostUpdateManyWithWhereWithoutAuthorInput>>,
    pub upsert:            Option<Vec<PostUpsertWithWhereUniqueWithoutAuthorInput>>,
    pub delete:            Option<Vec<PostWhereUniqueInput>>,
    pub delete_many:       Option<Vec<PostScalarWhereWithoutAuthorInput>>,
    pub connect_or_create: Option<Vec<PostConnectOrCreateWithoutAuthorInput>>,
}
```

`*WithoutAuthorInput` variants omit the FK fields the parent insert will fill in via `RETURNING`.

For `profile: Profile?` (to-one, FK on child):

```rust
#[derive(Debug, Clone, Default)]
pub struct ProfileCreateNestedInput {
    pub create:            Option<Box<ProfileCreateWithoutUserInput>>,
    pub connect:           Option<ProfileWhereUniqueInput>,
    pub connect_or_create: Option<Box<ProfileConnectOrCreateWithoutUserInput>>,
}
```

### `NestedWritePlan`

```rust
pub enum NestedWriteOp {
    InsertSelf { columns: Vec<&'static str>, values: Vec<FilterValue>, returning: ReturningSpec },
    InsertChild { parent_id_slot: SlotId, model: &'static str, columns: Vec<&'static str>, values: Vec<FilterValue> },
    Connect { parent_id_slot: SlotId, child_pk: WhereUniqueInput, fk_column: &'static str },
    ConnectOrCreate { /* ... */ },
    UpsertChild { /* ... */ },
    UpdateChild { /* ... */ },
    DeleteChild { /* ... */ },
    SetRelation { /* ... */ },
}

pub struct NestedWritePlan {
    pub ops: Vec<NestedWriteOp>,
    pub return_root_via: SlotId,
}
```

`SlotId` placeholders are substituted with PKs returned from earlier ops. The executor walks the plan in order; `Self` inserts before children; inverse-direction creates run the parent op first.

### `connect_or_create`

Two lowerings:

- **Engines with upsert syntax** (Postgres `ON CONFLICT`, SQLite `ON CONFLICT`, MySQL `INSERT … ON DUPLICATE KEY UPDATE`, MSSQL `MERGE`): single statement.
- **Other engines**: two-statement `SELECT pk WHERE unique = …; INSERT if missing` inside the surrounding transaction.

### `set: [...]` (full relation replacement)

Diff-based: fetch current children → DELETE those not in the new set → INSERT/CONNECT those that are new. Heavy but consistent. **Enabled by default** to match Prisma. Document the cost.

### Returning typed shape after a nested write

After commit, the executor runs a final find with the requested `include`/`select` against the inserted PK. Adds one roundtrip but reuses the existing find-with-include path.

### Capability gating

- All SQL drivers + MongoDB: full support.
- CQL (ScyllaDB / Cassandra): `*CreateNestedInput` / `*UpdateNestedInput` types are gated behind `SupportsNestedWrites`. CQL engines don't impl it, so nested writes don't compile.

### `QueryEngine` additions

- `fn in_transaction(&self) -> bool` — additive, default returns `false`. Drivers that support transactions override.

## 7. Error handling

Three categories:

### Compile-time errors

- Unknown field, wrong operator for scalar type, `some` on non-relation, relation cardinality mismatch, `where` not unique where required, `select` and `include` both set, capability-trait not satisfied.
- Emitted via `syn::Error::to_compile_error` with token-level spans.
- Capability errors carry `#[diagnostic::on_unimplemented]` messages.

### Runtime errors (additive `QueryError` variants, `#[non_exhaustive]`)

- `QueryError::RelationNotFound { relation, parent_id }`
- `QueryError::UniqueViolationOnConnectOrCreate { model, unique }`
- `QueryError::RequiredWhereMissing { operation }`
- `QueryError::NestedWriteFailed { op, source: Box<QueryError> }`

### Capability mismatches caught early

Marker-trait bounds on input methods plus `with_*_input` extension methods make capability errors compile-time, not runtime.

## 8. Coexistence and migration path

### Coexistence

- Existing `.r#where(filter)` / `.include(spec)` / `CreateData::new().set(…)` API unchanged.
- New `with_where_input<W: WhereInput<Model = M>>(self, w: W) -> Self` and siblings on every `Operation`.
- Old and new can be combined; later calls AND-compose with earlier filters.
- Third interface: explicit struct construction (`FindManyArgs { r#where: Some(UserWhereInput { … }), .. }`) is supported and documented.

### Phasing (non-breaking, each phase ships independently)

| Phase | Crates | Deliverable | Effort |
|-------|--------|-------------|--------|
| 1 | `prax-query` | New traits, scalar filter wrappers, `ListRelationFilter`/`SingleRelationFilter`, capability marker traits, `*Args` structs, `with_*_input` ext methods | 3–4 days |
| 2 | `prax-codegen`, engine crates | `prax generate` / `prax_schema!` / `#[derive(Model)]` emit `inputs/` modules. Engines impl capability marker traits | 4–5 days |
| 3 | `prax-codegen` | Read operation macros + schema discovery + cache + `trybuild` UI tests | 4–5 days |
| 4 | `prax-codegen` | Shape macros (`where!`, `include!`, …), spread, conditional, bare-ident enum resolution | 2–3 days |
| 5 | `prax-codegen`, `prax-query` | Write macros + `NestedWritePlan` executor + `connect_or_create` paths + `SupportsNestedWrites` gating | 5–7 days |
| 5.5 | `prax-schema`, `prax-codegen`, `prax-migrate`, engine crates | Computed/virtual field support (§9) | 3 days |
| 6 | `prax-codegen`, `prax-query` | Aggregate macros (`count!`, `aggregate!`, `group_by!`) | 2–3 days |
| 7 | repo-wide | Examples migrated, docs site updated (§10), cookbook, rustdoc chapter, cross-engine consistency tests | 5–8 days |

**Total: 28–38 feature-days.**

### Compatibility guarantees

- Every public symbol in `prax-query`'s current API remains.
- New `Filter::ScalarSubquery` variant lands behind `#[non_exhaustive]`.
- `QueryError` is `#[non_exhaustive]`; new variants are additive.
- `Model` and `ModelAccessor` traits unchanged.

## 9. Computed and virtual fields

Three classes:

### (1) DB-generated columns

```text
model User {
    id        Int    @id @auto
    firstName String
    lastName  String
    fullName  String @generated("first_name || ' ' || last_name") @stored
    searchKey String @generated("LOWER(email)") @virtual
}
```

- New `FieldKind::DbGenerated { expr: String, stored: bool }` variant in the `prax-schema` AST.
- Real scalar field in the result struct.
- Included in `WhereInput` and `SelectInput`.
- Excluded from `CreateInput` and `UpdateInput`.
- `prax-migrate` SQL generator emits `GENERATED ALWAYS AS (expr) STORED|VIRTUAL` DDL; CQL dialect rejects with clear error.
- Capability gate: `SupportsGeneratedColumns` — PG/MySQL/SQLite/MSSQL/DuckDB.

### (2) Relation aggregate virtuals

```text
model User {
    id         Int    @id @auto
    posts      Post[]
    postCount  Int    @count(posts)
    totalViews Int    @sum(posts.views)
    lastPostAt DateTime? @max(posts.created_at)
}
```

- New `FieldKind::Aggregate { kind, relation, field }` variant.
- Scalar field on result struct; no underlying column.
- Included in `WhereInput` and `SelectInput`; excluded from `CreateInput`/`UpdateInput`.
- `WhereInput::into_ir` produces `Filter::ScalarSubquery { sql, params }` (new IR variant).
- SELECT lowering: scalar subquery `(SELECT COUNT(*) FROM posts p WHERE p.author_id = u.id) AS post_count`. Lateral-join is an optimization, not baseline.
- `_count` accessor in `select` / `include` reuses the same machinery — `select: { _count: { posts: true } }` and a schema-level `@count(posts)` field produce the same lowering.
- Capability gate: `SupportsScalarSubqueryInSelect`. Implemented at phase 5.5 by every SQL engine (PG / MySQL / SQLite / MSSQL / DuckDB) via scalar subqueries. **Not** implemented by `prax-mongodb` until the follow-up `$lookup`-lowering plan ships; until then, MongoDB code that uses relation-aggregate virtuals fails to compile with a clear capability-trait diagnostic. CQL engines never implement this gate.

### (3) Pure-Rust computed methods

Regular `impl User { fn display_name(&self) -> String { … } }`. No DSL involvement. Documented as "for derived values that depend only on already-loaded fields and never need to be filtered on."

## 10. Documentation site updates (Angular `docs/` workspace)

### New pages and routes

| Route | Page module | Content |
|-------|-------------|---------|
| `/queries/input-types` | `queries-input-types.page` | Reference for the trait family and per-model generated structs |
| `/queries/macros` | `queries-macros.page` | Full grammar reference for every operation macro and shape macro; spread + conditional syntax; capability-trait error messages |
| `/queries/relation-filters` | `queries-relation-filters.page` | `some`/`every`/`none`/`is`/`is_not` — examples, generated SQL per dialect, cross-engine support matrix |
| `/queries/nested-writes` | `queries-nested-writes.page` | Nested `connect` / `create` / `connect_or_create` / `disconnect` / `set` / `update` / `upsert` / `delete` / `delete_many`; transaction semantics; CQL unsupported note |
| `/queries/computed-fields` | `queries-computed-fields.page` | `@generated` columns and `@count` / `@sum` / `@avg` / `@min` / `@max` virtuals |
| `/migration/from-fluent-builder` | `migration-from-fluent.page` | Side-by-side fluent → macro cookbook |
| `/migration/prisma-to-prax` | `migration-prisma-to-prax.page` | Prisma TS → Prax cheat sheet (snake_case operator names, Rust syntax differences) |

### Existing pages requiring substantive updates

Each gets a new "Macro DSL" or "Input types" section. The fluent-builder example moves under a "Dynamic composition" sub-heading.

- `queries-crud.page` — primary CRUD examples switch to macros.
- `queries-filtering.page` — refactor around `WhereInput` and scalar filters.
- `queries-pagination.page` — `skip`/`take`/`cursor` inside macros.
- `queries-aggregations.page` — `aggregate!`, `group_by!`, `count!`, `_count` virtual.
- `queries-upsert.page` — `upsert!` macro.
- `queries-json.page` — JSON-field filters; capability gating per engine.
- `queries-search.page` — `mode: insensitive` and per-engine behavior.
- `schema-relations.page` — include/select/relation-filter shapes; relation cardinality drives `ListRelationFilter` vs `SingleRelationFilter`.
- `schema-overview.page` — link to new generated-types reference.
- `schema-fields.page` — `@generated`, `@stored`, `@virtual`, `@count` / `@sum` / `@avg` / `@min` / `@max` attributes.
- `schema-attributes.page` — same attribute reference detail.
- `home.page`, `quickstart.page` — landing examples switch to macro form.
- `advanced-errors.page` — new `QueryError` variants and error mapping.
- `examples.page` — every example rewritten in the new style.
- `database-{postgresql,mysql,sqlite,mssql,duckdb,mongodb}.page` — capability matrix table noting which marker traits each engine implements. New pages to add: `database-scylladb.page` and `database-cassandra.page` (route + module + html) — these don't exist on the docs site today and the new feature surface is a natural moment to introduce them.

### Navigation / IA changes

- New "Macro DSL" sidebar group: `/queries/macros`, `/queries/input-types`, `/queries/relation-filters`, `/queries/nested-writes`, `/queries/computed-fields`.
- New "Migration" sidebar group with the two migration pages.
- Existing "Queries" group reordered so `queries/crud` and `queries/filtering` lead with macro examples and link out to the new Macro DSL group.

### Cross-link audit

- Every `prax::*!` macro reference on any page links to `/queries/macros#<macro>`.
- Every `WhereInput` / `IncludeInput` / capability-trait mention links to `/queries/input-types#<anchor>`.

### Build verification

- `pnpm --filter docs build`, `pnpm --filter docs lint`, `pnpm --filter docs test` must pass before phase 7 closes.
- If a docs CI job exists, extend it to cover the new pages; otherwise add one that triggers on `docs/**` changes.

## 11. Testing strategy

| Layer | Location | What it catches |
|-------|----------|-----------------|
| Unit: `into_ir` lowerings | `prax-query/tests/inputs/` | Hand-built `UserWhereInput` lowers to expected `Filter`. Pure data, no engine. |
| Macro UI tests | `prax-codegen/tests/ui/` | DSL parsing, diagnostics, "did you mean", capability-trait errors. `trybuild` with `.stderr` snapshots. Schema-aware diagnostics pinned via committed `tests/fixtures/schema.prax`, found via `PRAX_SCHEMA`. |
| Per-engine integration | `prax-postgres/tests/inputs_dsl.rs` (and siblings) | Macros against real engines, assert rows. Reuses existing test containers. |
| Cross-engine consistency | `prax-query/tests/cross_engine_dsl/` | Same macro call against PG/MySQL/SQLite/MSSQL/DuckDB; results agree. |
| `compile_fail.rs` | `prax-query/tests/` | Capability gates: `prax::find_many!(scylla_client.user, { where: { posts: { some: ... } } })` fails to compile. |

Test schema (`tests/fixtures/schema.prax`) covers: one-to-one, one-to-many, many-to-many (implicit join), self-relation, composite PK, nullable-FK, enum field, JSON field, `@generated` columns, `@count` / `@sum` virtuals.

Shared test schemas live in a new private workspace member `prax-test-schema` so all engine crates pull from one fixture set.

## 12. Risks and mitigations

| Risk | Probability | Mitigation |
|------|-------------|------------|
| `proc_macro::tracked_path` not on MSRV | Medium | `include_bytes!`-inside-hidden-module fallback; build.rs / env-var hash as final fallback |
| Compile-time bloat from generated input types | Medium | Feature-gate `inputs` codegen behind `prax/inputs` (default-on); ~16 kLOC per 50-model schema is acceptable |
| Diagnostic span quality on macro errors | Medium | Use `syn::Error::to_compile_error` with per-token spans; `trybuild` snapshots gate regressions |
| `connect_or_create` race semantics | Medium | One concrete path per engine; document the per-engine atomicity guarantees |
| Many-to-many through implicit join tables in nested writes | Medium | Codegen reads the schema's join table; phase 5 covers implicit-join only; explicit-join-model M2M deferred |
| Capability-trait proliferation | Low | Cap at the nine traits in §2; finer-grained gates become runtime errors |

## 13. Open follow-ups (post-design)

- MongoDB `$lookup` lowering for relation-aggregate virtuals (separate plan after phase 6).
- Generic result types parameterized by include shape (a future research design — not promised).
- Explicit-join-model M2M in nested writes (separate plan).
- Migration helpers for translating Prisma schemas to Prax schemas (could live in `prax-import`).
