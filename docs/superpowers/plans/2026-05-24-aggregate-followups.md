# Aggregate Macros Phase-6 Follow-ups Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Close the three phase-6 follow-up gaps in one PR — per-column count hydration in `AggregateResult`, `group_by!` `order_by:`, and the `count_distinct` macro shape.

**Architecture:** Gap 1 is a pure `prax-query` runtime fix (`AggregateResult::from_row` + new map fields). Gap 3 changes the codegen-emitted `<Model>CountSelect` field type to a new `prax-query` `CountSelectMode` enum, splits the `fields_set` accessor, adds `GroupByOperation::count_column`/`count_distinct` builders, and teaches `lower_agg_select` the `{ distinct: true }` form. Gap 2 reshapes the `<Model>GroupByOrderBy` holder, adds an order-by lowering helper that maps aggregate blocks to SELECT-alias `OrderByField`s, and wires `group_by!` + `with_group_by_args`.

**Tech Stack:** Rust 2024 workspace. Runtime in `prax-query/src/operations/aggregate.rs` + `prax-query/src/types.rs`. Codegen in `prax-codegen/src/generators/aggregate.rs` and `prax-codegen/src/macros/{lower,ops}/`. Tests via `cargo test`, trybuild, and the `RecordingEngine` e2e pattern.

**Spec:** `docs/superpowers/specs/2026-05-24-aggregate-followups-design.md`.

**Worktree:** `/home/joseph/Projects/prax/.worktrees/aggregate-followups/`, branch `feature/aggregate-followups`.

**Current runtime facts (verified):**
- `AggregateField::alias()` already emits `_count` / `_count_<col>` / `_count_distinct_<col>` / `_sum_<col>` / `_avg_<col>` / `_min_<col>` / `_max_<col>`.
- `AggregateOperation` has `count()`, `count_column(impl Into<String>)`, `count_distinct(impl Into<String>)`. `GroupByOperation` has only `count()` (no column/distinct variants yet).
- `with_aggregate_args` / `with_group_by_args` are emitted as **extension traits** (`AggregateOperationExt` / `GroupByOperationExt`) in `generators/aggregate.rs`. The group-by `_count` branch currently does `if c.all_set() || !c.fields_set().is_empty() { self = self.count(); }`.
- `OrderByField` (`prax-query/src/types.rs`) = `{ column: Cow<'static,str>, order: SortOrder, nulls: Option<NullsOrder> }`, constructor `OrderByField::new(col, SortOrder)`.

---

## Task 1: Baseline check

- [ ] **Step 1:** `cd /home/joseph/Projects/prax/.worktrees/aggregate-followups && git rev-parse --abbrev-ref HEAD` → `feature/aggregate-followups`; `git log --oneline -1` → `2761388 docs(query): design spec ...`.
- [ ] **Step 2:** `cargo test -p prax-query --lib operations::aggregate 2>&1 | tail -3` — green baseline.
- [ ] **Step 3:** No commit.

---

## Task 2: `AggregateResult` per-column count hydration (gap 1)

**Files:**
- Modify: `prax-query/src/operations/aggregate.rs`

- [ ] **Step 1: Add the failing test** in the `#[cfg(test)] mod tests` block of `aggregate.rs`:

```rust
#[test]
fn from_row_hydrates_per_column_and_distinct_counts() {
    use crate::filter::FilterValue;
    use std::collections::HashMap;
    let mut row = HashMap::new();
    row.insert("_count".to_string(), FilterValue::Int(5));
    row.insert("_count_email".to_string(), FilterValue::Int(3));
    row.insert("_count_distinct_email".to_string(), FilterValue::Int(2));
    let r = AggregateResult::from_row(row);
    assert_eq!(r.count, Some(5));
    assert_eq!(r.count_of("email"), Some(3));
    assert_eq!(r.count_distinct_of("email"), Some(2));
    // The distinct entry must NOT leak into count_columns under the
    // "email" key via the _count_ prefix (ordering trap).
    assert_eq!(r.count_columns.get("distinct_email"), None);
}
```

- [ ] **Step 2:** `cargo test -p prax-query --lib from_row_hydrates_per_column_and_distinct_counts` — FAILS (`count_of` / `count_distinct_of` don't exist).

- [ ] **Step 3: Add the fields** to `AggregateResult`:

```rust
    /// Per-column non-null counts, keyed by column (`COUNT(col)`).
    pub count_columns: std::collections::HashMap<String, i64>,
    /// Per-column distinct counts, keyed by column (`COUNT(DISTINCT col)`).
    pub count_distinct: std::collections::HashMap<String, i64>,
```

- [ ] **Step 4: Add `value_to_i64`** next to `value_to_f64`:

```rust
fn value_to_i64(v: &crate::filter::FilterValue) -> Option<i64> {
    use crate::filter::FilterValue;
    match v {
        FilterValue::Int(n) => Some(*n),
        FilterValue::String(s) => s.parse::<i64>().ok(),
        _ => None,
    }
}
```

- [ ] **Step 5: Extend `from_row`** — insert these arms **before** the `_sum_` arm and order distinct first:

```rust
            } else if let Some(col) = k.strip_prefix("_count_distinct_") {
                if let Some(n) = value_to_i64(&v) {
                    out.count_distinct.insert(col.to_string(), n);
                }
            } else if let Some(col) = k.strip_prefix("_count_") {
                if let Some(n) = value_to_i64(&v) {
                    out.count_columns.insert(col.to_string(), n);
                }
```

- [ ] **Step 6: Add accessors** to `impl AggregateResult`:

```rust
    /// Non-null count of a column (`COUNT(col)`), if present.
    pub fn count_of(&self, column: &str) -> Option<i64> {
        self.count_columns.get(column).copied()
    }
    /// Distinct count of a column (`COUNT(DISTINCT col)`), if present.
    pub fn count_distinct_of(&self, column: &str) -> Option<i64> {
        self.count_distinct.get(column).copied()
    }
```

- [ ] **Step 7:** `cargo test -p prax-query --lib operations::aggregate` — all green incl. the new test.

- [ ] **Step 8: Commit**

```
fix(query): hydrate per-column and distinct counts in AggregateResult
```

Body: notes the `_count_<col>` / `_count_distinct_<col>` aliases were emitted by `AggregateField::alias` but dropped by `from_row`; adds `count_columns`/`count_distinct` maps, `value_to_i64`, `count_of`/`count_distinct_of`; the `_count_distinct_` arm precedes `_count_` because the latter is a prefix of the former.

---

## Task 3: `GroupByOperation::count_column` / `count_distinct` builders (gap 3 runtime)

**Files:**
- Modify: `prax-query/src/operations/aggregate.rs`

- [ ] **Step 1: Add failing test** in `aggregate.rs::tests`:

```rust
#[test]
fn group_by_build_sql_emits_count_column_and_distinct() {
    use crate::dialect::Postgres;
    let op = GroupByOperation::<TestModel, NoEngine>::new(vec!["team_id".to_string()])
        .count_column("email")
        .count_distinct("region");
    let (sql, _) = op.build_sql(&Postgres);
    assert!(sql.contains("COUNT(\"email\") AS \"_count_email\""), "got: {sql}");
    assert!(
        sql.contains("COUNT(DISTINCT \"region\") AS \"_count_distinct_region\""),
        "got: {sql}"
    );
}
```

Use whatever the existing group-by tests use for the model + no-engine type (check the existing `GroupByOperation` tests in the same file for `TestModel` / `NoEngine` equivalents; reuse those names).

- [ ] **Step 2:** Run it — FAILS (`count_column`/`count_distinct` not on `GroupByOperation`).

- [ ] **Step 3: Add the builders** to `impl<M: Model, E: QueryEngine> GroupByOperation<M, E>` (right after the existing `count()` method ~line 489):

```rust
    /// Add a per-column non-null count (`COUNT(col)`).
    pub fn count_column(mut self, column: impl Into<String>) -> Self {
        self.agg_fields.push(AggregateField::CountColumn(column.into()));
        self
    }

    /// Add a distinct count (`COUNT(DISTINCT col)`).
    pub fn count_distinct(mut self, column: impl Into<String>) -> Self {
        self.agg_fields.push(AggregateField::CountDistinct(column.into()));
        self
    }
```

- [ ] **Step 4:** `cargo test -p prax-query --lib operations::aggregate` — green.

- [ ] **Step 5: Commit**

```
feat(query): GroupByOperation count_column and count_distinct builders
```

---

## Task 4: `CountSelectMode` enum + `<Model>CountSelect` field-type change (gap 3 codegen types)

**Files:**
- Modify: `prax-query/src/lib.rs` (or wherever public enums are re-exported) + a module to host `CountSelectMode`
- Modify: `prax-codegen/src/generators/aggregate.rs` (`emit_select_inputs`)

- [ ] **Step 1: Add `CountSelectMode`** to `prax-query`. Put it in `prax-query/src/operations/aggregate.rs` next to `AggregateField` (it is aggregate-related), and re-export from `prax-query/src/lib.rs` (check how `AggregateOperation` etc. are re-exported and mirror):

```rust
/// How a `_count` select column is counted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CountSelectMode {
    /// `COUNT(col)` — counts non-null values.
    NonNull,
    /// `COUNT(DISTINCT col)`.
    Distinct,
}
```

Re-export so codegen output can name it `::prax_query::CountSelectMode`. Verify the path: `grep -n "pub use.*AggregateOperation\|pub use operations" prax-query/src/lib.rs`.

- [ ] **Step 2: Change `emit_select_inputs`** in `generators/aggregate.rs` — the `<Model>CountSelect` per-column fields go from `Option<bool>` to `Option<::prax_query::CountSelectMode>`. Find the `count_fields` iterator (~line 90) and change:

```rust
    let count_fields = scalars.iter().map(|f| {
        let ident = f.ident;
        quote! { pub #ident: ::core::option::Option<::prax_query::CountSelectMode> }
    });
```

`_all` stays `Option<bool>`.

- [ ] **Step 3: Update the Task-2 codegen unit test** (`emit_select_inputs_count_includes_all_scalars`) in `aggregate.rs::tests` — its assertion that CountSelect has `_all` + columns still holds, but if it asserts the column field type was `Option<bool>`, update to `CountSelectMode`. Re-run `cargo test -p prax-codegen --lib generators::aggregate`.

- [ ] **Step 4:** `cargo build --workspace --all-features` — will surface every site that constructs a `<Model>CountSelect` with `Some(true)`. The `fields_set`/`with_*_args` emit (Task 5) and `lower_agg_select` (Task 6) are those sites; they're updated in the next tasks. If the build breaks ONLY in those two spots, that's expected — proceed to Task 5/6. If it breaks elsewhere (e.g. existing tests), fix those call sites to use `CountSelectMode::NonNull`.

- [ ] **Step 5: Commit** (may not fully build standalone — note in the body that Tasks 5-6 complete the change; if the repo requires every commit to build, fold Tasks 4-6 into one commit instead — decide based on whether `cargo build` is green here):

```
feat(query): CountSelectMode enum; <Model>CountSelect uses it per column
```

**Note:** if `cargo build --workspace` is red at this point because the generated `fields_set`/`lower_agg_select` still emit `Some(true)`, do NOT commit yet — continue to Tasks 5 and 6 and make a single combined commit at the end of Task 6 covering Tasks 4+5+6. The pre-commit hook runs clippy which requires a building workspace.

---

## Task 5: Split `fields_set` + route nonnull/distinct in extensions (gap 3 codegen wiring)

**Files:**
- Modify: `prax-codegen/src/generators/aggregate.rs` (`emit_accessors_and_extensions`)

- [ ] **Step 1:** In `emit_accessors_and_extensions`, replace the `CountSelect`'s single `fields_set()` with `nonnull_fields_set()` + `distinct_fields_set()` (keep `all_set()`):

```rust
        impl #count_select {
            pub fn all_set(&self) -> bool {
                matches!(self._all, ::core::option::Option::Some(true))
            }
            pub fn nonnull_fields_set(&self) -> ::std::vec::Vec<&'static str> {
                let mut out = ::std::vec::Vec::new();
                #(#count_nonnull_arms)*
                out
            }
            pub fn distinct_fields_set(&self) -> ::std::vec::Vec<&'static str> {
                let mut out = ::std::vec::Vec::new();
                #(#count_distinct_arms)*
                out
            }
        }
```

where the arms test the mode:

```rust
    let count_nonnull_arms = scalars.iter().map(|f| {
        let ident = f.ident; let col = f.column_name;
        quote! {
            if matches!(self.#ident, ::core::option::Option::Some(::prax_query::CountSelectMode::NonNull)) {
                out.push(#col);
            }
        }
    });
    let count_distinct_arms = scalars.iter().map(|f| {
        let ident = f.ident; let col = f.column_name;
        quote! {
            if matches!(self.#ident, ::core::option::Option::Some(::prax_query::CountSelectMode::Distinct)) {
                out.push(#col);
            }
        }
    });
```

- [ ] **Step 2:** Update the `_count` branches in both `with_aggregate_args` and `with_group_by_args` to:

```rust
                if let ::core::option::Option::Some(c) = args._count {
                    if c.all_set() { self = self.count(); }
                    for col in c.nonnull_fields_set() { self = self.count_column(col); }
                    for col in c.distinct_fields_set() { self = self.count_distinct(col); }
                }
```

This applies to BOTH the aggregate-ext and group-by-ext impls (group-by now has `count_column`/`count_distinct` from Task 3). Remove the old group-by `if c.all_set() || !c.fields_set().is_empty() { self = self.count(); }` collapse.

- [ ] **Step 3:** `cargo build --workspace --all-features` — the only remaining `Some(true)` emitter is `lower_agg_select` (Task 6).

- [ ] **Step 4:** No separate commit if folding per Task 4's note; otherwise commit:

```
feat(codegen): split CountSelect fields_set; route nonnull vs distinct
```

---

## Task 6: `lower_agg_select` distinct parsing (gap 3 macro DSL)

**Files:**
- Modify: `prax-codegen/src/macros/lower/aggregate_select.rs`

- [ ] **Step 1:** In `lower_agg_select`, the per-column value handling for `AggKind::Count` must accept `true` OR `{ distinct: true }`. Replace the current `if !matches!(value, DslValue::Bool(true))` gate so that:

For `_all`: only `true` allowed; `_all: { distinct: true }` errors ("`_all` has no distinct form; use COUNT(*) via `_all: true`").

For a normal column under `_count`:
- `DslValue::Bool(true)` → emit `__s.#col_ident = Some(::prax_query::CountSelectMode::NonNull);`
- `DslValue::Block(b)` where `b` is exactly `{ distinct: true }` → emit `__s.#col_ident = Some(::prax_query::CountSelectMode::Distinct);`
- anything else → error ("value for `_count.<col>` must be `true` or `{ distinct: true }`").

For non-Count kinds (`_sum`/`_avg`/`_min`/`_max`): value must be `true`; a `{ distinct: true }` errors ("`distinct` is only valid inside `_count`"). The setter stays `Some(true)` for those select structs (their fields are still `Option<bool>`).

Sketch for the Count column path:

```rust
// kind == Count, key_str is a real column:
let mode_ts = match value {
    DslValue::Bool(true) => quote! { ::prax_query::CountSelectMode::NonNull },
    DslValue::Block(b) if is_distinct_true_block(b) => quote! { ::prax_query::CountSelectMode::Distinct },
    _ => return Err(syn::Error::new(
        key.span(),
        format!("value for `_count.{}` must be `true` or `{{ distinct: true }}`", key_str),
    )),
};
setters.push(quote! { __s.#col_ident = ::core::option::Option::Some(#mode_ts); });
```

Add a small `is_distinct_true_block(&DslBlock) -> bool` helper: exactly one field, key `distinct`, value `DslValue::Bool(true)`. Anything else (`distinct: false`, extra keys, other keys) → false (caller emits the "must be true or { distinct: true }" error, OR add a more specific error for `distinct: false`).

Keep the existing `_all` and unknown-column / relation / aggregate-on-aggregate / non-numeric validation intact. The non-Count kinds keep emitting `Some(true)`.

- [ ] **Step 2: Update the Task-6 (phase-6) unit tests** in `aggregate_select.rs::tests` that asserted Count columns emit `Some(true)` — they now emit `Some(CountSelectMode::NonNull)`. Add new tests:

```rust
// (Build a LowerCtx with a model that has a String `email` column,
//  mirroring the existing aggregate_select tests.)
// - _count: { email: true }            → emits CountSelectMode :: NonNull
// - _count: { email: { distinct: true } } → emits CountSelectMode :: Distinct
// - _count: { _all: { distinct: true } }  → Err "_all has no distinct form"
// - _sum:   { views: { distinct: true } } → Err "distinct is only valid inside _count"
```

If the existing test harness builds a real `LowerCtx`, reuse it; otherwise assert on the emitted token string.

- [ ] **Step 3:** `cargo build --workspace --all-features` then `cargo test -p prax-codegen --lib` — green.

- [ ] **Step 4: Commit** (this is the combined Tasks 4+5+6 commit if you deferred per Task 4's note):

```
feat(codegen): count_distinct macro shape via _count: { col: { distinct: true } }
```

Body: `<Model>CountSelect` columns now `Option<CountSelectMode>`; `lower_agg_select` accepts `{ distinct: true }`; `fields_set` split into `nonnull_fields_set`/`distinct_fields_set`; extensions route to `count_column` / `count_distinct`; `_all` and non-count blocks reject distinct.

---

## Task 7: `<Model>GroupByOrderBy` reshape + order-by lowering helper (gap 2 part 1)

**Files:**
- Modify: `prax-codegen/src/generators/aggregate.rs` (the `<Model>GroupByOrderBy` emit in `emit_args_and_columns_enum`)
- Create: `prax-codegen/src/macros/lower/group_by_order_by.rs`
- Modify: `prax-codegen/src/macros/lower/mod.rs` — `pub(crate) mod group_by_order_by;`

- [ ] **Step 1: Reshape the holder.** In `emit_args_and_columns_enum`, change `<Model>GroupByOrderBy` from the `Vec<String>` placeholder to:

```rust
        #[derive(Debug, Default, Clone)]
        pub struct #order_by_ty {
            pub items: ::std::vec::Vec<::prax_query::types::OrderByField>,
        }
```

Verify the `OrderByField` path: `grep -n "pub use.*OrderByField\|pub mod types" prax-query/src/lib.rs`. Adjust to the actual public path (might be `::prax_query::OrderByField`).

- [ ] **Step 2: Create `group_by_order_by.rs`** with the lowering helper. It needs the set of `(AggKind, column)` pairs present in the call (to validate that an aggregate ordering has a matching block) and the set of `by:` column names (to validate bare-column ordering):

```rust
//! Lower `order_by: { _sum: { views: desc }, team_id: asc }` in a
//! group_by! call to a Vec<OrderByField> token stream. Aggregate
//! orderings reference the SELECT-list alias emitted by
//! AggregateField::alias (`_sum_views`, `_count`, `_count_<col>`, …);
//! bare-column orderings reference a `by:` column.

use proc_macro2::{Span, TokenStream};
use quote::quote;

use super::LowerCtx;
use crate::macros::dsl::ast::{DslBlock, DslField, DslValue};
use crate::macros::lower::aggregate_select::AggKind;

/// `(AggKind, Option<column>)` — column is None for `_count`'s `_all`.
pub struct AggPresence {
    pub kind: AggKind,
    pub column: Option<String>,
}

pub fn lower_group_by_order_by(
    block: &DslBlock,
    ctx: &LowerCtx<'_>,
    present_aggs: &[AggPresence],
    by_columns: &[String],
) -> syn::Result<TokenStream> {
    let mut items: Vec<TokenStream> = Vec::new();

    for entry in &block.fields {
        let DslField::Pair { key, value, .. } = entry else {
            return Err(syn::Error::new(Span::call_site(),
                "order_by block does not support spread or conditional fields"));
        };
        let key_str = key.to_string();
        let agg_kind = match key_str.as_str() {
            "_count" => Some(AggKind::Count),
            "_sum" => Some(AggKind::Sum),
            "_avg" => Some(AggKind::Avg),
            "_min" => Some(AggKind::Min),
            "_max" => Some(AggKind::Max),
            _ => None,
        };

        if let Some(kind) = agg_kind {
            // Inner: { col: dir } or { _all: dir } (count only)
            let DslValue::Block(inner) = value else {
                return Err(syn::Error::new(key.span(),
                    format!("order_by `{}` must be a `{{ col: asc|desc }}` block", key_str)));
            };
            for ie in &inner.fields {
                let DslField::Pair { key: ck, value: cv, .. } = ie else { continue; };
                let col = ck.to_string();
                let dir_ts = parse_dir(cv, ck.span())?;
                let alias = alias_for(kind, &col); // see helper below
                // Validate the aggregate is actually selected.
                let present = present_aggs.iter().any(|p| {
                    p.kind == kind && match (&p.column, col.as_str()) {
                        (None, "_all") => true,
                        (Some(c), other) => c == other,
                        _ => false,
                    }
                });
                if !present {
                    return Err(syn::Error::new(ck.span(), format!(
                        "order by `{}.{}` requires a matching `{}: {{ {} }}` block",
                        key_str, col, key_str, col)));
                }
                items.push(quote! {
                    ::prax_query::types::OrderByField::new(#alias, #dir_ts)
                });
            }
        } else {
            // Bare model column — must be in by:.
            let col = key_str;
            if !by_columns.iter().any(|c| c == &col) {
                return Err(syn::Error::new(key.span(), format!(
                    "order by `{}` requires `{}` in `by:`", col, col)));
            }
            let dir_ts = parse_dir(value, key.span())?;
            items.push(quote! {
                ::prax_query::types::OrderByField::new(#col, #dir_ts)
            });
        }
    }

    Ok(quote! {
        {
            let mut __ob: ::std::vec::Vec<::prax_query::types::OrderByField>
                = ::std::vec::Vec::new();
            #( __ob.push(#items); )*
            __ob
        }
    })
}

/// Compute the SELECT-list alias for an aggregate ordering, matching
/// AggregateField::alias.
fn alias_for(kind: AggKind, col: &str) -> String {
    match kind {
        AggKind::Count if col == "_all" => "_count".to_string(),
        AggKind::Count => format!("_count_{}", col),
        AggKind::Sum => format!("_sum_{}", col),
        AggKind::Avg => format!("_avg_{}", col),
        AggKind::Min => format!("_min_{}", col),
        AggKind::Max => format!("_max_{}", col),
    }
}

/// Parse a direction value (`asc` / `desc` bare ident) into a SortOrder token.
fn parse_dir(v: &DslValue, span: Span) -> syn::Result<TokenStream> {
    // Adapt to how the DSL represents bare idents (DslValue::BareIdent per
    // phase-6's group_by.rs). Reuse the existing order_by! direction parser
    // if one exists in lower/order_by_input.rs.
    let name = match v {
        DslValue::BareIdent(i) => i.to_string(),
        _ => return Err(syn::Error::new(span, "order direction must be `asc` or `desc`")),
    };
    match name.as_str() {
        "asc" => Ok(quote! { ::prax_query::types::SortOrder::Asc }),
        "desc" => Ok(quote! { ::prax_query::types::SortOrder::Desc }),
        other => Err(syn::Error::new(span, format!("unknown order direction `{}`; use asc or desc", other))),
    }
}
```

Adapt `DslValue::BareIdent`, the `SortOrder` path (`grep -n "pub enum SortOrder\|pub use.*SortOrder" prax-query/src/types.rs prax-query/src/lib.rs`), and reuse the existing direction parser from `lower/order_by_input.rs` if it already maps `asc`/`desc` (prefer reuse over the inline `parse_dir`).

- [ ] **Step 2: Register** `pub(crate) mod group_by_order_by;` in `lower/mod.rs`.

- [ ] **Step 3: Unit tests** in `group_by_order_by.rs::tests` (mirror how `having.rs` tests are structured — if a full LowerCtx is awkward, test `alias_for` + `parse_dir` directly):

```rust
#[test]
fn alias_for_matches_aggregate_field_alias() {
    assert_eq!(alias_for(AggKind::Count, "_all"), "_count");
    assert_eq!(alias_for(AggKind::Count, "email"), "_count_email");
    assert_eq!(alias_for(AggKind::Sum, "views"), "_sum_views");
    assert_eq!(alias_for(AggKind::Max, "created_at"), "_max_created_at");
}
```

- [ ] **Step 4:** `cargo build --workspace --all-features && cargo test -p prax-codegen --lib macros::lower::group_by_order_by` — green.

- [ ] **Step 5: Commit**

```
feat(codegen): group_by! order_by lowering helper + GroupByOrderBy holder
```

---

## Task 8: Wire `order_by:` into `group_by!` (gap 2 part 2)

**Files:**
- Modify: `prax-codegen/src/macros/ops/group_by.rs`

- [ ] **Step 1:** In `lower_group_by`, remove the `"order_by"` deferred-rejection arm. Replace with capturing the block: `"order_by" => order_by_block = Some(block_or_err(value)?),`.

- [ ] **Step 2:** Build the `present_aggs: Vec<AggPresence>` and `by_columns: Vec<String>` from the already-parsed `_count`/`_sum`/etc. blocks and the `by:` list. For each aggregate block present, walk its columns (and `_all` for count) to build `AggPresence` entries. (The blocks are parsed earlier in the fn for `lower_agg_select`; reuse that parse or re-walk the `DslBlock` fields for names.)

- [ ] **Step 3:** Lower the order-by block and populate the args:

```rust
let order_by_ts = match order_by_block {
    Some(ob) => {
        let items = crate::macros::lower::group_by_order_by::lower_group_by_order_by(
            ob, &ctx, &present_aggs, &by_columns,
        )?;
        let order_by_ty = format_ident!("{}GroupByOrderBy", ctx.model.name());
        quote! { ::core::option::Option::Some(#module_ident::#order_by_ty { items: #items }) }
    }
    None => quote! { ::core::option::Option::None },
};
```

And set `order_by: #order_by_ts,` in the `<Model>GroupByArgs` literal (replacing the hardcoded `order_by: ::core::option::Option::None,`).

- [ ] **Step 4:** Update `with_group_by_args` in `generators/aggregate.rs` — replace `let _ = args.order_by;` with:

```rust
                if let ::core::option::Option::Some(ob) = args.order_by {
                    for o in ob.items {
                        self = self.order_by(o);
                    }
                }
```

- [ ] **Step 5:** `cargo build --workspace --all-features` — green.

- [ ] **Step 6: Commit**

```
feat(codegen): group_by! order_by: support (removes phase-6 deferral)
```

---

## Task 9: trybuild diagnostic fixtures

**Files:**
- Create under `prax-codegen/tests/ui/` (mirror the existing phase-6 layout — single dir or `aggregate`/`group_by` subdirs as the harness expects):
  - `count_distinct_on_all_fail.rs` — `count!(c.user, { select: { _all: { distinct: true } } })` → "`_all` has no distinct form".
  - `count_distinct_bad_value_fail.rs` — `_count: { email: { distinct: false } }` → distinct-must-be-true / "must be `true` or `{ distinct: true }`".
  - `sum_distinct_fail.rs` — `aggregate!(c.user, { _sum: { views: { distinct: true } } })` → "distinct is only valid inside `_count`".
  - `group_by_order_by_unmatched_agg_fail.rs` — `group_by!(c.user, { by: [team_id], order_by: { _sum: { views: desc } } })` (no `_sum` block) → "requires a matching `_sum: { views }` block".
  - `group_by_order_by_bare_col_not_in_by_fail.rs` — `group_by!(c.user, { by: [team_id], _count: { _all: true }, order_by: { region: desc } })` → "requires `region` in `by:`".
- Modify the trybuild harness file to include the new fixtures (auto-included if it globs).

- [ ] **Step 1:** Create the fixtures. Use the same model-resolution approach the phase-6 fixtures use (they resolve `User` from `prax-codegen/tests/fixtures/schema.prax` via `PRAX_SCHEMA` — confirm the fixture schema has the columns each fixture needs: `email`, `team_id`, `region`, `views`; add them to the fixture schema if missing, but check first since phase 6 already used these).

- [ ] **Step 2:** `TRYBUILD=overwrite cargo test -p prax-codegen --test ui 2>&1 | tail`, inspect each generated `.stderr` for the intended message, then verify stable with `cargo test -p prax-codegen --test ui`.

- [ ] **Step 3: Commit**

```
test(codegen): trybuild fixtures for distinct + group_by order_by diagnostics
```

---

## Task 10: e2e + live PG tests

**Files:**
- Modify: `tests/aggregate_macros_e2e.rs`
- Modify: `prax-postgres/tests/aggregate_macros.rs`

- [ ] **Step 1: e2e** — add to `tests/aggregate_macros_e2e.rs` (runtime-API style, matching the existing tests there):

```rust
#[test]
fn distinct_count_emits_count_distinct_sql() {
    // AggregateOperation::with_engine(...).count_distinct("region")
    // build_sql(&Postgres) → contains COUNT(DISTINCT "region") AS "_count_distinct_region"
}

#[test]
fn group_by_order_by_emits_order_by_clause() {
    // GroupByOperation::...new(vec!["team_id"]).sum("views")
    //   .order_by(OrderByField::new("_sum_views", SortOrder::Desc))
    // build_sql → contains ORDER BY "_sum_views" DESC
}

#[test]
fn aggregate_result_hydrates_per_column_counts() {
    // Build an AggregateResult::from_row with _count_email + _count_distinct_email
    // and assert count_of / count_distinct_of (covers gap 1 at the e2e layer).
}
```

Inspect the actual SQL quoting/format first (write one, run, then pin the assertion) — phase-6 e2e found identifiers like `views`/`region` are not quoted but aliases are double-quoted; confirm for the count-distinct alias.

- [ ] **Step 2: live PG** — add to `prax-postgres/tests/aggregate_macros.rs` (`#[ignore]`-gated, runtime-API style):

```rust
#[tokio::test]
#[ignore = "requires postgres container or DATABASE_URL"]
async fn distinct_count_round_trip() {
    // CREATE TABLE t (id serial pk, region text);
    // INSERT regions: 'a','a','b','b','c'  (5 rows, 3 distinct)
    // aggregate count_column("region") + count_distinct("region")
    // assert count_of("region") == 5, count_distinct_of("region") == 3
}

#[tokio::test]
#[ignore = "requires postgres container or DATABASE_URL"]
async fn group_by_order_by_round_trip() {
    // CREATE TABLE t (team_id int, views int);
    // INSERT so team 1 sum=100, team 2 sum=300;
    // group_by(["team_id"]).sum("views").order_by(_sum_views desc)
    // assert first returned group is team 2.
}
```

- [ ] **Step 3:** `cargo test --test aggregate_macros_e2e` green; `cargo test -p prax-postgres --test aggregate_macros` compiles (tests skipped without live PG).

- [ ] **Step 4: Commit**

```
test: e2e + live coverage for per-column counts, distinct, group_by order_by
```

---

## Task 11: CHANGELOG + workspace sweep

**Files:**
- Modify: `CHANGELOG.md`

- [ ] **Step 1:** Under `[Unreleased]`, update the phase-6 known-limitations to reflect the closed gaps and add to Added/Fixed:

```
### Fixed
- `AggregateResult::from_row` now hydrates per-column non-null counts
  (`count_columns`) and distinct counts (`count_distinct`) from the
  `_count_<col>` / `_count_distinct_<col>` aliases (previously dropped).
  New `count_of(col)` / `count_distinct_of(col)` accessors.

### Added
- `count!` / `aggregate!` `_count` blocks accept
  `{ col: { distinct: true } }` for `COUNT(DISTINCT col)`. New
  `prax_query::CountSelectMode` enum; `<Model>CountSelect` columns are
  now `Option<CountSelectMode>`. `GroupByOperation` gains
  `count_column` / `count_distinct` builders.
- `group_by!` now supports `order_by: { _sum: { views: desc },
  <by_col>: asc }`, ordering by aggregate SELECT-list aliases or by
  group-by columns (removes the phase-6 macro-time rejection).
```

Remove the corresponding bullets from the phase-6 "Known limitations" block (per-column count hydration; group_by! order_by; count_distinct macro shape).

- [ ] **Step 2: Sweep:**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features --no-fail-fast
```

- [ ] **Step 3: Commit**

```
docs(query): CHANGELOG for aggregate follow-ups
```

---

## Task 12: Push + PR + auto-merge (controller-handled)

- [ ] `git push -u origin feature/aggregate-followups`
- [ ] `gh pr create --base develop --title "feat: aggregate macro follow-ups (per-column counts, distinct, group_by order_by)"`
- [ ] `gh pr merge <PR#> --squash --auto`

---

## Out of scope (still deferred)

- Typed `<Model>CountSelectResult` hydration from `AggregateResult` (gated on schema-path `relation_helpers` fix).
- MongoDB `$group`, CQL `GROUP BY`.
- Multi-column `_min`/`_max`.
- `include: { _count }`.
- Ordering a `group_by!` by a column neither in `by:` nor aggregated.
