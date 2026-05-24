# Aggregate Macros — Phase 6 Follow-ups Design

> Closes three gaps left open when phase 6 (`docs/superpowers/specs/2026-05-23-aggregate-macros-design.md`) shipped in PR #114, documented under that PR's CHANGELOG "Known limitations". Single PR.

## 1. Goals

Three follow-up gaps, all on top of the shipped phase-6 surface:

1. **Per-column count hydration** — `AggregateResult::from_row` drops the
   `_count_<col>` / `_count_distinct_<col>` aliases that
   `AggregateField::alias()` already emits, so `count!`'s `select:` and
   `aggregate!`'s `_count: { col }` emit correct SQL but the per-column
   values never reach the typed result. `COUNT(*)` (`_all`) works.
2. **`group_by!` `order_by:`** — currently rejected at macro time with a
   deferred-follow-up message; wire it.
3. **`count_distinct` macro shape** — expose `COUNT(DISTINCT col)` via
   `_count: { col: { distinct: true } }`. The runtime
   `AggregateField::CountDistinct` already exists.

## 2. Non-goals

- MongoDB `$group` / CQL `GROUP BY` lowering.
- Multi-column `_min`/`_max` in one call.
- `include: { _count }` form.
- Ordering a `group_by!` by a non-aggregate, non-by column.

## 3. Per-column count hydration (`prax-query`)

### 3.1 The gap

`AggregateField::alias()` emits:
- `CountAll` → `_count`
- `CountColumn(col)` → `_count_<col>`
- `CountDistinct(col)` → `_count_distinct_<col>`

`AggregateResult::from_row` (`prax-query/src/operations/aggregate.rs`)
only matches `k == "_count"`, `_sum_`, `_avg_`, `_min_`, `_max_`. The
`_count_<col>` and `_count_distinct_<col>` entries fall through and are
dropped.

### 3.2 Fix

Add two fields to `AggregateResult`:

```rust
#[derive(Debug, Clone, Default)]
pub struct AggregateResult {
    pub count: Option<i64>,
    pub count_columns: std::collections::HashMap<String, i64>,    // COUNT(col)
    pub count_distinct: std::collections::HashMap<String, i64>,   // COUNT(DISTINCT col)
    pub sum: std::collections::HashMap<String, f64>,
    pub avg: std::collections::HashMap<String, f64>,
    pub min: std::collections::HashMap<String, serde_json::Value>,
    pub max: std::collections::HashMap<String, serde_json::Value>,
}
```

In `from_row`, **check `_count_distinct_` before `_count_`** (the former
is a prefix-superset of the latter):

```rust
if k == "_count" {
    if let FilterValue::Int(n) = v { out.count = Some(n); }
} else if let Some(col) = k.strip_prefix("_count_distinct_") {
    if let Some(n) = value_to_i64(&v) { out.count_distinct.insert(col.to_string(), n); }
} else if let Some(col) = k.strip_prefix("_count_") {
    if let Some(n) = value_to_i64(&v) { out.count_columns.insert(col.to_string(), n); }
} else if let Some(col) = k.strip_prefix("_sum_") {
    // … unchanged …
}
```

Add a `value_to_i64` helper (mirrors `value_to_f64`: accepts
`Int`, and `String` parsed as i64; counts are never `Float`).

Accessors:

```rust
impl AggregateResult {
    pub fn count_of(&self, column: &str) -> Option<i64> {
        self.count_columns.get(column).copied()
    }
    pub fn count_distinct_of(&self, column: &str) -> Option<i64> {
        self.count_distinct.get(column).copied()
    }
}
```

`GroupByResult::exec` routes its non-group columns through
`AggregateResult::from_row`, so grouped per-column counts hydrate with
no additional change.

### 3.3 Codegen hydration (deferred boundary)

The codegen-emitted `<Model>CountSelectResult { _all: i64, <col>: i64 }`
typed struct is populated by the schema-path result mapping, which is
blocked by the pre-existing `relation_helpers` bug (see phase-6 CHANGELOG).
This follow-up fixes the **runtime** `AggregateResult` hydration — the
generic, engine-level result. Mapping that into the typed
`<Model>CountSelectResult` remains gated on the schema-path fix and is
**not** in scope here. Tests assert against `AggregateResult` directly.

## 4. `group_by!` `order_by:` (`prax-codegen` + small runtime reuse)

### 4.1 Runtime — already present

`GroupByOperation` has `order_by: Vec<OrderByField>`, an `order_by(...)`
builder, and `build_sql` emits `ORDER BY <quote_identifier(col)> <dir>`.
Aggregate aliases (`_sum_views`, `_count`, `_count_<col>`) are valid
identifiers that reference the SELECT-list aliases, which PG / MySQL /
SQLite / DuckDB / MSSQL all allow in `ORDER BY`. So no `build_sql`
change is needed — ordering by an aggregate lowers to
`OrderByField::new("_sum_views", SortOrder::Desc)`.

### 4.2 Codegen `<Model>GroupByOrderBy`

Replace the placeholder `{ items: Vec<String> }` with:

```rust
#[derive(Debug, Default, Clone)]
pub struct <Model>GroupByOrderBy {
    pub items: Vec<::prax_query::types::OrderByField>,
}
```

### 4.3 New lowering helper `order_by` parsing

`order_by: { _sum: { views: desc }, _count: { _all: asc }, team_id: desc }`
lowers each entry to an `OrderByField`:

- Aggregate keys (`_sum`/`_avg`/`_min`/`_max`/`_count`) take a
  `{ col: dir }` (or `{ _all: dir }` for count) block. The alias is
  computed the same way `AggregateField::alias()` does:
  - `_count` + `_all` → `_count`
  - `_count` + `col` → `_count_<col>`
  - `_sum`/`_avg`/`_min`/`_max` + `col` → `_<agg>_<col>`
- A bare model column key (`team_id: desc`) → `OrderByField::new("team_id", dir)`.
- Direction values: `asc` / `desc` (bare idents, matching the existing
  `order_by!`/`find_many!` order parsing — reuse that direction parser).

Validation:
- An aggregate ordering whose `(agg, col)` is not present in a
  corresponding `_<agg>:` block of the same `group_by!` call →
  `"order by `_sum.views` requires a matching `_sum: { views }` block"`.
  (The aggregate must be in the SELECT list for the alias to resolve.)
- Unknown bare column → did-you-mean against the model's scalar columns.
- A bare column ordering that isn't in the `by:` list → allowed by SQL
  only if it's also aggregated; to keep it simple, **require** bare
  order-by columns to appear in `by:` →
  `"order by `region` requires `region` in `by:`"`.

This helper lives in `prax-codegen/src/macros/lower/group_by_order_by.rs`
(or folded into the existing `having.rs` sibling — decide during
implementation; a dedicated file is cleaner).

### 4.4 Macro + extension wiring

- `group_by!` (`ops/group_by.rs`): remove the `order_by:` deferred-error
  arm; parse the block via the new helper; populate
  `args.order_by = Some(<Model>GroupByOrderBy { items })`.
- `with_group_by_args` (generated in `generators/aggregate.rs`): replace
  `let _ = args.order_by;` with
  `if let Some(ob) = args.order_by { for o in ob.items { self = self.order_by(o); } }`.

The validation in §4.3 needs to know which aggregates are in the call —
the lowering has that context (it parses the `_<agg>:` blocks before
`order_by:`), so thread the set of `(AggKind, col)` pairs into the
order-by lowering.

## 5. `count_distinct` macro shape (`prax-codegen` + runtime enum)

### 5.1 `CountSelect` field type change (option A)

New runtime enum in `prax-query` (e.g. `prax-query/src/inputs/` or
alongside the aggregate types):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CountSelectMode {
    /// COUNT(col) — non-null count.
    NonNull,
    /// COUNT(DISTINCT col).
    Distinct,
}
```

Each per-column field of the codegen-emitted `<Model>CountSelect`
changes from `Option<bool>` to `Option<CountSelectMode>`. `_all` stays
`Option<bool>` (no distinct form of `COUNT(*)`).

```rust
#[derive(Debug, Default, Clone)]
pub struct <Model>CountSelect {
    pub _all: Option<bool>,
    pub email: Option<::prax_query::CountSelectMode>,
    // … one per scalar column …
}
```

### 5.2 `fields_set` split

`generators/aggregate.rs` emits two accessors instead of one:

```rust
impl <Model>CountSelect {
    pub fn all_set(&self) -> bool { matches!(self._all, Some(true)) }
    /// Columns requesting plain COUNT(col).
    pub fn nonnull_fields_set(&self) -> Vec<&'static str> { … Some(NonNull) … }
    /// Columns requesting COUNT(DISTINCT col).
    pub fn distinct_fields_set(&self) -> Vec<&'static str> { … Some(Distinct) … }
}
```

`with_aggregate_args` / `with_group_by_args`:

```rust
if let Some(c) = args._count {
    if c.all_set() { self = self.count(); }
    for col in c.nonnull_fields_set() { self = self.count_column(col); }
    for col in c.distinct_fields_set() { self = self.count_distinct(col); }
}
```

(`GroupByOperation` currently has no `count_column` / `count_distinct`
builder — phase-6 collapsed `_count` to `count()` for group-by. Add
`count_column` and `count_distinct` builders to `GroupByOperation`
mirroring `AggregateOperation`, so grouped per-column / distinct counts
work too.)

### 5.3 `lower_agg_select` distinct parsing

In `lower_agg_select` for `AggKind::Count`, the per-column value can now
be either:
- `col: true` → `Some(CountSelectMode::NonNull)`
- `col: { distinct: true }` → `Some(CountSelectMode::Distinct)`
- `_all: true` → `_all = Some(true)` (unchanged; `_all: { distinct: true }`
  is an error: "`_all` has no distinct form; use COUNT(*) via `_all: true`")

For non-Count kinds (`_sum`/etc.), the value must remain `true` — a
`{ distinct: true }` there errors ("distinct is only valid inside
`_count`").

The emitted constructor for a Count select changes from
`__s.email = Some(true)` to `__s.email = Some(CountSelectMode::NonNull)`
/ `Some(CountSelectMode::Distinct)`.

## 6. Testing

| Layer | Location | What |
|-------|----------|------|
| Runtime hydration | `prax-query/src/operations/aggregate.rs::tests` | `from_row` populates `count_columns` + `count_distinct`; the `_count_distinct_` vs `_count_` prefix-ordering trap; `count_of` / `count_distinct_of` accessors |
| Runtime group-by builders | same | new `GroupByOperation::count_column` / `count_distinct`; `build_sql` emits `COUNT(DISTINCT col) AS _count_distinct_col` and `ORDER BY _sum_views DESC` |
| Codegen lowering | `prax-codegen/src/macros/lower/…::tests` | order-by alias mapping; distinct parsing; the validation errors |
| trybuild | `prax-codegen/tests/ui/…` | order-by unmatched aggregate; bare order-by col not in `by:`; distinct on `_all`; distinct value not `true`; distinct in a non-count block |
| e2e | `tests/aggregate_macros_e2e.rs` | `COUNT(DISTINCT …)` SQL; `group_by!` with `order_by:` emits `ORDER BY` |
| Live PG | `prax-postgres/tests/aggregate_macros.rs` | distinct-count round-trip (5 rows, 3 distinct values → 3); order-by-aggregate round-trip |

E2e + live tests continue to drive the runtime API where the
schema-path bug blocks full macro use, mirroring phase 6.

## 7. Compatibility

- `AggregateResult` new fields — additive (`Default`-derived).
- `<Model>CountSelect` field-type change (`Option<bool>` →
  `Option<CountSelectMode>`) — a breaking change to a type introduced
  one release ago, still under `[Unreleased]`. The macro is the primary
  constructor; hand-built `CountSelect` values (only in tests) update
  trivially.
- `GroupByOrderBy` placeholder reshaped — was unused (order_by rejected
  at macro time), so no real consumers.
- New `GroupByOperation::count_column` / `count_distinct` — additive.
- `CountSelectMode` — new public enum.

## 8. Open follow-ups (still deferred)

- Typed `<Model>CountSelectResult` hydration from `AggregateResult`
  (gated on the schema-path `relation_helpers` fix).
- MongoDB `$group`, CQL `GROUP BY`.
- Multi-column `_min`/`_max`.
- `include: { _count }`.
- Ordering a `group_by!` by a column that is neither in `by:` nor
  aggregated.
