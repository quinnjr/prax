# Aggregate Macros — Phase 6 Design

> Companion to `2026-05-18-typed-query-traits-design.md` §8 phase 6.
> Single PR covering `count!` `select:` extension, `aggregate!`, and
> `group_by!`.

## 1. Goals

- Ship the three Prisma-style aggregate macros on top of the existing
  `AggregateOperation` and `GroupByOperation` runtime builders in
  `prax-query/src/operations/aggregate.rs`:
  - **`count!` `select:` block** (extends the phase-3 `count!` which is
    `where:`-only) — Prisma-style per-column non-null counts.
  - **`aggregate!`** — new macro returning sum/avg/min/max/count
    scalars.
  - **`group_by!`** — new macro with `by:`, `having:`, `order_by:`.
- All three target the five SQL engines that already implement
  `AggregateOperation`/`GroupByOperation` (Postgres, MySQL, SQLite,
  MSSQL, DuckDB).
- Single PR matches the spec's "2-3 day" framing and the phase 5.5
  precedent.

## 2. Non-goals

- MongoDB `$group` pipeline lowering — separate follow-up.
- CQL `GROUP BY` (limited to partition-key prefixes; needs a separate
  spec).
- `_min`/`_max` against multiple columns in one builder call (Prisma
  supports this; we ship one column per call for now).
- Window-function aggregates, percentiles, cumulative counts.
- `DISTINCT` variants on `_count` (the runtime already supports
  `count_distinct`; macro DSL for it is a follow-up).

## 3. Macro surface

### 3.1 `count!` with `select:`

Without `select:`, behavior is unchanged from phase 3 — returns plain
`i64`:

```rust
let n: i64 = prax::count!(c.user, { where: { active: true } })
    .exec()
    .await?;
```

With `select:`, returns a per-model `<Model>CountSelectResult`:

```rust
let counts: UserCountSelectResult = prax::count!(c.user, {
    where: { active: true },
    select: { _all: true, email: true, deleted_at: true },
})
.exec()
.await?;
// counts._all          -> i64  (COUNT(*))
// counts.email         -> i64  (COUNT("email") — null-skipping)
// counts.deleted_at    -> i64  (COUNT("deleted_at"))
```

Lowered SQL on Postgres:
```sql
SELECT
    COUNT(*) AS "_all",
    COUNT("email") AS "email",
    COUNT("deleted_at") AS "deleted_at"
FROM "users" WHERE "active" = $1
```

### 3.2 `aggregate!`

```rust
let agg: UserAggregateResult = prax::aggregate!(c.user, {
    where: { active: true },
    _sum: { views: true, score: true },
    _avg: { score: true },
    _min: { created_at: true },
    _max: { created_at: true },
    _count: { _all: true, email: true },
})
.exec()
.await?;

// agg._sum.views       -> Option<i64>
// agg._sum.score       -> Option<i64>
// agg._avg.score       -> Option<f64>
// agg._min.created_at  -> Option<DateTime<Utc>>
// agg._max.created_at  -> Option<DateTime<Utc>>
// agg._count._all      -> i64
// agg._count.email     -> i64
```

Empty `_<agg>` blocks are skipped. The result struct shape is fixed per
model (all five substructs exist as `Option<<Model><Agg>Result>` fields,
populated `Some(_)` only when that agg block appeared in the call).

Aggregate macros require at least one of `_count`, `_sum`, `_avg`,
`_min`, `_max` — otherwise the macro errors with
"aggregate! requires at least one of _count, _sum, _avg, _min, _max".

### 3.3 `group_by!`

```rust
let rows: Vec<UserGroupByResult> = prax::group_by!(c.user, {
    by: [team_id, region],
    where: { active: true },
    _sum: { views: true },
    _count: { _all: true },
    having: { _count: { _all: { gt: 5 } } },
    order_by: { _sum: { views: desc } },
})
.exec()
.await?;
```

Each row contains:
- The group-by columns as scalar fields (`team_id: i32`, `region: String`)
- The aggregate substructs (`_sum`, `_count`, etc.) populated according
  to the call

Lowered SQL on Postgres:
```sql
SELECT "team_id", "region",
       SUM("views") AS "_sum_views",
       COUNT(*) AS "_count__all"
FROM "users"
WHERE "active" = $1
GROUP BY "team_id", "region"
HAVING COUNT(*) > $2
ORDER BY SUM("views") DESC
```

`by: []` is rejected at macro expansion with "group_by! requires at
least one column in by:".

`having:` operators support: `equals`, `not_equals`, `lt`, `lte`, `gt`,
`gte`, `in`, `not_in`. Same as the aggregate-filter operators from
phase 5.5.

## 4. Codegen — per-model emit

For every model the codegen emits the following types in the model's
`inputs` module:

### 4.1 Select-shape inputs

```rust
#[derive(Debug, Default, Clone)]
pub struct UserCountSelect {
    pub _all: Option<bool>,
    pub id: Option<bool>,
    pub email: Option<bool>,
    pub deleted_at: Option<bool>,
    // one Option<bool> per scalar column (no relations, no aggregates,
    // no @generated columns are excluded from the count-select shape
    // since COUNT(<column>) is meaningful for any scalar including
    // @generated)
}

#[derive(Debug, Default, Clone)]
pub struct UserSumSelect {
    pub views: Option<bool>,
    pub score: Option<bool>,
    // one Option<bool> per *numeric* scalar column only
}

pub struct UserAvgSelect { /* same shape as UserSumSelect (numeric) */ }

#[derive(Debug, Default, Clone)]
pub struct UserMinSelect {
    pub id: Option<bool>,
    pub email: Option<bool>,
    pub views: Option<bool>,
    pub created_at: Option<bool>,
    // one Option<bool> per *sortable* scalar column (excludes JSON,
    // bytes, vector types)
}

pub struct UserMaxSelect { /* same shape as UserMinSelect */ }
```

### 4.2 Result-shape outputs

```rust
#[derive(Debug, Clone)]
pub struct UserCountSelectResult {
    pub _all: i64,
    pub id: i64,
    pub email: i64,
    pub deleted_at: i64,
}

#[derive(Debug, Clone)]
pub struct UserSumResult {
    pub views: Option<i64>,
    pub score: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct UserAvgResult {
    pub views: Option<f64>,
    pub score: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct UserMinResult {
    pub id: Option<i32>,
    pub email: Option<String>,
    pub views: Option<i32>,
    pub created_at: Option<DateTime<Utc>>,
}

pub struct UserMaxResult { /* same shape as UserMinResult */ }

#[derive(Debug, Default, Clone)]
pub struct UserAggregateResult {
    pub _sum:   Option<UserSumResult>,
    pub _avg:   Option<UserAvgResult>,
    pub _min:   Option<UserMinResult>,
    pub _max:   Option<UserMaxResult>,
    pub _count: Option<UserCountSelectResult>,
}

#[derive(Debug, Clone)]
pub struct UserGroupByResult {
    pub team_id: i32,
    pub region: String,
    // ... one field per group-by column in the call
    pub _sum:   Option<UserSumResult>,
    pub _avg:   Option<UserAvgResult>,
    pub _min:   Option<UserMinResult>,
    pub _max:   Option<UserMaxResult>,
    pub _count: Option<UserCountSelectResult>,
}
```

`UserGroupByResult` is emitted **once per model** with a stable
superset shape: every scalar column of the model appears as an
`Option<T>` field, plus the five aggregate substructs. Only the
columns named in the call's `by:` list are populated; the rest stay
`None`. Codegen can't synthesize a per-call type without an extra
macro-emitted module, and a stable per-model type keeps user code
type-friendly (no fresh result struct per call site). Users who want
a tighter type can `Into::into` their own struct.

### 4.3 Args structs

```rust
#[derive(Debug, Default, Clone)]
pub struct UserAggregateArgs {
    pub where_input: Option<UserWhereInput>,
    pub _sum:   Option<UserSumSelect>,
    pub _avg:   Option<UserAvgSelect>,
    pub _min:   Option<UserMinSelect>,
    pub _max:   Option<UserMaxSelect>,
    pub _count: Option<UserCountSelect>,
}

#[derive(Debug, Default, Clone)]
pub struct UserGroupByArgs {
    pub by:           Vec<UserGroupByColumn>,  // strongly-typed column enum
    pub where_input:  Option<UserWhereInput>,
    pub _sum:         Option<UserSumSelect>,
    pub _avg:         Option<UserAvgSelect>,
    pub _min:         Option<UserMinSelect>,
    pub _max:         Option<UserMaxSelect>,
    pub _count:       Option<UserCountSelect>,
    pub having:       Option<UserGroupByHaving>,
    pub order_by:     Option<UserGroupByOrderBy>,
}
```

`UserGroupByColumn` is a per-model enum naming the model's scalar
columns (lets `by:` resolve column references at compile time without
string-typing).

## 5. Macro entry points (`prax-codegen/src/macros/ops/`)

New files:
- `aggregate.rs` — entry point for `aggregate!`, parses the brace-block
  DSL and lowers to `AggregateOperation` builder calls via a generated
  `with_aggregate_args(args: <Model>AggregateArgs)` extension on
  `ModelAccessor`.
- `group_by.rs` — entry point for `group_by!`, parses DSL with the
  additional `by:`, `having:`, `order_by:` keys; lowers to
  `GroupByOperation`.

Extended file:
- `count.rs` — replace the existing
  `"select: on count! is a phase-6 feature"` rejection with the
  Prisma-compatible `select:` parser. When `select:` is present, build
  a `<Model>CountSelect` value, switch the operation to
  `AggregateOperation::count_columns(...)`, and return the new
  `<Model>CountSelectResult`.

## 6. Lowering helpers (`prax-codegen/src/macros/lower/`)

New helpers:
- `aggregate_select.rs` — parses each `_<agg>: { col: true, ... }`
  brace block into the corresponding `<Model><Agg>Select` struct
  initialization. Shared between `count!`, `aggregate!`, and
  `group_by!`.
- `having.rs` — parses `having: { _count: { _all: { gt: 5 } } }` into
  a sequence of `HavingCondition` calls. Operator set: `equals`,
  `not_equals`, `lt`, `lte`, `gt`, `gte`, `in`, `not_in` (same as
  phase 5.5 aggregate filters).

## 7. Runtime wiring

No new runtime types needed. Use the existing
`AggregateOperation` / `GroupByOperation` (in
`prax-query/src/operations/aggregate.rs`) plus two new extension
methods generated by codegen:

```rust
impl<E: QueryEngine> ModelAccessor<E, User> {
    pub fn aggregate(&self) -> AggregateOperation<User, E> { ... }
    pub fn group_by(&self, by: Vec<UserGroupByColumn>) -> GroupByOperation<User, E> { ... }
}

impl<E: QueryEngine> AggregateOperation<User, E> {
    pub fn with_aggregate_args(mut self, args: UserAggregateArgs) -> Self {
        if let Some(w) = args.where_input { self = self.with_where_input(w); }
        if let Some(s) = args._sum {
            for col in s.fields_set() { self = self.sum(col); }
        }
        // ... etc for _avg / _min / _max / _count ...
        self
    }
}

impl<E: QueryEngine> GroupByOperation<User, E> {
    pub fn with_group_by_args(mut self, args: UserGroupByArgs) -> Self {
        // Similar: apply where_input, agg blocks, having, order_by.
        self
    }
}
```

The `fields_set()` helper on each `<Model><Agg>Select` struct walks
its fields and returns the list of column names whose `Option<bool>`
is `Some(true)`.

## 8. Capability gating

No new marker trait. The macros emit `AggregateOperation`/`GroupByOperation`
calls, which already require `QueryEngine` and which engines vary in
support for natively. The existing engine impls cover PG, MySQL, SQLite,
MSSQL, DuckDB. MongoDB and CQL engines that don't impl these operations
fail to compile via the existing trait-bound diagnostics, same way phase
5 nested writes are gated.

If we want a clearer error message, add `SupportsAggregateMacros` (impl'd
by the same five SQL engines) and gate `aggregate()` / `group_by()` on
it. **Decision**: skip the new marker — the existing missing-`exec()`
error is acceptable and we don't want capability-trait proliferation
(risk listed at phase-1 spec §12).

## 9. Diagnostics

All emitted via `syn::Error::new(span, msg)` at macro expansion. Use
the existing `strsim::jaro_winkler` did-you-mean machinery where
applicable.

| Trigger | Message |
|---------|---------|
| `_sum` / `_avg` on non-numeric column | `field \`email\` is not numeric; \`_sum\` requires a numeric column` |
| Aggregate on relation field | `field \`posts\` is a relation; aggregates require a scalar column` |
| Aggregate on `@generated` column | OK (treated like normal scalar — they're real columns) |
| Aggregate on aggregate field (e.g. `_sum` of `post_count`) | `field \`post_count\` is itself an aggregate; cannot aggregate an aggregate` |
| `by: [unknown_col]` | `unknown column \`foo\`; did you mean \`team_id\`?` |
| `by: []` | `group_by! requires at least one column in \`by:\`` |
| `having: { _count: { _all: { unknown_op: ... } } }` | `unsupported operator \`like\`; use one of equals/not_equals/lt/lte/gt/gte/in/not_in` |
| `having:` referencing field not in any agg block | `field \`views\` is not in any aggregate block; cannot \`having\` on it` |
| Empty all `_<agg>` blocks in `aggregate!` | `aggregate! requires at least one of _count, _sum, _avg, _min, _max` |
| Empty `_<agg>: {}` block | `\`_sum\` block is empty; specify at least one column or remove the block` |
| Unknown column in any `_<agg>: { col: true }` | `unknown column \`foo\` on model \`User\`; did you mean \`fop\`?` |
| `select:` on `count!` with non-`bool` value | `select on count! must be \`{ col: true }\`; got \`{ col: 5 }\`` |

## 10. Testing strategy

| Layer | Location | What it catches |
|-------|----------|----------------|
| Lowering unit tests | `prax-codegen/src/macros/lower/aggregate_select.rs::tests`, `having.rs::tests` | DSL → token-stream correctness; helper output stability |
| Macro UI tests | `prax-codegen/tests/ui/aggregate_*.rs`, `group_by_*.rs`, `count_select_*.rs` | trybuild fixtures for the 11 diagnostics in §9 |
| Runtime SQL emit | `prax-query/src/operations/aggregate.rs::tests` | `with_aggregate_args` / `with_group_by_args` build the right SQL |
| e2e | `tests/aggregate_macros_e2e.rs` (new) | RecordingEngine assertions on full macro call sites |
| Live PG | `prax-postgres/tests/aggregate_macros.rs` (new, `#[ignore]`-gated) | round-trip against real Postgres |

The e2e file uses the same `RecordingEngine` + `DialectKind` pattern
established in `tests/nested_writes_e2e.rs` and
`tests/computed_fields_e2e.rs`.

## 11. Compatibility

- `count!` `select:` is additive — existing `count!` calls without
  `select:` continue to return `i64`.
- `aggregate!` and `group_by!` are new macros; no existing call sites.
- New per-model types live in the model's `inputs` module; additive.
- `AggregateOperation` / `GroupByOperation` runtime types unchanged
  (we only add extension methods).

## 12. Open follow-ups (deferred)

- MongoDB `$group` pipeline lowering.
- CQL `GROUP BY` (partition-key prefix only).
- `count_distinct` variant on `_count` shape (runtime already
  supports it).
- `_min`/`_max` against multiple columns at once.
- Window-function aggregates, percentiles.
- `having:` support for arbitrary scalar fields (not just aggregates).
- `include:` form for aggregates (analogous to `include: { _count }`
  which we already deferred from phase 5.5).
