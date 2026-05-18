# Typed Input Traits — Runtime Foundation (Phase 1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the runtime trait spine and hand-buildable input types that codegen and macros (phases 2–7) will lower onto. All new symbols live in `prax-query`; nothing else is touched. Existing fluent-builder API stays byte-for-byte unchanged.

**Architecture:**
- New module `prax-query/src/inputs/` holds the 10 input traits, every shared scalar/relation/update filter wrapper, and the per-operation `*Args` containers. Each trait has one method (`into_ir`) that lowers the input to existing runtime IR (`Filter` / `IncludeSpec` / `OrderBy` / `CreateData::Data` / `UpdateData::Data`).
- New module `prax-query/src/capabilities.rs` holds the empty marker traits engine crates will implement later (`SupportsRelationFilter`, `SupportsNestedWrites`, etc.).
- `Filter` gains one additive variant — `Filter::ScalarSubquery { sql, params }` — plus `#[non_exhaustive]`, so phase 5.5 (computed/virtual fields) and phase 5 (nested writes) can splice SQL fragments without further IR changes. The variant is dormant in phase 1: nothing in the workspace constructs it.
- Each existing `*Operation` in `prax-query/src/operations/` gains additive `with_*_input` extension methods. Old fluent setters (`r#where`, `.include(...)`, ...) remain. Both styles AND-compose for `where`.
- `QueryEngine` gains a default `in_transaction(&self) -> bool` so the nested-write executor can detect a wrapping transaction.

**Tech Stack:** Rust 2024, `serde` (the input types derive `Serialize`/`Deserialize` so option 4 from the brainstorm — typed-structs-with-serde — is also satisfied for free), `chrono`, `uuid`, `rust_decimal`, `serde_json` (already direct deps of `prax-query`).

---

## File Structure

### New files (all in `prax-query/`)

- `prax-query/src/inputs/mod.rs` — module root, re-exports
- `prax-query/src/inputs/traits.rs` — the 10 input traits
- `prax-query/src/inputs/scalar.rs` — `QueryMode` + `StringFilter`, `IntFilter`, etc. and their nullable variants
- `prax-query/src/inputs/scalar_update.rs` — `StringFieldUpdate`, `IntFieldUpdate`, etc. and nullable variants
- `prax-query/src/inputs/relation.rs` — `ListRelationFilter<W>`, `SingleRelationFilter<W>`
- `prax-query/src/inputs/args.rs` — `FindManyArgs`, `FindUniqueArgs`, `FindFirstArgs`, `CreateArgs`, `CreateManyArgs`, `UpdateArgs`, `UpdateManyArgs`, `DeleteArgs`, `DeleteManyArgs`, `UpsertArgs`, `CountArgs`, `AggregateArgs`, `GroupByArgs`
- `prax-query/src/capabilities.rs` — marker traits
- `prax-query/tests/inputs_scalar.rs` — unit tests for scalar filter lowering
- `prax-query/tests/inputs_relation.rs` — unit tests for relation filter lowering
- `prax-query/tests/inputs_update.rs` — unit tests for scalar update wrappers
- `prax-query/tests/inputs_args.rs` — unit tests for `*Args` construction + lowering
- `prax-query/tests/operation_ext_methods.rs` — unit tests for `with_*_input` on each operation

### Modified files

- `prax-query/src/lib.rs:14` — add `pub mod capabilities;` and `pub mod inputs;`; re-export the input traits/types in the same shape as the existing `Filter`/`FilterValue` re-exports
- `prax-query/src/filter.rs:551-602` — add `#[non_exhaustive]` to `Filter` and the `ScalarSubquery { sql: Cow<'static, str>, params: Vec<FilterValue> }` variant
- `prax-query/src/filter.rs:965-1115` — extend `to_sql_with_params` to lower `ScalarSubquery` (inline the sql with `{N}` → dialect placeholder substitution, append params)
- `prax-query/src/filter.rs` (after existing impls) — wildcard arm for `#[non_exhaustive]` in any internal `match` that currently is exhaustive
- `prax-query/src/traits.rs:108-264` — add `fn in_transaction(&self) -> bool { false }` to `QueryEngine`
- `prax-query/src/operations/find_many.rs` — `with_where_input`, `with_include_input`, `with_select_input`, `with_order_by_input`, `with_cursor_input`
- `prax-query/src/operations/find_unique.rs` — `with_where_input` (unique), `with_include_input`, `with_select_input`
- `prax-query/src/operations/find_first.rs` — `with_where_input`, `with_include_input`, `with_select_input`, `with_order_by_input`, `with_cursor_input`
- `prax-query/src/operations/create.rs` — `with_data_input`, `with_include_input`, `with_select_input`
- `prax-query/src/operations/update.rs` — `with_where_input` (unique), `with_data_input`, `with_include_input`, `with_select_input`
- `prax-query/src/operations/delete.rs` — `with_where_input` (unique), `with_include_input`, `with_select_input`
- `prax-query/src/operations/upsert.rs` — `with_where_input` (unique), `with_create_input`, `with_update_input`, `with_include_input`, `with_select_input`
- `prax-query/src/operations/count.rs` — `with_where_input`, `with_order_by_input`, `with_cursor_input`
- `prax-query/src/operations/aggregate.rs` — `with_where_input`, `with_order_by_input`, `with_aggregate_input`, `with_group_by_input`

### Deleted

- None.

---

## Task 1: Verify clean baseline

**Files:**
- None (verification only)

- [ ] **Step 1: Confirm worktree and branch**

Run: `git -C /home/joseph/Projects/prax/.worktrees/typed-inputs-runtime-traits rev-parse --abbrev-ref HEAD`
Expected: `feature/typed-inputs-runtime-traits`

- [ ] **Step 2: Workspace check**

Run: `cargo check --workspace --all-features` (from the worktree root)
Expected: zero compile errors. Any pre-existing failure must be fixed before continuing.

- [ ] **Step 3: prax-query unit tests**

Run: `cargo test -p prax-query --lib`
Expected: every test passes.

- [ ] **Step 4: No commit — verification only**

---

## Task 2: Add `Filter::ScalarSubquery` variant

**Files:**
- Modify: `prax-query/src/filter.rs:551-602`
- Modify: `prax-query/src/filter.rs:965-1115`

The codegen plans (phase 5.5 and phase 5) need to splice raw SQL fragments into WHERE clauses for relation-aggregate virtuals and nested-write child lookups. Adding the variant now keeps phase 1 self-contained as the IR foundation.

- [ ] **Step 1: Write the failing test**

Append inside `mod tests` at the bottom of `prax-query/src/filter.rs`:

```rust
    #[test]
    fn scalar_subquery_lowers_to_inline_sql_with_dialect_placeholders() {
        use crate::dialect::Postgres;
        let f = Filter::ScalarSubquery {
            sql: std::borrow::Cow::Borrowed(
                "(SELECT COUNT(*) FROM posts p WHERE p.author_id = users.id AND p.published = {0}) > {1}",
            ),
            params: vec![FilterValue::Bool(true), FilterValue::Int(5)],
        };
        let (sql, params) = f.to_sql(0, &Postgres);
        assert_eq!(
            sql,
            "((SELECT COUNT(*) FROM posts p WHERE p.author_id = users.id AND p.published = $1) > $2)"
        );
        assert_eq!(params, vec![FilterValue::Bool(true), FilterValue::Int(5)]);
    }

    #[test]
    fn scalar_subquery_offsets_placeholders_inside_and() {
        use crate::dialect::Postgres;
        let f = Filter::and([
            Filter::Equals("active".into(), FilterValue::Bool(true)),
            Filter::ScalarSubquery {
                sql: std::borrow::Cow::Borrowed(
                    "(SELECT COUNT(*) FROM posts p WHERE p.author_id = users.id) >= {0}",
                ),
                params: vec![FilterValue::Int(1)],
            },
        ]);
        let (sql, params) = f.to_sql(0, &Postgres);
        // First filter takes $1, the scalar subquery's {0} maps to $2.
        assert!(sql.contains("$1"));
        assert!(sql.contains("$2"));
        assert_eq!(params.len(), 2);
        assert_eq!(params[0], FilterValue::Bool(true));
        assert_eq!(params[1], FilterValue::Int(1));
    }
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p prax-query --lib filter::tests::scalar_subquery`
Expected: compile error — `Filter::ScalarSubquery` variant not found.

- [ ] **Step 3: Add the variant + `#[non_exhaustive]`**

In `prax-query/src/filter.rs`, replace the existing `Filter` definition (around line 551) with:

```rust
#[derive(Debug, Clone, PartialEq)]
#[repr(C)]
#[derive(Default)]
#[non_exhaustive]
pub enum Filter {
    /// No filter (always true).
    #[default]
    None,

    /// Equals comparison.
    Equals(FieldName, FilterValue),
    /// Not equals comparison.
    NotEquals(FieldName, FilterValue),

    /// Less than comparison.
    Lt(FieldName, FilterValue),
    /// Less than or equal comparison.
    Lte(FieldName, FilterValue),
    /// Greater than comparison.
    Gt(FieldName, FilterValue),
    /// Greater than or equal comparison.
    Gte(FieldName, FilterValue),

    /// In a list of values.
    In(FieldName, ValueList),
    /// Not in a list of values.
    NotIn(FieldName, ValueList),

    /// Contains (LIKE %value%).
    Contains(FieldName, FilterValue),
    /// Starts with (LIKE value%).
    StartsWith(FieldName, FilterValue),
    /// Ends with (LIKE %value).
    EndsWith(FieldName, FilterValue),

    /// Is null check.
    IsNull(FieldName),
    /// Is not null check.
    IsNotNull(FieldName),

    /// Logical AND of multiple filters.
    And(Box<[Filter]>),
    /// Logical OR of multiple filters.
    Or(Box<[Filter]>),
    /// Logical NOT of a filter.
    Not(Box<Filter>),

    /// A pre-built scalar subquery fragment.
    ///
    /// Used by relation-aggregate virtual fields and nested-write child
    /// lookups (phase 5 / 5.5). The `sql` string contains portable `{N}`
    /// placeholders that are substituted with dialect-specific placeholders
    /// at SQL build time; `N` is the zero-based index into `params`.
    ///
    /// Phase 1 introduces the variant for forward compatibility; nothing
    /// in the workspace constructs it yet.
    ScalarSubquery {
        /// SQL fragment with `{N}` placeholders.
        sql: std::borrow::Cow<'static, str>,
        /// Parameter values referenced by the `{N}` placeholders.
        params: Vec<FilterValue>,
    },
}
```

- [ ] **Step 4: Extend `to_sql_with_params` for the new variant**

In `prax-query/src/filter.rs::to_sql_with_params`, immediately before the closing `}` of the outer `match self {` (currently around line 1115), add:

```rust
            Self::ScalarSubquery { sql, params: inner_params } => {
                // Reserve global placeholder indices for each inner param.
                let base = param_idx + params.len();
                let mut out = String::with_capacity(sql.len() + inner_params.len() * 4);
                let mut chars = sql.chars().peekable();
                while let Some(ch) = chars.next() {
                    if ch == '{' {
                        // Read the integer index up to '}'.
                        let mut digits = String::new();
                        while let Some(&c) = chars.peek() {
                            if c == '}' {
                                chars.next();
                                break;
                            }
                            digits.push(c);
                            chars.next();
                        }
                        let n: usize = digits.parse().unwrap_or_else(|_| {
                            panic!("Filter::ScalarSubquery: invalid placeholder index `{{{}}}`", digits)
                        });
                        let value = inner_params.get(n).unwrap_or_else(|| {
                            panic!(
                                "Filter::ScalarSubquery: placeholder {{{}}} out of range (have {} params)",
                                n,
                                inner_params.len()
                            )
                        });
                        params.push(value.clone());
                        out.push_str(&dialect.placeholder(base + n + 1));
                    } else {
                        out.push(ch);
                    }
                }
                format!("({})", out)
            }
```

- [ ] **Step 5: Audit internal `match Filter` sites for the `#[non_exhaustive]` break**

Run: `cargo check -p prax-query --all-features 2>&1 | grep -E 'non-exhaustive|E0004' | head -20`

If any non-exhaustive-match warnings or errors come back, edit the call sites listed in the output and add `_ => unreachable!("unhandled Filter variant; see prax-query/src/inputs/lowering.rs")` arms. Re-run until clean.

- [ ] **Step 6: Run new tests**

Run: `cargo test -p prax-query --lib filter::tests::scalar_subquery`
Expected: both new tests pass.

- [ ] **Step 7: Commit**

```bash
git add prax-query/src/filter.rs
git commit -m "feat(query): add Filter::ScalarSubquery variant + non_exhaustive

Future phases (5 nested writes, 5.5 computed/virtual fields) need to
splice raw SQL fragments into WHERE clauses without enumerating every
shape in the IR. ScalarSubquery stores a SQL fragment with {N}
placeholders and a params vec; to_sql_with_params substitutes the
placeholders with dialect-correct positional placeholders at build
time. Filter is marked non_exhaustive so further additions stay
non-breaking."
```

---

## Task 3: Add `QueryEngine::in_transaction` default

**Files:**
- Modify: `prax-query/src/traits.rs:108-264`

The phase-5 nested-write executor inlines its ops into a wrapping transaction when one already exists. We add the detection hook now so phase 5 doesn't have to change the engine trait at the same time as its other work.

- [ ] **Step 1: Write the failing test**

Append at the end of the `mod tests` block in `prax-query/src/traits.rs`:

```rust
    #[test]
    fn default_in_transaction_returns_false() {
        // Re-use the DefaultEngine declared in the query_engine_dialect test below.
        // Construct it inline to avoid scope issues.
        #[derive(Clone)]
        struct E;

        impl QueryEngine for E {
            fn query_many<T: Model + crate::row::FromRow + Send + 'static>(
                &self,
                _sql: &str,
                _params: Vec<crate::filter::FilterValue>,
            ) -> BoxFuture<'_, QueryResult<Vec<T>>> { Box::pin(async { Ok(Vec::new()) }) }
            fn query_one<T: Model + crate::row::FromRow + Send + 'static>(
                &self,
                _sql: &str,
                _params: Vec<crate::filter::FilterValue>,
            ) -> BoxFuture<'_, QueryResult<T>> { Box::pin(async { Err(crate::error::QueryError::not_found("t")) }) }
            fn query_optional<T: Model + crate::row::FromRow + Send + 'static>(
                &self,
                _sql: &str,
                _params: Vec<crate::filter::FilterValue>,
            ) -> BoxFuture<'_, QueryResult<Option<T>>> { Box::pin(async { Ok(None) }) }
            fn execute_insert<T: Model + crate::row::FromRow + Send + 'static>(
                &self,
                _sql: &str,
                _params: Vec<crate::filter::FilterValue>,
            ) -> BoxFuture<'_, QueryResult<T>> { Box::pin(async { Err(crate::error::QueryError::not_found("t")) }) }
            fn execute_update<T: Model + crate::row::FromRow + Send + 'static>(
                &self,
                _sql: &str,
                _params: Vec<crate::filter::FilterValue>,
            ) -> BoxFuture<'_, QueryResult<Vec<T>>> { Box::pin(async { Ok(Vec::new()) }) }
            fn execute_delete(
                &self,
                _sql: &str,
                _params: Vec<crate::filter::FilterValue>,
            ) -> BoxFuture<'_, QueryResult<u64>> { Box::pin(async { Ok(0) }) }
            fn execute_raw(
                &self,
                _sql: &str,
                _params: Vec<crate::filter::FilterValue>,
            ) -> BoxFuture<'_, QueryResult<u64>> { Box::pin(async { Ok(0) }) }
            fn count(
                &self,
                _sql: &str,
                _params: Vec<crate::filter::FilterValue>,
            ) -> BoxFuture<'_, QueryResult<u64>> { Box::pin(async { Ok(0) }) }
        }

        let e = E;
        assert!(!e.in_transaction());
    }
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p prax-query --lib traits::tests::default_in_transaction_returns_false`
Expected: compile error — no method `in_transaction` on `QueryEngine`.

- [ ] **Step 3: Add the method to the trait**

In `prax-query/src/traits.rs` inside `pub trait QueryEngine`, immediately above `fn transaction<…>`, add:

```rust
    /// Whether this engine is currently executing inside an open
    /// transaction.
    ///
    /// The nested-write executor (phase 5) checks this to decide
    /// whether to issue its own `BEGIN`/`COMMIT` around a write plan
    /// or inline into a transaction the caller already started.
    ///
    /// Default returns `false`. Driver engines that wrap a
    /// driver-native transaction object override and return `true`.
    fn in_transaction(&self) -> bool {
        false
    }
```

- [ ] **Step 4: Run test**

Run: `cargo test -p prax-query --lib traits::tests::default_in_transaction_returns_false`
Expected: pass.

- [ ] **Step 5: Commit**

```bash
git add prax-query/src/traits.rs
git commit -m "feat(query): add QueryEngine::in_transaction default

Lets the nested-write executor (phase 5) detect whether it's running
inside a caller-started transaction and inline its child operations
instead of opening a new BEGIN/COMMIT. Default is false; driver
crates that own a real transaction object override."
```

---

## Task 4: Create `capabilities` module with marker traits

**Files:**
- Create: `prax-query/src/capabilities.rs`
- Modify: `prax-query/src/lib.rs` (add `pub mod capabilities;`)

- [ ] **Step 1: Create the file**

Create `prax-query/src/capabilities.rs`:

```rust
//! Engine capability marker traits.
//!
//! Each trait in this module marks a capability that some `QueryEngine`
//! impls satisfy and others don't. The macro DSL (phase 3+) and the
//! generated input types (phase 2) carry `where E: SupportsX` bounds on
//! the methods that produce capability-dependent SQL. Using such a
//! method against an engine that doesn't impl the trait fails to compile
//! with a clear diagnostic.
//!
//! Engine crates (`prax-postgres`, `prax-mysql`, ...) impl the traits
//! they satisfy on their concrete engine types. Phase 1 only defines
//! the traits; engine impls land in phase 2.

use crate::traits::QueryEngine;

/// Engine supports relation filters (`some`/`every`/`none`/`is`/`is_not`)
/// that lower to correlated EXISTS / NOT EXISTS subqueries (or the
/// equivalent in non-SQL engines).
#[diagnostic::on_unimplemented(
    message = "the engine `{Self}` does not support relation filters (`some` / `every` / `none` / `is` / `is_not`)",
    note = "ScyllaDB / Cassandra do not support correlated subqueries. Consider flattening the join or restructuring the model."
)]
pub trait SupportsRelationFilter: QueryEngine {}

/// Engine supports correlated subqueries in WHERE clauses.
///
/// Superset of `SupportsRelationFilter` — used by features that need
/// arbitrary subqueries (e.g. computed-field WHERE lowering in phase 5.5).
#[diagnostic::on_unimplemented(
    message = "the engine `{Self}` does not support correlated subqueries in WHERE clauses"
)]
pub trait SupportsCorrelatedSubquery: QueryEngine {}

/// Engine supports JSON-path filter operators (`path_eq`, `path_gt`, etc.).
#[diagnostic::on_unimplemented(
    message = "the engine `{Self}` does not support JSON path operators",
    note = "Postgres / MySQL >= 5.7 / SQLite + JSON1 / MSSQL support JSON paths."
)]
pub trait SupportsJsonPath: QueryEngine {}

/// Engine has native case-insensitive comparison (`ILIKE`, `COLLATE NOCASE`,
/// equivalent). Engines without it fall back to `LOWER(...)` comparisons and
/// **do not** need to impl this trait.
#[diagnostic::on_unimplemented(
    message = "the engine `{Self}` does not advertise native case-insensitive comparison"
)]
pub trait SupportsCaseInsensitiveMode: QueryEngine {}

/// Engine supports full-text search predicates.
#[diagnostic::on_unimplemented(
    message = "the engine `{Self}` does not support full-text search"
)]
pub trait SupportsFullTextSearch: QueryEngine {}

/// Engine supports native array column operators (`contains`, `overlaps`, ...).
#[diagnostic::on_unimplemented(
    message = "the engine `{Self}` does not support array operators"
)]
pub trait SupportsArrayOps: QueryEngine {}

/// Engine supports DDL for `GENERATED ALWAYS AS (expr) STORED|VIRTUAL`
/// computed columns.
#[diagnostic::on_unimplemented(
    message = "the engine `{Self}` does not support generated columns",
    note = "Postgres / MySQL / SQLite / MSSQL / DuckDB support GENERATED ALWAYS AS."
)]
pub trait SupportsGeneratedColumns: QueryEngine {}

/// Engine supports scalar subqueries in the SELECT list.
///
/// Required for relation-aggregate virtual fields (`@count`, `@sum`,
/// `@avg`, `@min`, `@max`) and Prisma-style `_count`.
#[diagnostic::on_unimplemented(
    message = "the engine `{Self}` does not support scalar subqueries in SELECT",
    note = "All SQL engines satisfy this. MongoDB requires the $lookup-lowering follow-up plan."
)]
pub trait SupportsScalarSubqueryInSelect: QueryEngine {}

/// Engine supports Prisma-style nested writes
/// (`create` / `connect` / `connect_or_create` / `disconnect` / `set`
/// / `update` / `upsert` / `delete` / `delete_many` inside `data`).
///
/// CQL engines (`prax-scylladb`, `prax-cassandra`) deliberately do not
/// impl this trait — phase 5's `*CreateNestedInput` / `*UpdateNestedInput`
/// types carry `where E: SupportsNestedWrites` bounds so misuse fails
/// to compile.
#[diagnostic::on_unimplemented(
    message = "the engine `{Self}` does not support nested writes",
    note = "ScyllaDB / Cassandra batch semantics don't map onto Prisma-style nested writes. Use the engine-native BATCH API or restructure."
)]
pub trait SupportsNestedWrites: QueryEngine {}

#[cfg(test)]
mod tests {
    use super::*;

    // A tiny stub engine for trait-impl smoke tests.
    #[derive(Clone)]
    struct StubEngine;

    impl QueryEngine for StubEngine {
        fn query_many<T: crate::traits::Model + crate::row::FromRow + Send + 'static>(
            &self,
            _sql: &str,
            _params: Vec<crate::filter::FilterValue>,
        ) -> crate::traits::BoxFuture<'_, crate::error::QueryResult<Vec<T>>> {
            Box::pin(async { Ok(Vec::new()) })
        }
        fn query_one<T: crate::traits::Model + crate::row::FromRow + Send + 'static>(
            &self,
            _sql: &str,
            _params: Vec<crate::filter::FilterValue>,
        ) -> crate::traits::BoxFuture<'_, crate::error::QueryResult<T>> {
            Box::pin(async { Err(crate::error::QueryError::not_found("t")) })
        }
        fn query_optional<T: crate::traits::Model + crate::row::FromRow + Send + 'static>(
            &self,
            _sql: &str,
            _params: Vec<crate::filter::FilterValue>,
        ) -> crate::traits::BoxFuture<'_, crate::error::QueryResult<Option<T>>> {
            Box::pin(async { Ok(None) })
        }
        fn execute_insert<T: crate::traits::Model + crate::row::FromRow + Send + 'static>(
            &self,
            _sql: &str,
            _params: Vec<crate::filter::FilterValue>,
        ) -> crate::traits::BoxFuture<'_, crate::error::QueryResult<T>> {
            Box::pin(async { Err(crate::error::QueryError::not_found("t")) })
        }
        fn execute_update<T: crate::traits::Model + crate::row::FromRow + Send + 'static>(
            &self,
            _sql: &str,
            _params: Vec<crate::filter::FilterValue>,
        ) -> crate::traits::BoxFuture<'_, crate::error::QueryResult<Vec<T>>> {
            Box::pin(async { Ok(Vec::new()) })
        }
        fn execute_delete(&self, _sql: &str, _params: Vec<crate::filter::FilterValue>) -> crate::traits::BoxFuture<'_, crate::error::QueryResult<u64>> {
            Box::pin(async { Ok(0) })
        }
        fn execute_raw(&self, _sql: &str, _params: Vec<crate::filter::FilterValue>) -> crate::traits::BoxFuture<'_, crate::error::QueryResult<u64>> {
            Box::pin(async { Ok(0) })
        }
        fn count(&self, _sql: &str, _params: Vec<crate::filter::FilterValue>) -> crate::traits::BoxFuture<'_, crate::error::QueryResult<u64>> {
            Box::pin(async { Ok(0) })
        }
    }

    impl SupportsRelationFilter for StubEngine {}

    fn needs_relation_filter<E: SupportsRelationFilter>() {}

    #[test]
    fn marker_trait_dispatch_compiles() {
        needs_relation_filter::<StubEngine>();
    }
}
```

- [ ] **Step 2: Register the module**

In `prax-query/src/lib.rs`, after the `pub mod dialect;` declaration block (search for an existing `pub mod` near the top), add:

```rust
pub mod capabilities;
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p prax-query --lib capabilities::tests::marker_trait_dispatch_compiles`
Expected: pass.

- [ ] **Step 4: Commit**

```bash
git add prax-query/src/capabilities.rs prax-query/src/lib.rs
git commit -m "feat(query): add engine capability marker traits

Nine marker traits gate features per engine: SupportsRelationFilter,
SupportsCorrelatedSubquery, SupportsJsonPath,
SupportsCaseInsensitiveMode, SupportsFullTextSearch,
SupportsArrayOps, SupportsGeneratedColumns,
SupportsScalarSubqueryInSelect, SupportsNestedWrites. Each carries a
#[diagnostic::on_unimplemented] message so misuse fails to compile
with engine-specific guidance. Engine crates impl the relevant
subset in phase 2."
```

---

## Task 5: Scaffold the `inputs` module and define the 10 input traits

**Files:**
- Create: `prax-query/src/inputs/mod.rs`
- Create: `prax-query/src/inputs/traits.rs`
- Modify: `prax-query/src/lib.rs`

- [ ] **Step 1: Create the module root**

Create `prax-query/src/inputs/mod.rs`:

```rust
//! Typed input shapes for the Prisma-style DSL.
//!
//! This module holds the trait spine (`WhereInput`, `IncludeInput`, …),
//! the reusable scalar filter wrappers (`StringFilter`, `IntFilter`, …),
//! the relation filter wrappers (`ListRelationFilter`,
//! `SingleRelationFilter`), the scalar update wrappers
//! (`IntFieldUpdate`, `StringFieldUpdate`, …), and the per-operation
//! containers (`FindManyArgs`, `CreateArgs`, …).
//!
//! Codegen (phase 2) emits per-model concrete structs that implement
//! these traits and use these wrappers. The operation macros (phase 3+)
//! emit token streams that construct these inputs and feed them to
//! existing `*Operation` builders via `with_*_input` extension methods.
//!
//! Layer-1 callers can also build these inputs by hand — they form the
//! "third interface" alongside the macro DSL and the existing fluent
//! builder.

pub mod args;
pub mod relation;
pub mod scalar;
pub mod scalar_update;
pub mod traits;

pub use args::*;
pub use relation::*;
pub use scalar::*;
pub use scalar_update::*;
pub use traits::*;
```

- [ ] **Step 2: Create the trait spine**

Create `prax-query/src/inputs/traits.rs`:

```rust
//! Traits implemented by per-model generated input types.
//!
//! Each trait has one method, `into_ir`, that lowers the input to the
//! runtime IR that the SQL builders already consume. The associated
//! `Model` type keeps generic bounds tight: a `FindManyOperation<E, User>`
//! can only accept a `WhereInput<Model = User>`, never a `PostWhereInput`.

use crate::filter::Filter;
use crate::pagination::Pagination;
use crate::relations::{Include, IncludeSpec};
use crate::traits::Model;
use crate::types::{OrderBy, Select};

/// A typed shape that lowers to a runtime [`Filter`].
///
/// Implemented by per-model `UserWhereInput`, `PostWhereInput`, ...
pub trait WhereInput {
    /// The model this WHERE shape applies to.
    type Model: Model;
    /// Lower this input to the runtime IR.
    fn into_ir(self) -> Filter;
}

/// A WHERE shape constrained to a unique key (PK or `@unique` column).
///
/// Used by `find_unique` / `update` / `upsert` / `delete` where the
/// operation requires the filter to identify at most one row.
pub trait WhereUniqueInput {
    /// The model this WHERE shape applies to.
    type Model: Model;
    /// Lower this input to the runtime IR.
    fn into_ir(self) -> Filter;
}

/// A typed shape that lowers to an [`Include`] specification.
pub trait IncludeInput {
    /// The model this include shape applies to.
    type Model: Model;
    /// Lower this input to the runtime IR.
    fn into_ir(self) -> Include;
}

/// A typed shape that lowers to a [`Select`] specification.
pub trait SelectInput {
    /// The model this select shape applies to.
    type Model: Model;
    /// Lower this input to the runtime IR.
    fn into_ir(self) -> Select;
}

/// A typed shape that lowers to an [`OrderBy`] specification.
pub trait OrderByInput {
    /// The model this order shape applies to.
    type Model: Model;
    /// Lower this input to the runtime IR.
    fn into_ir(self) -> OrderBy;
}

/// A typed shape that lowers to the `Data` payload for a `create`.
///
/// The associated `Data` type is the existing `<Model as CreateData>::Data`
/// from `prax_query::traits::CreateData` — phase 5 will introduce a
/// `NestedWritePlan` lowering path; phase 1 keeps the lowering simple.
pub trait CreateInput {
    /// The model this create input applies to.
    type Model: Model;
    /// The runtime payload type.
    type Data: Send + Sync;
    /// Lower this input to the runtime payload.
    fn into_ir(self) -> Self::Data;
}

/// A typed shape that lowers to the `Data` payload for an `update`.
pub trait UpdateInput {
    /// The model this update input applies to.
    type Model: Model;
    /// The runtime payload type.
    type Data: Send + Sync;
    /// Lower this input to the runtime payload.
    fn into_ir(self) -> Self::Data;
}

/// A typed shape that lowers to a `_count` aggregate selection.
pub trait CountSelect {
    /// The model this count selection applies to.
    type Model: Model;
    /// Concrete representation as a list of relation names to count.
    fn into_relation_names(self) -> Vec<String>;
}

/// A typed shape that lowers to an aggregate spec
/// (`_count` / `_avg` / `_sum` / `_min` / `_max`).
///
/// The IR target for this trait is finalized in phase 6 when aggregate
/// macros are wired up. For phase 1 the trait only carries the `Model`
/// associated type.
pub trait AggregateInput {
    /// The model this aggregate spec applies to.
    type Model: Model;
}

/// A typed shape that lowers to a group-by spec.
///
/// As with [`AggregateInput`], the IR target is finalized in phase 6.
pub trait GroupByInput {
    /// The model this group-by spec applies to.
    type Model: Model;
}

/// Pagination fragment shared by every read input.
///
/// Phase 1 keeps pagination on the operation itself (matching the
/// current builder API). This struct exists so phase 3+ macros can
/// surface `skip`/`take`/`cursor` inside the input AST without having
/// to construct an entire `*Args`.
#[derive(Debug, Clone, Default)]
pub struct PaginationInput {
    /// Number of rows to skip.
    pub skip: Option<u64>,
    /// Number of rows to take.
    pub take: Option<u64>,
}

impl From<PaginationInput> for Pagination {
    fn from(p: PaginationInput) -> Self {
        let mut out = Pagination::new();
        if let Some(n) = p.skip {
            out = out.skip(n);
        }
        if let Some(n) = p.take {
            out = out.take(n);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestModel;
    impl Model for TestModel {
        const MODEL_NAME: &'static str = "TestModel";
        const TABLE_NAME: &'static str = "test_models";
        const PRIMARY_KEY: &'static [&'static str] = &["id"];
        const COLUMNS: &'static [&'static str] = &["id"];
    }

    struct TestWhere;
    impl WhereInput for TestWhere {
        type Model = TestModel;
        fn into_ir(self) -> Filter {
            Filter::None
        }
    }

    #[test]
    fn where_input_lowers_to_filter_none() {
        assert!(matches!(TestWhere.into_ir(), Filter::None));
    }

    #[test]
    fn pagination_input_roundtrip() {
        let p = PaginationInput { skip: Some(5), take: Some(10) };
        let raw: Pagination = p.into();
        assert_eq!(raw.skip(), Some(5));
        assert_eq!(raw.take(), Some(10));
    }
}
```

- [ ] **Step 3: Register the module**

In `prax-query/src/lib.rs`, near the existing `pub mod capabilities;` you added in Task 4, add:

```rust
pub mod inputs;
```

- [ ] **Step 4: Build to check Pagination API**

Run: `cargo check -p prax-query --lib`

If the `Pagination::skip()` / `take()` getters are named differently, adapt the test. Find them with:

```bash
grep -n "pub fn skip\|pub fn take\|pub fn new" prax-query/src/pagination.rs
```

and update `pagination_input_roundtrip` accordingly. Re-run.

- [ ] **Step 5: Run tests**

Run: `cargo test -p prax-query --lib inputs::traits::tests`
Expected: both tests pass.

- [ ] **Step 6: Commit**

```bash
git add prax-query/src/inputs/ prax-query/src/lib.rs
git commit -m "feat(query): scaffold inputs module + ten input traits

Each trait (WhereInput, WhereUniqueInput, IncludeInput, SelectInput,
OrderByInput, CreateInput, UpdateInput, CountSelect, AggregateInput,
GroupByInput) has one method that lowers to existing runtime IR, plus
an associated Model type so per-operation generics stay tight.
AggregateInput / GroupByInput / CountSelect get their IR targets
finalized in phase 6; phase 1 only fixes their Model boundary.
PaginationInput rounds-trips into Pagination for phase 3+ macros."
```

---

## Task 6: Add `QueryMode` and the string-family scalar filters

**Files:**
- Create: `prax-query/src/inputs/scalar.rs`
- Create: `prax-query/tests/inputs_scalar.rs`

`scalar.rs` will hold every scalar filter wrapper. We add the string-family and `QueryMode` first so subsequent numeric/datetime tasks can follow the same template.

- [ ] **Step 1: Write the failing test**

Create `prax-query/tests/inputs_scalar.rs`:

```rust
use prax_query::filter::{Filter, FilterValue};
use prax_query::inputs::{QueryMode, ScalarFilter, StringFilter, StringNullableFilter};

#[test]
fn string_filter_equals_lowers_to_filter_equals() {
    let f = StringFilter::equals("alice@example.com");
    let filter = f.into_filter("email");
    assert_eq!(
        filter,
        Filter::Equals("email".into(), FilterValue::String("alice@example.com".into()))
    );
}

#[test]
fn string_filter_contains_lowers_to_filter_contains() {
    let f = StringFilter::contains("@example.com");
    let filter = f.into_filter("email");
    assert!(matches!(filter, Filter::Contains(_, _)));
}

#[test]
fn string_filter_combines_with_and_when_multiple_ops_set() {
    let f = StringFilter {
        contains: Some("@x.com".into()),
        starts_with: Some("a".into()),
        ..Default::default()
    };
    let filter = f.into_filter("email");
    match filter {
        Filter::And(parts) => assert_eq!(parts.len(), 2),
        other => panic!("expected Filter::And, got {:?}", other),
    }
}

#[test]
fn string_nullable_filter_is_null_lowers_to_is_null() {
    let f = StringNullableFilter { is_null: Some(true), ..Default::default() };
    let filter = f.into_filter("name");
    assert_eq!(filter, Filter::IsNull("name".into()));
}

#[test]
fn string_nullable_filter_is_not_null_lowers_to_is_not_null() {
    let f = StringNullableFilter { is_null: Some(false), ..Default::default() };
    let filter = f.into_filter("name");
    assert_eq!(filter, Filter::IsNotNull("name".into()));
}

#[test]
fn query_mode_default_is_default() {
    assert_eq!(QueryMode::default(), QueryMode::Default);
}

#[test]
fn string_filter_from_scalar_shortcut() {
    let f: StringFilter = "alice@x.com".into();
    assert_eq!(f.equals, Some("alice@x.com".into()));
}
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p prax-query --test inputs_scalar`
Expected: compile error — `StringFilter`, `StringNullableFilter`, `QueryMode`, `ScalarFilter` not found.

- [ ] **Step 3: Create `scalar.rs` with `QueryMode`, `ScalarFilter` helper trait, and the string filters**

Create `prax-query/src/inputs/scalar.rs`:

```rust
//! Reusable scalar filter wrappers shared by every generated `*WhereInput`.
//!
//! Each wrapper is a struct of `Option`-fields, one per operator. Empty
//! wrappers (all fields `None`) lower to `Filter::None`. Multiple set
//! fields AND-combine. `From<scalar>` impls support the macro shorthand
//! `email: "alice@x.com"` => `StringFilter { equals: Some("..."), .. }`.
//!
//! Every wrapper implements [`ScalarFilter`], whose `into_filter`
//! method takes the column name (which the parent `WhereInput` knows)
//! and produces a runtime [`Filter`].

use crate::filter::{Filter, FilterValue};
use serde::{Deserialize, Serialize};

/// Helper trait implemented by every scalar filter wrapper.
///
/// The wrapper itself doesn't know its column name — the parent
/// `WhereInput::into_ir` impl threads the column in when lowering.
pub trait ScalarFilter {
    /// Lower this scalar filter to a runtime [`Filter`] keyed by
    /// the given column name.
    fn into_filter(self, column: &str) -> Filter;
}

/// Comparison mode for string filters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum QueryMode {
    /// Default (case-sensitive) comparison.
    #[default]
    Default,
    /// Case-insensitive comparison. Requires `SupportsCaseInsensitiveMode`
    /// for engines that don't fall back to `LOWER(...)`.
    Insensitive,
}

/// Filter operators for a non-nullable `String` column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct StringFilter {
    /// `column = value`
    pub equals: Option<String>,
    /// Negation of the inner filter.
    pub not: Option<Box<StringFilter>>,
    /// `column IN (...)`
    pub in_list: Option<Vec<String>>,
    /// `column NOT IN (...)`
    pub not_in: Option<Vec<String>>,
    /// `column < value`
    pub lt: Option<String>,
    /// `column <= value`
    pub lte: Option<String>,
    /// `column > value`
    pub gt: Option<String>,
    /// `column >= value`
    pub gte: Option<String>,
    /// `column LIKE %value%`
    pub contains: Option<String>,
    /// `column LIKE value%`
    pub starts_with: Option<String>,
    /// `column LIKE %value`
    pub ends_with: Option<String>,
    /// Comparison mode (case sensitivity).
    pub mode: Option<QueryMode>,
}

impl StringFilter {
    /// `equals: Some(value)`.
    pub fn equals(v: impl Into<String>) -> Self {
        Self { equals: Some(v.into()), ..Default::default() }
    }
    /// `contains: Some(value)`.
    pub fn contains(v: impl Into<String>) -> Self {
        Self { contains: Some(v.into()), ..Default::default() }
    }
    /// `starts_with: Some(value)`.
    pub fn starts_with(v: impl Into<String>) -> Self {
        Self { starts_with: Some(v.into()), ..Default::default() }
    }
    /// `ends_with: Some(value)`.
    pub fn ends_with(v: impl Into<String>) -> Self {
        Self { ends_with: Some(v.into()), ..Default::default() }
    }
}

impl From<&str> for StringFilter {
    fn from(v: &str) -> Self { Self::equals(v) }
}
impl From<String> for StringFilter {
    fn from(v: String) -> Self { Self::equals(v) }
}

impl ScalarFilter for StringFilter {
    fn into_filter(self, column: &str) -> Filter {
        let mut parts: Vec<Filter> = Vec::new();
        let col = column.to_string();
        if let Some(v) = self.equals {
            parts.push(Filter::Equals(col.clone().into(), FilterValue::String(v)));
        }
        if let Some(boxed) = self.not {
            let inner = boxed.into_filter(column);
            parts.push(Filter::Not(Box::new(inner)));
        }
        if let Some(values) = self.in_list {
            let vs: Vec<FilterValue> = values.into_iter().map(FilterValue::String).collect();
            parts.push(Filter::In(col.clone().into(), vs));
        }
        if let Some(values) = self.not_in {
            let vs: Vec<FilterValue> = values.into_iter().map(FilterValue::String).collect();
            parts.push(Filter::NotIn(col.clone().into(), vs));
        }
        if let Some(v) = self.lt {
            parts.push(Filter::Lt(col.clone().into(), FilterValue::String(v)));
        }
        if let Some(v) = self.lte {
            parts.push(Filter::Lte(col.clone().into(), FilterValue::String(v)));
        }
        if let Some(v) = self.gt {
            parts.push(Filter::Gt(col.clone().into(), FilterValue::String(v)));
        }
        if let Some(v) = self.gte {
            parts.push(Filter::Gte(col.clone().into(), FilterValue::String(v)));
        }
        if let Some(v) = self.contains {
            parts.push(Filter::Contains(col.clone().into(), FilterValue::String(v)));
        }
        if let Some(v) = self.starts_with {
            parts.push(Filter::StartsWith(col.clone().into(), FilterValue::String(v)));
        }
        if let Some(v) = self.ends_with {
            parts.push(Filter::EndsWith(col.clone().into(), FilterValue::String(v)));
        }
        // `mode` is honored by the dialect layer in phase 2+; phase 1 ignores
        // it here. The field is kept so downstream phases don't need a
        // breaking-shape change.
        let _ = self.mode;
        match parts.len() {
            0 => Filter::None,
            1 => parts.into_iter().next().unwrap(),
            _ => Filter::and(parts),
        }
    }
}

/// Filter operators for a nullable `String` column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct StringNullableFilter {
    /// `column = value`
    pub equals: Option<String>,
    /// Negation of the inner filter.
    pub not: Option<Box<StringNullableFilter>>,
    /// `column IN (...)`
    pub in_list: Option<Vec<String>>,
    /// `column NOT IN (...)`
    pub not_in: Option<Vec<String>>,
    /// `column < value`
    pub lt: Option<String>,
    /// `column <= value`
    pub lte: Option<String>,
    /// `column > value`
    pub gt: Option<String>,
    /// `column >= value`
    pub gte: Option<String>,
    /// `column LIKE %value%`
    pub contains: Option<String>,
    /// `column LIKE value%`
    pub starts_with: Option<String>,
    /// `column LIKE %value`
    pub ends_with: Option<String>,
    /// Comparison mode.
    pub mode: Option<QueryMode>,
    /// `is_null: Some(true)` ⇒ `IS NULL`; `Some(false)` ⇒ `IS NOT NULL`.
    pub is_null: Option<bool>,
}

impl From<&str> for StringNullableFilter {
    fn from(v: &str) -> Self {
        Self { equals: Some(v.into()), ..Default::default() }
    }
}
impl From<String> for StringNullableFilter {
    fn from(v: String) -> Self {
        Self { equals: Some(v), ..Default::default() }
    }
}

impl ScalarFilter for StringNullableFilter {
    fn into_filter(self, column: &str) -> Filter {
        let mut parts: Vec<Filter> = Vec::new();
        let col = column.to_string();
        if let Some(b) = self.is_null {
            parts.push(if b {
                Filter::IsNull(col.clone().into())
            } else {
                Filter::IsNotNull(col.clone().into())
            });
        }
        // Reuse StringFilter's lowering for the remaining ops.
        let inner = StringFilter {
            equals: self.equals,
            not: self.not.map(|b| Box::new(StringFilter {
                equals: b.equals,
                in_list: b.in_list,
                not_in: b.not_in,
                lt: b.lt,
                lte: b.lte,
                gt: b.gt,
                gte: b.gte,
                contains: b.contains,
                starts_with: b.starts_with,
                ends_with: b.ends_with,
                mode: b.mode,
                not: None,
            })),
            in_list: self.in_list,
            not_in: self.not_in,
            lt: self.lt,
            lte: self.lte,
            gt: self.gt,
            gte: self.gte,
            contains: self.contains,
            starts_with: self.starts_with,
            ends_with: self.ends_with,
            mode: self.mode,
        };
        let inner_filter = inner.into_filter(column);
        if !matches!(inner_filter, Filter::None) {
            parts.push(inner_filter);
        }
        match parts.len() {
            0 => Filter::None,
            1 => parts.into_iter().next().unwrap(),
            _ => Filter::and(parts),
        }
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p prax-query --test inputs_scalar`
Expected: every test passes.

- [ ] **Step 5: Commit**

```bash
git add prax-query/src/inputs/scalar.rs prax-query/tests/inputs_scalar.rs
git commit -m "feat(query): add QueryMode and StringFilter / StringNullableFilter

ScalarFilter helper trait takes a column name and produces a runtime
Filter; per-model WhereInput impls thread the column in when
lowering. Multiple Option fields AND-combine. From<&str> / From<String>
shortcuts power the macro DSL's bare-value shorthand. mode is parsed
but lowered in phase 2 by the dialect layer."
```

---

## Task 7: Add numeric, boolean, bytes, uuid, json filters

**Files:**
- Modify: `prax-query/src/inputs/scalar.rs`
- Modify: `prax-query/tests/inputs_scalar.rs`

- [ ] **Step 1: Append failing tests**

Append at the bottom of `prax-query/tests/inputs_scalar.rs`:

```rust
use prax_query::inputs::{
    BigIntFilter, BigIntNullableFilter, BoolFilter, BoolNullableFilter, BytesFilter,
    BytesNullableFilter, DecimalFilter, DecimalNullableFilter, FloatFilter,
    FloatNullableFilter, IntFilter, IntNullableFilter, JsonFilter, JsonNullableFilter,
    UuidFilter, UuidNullableFilter,
};

#[test]
fn int_filter_equals_lowers() {
    let f = IntFilter::equals(42i32);
    let filter = f.into_filter("age");
    assert_eq!(filter, Filter::Equals("age".into(), FilterValue::Int(42)));
}

#[test]
fn int_filter_gt_lowers() {
    let f = IntFilter::gt(18i32);
    let filter = f.into_filter("age");
    assert_eq!(filter, Filter::Gt("age".into(), FilterValue::Int(18)));
}

#[test]
fn int_filter_in_list_lowers() {
    let f = IntFilter { in_list: Some(vec![1, 2, 3]), ..Default::default() };
    let filter = f.into_filter("id");
    match filter {
        Filter::In(col, values) => {
            assert_eq!(col, "id");
            assert_eq!(values.len(), 3);
        }
        other => panic!("expected Filter::In, got {:?}", other),
    }
}

#[test]
fn int_nullable_filter_is_null() {
    let f = IntNullableFilter { is_null: Some(true), ..Default::default() };
    let filter = f.into_filter("deleted_at");
    assert_eq!(filter, Filter::IsNull("deleted_at".into()));
}

#[test]
fn bool_filter_equals_lowers() {
    let f = BoolFilter::equals(true);
    let filter = f.into_filter("active");
    assert_eq!(filter, Filter::Equals("active".into(), FilterValue::Bool(true)));
}

#[test]
fn big_int_filter_equals_lowers() {
    let f = BigIntFilter::equals(9_999_999_999i64);
    let filter = f.into_filter("counter");
    assert_eq!(filter, Filter::Equals("counter".into(), FilterValue::Int(9_999_999_999)));
}

#[test]
fn float_filter_equals_lowers() {
    let f = FloatFilter::equals(3.14f64);
    let filter = f.into_filter("score");
    assert_eq!(filter, Filter::Equals("score".into(), FilterValue::Float(3.14)));
}

#[test]
fn decimal_filter_equals_lowers() {
    use rust_decimal::Decimal;
    use std::str::FromStr;
    let v = Decimal::from_str("12.34").unwrap();
    let f = DecimalFilter::equals(v);
    let filter = f.into_filter("amount");
    // Decimal flows through as a string in FilterValue::String — see lowering.
    match filter {
        Filter::Equals(col, FilterValue::String(s)) => {
            assert_eq!(col, "amount");
            assert_eq!(s, "12.34");
        }
        other => panic!("expected Decimal Equals to string, got {:?}", other),
    }
}

#[test]
fn uuid_filter_equals_lowers() {
    use uuid::Uuid;
    let id = Uuid::nil();
    let f = UuidFilter::equals(id);
    let filter = f.into_filter("id");
    match filter {
        Filter::Equals(col, FilterValue::String(s)) => {
            assert_eq!(col, "id");
            assert_eq!(s, id.to_string());
        }
        other => panic!("expected Uuid Equals to string, got {:?}", other),
    }
}

#[test]
fn json_filter_equals_lowers() {
    let f = JsonFilter { equals: Some(serde_json::json!({"k": 1})), ..Default::default() };
    let filter = f.into_filter("data");
    match filter {
        Filter::Equals(col, FilterValue::Json(v)) => {
            assert_eq!(col, "data");
            assert_eq!(v, serde_json::json!({"k": 1}));
        }
        other => panic!("expected Json Equals, got {:?}", other),
    }
}

#[test]
fn bytes_filter_equals_lowers() {
    let f = BytesFilter::equals(vec![1u8, 2, 3]);
    let filter = f.into_filter("blob");
    match filter {
        Filter::Equals(col, FilterValue::String(s)) => {
            assert_eq!(col, "blob");
            assert!(!s.is_empty());
        }
        other => panic!("expected Bytes Equals (base64-encoded), got {:?}", other),
    }
}
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p prax-query --test inputs_scalar`
Expected: compile error — none of the new filters exist yet.

- [ ] **Step 3: Append the numeric/bool/bytes/uuid/json filter definitions**

Append to `prax-query/src/inputs/scalar.rs`:

```rust
/// Macro to emit a scalar filter wrapper + nullable counterpart that
/// lowers to a `FilterValue::$variant`. Keeps the table of integer /
/// floating / temporal / blob types DRY without sacrificing rustdoc
/// per-type.
macro_rules! scalar_filter {
    (
        $(#[$nn_meta:meta])*
        $name:ident<$rust:ty> => |$conv_v:ident: $rust2:ty| $conv:block as $fv:expr,
        $(#[$null_meta:meta])*
        nullable $null:ident
    ) => {
        $(#[$nn_meta])*
        #[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub struct $name {
            /// `column = value`
            pub equals: Option<$rust>,
            /// Negation.
            pub not: Option<Box<$name>>,
            /// `column IN (...)`
            pub in_list: Option<Vec<$rust>>,
            /// `column NOT IN (...)`
            pub not_in: Option<Vec<$rust>>,
            /// `column < value`
            pub lt: Option<$rust>,
            /// `column <= value`
            pub lte: Option<$rust>,
            /// `column > value`
            pub gt: Option<$rust>,
            /// `column >= value`
            pub gte: Option<$rust>,
        }

        impl $name {
            /// `equals: Some(value)`.
            pub fn equals(v: impl Into<$rust>) -> Self {
                Self { equals: Some(v.into()), ..Default::default() }
            }
            /// `lt: Some(value)`.
            pub fn lt(v: impl Into<$rust>) -> Self {
                Self { lt: Some(v.into()), ..Default::default() }
            }
            /// `lte: Some(value)`.
            pub fn lte(v: impl Into<$rust>) -> Self {
                Self { lte: Some(v.into()), ..Default::default() }
            }
            /// `gt: Some(value)`.
            pub fn gt(v: impl Into<$rust>) -> Self {
                Self { gt: Some(v.into()), ..Default::default() }
            }
            /// `gte: Some(value)`.
            pub fn gte(v: impl Into<$rust>) -> Self {
                Self { gte: Some(v.into()), ..Default::default() }
            }
        }

        impl ScalarFilter for $name {
            fn into_filter(self, column: &str) -> Filter {
                fn to_fv($conv_v: $rust2) -> FilterValue $conv
                let col: crate::filter::FieldName = column.to_string().into();
                let mut parts: Vec<Filter> = Vec::new();
                if let Some(v) = self.equals {
                    parts.push(Filter::Equals(col.clone(), to_fv(v)));
                }
                if let Some(boxed) = self.not {
                    let inner = boxed.into_filter(column);
                    parts.push(Filter::Not(Box::new(inner)));
                }
                if let Some(values) = self.in_list {
                    let vs: Vec<FilterValue> = values.into_iter().map(to_fv).collect();
                    parts.push(Filter::In(col.clone(), vs));
                }
                if let Some(values) = self.not_in {
                    let vs: Vec<FilterValue> = values.into_iter().map(to_fv).collect();
                    parts.push(Filter::NotIn(col.clone(), vs));
                }
                if let Some(v) = self.lt { parts.push(Filter::Lt(col.clone(), to_fv(v))); }
                if let Some(v) = self.lte { parts.push(Filter::Lte(col.clone(), to_fv(v))); }
                if let Some(v) = self.gt { parts.push(Filter::Gt(col.clone(), to_fv(v))); }
                if let Some(v) = self.gte { parts.push(Filter::Gte(col, to_fv(v))); }
                match parts.len() {
                    0 => Filter::None,
                    1 => parts.into_iter().next().unwrap(),
                    _ => Filter::and(parts),
                }
            }
        }

        $(#[$null_meta])*
        #[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub struct $null {
            /// `column = value`
            pub equals: Option<$rust>,
            /// Negation.
            pub not: Option<Box<$null>>,
            /// `column IN (...)`
            pub in_list: Option<Vec<$rust>>,
            /// `column NOT IN (...)`
            pub not_in: Option<Vec<$rust>>,
            /// `column < value`
            pub lt: Option<$rust>,
            /// `column <= value`
            pub lte: Option<$rust>,
            /// `column > value`
            pub gt: Option<$rust>,
            /// `column >= value`
            pub gte: Option<$rust>,
            /// IS NULL / IS NOT NULL.
            pub is_null: Option<bool>,
        }

        impl ScalarFilter for $null {
            fn into_filter(self, column: &str) -> Filter {
                let mut parts: Vec<Filter> = Vec::new();
                if let Some(b) = self.is_null {
                    parts.push(if b {
                        Filter::IsNull(column.to_string().into())
                    } else {
                        Filter::IsNotNull(column.to_string().into())
                    });
                }
                let inner = $name {
                    equals: self.equals,
                    not: self.not.map(|b| Box::new($name {
                        equals: b.equals,
                        in_list: b.in_list,
                        not_in: b.not_in,
                        lt: b.lt, lte: b.lte, gt: b.gt, gte: b.gte,
                        not: None,
                    })),
                    in_list: self.in_list,
                    not_in: self.not_in,
                    lt: self.lt, lte: self.lte, gt: self.gt, gte: self.gte,
                };
                let f = inner.into_filter(column);
                if !matches!(f, Filter::None) { parts.push(f); }
                match parts.len() {
                    0 => Filter::None,
                    1 => parts.into_iter().next().unwrap(),
                    _ => Filter::and(parts),
                }
            }
        }
    };
}

scalar_filter!(
    /// Filter for non-nullable `Int` (`i32`) columns.
    IntFilter<i32> => |v: i32| { FilterValue::Int(v as i64) } as FilterValue::Int,
    /// Filter for nullable `Int` columns.
    nullable IntNullableFilter
);

scalar_filter!(
    /// Filter for non-nullable `BigInt` (`i64`) columns.
    BigIntFilter<i64> => |v: i64| { FilterValue::Int(v) } as FilterValue::Int,
    /// Filter for nullable `BigInt` columns.
    nullable BigIntNullableFilter
);

scalar_filter!(
    /// Filter for non-nullable `Float` (`f64`) columns.
    FloatFilter<f64> => |v: f64| { FilterValue::Float(v) } as FilterValue::Float,
    /// Filter for nullable `Float` columns.
    nullable FloatNullableFilter
);

scalar_filter!(
    /// Filter for non-nullable `Decimal` (`rust_decimal::Decimal`) columns.
    ///
    /// Lowered as `FilterValue::String` because the runtime IR does not
    /// have a dedicated `Decimal` variant; the driver layer parses it on
    /// the wire.
    DecimalFilter<rust_decimal::Decimal> => |v: rust_decimal::Decimal| { FilterValue::String(v.to_string()) } as FilterValue::String,
    /// Filter for nullable `Decimal` columns.
    nullable DecimalNullableFilter
);

scalar_filter!(
    /// Filter for non-nullable `Uuid` columns.
    UuidFilter<uuid::Uuid> => |v: uuid::Uuid| { FilterValue::String(v.to_string()) } as FilterValue::String,
    /// Filter for nullable `Uuid` columns.
    nullable UuidNullableFilter
);

scalar_filter!(
    /// Filter for non-nullable `Bytes` (`Vec<u8>`) columns.
    ///
    /// Encoded as a base64-of-bytes string in FilterValue::String. The
    /// driver layer decodes back to bytes on the wire.
    BytesFilter<Vec<u8>> => |v: Vec<u8>| {
        use base64::Engine as _;
        FilterValue::String(base64::engine::general_purpose::STANDARD.encode(&v))
    } as FilterValue::String,
    /// Filter for nullable `Bytes` columns.
    nullable BytesNullableFilter
);

/// Filter operators for a non-nullable `Boolean` column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BoolFilter {
    /// `column = value`
    pub equals: Option<bool>,
    /// Negation of the inner filter.
    pub not: Option<Box<BoolFilter>>,
}

impl BoolFilter {
    /// `equals: Some(value)`.
    pub fn equals(v: bool) -> Self { Self { equals: Some(v), ..Default::default() } }
}

impl From<bool> for BoolFilter {
    fn from(v: bool) -> Self { Self::equals(v) }
}

impl ScalarFilter for BoolFilter {
    fn into_filter(self, column: &str) -> Filter {
        let col: crate::filter::FieldName = column.to_string().into();
        let mut parts: Vec<Filter> = Vec::new();
        if let Some(v) = self.equals {
            parts.push(Filter::Equals(col.clone(), FilterValue::Bool(v)));
        }
        if let Some(boxed) = self.not {
            parts.push(Filter::Not(Box::new(boxed.into_filter(column))));
        }
        match parts.len() {
            0 => Filter::None,
            1 => parts.into_iter().next().unwrap(),
            _ => Filter::and(parts),
        }
    }
}

/// Filter operators for a nullable `Boolean` column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BoolNullableFilter {
    /// `column = value`
    pub equals: Option<bool>,
    /// Negation.
    pub not: Option<Box<BoolNullableFilter>>,
    /// IS NULL / IS NOT NULL.
    pub is_null: Option<bool>,
}

impl ScalarFilter for BoolNullableFilter {
    fn into_filter(self, column: &str) -> Filter {
        let mut parts: Vec<Filter> = Vec::new();
        if let Some(b) = self.is_null {
            parts.push(if b {
                Filter::IsNull(column.to_string().into())
            } else {
                Filter::IsNotNull(column.to_string().into())
            });
        }
        let inner = BoolFilter {
            equals: self.equals,
            not: self.not.map(|b| Box::new(BoolFilter { equals: b.equals, not: None })),
        };
        let f = inner.into_filter(column);
        if !matches!(f, Filter::None) { parts.push(f); }
        match parts.len() {
            0 => Filter::None,
            1 => parts.into_iter().next().unwrap(),
            _ => Filter::and(parts),
        }
    }
}

/// Filter operators for a non-nullable `Json` column.
///
/// Phase 1 supports `equals`/`not`. JSON-path operators land behind
/// `SupportsJsonPath` in a follow-up.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct JsonFilter {
    /// `column = value`
    pub equals: Option<serde_json::Value>,
    /// Negation.
    pub not: Option<Box<JsonFilter>>,
}

impl ScalarFilter for JsonFilter {
    fn into_filter(self, column: &str) -> Filter {
        let col: crate::filter::FieldName = column.to_string().into();
        let mut parts: Vec<Filter> = Vec::new();
        if let Some(v) = self.equals {
            parts.push(Filter::Equals(col.clone(), FilterValue::Json(v)));
        }
        if let Some(boxed) = self.not {
            parts.push(Filter::Not(Box::new(boxed.into_filter(column))));
        }
        match parts.len() {
            0 => Filter::None,
            1 => parts.into_iter().next().unwrap(),
            _ => Filter::and(parts),
        }
    }
}

/// Filter operators for a nullable `Json` column.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct JsonNullableFilter {
    /// `column = value`
    pub equals: Option<serde_json::Value>,
    /// Negation.
    pub not: Option<Box<JsonNullableFilter>>,
    /// IS NULL / IS NOT NULL.
    pub is_null: Option<bool>,
}

impl ScalarFilter for JsonNullableFilter {
    fn into_filter(self, column: &str) -> Filter {
        let mut parts: Vec<Filter> = Vec::new();
        if let Some(b) = self.is_null {
            parts.push(if b {
                Filter::IsNull(column.to_string().into())
            } else {
                Filter::IsNotNull(column.to_string().into())
            });
        }
        let inner = JsonFilter {
            equals: self.equals,
            not: self.not.map(|b| Box::new(JsonFilter { equals: b.equals, not: None })),
        };
        let f = inner.into_filter(column);
        if !matches!(f, Filter::None) { parts.push(f); }
        match parts.len() {
            0 => Filter::None,
            1 => parts.into_iter().next().unwrap(),
            _ => Filter::and(parts),
        }
    }
}
```

- [ ] **Step 4: Add `base64` to `prax-query` deps**

The bytes filter uses `base64`. Edit `prax-query/Cargo.toml`'s `[dependencies]` and add:

```toml
base64 = { workspace = true }
```

If `base64` isn't already in `[workspace.dependencies]`, add it to the root `Cargo.toml`'s `[workspace.dependencies]` first:

```toml
base64 = "0.22"
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p prax-query --test inputs_scalar`
Expected: all numeric/bool/bytes/uuid/json tests pass.

- [ ] **Step 6: Commit**

```bash
git add prax-query/src/inputs/scalar.rs prax-query/tests/inputs_scalar.rs prax-query/Cargo.toml Cargo.toml Cargo.lock
git commit -m "feat(query): add numeric, bool, bytes, uuid, json scalar filters

IntFilter / BigIntFilter / FloatFilter / DecimalFilter / UuidFilter /
BytesFilter generated via a scalar_filter! macro for consistency.
BoolFilter / JsonFilter and their nullable variants are written out
because their op set is smaller. Decimal / Uuid / Bytes lower to
FilterValue::String (Decimal as digits, Uuid as hyphenated, Bytes as
base64) since the runtime IR has no dedicated variants; the driver
layer parses on the wire."
```

---

## Task 8: Add datetime/date/time/enum filters

**Files:**
- Modify: `prax-query/src/inputs/scalar.rs`
- Modify: `prax-query/tests/inputs_scalar.rs`

- [ ] **Step 1: Append failing tests**

Append to `prax-query/tests/inputs_scalar.rs`:

```rust
use prax_query::inputs::{
    DateFilter, DateNullableFilter, DateTimeFilter, DateTimeNullableFilter, EnumFilter,
    EnumNullableFilter, TimeFilter, TimeNullableFilter,
};

#[test]
fn datetime_filter_equals_lowers() {
    use chrono::{TimeZone, Utc};
    let dt = Utc.with_ymd_and_hms(2026, 5, 18, 12, 0, 0).unwrap();
    let f = DateTimeFilter::equals(dt);
    let filter = f.into_filter("created_at");
    // Encoded as RFC3339 string.
    match filter {
        Filter::Equals(col, FilterValue::String(s)) => {
            assert_eq!(col, "created_at");
            assert!(s.starts_with("2026-05-18T12:00:00"));
        }
        other => panic!("expected DateTime Equals, got {:?}", other),
    }
}

#[test]
fn date_filter_equals_lowers() {
    use chrono::NaiveDate;
    let d = NaiveDate::from_ymd_opt(2026, 5, 18).unwrap();
    let f = DateFilter::equals(d);
    let filter = f.into_filter("birthday");
    match filter {
        Filter::Equals(col, FilterValue::String(s)) => {
            assert_eq!(col, "birthday");
            assert_eq!(s, "2026-05-18");
        }
        other => panic!("expected Date Equals, got {:?}", other),
    }
}

#[test]
fn time_filter_equals_lowers() {
    use chrono::NaiveTime;
    let t = NaiveTime::from_hms_opt(13, 45, 0).unwrap();
    let f = TimeFilter::equals(t);
    let filter = f.into_filter("opens_at");
    match filter {
        Filter::Equals(col, FilterValue::String(s)) => {
            assert_eq!(col, "opens_at");
            assert_eq!(s, "13:45:00");
        }
        other => panic!("expected Time Equals, got {:?}", other),
    }
}

#[test]
fn enum_filter_equals_lowers() {
    let f: EnumFilter<&str> = EnumFilter::equals("Admin");
    let filter = f.into_filter("role");
    assert_eq!(filter, Filter::Equals("role".into(), FilterValue::String("Admin".into())));
}
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p prax-query --test inputs_scalar`
Expected: compile errors — the new filter types don't exist.

- [ ] **Step 3: Append the datetime/date/time/enum filters**

Append to `prax-query/src/inputs/scalar.rs`:

```rust
scalar_filter!(
    /// Filter for non-nullable `DateTime` columns (encoded RFC3339).
    DateTimeFilter<chrono::DateTime<chrono::Utc>> => |v: chrono::DateTime<chrono::Utc>| {
        FilterValue::String(v.to_rfc3339())
    } as FilterValue::String,
    /// Filter for nullable `DateTime` columns.
    nullable DateTimeNullableFilter
);

scalar_filter!(
    /// Filter for non-nullable `Date` columns (encoded YYYY-MM-DD).
    DateFilter<chrono::NaiveDate> => |v: chrono::NaiveDate| {
        FilterValue::String(v.to_string())
    } as FilterValue::String,
    /// Filter for nullable `Date` columns.
    nullable DateNullableFilter
);

scalar_filter!(
    /// Filter for non-nullable `Time` columns (encoded HH:MM:SS).
    TimeFilter<chrono::NaiveTime> => |v: chrono::NaiveTime| {
        FilterValue::String(v.format("%H:%M:%S").to_string())
    } as FilterValue::String,
    /// Filter for nullable `Time` columns.
    nullable TimeNullableFilter
);

/// Filter operators for an enum-typed column.
///
/// `E` is the user-defined enum. Codegen `impl From<E> for String` so
/// the macro's bare-ident shorthand (`role: Admin`) flows through.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", bound = "E: Serialize + for<'de2> Deserialize<'de2>")]
pub struct EnumFilter<E> {
    /// `column = value`
    pub equals: Option<E>,
    /// Negation.
    pub not: Option<Box<EnumFilter<E>>>,
    /// `column IN (...)`
    pub in_list: Option<Vec<E>>,
    /// `column NOT IN (...)`
    pub not_in: Option<Vec<E>>,
}

impl<E> EnumFilter<E> {
    /// `equals: Some(value)`.
    pub fn equals(v: E) -> Self {
        Self { equals: Some(v), not: None, in_list: None, not_in: None }
    }
}

impl<E: ToString + Clone> ScalarFilter for EnumFilter<E> {
    fn into_filter(self, column: &str) -> Filter {
        let col: crate::filter::FieldName = column.to_string().into();
        let mut parts: Vec<Filter> = Vec::new();
        if let Some(v) = self.equals {
            parts.push(Filter::Equals(col.clone(), FilterValue::String(v.to_string())));
        }
        if let Some(boxed) = self.not {
            parts.push(Filter::Not(Box::new(boxed.into_filter(column))));
        }
        if let Some(values) = self.in_list {
            let vs: Vec<FilterValue> = values.into_iter().map(|v| FilterValue::String(v.to_string())).collect();
            parts.push(Filter::In(col.clone(), vs));
        }
        if let Some(values) = self.not_in {
            let vs: Vec<FilterValue> = values.into_iter().map(|v| FilterValue::String(v.to_string())).collect();
            parts.push(Filter::NotIn(col, vs));
        }
        match parts.len() {
            0 => Filter::None,
            1 => parts.into_iter().next().unwrap(),
            _ => Filter::and(parts),
        }
    }
}

/// Filter operators for a nullable enum-typed column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", bound = "E: Serialize + for<'de2> Deserialize<'de2>")]
pub struct EnumNullableFilter<E> {
    /// `column = value`
    pub equals: Option<E>,
    /// Negation.
    pub not: Option<Box<EnumNullableFilter<E>>>,
    /// `column IN (...)`
    pub in_list: Option<Vec<E>>,
    /// `column NOT IN (...)`
    pub not_in: Option<Vec<E>>,
    /// IS NULL / IS NOT NULL.
    pub is_null: Option<bool>,
}

impl<E: ToString + Clone> ScalarFilter for EnumNullableFilter<E> {
    fn into_filter(self, column: &str) -> Filter {
        let mut parts: Vec<Filter> = Vec::new();
        if let Some(b) = self.is_null {
            parts.push(if b {
                Filter::IsNull(column.to_string().into())
            } else {
                Filter::IsNotNull(column.to_string().into())
            });
        }
        let inner = EnumFilter::<E> {
            equals: self.equals,
            not: self.not.map(|b| Box::new(EnumFilter {
                equals: b.equals,
                in_list: b.in_list,
                not_in: b.not_in,
                not: None,
            })),
            in_list: self.in_list,
            not_in: self.not_in,
        };
        let f = inner.into_filter(column);
        if !matches!(f, Filter::None) { parts.push(f); }
        match parts.len() {
            0 => Filter::None,
            1 => parts.into_iter().next().unwrap(),
            _ => Filter::and(parts),
        }
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p prax-query --test inputs_scalar`
Expected: all datetime/date/time/enum tests pass.

- [ ] **Step 5: Commit**

```bash
git add prax-query/src/inputs/scalar.rs prax-query/tests/inputs_scalar.rs
git commit -m "feat(query): add datetime/date/time/enum scalar filters

DateTimeFilter encodes as RFC3339, DateFilter as YYYY-MM-DD,
TimeFilter as HH:MM:SS, EnumFilter<E> via the user enum's ToString.
All lower to FilterValue::String — the driver layer parses on the
wire."
```

---

## Task 9: Relation filter wrappers

**Files:**
- Create: `prax-query/src/inputs/relation.rs`
- Create: `prax-query/tests/inputs_relation.rs`

`ListRelationFilter<W>` and `SingleRelationFilter<W>` lower to
`Filter::ScalarSubquery` via a per-relation `RelationMeta` adapter that
phase 2's codegen will supply. Phase 1 introduces the wrappers and a
hand-built adapter trait used in tests.

- [ ] **Step 1: Write the failing test**

Create `prax-query/tests/inputs_relation.rs`:

```rust
use prax_query::filter::{Filter, FilterValue};
use prax_query::inputs::{
    relation::{LowerRelationFilter, RelationMeta, ListRelationFilter, SingleRelationFilter},
    StringFilter, WhereInput,
};
use prax_query::traits::Model;

struct Post;
impl Model for Post {
    const MODEL_NAME: &'static str = "Post";
    const TABLE_NAME: &'static str = "posts";
    const PRIMARY_KEY: &'static [&'static str] = &["id"];
    const COLUMNS: &'static [&'static str] = &["id", "author_id", "published"];
}

#[derive(Default, Clone)]
struct PostWhereInput {
    pub published: Option<prax_query::inputs::BoolFilter>,
}
impl WhereInput for PostWhereInput {
    type Model = Post;
    fn into_ir(self) -> Filter {
        use prax_query::inputs::ScalarFilter;
        match self.published {
            Some(f) => f.into_filter("published"),
            None => Filter::None,
        }
    }
}

// Hand-built relation meta for `User.posts` so we don't need the codegen.
struct UserPostsMeta;
impl RelationMeta for UserPostsMeta {
    const PARENT_TABLE: &'static str = "users";
    const PARENT_PK: &'static str = "id";
    const CHILD_TABLE: &'static str = "posts";
    const CHILD_FK: &'static str = "author_id";
}

#[test]
fn list_relation_some_lowers_to_exists_scalar_subquery() {
    let rf = ListRelationFilter {
        some: Some(PostWhereInput {
            published: Some(prax_query::inputs::BoolFilter::equals(true)),
            ..Default::default()
        }),
        ..Default::default()
    };
    let filter = rf.lower::<UserPostsMeta>();
    match filter {
        Filter::ScalarSubquery { sql, params } => {
            assert!(sql.contains("EXISTS"));
            assert!(sql.contains("posts"));
            assert!(sql.contains("author_id"));
            // The inner filter pulls `published = $?` into the subquery.
            assert!(params.iter().any(|p| matches!(p, FilterValue::Bool(true))));
        }
        other => panic!("expected Filter::ScalarSubquery, got {:?}", other),
    }
}

#[test]
fn list_relation_none_lowers_to_not_exists() {
    let rf = ListRelationFilter::<PostWhereInput> {
        none: Some(PostWhereInput {
            published: Some(prax_query::inputs::BoolFilter::equals(true)),
        }),
        ..Default::default()
    };
    let filter = rf.lower::<UserPostsMeta>();
    match filter {
        Filter::ScalarSubquery { sql, .. } => assert!(sql.starts_with("NOT EXISTS")),
        other => panic!("expected NOT EXISTS subquery, got {:?}", other),
    }
}

#[test]
fn list_relation_every_lowers_to_not_exists_negated() {
    let rf = ListRelationFilter::<PostWhereInput> {
        every: Some(PostWhereInput {
            published: Some(prax_query::inputs::BoolFilter::equals(true)),
        }),
        ..Default::default()
    };
    let filter = rf.lower::<UserPostsMeta>();
    // `every: F` == `NOT EXISTS (child WHERE parent.pk = child.fk AND NOT (F))`.
    match filter {
        Filter::ScalarSubquery { sql, .. } => {
            assert!(sql.starts_with("NOT EXISTS"));
            assert!(sql.contains("NOT ("));
        }
        other => panic!("expected NOT EXISTS subquery, got {:?}", other),
    }
}

#[test]
fn single_relation_is_lowers_to_exists() {
    let rf = SingleRelationFilter::<PostWhereInput> {
        is: Some(PostWhereInput {
            published: Some(prax_query::inputs::BoolFilter::equals(true)),
        }),
        ..Default::default()
    };
    let filter = rf.lower::<UserPostsMeta>();
    match filter {
        Filter::ScalarSubquery { sql, .. } => assert!(sql.starts_with("EXISTS")),
        other => panic!("expected EXISTS subquery, got {:?}", other),
    }
}
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p prax-query --test inputs_relation`
Expected: compile error — types not found.

- [ ] **Step 3: Implement the relation module**

Create `prax-query/src/inputs/relation.rs`:

```rust
//! Relation-aware filter wrappers.
//!
//! `ListRelationFilter<W>` and `SingleRelationFilter<W>` carry the
//! Prisma operator shape (`some`/`every`/`none` for to-many;
//! `is`/`is_not` for to-one). They lower to [`Filter::ScalarSubquery`]
//! fragments via a per-relation [`RelationMeta`] adapter that codegen
//! emits (phase 2) but tests / hand-built users can supply directly.

use crate::filter::{Filter, FilterValue};
use crate::inputs::traits::WhereInput;

/// Static metadata for one parent→child relation.
///
/// Phase 2 codegen emits one impl per relation declared in the schema.
/// Hand-rolled callers can implement this trait themselves.
pub trait RelationMeta {
    /// Parent SQL table name.
    const PARENT_TABLE: &'static str;
    /// Parent primary-key column name.
    const PARENT_PK: &'static str;
    /// Child SQL table name.
    const CHILD_TABLE: &'static str;
    /// Child foreign-key column name pointing back at the parent.
    const CHILD_FK: &'static str;
}

/// Filter operators for a to-many relation.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ListRelationFilter<W> {
    /// At least one child matches `W`.
    pub some: Option<W>,
    /// Every existing child matches `W`.
    pub every: Option<W>,
    /// No child matches `W`.
    pub none: Option<W>,
}

/// Filter operators for a to-one relation.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SingleRelationFilter<W> {
    /// The related row matches `W`.
    pub is: Option<W>,
    /// The related row does NOT match `W` (or doesn't exist).
    pub is_not: Option<W>,
}

/// Lowering helper: produces `Filter::ScalarSubquery` from a relation
/// filter + `RelationMeta`.
///
/// Implemented blanket-style for any `W: WhereInput` so neither the
/// codegen nor the macro layer needs to manually thread metadata.
pub trait LowerRelationFilter {
    /// Lower this relation filter to a runtime [`Filter`] using the
    /// supplied metadata.
    fn lower<M: RelationMeta>(self) -> Filter;
}

fn render_inline_filter(
    inner: Filter,
    out_sql: &mut String,
    out_params: &mut Vec<FilterValue>,
    base: usize,
) {
    // Walk a Filter producing `{N}`-placeholder SQL keyed by inner-param indices.
    // For phase 1 we keep this minimal: support the AND/OR/NOT plus the leaf
    // operators that the scalar filters emit. ScalarSubquery nesting is not
    // expected at phase 1.
    fn write(f: &Filter, sql: &mut String, params: &mut Vec<FilterValue>) {
        match f {
            Filter::None => sql.push_str("TRUE"),
            Filter::Equals(c, v) => {
                if matches!(v, FilterValue::Null) {
                    sql.push_str(&format!("{} IS NULL", c));
                } else {
                    let idx = params.len();
                    params.push(v.clone());
                    sql.push_str(&format!("{} = {{{}}}", c, idx));
                }
            }
            Filter::NotEquals(c, v) => {
                if matches!(v, FilterValue::Null) {
                    sql.push_str(&format!("{} IS NOT NULL", c));
                } else {
                    let idx = params.len();
                    params.push(v.clone());
                    sql.push_str(&format!("{} <> {{{}}}", c, idx));
                }
            }
            Filter::Lt(c, v) => { let i = params.len(); params.push(v.clone()); sql.push_str(&format!("{} < {{{}}}", c, i)); }
            Filter::Lte(c, v) => { let i = params.len(); params.push(v.clone()); sql.push_str(&format!("{} <= {{{}}}", c, i)); }
            Filter::Gt(c, v) => { let i = params.len(); params.push(v.clone()); sql.push_str(&format!("{} > {{{}}}", c, i)); }
            Filter::Gte(c, v) => { let i = params.len(); params.push(v.clone()); sql.push_str(&format!("{} >= {{{}}}", c, i)); }
            Filter::IsNull(c) => sql.push_str(&format!("{} IS NULL", c)),
            Filter::IsNotNull(c) => sql.push_str(&format!("{} IS NOT NULL", c)),
            Filter::Contains(c, FilterValue::String(s)) => {
                let i = params.len(); params.push(FilterValue::String(format!("%{}%", s)));
                sql.push_str(&format!("{} LIKE {{{}}}", c, i));
            }
            Filter::StartsWith(c, FilterValue::String(s)) => {
                let i = params.len(); params.push(FilterValue::String(format!("{}%", s)));
                sql.push_str(&format!("{} LIKE {{{}}}", c, i));
            }
            Filter::EndsWith(c, FilterValue::String(s)) => {
                let i = params.len(); params.push(FilterValue::String(format!("%{}", s)));
                sql.push_str(&format!("{} LIKE {{{}}}", c, i));
            }
            Filter::Contains(_, _) | Filter::StartsWith(_, _) | Filter::EndsWith(_, _) => {
                panic!("phase 1 inline lowering supports only String LIKE values");
            }
            Filter::In(c, values) => {
                if values.is_empty() { sql.push_str("FALSE"); return; }
                sql.push_str(&format!("{} IN (", c));
                for (n, v) in values.iter().enumerate() {
                    if n > 0 { sql.push_str(", "); }
                    let i = params.len(); params.push(v.clone()); sql.push_str(&format!("{{{}}}", i));
                }
                sql.push(')');
            }
            Filter::NotIn(c, values) => {
                if values.is_empty() { sql.push_str("TRUE"); return; }
                sql.push_str(&format!("{} NOT IN (", c));
                for (n, v) in values.iter().enumerate() {
                    if n > 0 { sql.push_str(", "); }
                    let i = params.len(); params.push(v.clone()); sql.push_str(&format!("{{{}}}", i));
                }
                sql.push(')');
            }
            Filter::And(parts) => {
                if parts.is_empty() { sql.push_str("TRUE"); return; }
                sql.push('(');
                for (n, p) in parts.iter().enumerate() {
                    if n > 0 { sql.push_str(" AND "); }
                    write(p, sql, params);
                }
                sql.push(')');
            }
            Filter::Or(parts) => {
                if parts.is_empty() { sql.push_str("FALSE"); return; }
                sql.push('(');
                for (n, p) in parts.iter().enumerate() {
                    if n > 0 { sql.push_str(" OR "); }
                    write(p, sql, params);
                }
                sql.push(')');
            }
            Filter::Not(inner) => { sql.push_str("NOT ("); write(inner, sql, params); sql.push(')'); }
            Filter::ScalarSubquery { .. } => {
                panic!("phase 1 does not support nesting ScalarSubquery inside relation filters");
            }
        }
    }
    let mut sql = String::new();
    write(&inner, &mut sql, out_params);
    // Re-key placeholders to start at `base` instead of 0.
    let rekeyed = sql
        .as_bytes()
        .iter()
        .scan(false, |in_brace, b| Some((b, in_brace)))
        .map(|(_, _)| ()) // placeholder so we can use sql below
        .count();
    let _ = rekeyed;
    // Rewrite `{n}` => `{base+n}` so the outer to_sql_with_params indexes correctly.
    let mut rewritten = String::with_capacity(sql.len());
    let mut chars = sql.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '{' {
            let mut digits = String::new();
            while let Some(&p) = chars.peek() {
                if p == '}' { chars.next(); break; }
                digits.push(p); chars.next();
            }
            let n: usize = digits.parse().expect("placeholder index");
            rewritten.push_str(&format!("{{{}}}", base + n));
        } else {
            rewritten.push(c);
        }
    }
    out_sql.push_str(&rewritten);
}

impl<W: WhereInput> LowerRelationFilter for ListRelationFilter<W> {
    fn lower<M: RelationMeta>(self) -> Filter {
        let mut clauses: Vec<Filter> = Vec::new();
        if let Some(w) = self.some {
            let inner = w.into_ir();
            let mut params: Vec<FilterValue> = Vec::new();
            let mut body = String::new();
            render_inline_filter(inner, &mut body, &mut params, 0);
            let sql = format!(
                "EXISTS (SELECT 1 FROM {} WHERE {}.{} = {}.{} AND {})",
                M::CHILD_TABLE, M::CHILD_TABLE, M::CHILD_FK, M::PARENT_TABLE, M::PARENT_PK, body
            );
            clauses.push(Filter::ScalarSubquery { sql: sql.into(), params });
        }
        if let Some(w) = self.every {
            let inner = w.into_ir();
            let mut params: Vec<FilterValue> = Vec::new();
            let mut body = String::new();
            render_inline_filter(inner, &mut body, &mut params, 0);
            let sql = format!(
                "NOT EXISTS (SELECT 1 FROM {} WHERE {}.{} = {}.{} AND NOT ({}))",
                M::CHILD_TABLE, M::CHILD_TABLE, M::CHILD_FK, M::PARENT_TABLE, M::PARENT_PK, body
            );
            clauses.push(Filter::ScalarSubquery { sql: sql.into(), params });
        }
        if let Some(w) = self.none {
            let inner = w.into_ir();
            let mut params: Vec<FilterValue> = Vec::new();
            let mut body = String::new();
            render_inline_filter(inner, &mut body, &mut params, 0);
            let sql = format!(
                "NOT EXISTS (SELECT 1 FROM {} WHERE {}.{} = {}.{} AND {})",
                M::CHILD_TABLE, M::CHILD_TABLE, M::CHILD_FK, M::PARENT_TABLE, M::PARENT_PK, body
            );
            clauses.push(Filter::ScalarSubquery { sql: sql.into(), params });
        }
        match clauses.len() {
            0 => Filter::None,
            1 => clauses.into_iter().next().unwrap(),
            _ => Filter::and(clauses),
        }
    }
}

impl<W: WhereInput> LowerRelationFilter for SingleRelationFilter<W> {
    fn lower<M: RelationMeta>(self) -> Filter {
        let mut clauses: Vec<Filter> = Vec::new();
        if let Some(w) = self.is {
            let inner = w.into_ir();
            let mut params: Vec<FilterValue> = Vec::new();
            let mut body = String::new();
            render_inline_filter(inner, &mut body, &mut params, 0);
            let sql = format!(
                "EXISTS (SELECT 1 FROM {} WHERE {}.{} = {}.{} AND {})",
                M::CHILD_TABLE, M::CHILD_TABLE, M::CHILD_FK, M::PARENT_TABLE, M::PARENT_PK, body
            );
            clauses.push(Filter::ScalarSubquery { sql: sql.into(), params });
        }
        if let Some(w) = self.is_not {
            let inner = w.into_ir();
            let mut params: Vec<FilterValue> = Vec::new();
            let mut body = String::new();
            render_inline_filter(inner, &mut body, &mut params, 0);
            let sql = format!(
                "NOT EXISTS (SELECT 1 FROM {} WHERE {}.{} = {}.{} AND {})",
                M::CHILD_TABLE, M::CHILD_TABLE, M::CHILD_FK, M::PARENT_TABLE, M::PARENT_PK, body
            );
            clauses.push(Filter::ScalarSubquery { sql: sql.into(), params });
        }
        match clauses.len() {
            0 => Filter::None,
            1 => clauses.into_iter().next().unwrap(),
            _ => Filter::and(clauses),
        }
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p prax-query --test inputs_relation`
Expected: every relation test passes.

- [ ] **Step 5: Commit**

```bash
git add prax-query/src/inputs/relation.rs prax-query/tests/inputs_relation.rs
git commit -m "feat(query): add relation filter wrappers + EXISTS/NOT EXISTS lowering

ListRelationFilter (some/every/none) and SingleRelationFilter
(is/is_not) lower to Filter::ScalarSubquery via a per-relation
RelationMeta adapter. Phase 1 supplies a hand-rolled inline filter
renderer scoped to the operators the scalar filters emit;
phase 2 codegen supplies RelationMeta impls per relation."
```

---

## Task 10: Scalar field update wrappers

**Files:**
- Create: `prax-query/src/inputs/scalar_update.rs`
- Create: `prax-query/tests/inputs_update.rs`

These match the spec's `IntFieldUpdate { set, increment, decrement, multiply, divide }` and string/bool/enum cousins. Phase 1 stores them; phase 5 (writes) consumes them.

- [ ] **Step 1: Write the failing test**

Create `prax-query/tests/inputs_update.rs`:

```rust
use prax_query::inputs::{
    BoolFieldUpdate, IntFieldUpdate, IntNullableFieldUpdate, StringFieldUpdate,
    StringNullableFieldUpdate,
};

#[test]
fn int_field_update_from_scalar_shortcut() {
    let u: IntFieldUpdate = 5i32.into();
    assert_eq!(u.set, Some(5));
    assert!(u.increment.is_none());
}

#[test]
fn int_field_update_increment_and_set_keeps_both() {
    let u = IntFieldUpdate { set: Some(0), increment: Some(1), ..Default::default() };
    assert_eq!(u.set, Some(0));
    assert_eq!(u.increment, Some(1));
}

#[test]
fn string_nullable_field_update_unset_marker() {
    let u = StringNullableFieldUpdate { unset: Some(true), ..Default::default() };
    assert_eq!(u.unset, Some(true));
}

#[test]
fn string_field_update_from_scalar_shortcut() {
    let u: StringFieldUpdate = "Alice".into();
    assert_eq!(u.set.as_deref(), Some("Alice"));
}

#[test]
fn bool_field_update_from_scalar_shortcut() {
    let u: BoolFieldUpdate = true.into();
    assert_eq!(u.set, Some(true));
}

#[test]
fn int_nullable_field_update_unset_marker() {
    let u = IntNullableFieldUpdate { unset: Some(true), ..Default::default() };
    assert_eq!(u.unset, Some(true));
}
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p prax-query --test inputs_update`
Expected: compile error.

- [ ] **Step 3: Implement the module**

Create `prax-query/src/inputs/scalar_update.rs`:

```rust
//! Scalar field update wrappers.
//!
//! Each `*FieldUpdate` struct carries the atomic operators expressible
//! against one scalar type. Phase 5 (write macros) consumes these;
//! phase 1 only defines them so the codegen scaffolding in phase 2
//! can refer to them.

use serde::{Deserialize, Serialize};

/// Update operators for a non-nullable `Int` (`i32`) column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IntFieldUpdate {
    /// `SET column = value`
    pub set: Option<i64>,
    /// `SET column = column + value`
    pub increment: Option<i64>,
    /// `SET column = column - value`
    pub decrement: Option<i64>,
    /// `SET column = column * value`
    pub multiply: Option<i64>,
    /// `SET column = column / value`
    pub divide: Option<i64>,
}

impl From<i32> for IntFieldUpdate {
    fn from(v: i32) -> Self { Self { set: Some(v as i64), ..Default::default() } }
}
impl From<i64> for IntFieldUpdate {
    fn from(v: i64) -> Self { Self { set: Some(v), ..Default::default() } }
}

/// Update operators for a nullable `Int` column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IntNullableFieldUpdate {
    /// `SET column = value`
    pub set: Option<i64>,
    /// `SET column = column + value`
    pub increment: Option<i64>,
    /// `SET column = column - value`
    pub decrement: Option<i64>,
    /// `SET column = column * value`
    pub multiply: Option<i64>,
    /// `SET column = column / value`
    pub divide: Option<i64>,
    /// `SET column = NULL`
    pub unset: Option<bool>,
}

/// Update operators for a non-nullable `BigInt` (`i64`) column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BigIntFieldUpdate {
    /// `SET column = value`
    pub set: Option<i64>,
    /// `SET column = column + value`
    pub increment: Option<i64>,
    /// `SET column = column - value`
    pub decrement: Option<i64>,
    /// `SET column = column * value`
    pub multiply: Option<i64>,
    /// `SET column = column / value`
    pub divide: Option<i64>,
}

impl From<i64> for BigIntFieldUpdate {
    fn from(v: i64) -> Self { Self { set: Some(v), ..Default::default() } }
}

/// Update operators for a nullable `BigInt` column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BigIntNullableFieldUpdate {
    /// `SET column = value`
    pub set: Option<i64>,
    /// `SET column = column + value`
    pub increment: Option<i64>,
    /// `SET column = column - value`
    pub decrement: Option<i64>,
    /// `SET column = column * value`
    pub multiply: Option<i64>,
    /// `SET column = column / value`
    pub divide: Option<i64>,
    /// `SET column = NULL`
    pub unset: Option<bool>,
}

/// Update operators for a non-nullable `Float` column.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct FloatFieldUpdate {
    /// `SET column = value`
    pub set: Option<f64>,
    /// `SET column = column + value`
    pub increment: Option<f64>,
    /// `SET column = column - value`
    pub decrement: Option<f64>,
    /// `SET column = column * value`
    pub multiply: Option<f64>,
    /// `SET column = column / value`
    pub divide: Option<f64>,
}

impl From<f64> for FloatFieldUpdate {
    fn from(v: f64) -> Self { Self { set: Some(v), ..Default::default() } }
}

/// Update operators for a nullable `Float` column.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct FloatNullableFieldUpdate {
    /// `SET column = value`
    pub set: Option<f64>,
    /// `SET column = column + value`
    pub increment: Option<f64>,
    /// `SET column = column - value`
    pub decrement: Option<f64>,
    /// `SET column = column * value`
    pub multiply: Option<f64>,
    /// `SET column = column / value`
    pub divide: Option<f64>,
    /// `SET column = NULL`
    pub unset: Option<bool>,
}

/// Update operators for a non-nullable `Decimal` column (transmitted as string).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DecimalFieldUpdate {
    /// `SET column = value`
    pub set: Option<String>,
    /// `SET column = column + value`
    pub increment: Option<String>,
    /// `SET column = column - value`
    pub decrement: Option<String>,
    /// `SET column = column * value`
    pub multiply: Option<String>,
    /// `SET column = column / value`
    pub divide: Option<String>,
}

/// Update operators for a nullable `Decimal` column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DecimalNullableFieldUpdate {
    /// `SET column = value`
    pub set: Option<String>,
    /// `SET column = column + value`
    pub increment: Option<String>,
    /// `SET column = column - value`
    pub decrement: Option<String>,
    /// `SET column = column * value`
    pub multiply: Option<String>,
    /// `SET column = column / value`
    pub divide: Option<String>,
    /// `SET column = NULL`
    pub unset: Option<bool>,
}

/// Update operators for a non-nullable `String` column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct StringFieldUpdate {
    /// `SET column = value`
    pub set: Option<String>,
}

impl From<&str> for StringFieldUpdate {
    fn from(v: &str) -> Self { Self { set: Some(v.into()) } }
}
impl From<String> for StringFieldUpdate {
    fn from(v: String) -> Self { Self { set: Some(v) } }
}

/// Update operators for a nullable `String` column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct StringNullableFieldUpdate {
    /// `SET column = value`
    pub set: Option<String>,
    /// `SET column = NULL`
    pub unset: Option<bool>,
}

impl From<&str> for StringNullableFieldUpdate {
    fn from(v: &str) -> Self { Self { set: Some(v.into()), unset: None } }
}
impl From<String> for StringNullableFieldUpdate {
    fn from(v: String) -> Self { Self { set: Some(v), unset: None } }
}

/// Update operators for a non-nullable `Boolean` column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BoolFieldUpdate {
    /// `SET column = value`
    pub set: Option<bool>,
}

impl From<bool> for BoolFieldUpdate {
    fn from(v: bool) -> Self { Self { set: Some(v) } }
}

/// Update operators for a nullable `Boolean` column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BoolNullableFieldUpdate {
    /// `SET column = value`
    pub set: Option<bool>,
    /// `SET column = NULL`
    pub unset: Option<bool>,
}

/// Update operators for an enum-typed column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", bound = "E: Serialize + for<'de2> Deserialize<'de2>")]
pub struct EnumFieldUpdate<E> {
    /// `SET column = value`
    pub set: Option<E>,
}

/// Update operators for a nullable enum-typed column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", bound = "E: Serialize + for<'de2> Deserialize<'de2>")]
pub struct EnumNullableFieldUpdate<E> {
    /// `SET column = value`
    pub set: Option<E>,
    /// `SET column = NULL`
    pub unset: Option<bool>,
}

/// Update operators for a non-nullable `DateTime` column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DateTimeFieldUpdate {
    /// `SET column = value` (RFC3339-encoded).
    pub set: Option<String>,
}

/// Update operators for a nullable `DateTime` column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DateTimeNullableFieldUpdate {
    /// `SET column = value` (RFC3339-encoded).
    pub set: Option<String>,
    /// `SET column = NULL`
    pub unset: Option<bool>,
}

/// Update operators for a non-nullable `Bytes` column (base64-encoded).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BytesFieldUpdate {
    /// `SET column = value`
    pub set: Option<String>,
}

/// Update operators for a nullable `Bytes` column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BytesNullableFieldUpdate {
    /// `SET column = value`
    pub set: Option<String>,
    /// `SET column = NULL`
    pub unset: Option<bool>,
}

/// Update operators for a non-nullable `Uuid` column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct UuidFieldUpdate {
    /// `SET column = value`
    pub set: Option<String>,
}

/// Update operators for a nullable `Uuid` column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct UuidNullableFieldUpdate {
    /// `SET column = value`
    pub set: Option<String>,
    /// `SET column = NULL`
    pub unset: Option<bool>,
}

/// Update operators for a non-nullable `Json` column.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct JsonFieldUpdate {
    /// `SET column = value`
    pub set: Option<serde_json::Value>,
}

/// Update operators for a nullable `Json` column.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct JsonNullableFieldUpdate {
    /// `SET column = value`
    pub set: Option<serde_json::Value>,
    /// `SET column = NULL`
    pub unset: Option<bool>,
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p prax-query --test inputs_update`
Expected: every test passes.

- [ ] **Step 5: Commit**

```bash
git add prax-query/src/inputs/scalar_update.rs prax-query/tests/inputs_update.rs
git commit -m "feat(query): add scalar field update wrappers

One *FieldUpdate per scalar type, plus nullable variants with an
`unset` flag for SET column = NULL. Numeric variants carry the atomic
ops (increment/decrement/multiply/divide). Phase 5 (write macros)
consumes these; phase 1 only defines the shapes and From<scalar>
ergonomics so the macro DSL's `name: \"Alice\"` shorthand has a
runtime target."
```

---

## Task 11: Per-operation `*Args` containers

**Files:**
- Create: `prax-query/src/inputs/args.rs`
- Create: `prax-query/tests/inputs_args.rs`

`FindManyArgs<E, M>` etc. are the explicit-struct third interface promised in the spec. They round-trip through the operation builders.

- [ ] **Step 1: Write the failing test**

Create `prax-query/tests/inputs_args.rs`:

```rust
use prax_query::filter::Filter;
use prax_query::inputs::{
    CountArgs, CreateArgs, CreateManyArgs, DeleteArgs, DeleteManyArgs, FindFirstArgs,
    FindManyArgs, FindUniqueArgs, UpdateArgs, UpdateManyArgs, UpsertArgs,
};
use prax_query::traits::Model;

struct TestModel;
impl Model for TestModel {
    const MODEL_NAME: &'static str = "TestModel";
    const TABLE_NAME: &'static str = "test_models";
    const PRIMARY_KEY: &'static [&'static str] = &["id"];
    const COLUMNS: &'static [&'static str] = &["id"];
}

#[test]
fn find_many_args_default_is_empty() {
    let a: FindManyArgs<TestModel, Filter, (), ()> = FindManyArgs::default();
    assert!(a.r#where.is_none());
    assert!(a.include.is_none());
    assert!(a.select.is_none());
    assert!(a.order_by.is_none());
    assert!(a.cursor.is_none());
    assert_eq!(a.skip, None);
    assert_eq!(a.take, None);
}

#[test]
fn find_unique_args_carries_unique_filter() {
    let a: FindUniqueArgs<TestModel, Filter, (), ()> = FindUniqueArgs {
        r#where: Filter::None,
        include: None,
        select: None,
    };
    assert!(matches!(a.r#where, Filter::None));
}

#[test]
fn create_args_carries_data() {
    let a: CreateArgs<TestModel, (), (), ()> = CreateArgs { data: (), include: None, select: None };
    assert_eq!(std::mem::size_of_val(&a.data), 0);
}

#[test]
fn upsert_args_round_trip() {
    let a: UpsertArgs<TestModel, Filter, (), (), (), ()> = UpsertArgs {
        r#where: Filter::None,
        create: (),
        update: (),
        include: None,
        select: None,
    };
    assert!(matches!(a.r#where, Filter::None));
}
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p prax-query --test inputs_args`
Expected: compile error.

- [ ] **Step 3: Implement `args.rs`**

Create `prax-query/src/inputs/args.rs`:

```rust
//! Per-operation argument containers.
//!
//! Each struct is the layer-2 "explicit form" of an operation request:
//! the macro DSL (phase 3+) expands to a `*Args { ... }` literal that
//! the operation builder consumes via `.with_args(args)`. Direct
//! construction by hand is fully supported.
//!
//! Generic parameters:
//! - `M` — the model
//! - `W` — `WhereInput` impl for that model
//! - `I` — `IncludeInput` impl
//! - `S` — `SelectInput` impl
//! - `D` — `CreateInput::Data` / `UpdateInput::Data` payload (operation-specific)
//! - `O` — `OrderByInput` impl
//!
//! Phase 1 keeps the bounds open so hand-construction works even before
//! codegen lands. Phase 2 narrows them when the per-model types exist.

use core::marker::PhantomData;

/// Args for `find_unique`. `where` must identify at most one row.
#[derive(Debug, Clone)]
pub struct FindUniqueArgs<M, W, I, S> {
    /// Unique WHERE input.
    pub r#where: W,
    /// Optional include shape.
    pub include: Option<I>,
    /// Optional select shape.
    pub select: Option<S>,
    /// Phantom marker for the model type.
    #[doc(hidden)]
    pub _model: PhantomData<M>,
}

impl<M, W: Default, I, S> Default for FindUniqueArgs<M, W, I, S> {
    fn default() -> Self {
        Self { r#where: W::default(), include: None, select: None, _model: PhantomData }
    }
}

impl<M, W, I, S> FindUniqueArgs<M, W, I, S> {
    /// Construct with the given unique WHERE.
    pub fn new(r#where: W) -> Self {
        Self { r#where, include: None, select: None, _model: PhantomData }
    }
}

/// Args for `find_first`.
#[derive(Debug, Clone, Default)]
pub struct FindFirstArgs<M, W, I, S, O = (), C = ()> {
    /// Optional WHERE input.
    pub r#where: Option<W>,
    /// Optional include shape.
    pub include: Option<I>,
    /// Optional select shape.
    pub select: Option<S>,
    /// Optional order-by shape (single or vec).
    pub order_by: Option<Vec<O>>,
    /// Optional cursor value.
    pub cursor: Option<C>,
    /// Skip N rows.
    pub skip: Option<u64>,
    /// Take N rows.
    pub take: Option<u64>,
    /// Phantom marker for the model.
    #[doc(hidden)]
    pub _model: PhantomData<M>,
}

/// Args for `find_many`.
#[derive(Debug, Clone, Default)]
pub struct FindManyArgs<M, W, I, S, O = (), C = ()> {
    /// Optional WHERE input.
    pub r#where: Option<W>,
    /// Optional include shape.
    pub include: Option<I>,
    /// Optional select shape.
    pub select: Option<S>,
    /// Optional order-by shape (single or vec).
    pub order_by: Option<Vec<O>>,
    /// Optional cursor value.
    pub cursor: Option<C>,
    /// Skip N rows.
    pub skip: Option<u64>,
    /// Take N rows.
    pub take: Option<u64>,
    /// Distinct columns.
    pub distinct: Option<Vec<String>>,
    /// Phantom marker for the model.
    #[doc(hidden)]
    pub _model: PhantomData<M>,
}

/// Args for `create`.
#[derive(Debug, Clone)]
pub struct CreateArgs<M, D, I, S> {
    /// Create-data payload.
    pub data: D,
    /// Optional include shape on the returning row.
    pub include: Option<I>,
    /// Optional select shape on the returning row.
    pub select: Option<S>,
    /// Phantom marker for the model.
    #[doc(hidden)]
    pub _model: PhantomData<M>,
}

impl<M, D: Default, I, S> Default for CreateArgs<M, D, I, S> {
    fn default() -> Self {
        Self { data: D::default(), include: None, select: None, _model: PhantomData }
    }
}

/// Args for `create_many`.
#[derive(Debug, Clone)]
pub struct CreateManyArgs<M, D> {
    /// Create-data payloads.
    pub data: Vec<D>,
    /// Skip rows that would violate a unique constraint (instead of erroring).
    pub skip_duplicates: Option<bool>,
    /// Phantom marker for the model.
    #[doc(hidden)]
    pub _model: PhantomData<M>,
}

impl<M, D> Default for CreateManyArgs<M, D> {
    fn default() -> Self { Self { data: Vec::new(), skip_duplicates: None, _model: PhantomData } }
}

/// Args for `update`.
#[derive(Debug, Clone)]
pub struct UpdateArgs<M, W, U, I, S> {
    /// Unique WHERE input.
    pub r#where: W,
    /// Update-data payload.
    pub data: U,
    /// Optional include shape.
    pub include: Option<I>,
    /// Optional select shape.
    pub select: Option<S>,
    /// Phantom marker for the model.
    #[doc(hidden)]
    pub _model: PhantomData<M>,
}

/// Args for `update_many`.
#[derive(Debug, Clone, Default)]
pub struct UpdateManyArgs<M, W, U> {
    /// Optional WHERE input.
    pub r#where: Option<W>,
    /// Update-data payload.
    pub data: U,
    /// Phantom marker for the model.
    #[doc(hidden)]
    pub _model: PhantomData<M>,
}

/// Args for `upsert`.
#[derive(Debug, Clone)]
pub struct UpsertArgs<M, W, C, U, I, S> {
    /// Unique WHERE input.
    pub r#where: W,
    /// Create-data payload (used if no row matched).
    pub create: C,
    /// Update-data payload (used if a row matched).
    pub update: U,
    /// Optional include shape on the returning row.
    pub include: Option<I>,
    /// Optional select shape on the returning row.
    pub select: Option<S>,
    /// Phantom marker for the model.
    #[doc(hidden)]
    pub _model: PhantomData<M>,
}

/// Args for `delete`.
#[derive(Debug, Clone)]
pub struct DeleteArgs<M, W, I, S> {
    /// Unique WHERE input.
    pub r#where: W,
    /// Optional include shape on the returning row.
    pub include: Option<I>,
    /// Optional select shape on the returning row.
    pub select: Option<S>,
    /// Phantom marker for the model.
    #[doc(hidden)]
    pub _model: PhantomData<M>,
}

/// Args for `delete_many`.
#[derive(Debug, Clone, Default)]
pub struct DeleteManyArgs<M, W> {
    /// Optional WHERE input.
    pub r#where: Option<W>,
    /// Phantom marker for the model.
    #[doc(hidden)]
    pub _model: PhantomData<M>,
}

/// Args for `count`.
#[derive(Debug, Clone, Default)]
pub struct CountArgs<M, W, O = (), C = ()> {
    /// Optional WHERE input.
    pub r#where: Option<W>,
    /// Optional order-by.
    pub order_by: Option<Vec<O>>,
    /// Optional cursor.
    pub cursor: Option<C>,
    /// Skip N rows.
    pub skip: Option<u64>,
    /// Take N rows.
    pub take: Option<u64>,
    /// Phantom marker for the model.
    #[doc(hidden)]
    pub _model: PhantomData<M>,
}

/// Args for `aggregate`. The aggregate spec parameter is filled in by phase 6.
#[derive(Debug, Clone, Default)]
pub struct AggregateArgs<M, W, A, O = (), C = ()> {
    /// Optional WHERE input.
    pub r#where: Option<W>,
    /// Aggregate spec (`_count` / `_avg` / `_sum` / `_min` / `_max`).
    pub aggregate: Option<A>,
    /// Optional order-by.
    pub order_by: Option<Vec<O>>,
    /// Optional cursor.
    pub cursor: Option<C>,
    /// Skip N rows.
    pub skip: Option<u64>,
    /// Take N rows.
    pub take: Option<u64>,
    /// Phantom marker for the model.
    #[doc(hidden)]
    pub _model: PhantomData<M>,
}

/// Args for `group_by`. The grouping spec parameter is filled in by phase 6.
#[derive(Debug, Clone, Default)]
pub struct GroupByArgs<M, W, A, G, H = (), O = ()> {
    /// Group by these field names.
    pub by: Vec<G>,
    /// Optional WHERE input.
    pub r#where: Option<W>,
    /// Optional HAVING input.
    pub having: Option<H>,
    /// Aggregate spec.
    pub aggregate: Option<A>,
    /// Optional order-by.
    pub order_by: Option<Vec<O>>,
    /// Skip N rows.
    pub skip: Option<u64>,
    /// Take N rows.
    pub take: Option<u64>,
    /// Phantom marker for the model.
    #[doc(hidden)]
    pub _model: PhantomData<M>,
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p prax-query --test inputs_args`
Expected: pass.

- [ ] **Step 5: Commit**

```bash
git add prax-query/src/inputs/args.rs prax-query/tests/inputs_args.rs
git commit -m "feat(query): add per-operation Args containers

FindUniqueArgs / FindFirstArgs / FindManyArgs / CreateArgs /
CreateManyArgs / UpdateArgs / UpdateManyArgs / UpsertArgs /
DeleteArgs / DeleteManyArgs / CountArgs / AggregateArgs /
GroupByArgs are the explicit-struct third interface for the DSL.
The macro DSL (phase 3+) expands to these structs; users may
construct them by hand. Generic params are left open at phase 1
and narrowed by codegen (phase 2)."
```

---

## Task 12: Extension methods on read operations

**Files:**
- Modify: `prax-query/src/operations/find_many.rs`
- Modify: `prax-query/src/operations/find_unique.rs`
- Modify: `prax-query/src/operations/find_first.rs`
- Create: `prax-query/tests/operation_ext_methods.rs`

Each read operation gets `with_where_input` / `with_include_input` / `with_select_input` (+ `with_order_by_input`/`with_cursor_input` where applicable). They thread the input through `into_ir()` and AND-combine on top of any existing fluent state.

- [ ] **Step 1: Write the failing test**

Create `prax-query/tests/operation_ext_methods.rs`:

```rust
use prax_query::filter::{Filter, FilterValue};
use prax_query::inputs::{StringFilter, WhereInput};
use prax_query::operations::{FindFirstOperation, FindManyOperation, FindUniqueOperation};
use prax_query::traits::{BoxFuture, Model, QueryEngine};

struct U;
impl Model for U {
    const MODEL_NAME: &'static str = "U";
    const TABLE_NAME: &'static str = "users";
    const PRIMARY_KEY: &'static [&'static str] = &["id"];
    const COLUMNS: &'static [&'static str] = &["id", "email"];
}
impl prax_query::row::FromRow for U {
    fn from_row(_row: &impl prax_query::row::RowRef) -> Result<Self, prax_query::row::RowError> {
        Ok(U)
    }
}

#[derive(Clone)]
struct NoopEngine;
impl QueryEngine for NoopEngine {
    fn query_many<T: Model + prax_query::row::FromRow + Send + 'static>(&self, _: &str, _: Vec<FilterValue>) -> BoxFuture<'_, prax_query::error::QueryResult<Vec<T>>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn query_one<T: Model + prax_query::row::FromRow + Send + 'static>(&self, _: &str, _: Vec<FilterValue>) -> BoxFuture<'_, prax_query::error::QueryResult<T>> {
        Box::pin(async { Err(prax_query::error::QueryError::not_found("t")) })
    }
    fn query_optional<T: Model + prax_query::row::FromRow + Send + 'static>(&self, _: &str, _: Vec<FilterValue>) -> BoxFuture<'_, prax_query::error::QueryResult<Option<T>>> {
        Box::pin(async { Ok(None) })
    }
    fn execute_insert<T: Model + prax_query::row::FromRow + Send + 'static>(&self, _: &str, _: Vec<FilterValue>) -> BoxFuture<'_, prax_query::error::QueryResult<T>> {
        Box::pin(async { Err(prax_query::error::QueryError::not_found("t")) })
    }
    fn execute_update<T: Model + prax_query::row::FromRow + Send + 'static>(&self, _: &str, _: Vec<FilterValue>) -> BoxFuture<'_, prax_query::error::QueryResult<Vec<T>>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn execute_delete(&self, _: &str, _: Vec<FilterValue>) -> BoxFuture<'_, prax_query::error::QueryResult<u64>> {
        Box::pin(async { Ok(0) })
    }
    fn execute_raw(&self, _: &str, _: Vec<FilterValue>) -> BoxFuture<'_, prax_query::error::QueryResult<u64>> {
        Box::pin(async { Ok(0) })
    }
    fn count(&self, _: &str, _: Vec<FilterValue>) -> BoxFuture<'_, prax_query::error::QueryResult<u64>> {
        Box::pin(async { Ok(0) })
    }
}

#[derive(Default, Clone)]
struct UWhereInput {
    pub email: Option<StringFilter>,
}
impl WhereInput for UWhereInput {
    type Model = U;
    fn into_ir(self) -> Filter {
        use prax_query::inputs::ScalarFilter;
        match self.email {
            Some(f) => f.into_filter("email"),
            None => Filter::None,
        }
    }
}

#[test]
fn find_many_with_where_input_replaces_filter_when_first() {
    let op = FindManyOperation::<NoopEngine, U>::new(NoopEngine)
        .with_where_input(UWhereInput { email: Some(StringFilter::contains("@x.com")) });
    assert!(!matches!(op.filter_for_test(), Filter::None));
}

#[test]
fn find_many_with_where_input_ands_with_existing() {
    let op = FindManyOperation::<NoopEngine, U>::new(NoopEngine)
        .r#where(Filter::Equals("id".into(), FilterValue::Int(1)))
        .with_where_input(UWhereInput { email: Some(StringFilter::contains("@x.com")) });
    match op.filter_for_test() {
        Filter::And(parts) => assert_eq!(parts.len(), 2),
        other => panic!("expected Filter::And, got {:?}", other),
    }
}

#[test]
fn find_unique_with_where_input_overwrites_filter() {
    let op = FindUniqueOperation::<NoopEngine, U>::new(NoopEngine)
        .with_where_input(UWhereInput { email: Some(StringFilter::equals("x@y.com")) });
    assert!(!matches!(op.filter_for_test(), Filter::None));
}

#[test]
fn find_first_with_where_input_ands_with_existing() {
    let op = FindFirstOperation::<NoopEngine, U>::new(NoopEngine)
        .r#where(Filter::Equals("id".into(), FilterValue::Int(1)))
        .with_where_input(UWhereInput { email: Some(StringFilter::contains("@x.com")) });
    match op.filter_for_test() {
        Filter::And(parts) => assert_eq!(parts.len(), 2),
        other => panic!("expected Filter::And, got {:?}", other),
    }
}
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p prax-query --test operation_ext_methods`
Expected: compile error — `with_where_input` and `filter_for_test` don't exist.

- [ ] **Step 3: Add the methods to `FindManyOperation`**

Open `prax-query/src/operations/find_many.rs`. Inside the `impl<E: QueryEngine, M: Model + crate::row::FromRow> FindManyOperation<E, M>` block (after the `distinct` method), append:

```rust
    /// Apply a typed `WhereInput`. AND-composes with any previously set
    /// filter — same semantics as calling `.r#where(...)` again.
    pub fn with_where_input<W: crate::inputs::WhereInput<Model = M>>(mut self, w: W) -> Self {
        let f = w.into_ir();
        self.filter = self.filter.and_then(f);
        self
    }

    /// Apply a typed `IncludeInput`. Merges into any previously set
    /// includes (later wins on conflicting relation names).
    pub fn with_include_input<I: crate::inputs::IncludeInput<Model = M>>(mut self, i: I) -> Self {
        let inc = i.into_ir();
        for spec in inc.specs() {
            self.includes.push(spec.clone());
        }
        self
    }

    /// Apply a typed `SelectInput`.
    pub fn with_select_input<S: crate::inputs::SelectInput<Model = M>>(mut self, s: S) -> Self {
        self.select = s.into_ir();
        self
    }

    /// Apply a typed `OrderByInput` (single).
    pub fn with_order_by_input<O: crate::inputs::OrderByInput<Model = M>>(mut self, o: O) -> Self {
        self.order_by = o.into_ir();
        self
    }

    /// Apply a typed `OrderByInput` collection (later items take lower precedence).
    pub fn with_order_by_inputs<O: crate::inputs::OrderByInput<Model = M>>(
        mut self,
        items: impl IntoIterator<Item = O>,
    ) -> Self {
        let mut combined = self.order_by;
        for o in items {
            combined = combined.then(o.into_ir());
        }
        self.order_by = combined;
        self
    }

    /// Test-only accessor for the current filter — needed for unit tests
    /// that don't have a running engine to issue queries against.
    #[cfg(test)]
    pub fn filter_for_test(&self) -> &Filter {
        &self.filter
    }
}
```

(If the existing `impl` block already closes before `distinct`, place the new methods inside the same `impl` block — adjust the closing `}` as needed.)

If `OrderBy` does not have a `.then(other) -> Self` method, define a local fallback at the top of the file:

```rust
fn merge_order(left: OrderBy, right: OrderBy) -> OrderBy {
    // Conservative fallback: keep right if non-empty; else left.
    if right.is_empty() { left } else { right }
}
```

and use that instead of `combined.then(...)`. Verify with `grep -n "fn then\|fn is_empty" prax-query/src/types.rs` what is available.

- [ ] **Step 4: Also expose `filter_for_test` for use by integration test**

The test file uses `cfg(test)` accessors. Integration tests count as `cfg(test)` for the crate they live in, but **not** for `prax-query` itself. Replace `#[cfg(test)]` with `#[doc(hidden)]` on the helper methods so integration tests can call them:

```rust
    #[doc(hidden)]
    pub fn filter_for_test(&self) -> &Filter {
        &self.filter
    }
```

- [ ] **Step 5: Add the same methods to `FindUniqueOperation`**

Open `prax-query/src/operations/find_unique.rs`. After the existing setter methods, append:

```rust
    /// Apply a typed `WhereUniqueInput`. Overwrites the existing filter
    /// (a unique input is a complete predicate, not a constraint to AND-compose).
    pub fn with_where_input<W: crate::inputs::WhereUniqueInput<Model = M>>(mut self, w: W) -> Self {
        self.filter = w.into_ir();
        self
    }

    /// Apply a typed `IncludeInput`.
    pub fn with_include_input<I: crate::inputs::IncludeInput<Model = M>>(mut self, i: I) -> Self {
        let inc = i.into_ir();
        for spec in inc.specs() {
            self.includes.push(spec.clone());
        }
        self
    }

    /// Apply a typed `SelectInput`.
    pub fn with_select_input<S: crate::inputs::SelectInput<Model = M>>(mut self, s: S) -> Self {
        self.select = s.into_ir();
        self
    }

    #[doc(hidden)]
    pub fn filter_for_test(&self) -> &Filter {
        &self.filter
    }
```

If `FindUniqueOperation` does not currently carry `select` or `includes` fields, inspect the struct definition at the top of the file and only emit the methods whose backing fields exist; leave others for phase 2 codegen.

- [ ] **Step 6: Add the methods to `FindFirstOperation`**

Open `prax-query/src/operations/find_first.rs`. Mirror the `FindManyOperation` additions exactly (same five methods + `filter_for_test`). The `OrderBy` handling is identical.

- [ ] **Step 7: Run all tests**

Run: `cargo test -p prax-query --test operation_ext_methods`
Expected: pass.
Run: `cargo test -p prax-query --lib`
Expected: pass (no regression).

- [ ] **Step 8: Commit**

```bash
git add prax-query/src/operations/find_many.rs prax-query/src/operations/find_unique.rs prax-query/src/operations/find_first.rs prax-query/tests/operation_ext_methods.rs
git commit -m "feat(query): add with_*_input extension methods on read operations

FindManyOperation, FindUniqueOperation, FindFirstOperation gain
with_where_input / with_include_input / with_select_input
(+ with_order_by_input on find_many and find_first). The methods
thread inputs through into_ir() and AND-compose on top of any
existing fluent state, matching the spec's coexistence guarantee.
filter_for_test is a doc-hidden accessor used by unit tests."
```

---

## Task 13: Extension methods on write operations

**Files:**
- Modify: `prax-query/src/operations/create.rs`
- Modify: `prax-query/src/operations/update.rs`
- Modify: `prax-query/src/operations/delete.rs`
- Modify: `prax-query/src/operations/upsert.rs`
- Modify: `prax-query/tests/operation_ext_methods.rs`

- [ ] **Step 1: Append failing tests**

Append to `prax-query/tests/operation_ext_methods.rs`:

```rust
use prax_query::operations::{CreateOperation, DeleteOperation, UpdateOperation};

#[test]
fn create_op_accepts_with_include_input() {
    // Use the existing CreateData::Data path. Phase 5 wires nested writes.
    // For phase 1 we only assert the method compiles and is callable.
    fn _compiles<E: QueryEngine, M: Model + prax_query::row::FromRow>(
        op: CreateOperation<E, M>,
    ) -> CreateOperation<E, M>
    where
        M: prax_query::traits::CreateData,
    {
        struct NoopInclude<M>(core::marker::PhantomData<M>);
        impl<M: Model> prax_query::inputs::IncludeInput for NoopInclude<M> {
            type Model = M;
            fn into_ir(self) -> prax_query::relations::Include {
                prax_query::relations::Include::new()
            }
        }
        op.with_include_input(NoopInclude::<M>(core::marker::PhantomData))
    }
    let _ = _compiles::<NoopEngine, U>;
}

#[test]
fn update_op_with_where_input_ands_with_existing() {
    let op = UpdateOperation::<NoopEngine, U>::new(NoopEngine)
        .r#where(Filter::Equals("id".into(), FilterValue::Int(1)))
        .with_where_input(UWhereInput { email: Some(StringFilter::contains("@x.com")) });
    match op.filter_for_test() {
        Filter::And(parts) => assert_eq!(parts.len(), 2),
        other => panic!("expected Filter::And, got {:?}", other),
    }
}

#[test]
fn delete_op_with_where_input_overwrites_filter_for_unique() {
    let op = DeleteOperation::<NoopEngine, U>::new(NoopEngine)
        .with_where_input(UWhereInput { email: Some(StringFilter::equals("x@y.com")) });
    assert!(!matches!(op.filter_for_test(), Filter::None));
}
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p prax-query --test operation_ext_methods`
Expected: compile error.

- [ ] **Step 3: Extend `UpdateOperation`**

Open `prax-query/src/operations/update.rs`. After the existing setter methods, append:

```rust
    /// Apply a typed `WhereUniqueInput`. AND-composes with any
    /// previously set filter so callers can combine the unique key
    /// with side conditions when they need to.
    pub fn with_where_input<W: crate::inputs::WhereUniqueInput<Model = M>>(mut self, w: W) -> Self {
        let f = w.into_ir();
        self.filter = self.filter.and_then(f);
        self
    }

    /// Apply a typed `UpdateInput`. Replaces the current update payload.
    pub fn with_data_input<U: crate::inputs::UpdateInput<Model = M, Data = M::Data>>(mut self, u: U) -> Self
    where
        M: crate::traits::UpdateData,
    {
        self.data = u.into_ir();
        self
    }

    /// Apply a typed `IncludeInput`.
    pub fn with_include_input<I: crate::inputs::IncludeInput<Model = M>>(mut self, i: I) -> Self {
        let inc = i.into_ir();
        for spec in inc.specs() {
            self.includes.push(spec.clone());
        }
        self
    }

    /// Apply a typed `SelectInput`.
    pub fn with_select_input<S: crate::inputs::SelectInput<Model = M>>(mut self, s: S) -> Self {
        self.select = s.into_ir();
        self
    }

    #[doc(hidden)]
    pub fn filter_for_test(&self) -> &Filter {
        &self.filter
    }
```

If the `UpdateOperation` struct doesn't have `includes` / `select` fields, skip the corresponding methods — phase 2 codegen wires them up.

- [ ] **Step 4: Extend `DeleteOperation`**

Open `prax-query/src/operations/delete.rs`. Append the same pattern (omit `with_data_input`):

```rust
    /// Apply a typed `WhereUniqueInput`. Overwrites any previously set
    /// filter — delete operations are intentionally precise.
    pub fn with_where_input<W: crate::inputs::WhereUniqueInput<Model = M>>(mut self, w: W) -> Self {
        self.filter = w.into_ir();
        self
    }

    /// Apply a typed `IncludeInput`.
    pub fn with_include_input<I: crate::inputs::IncludeInput<Model = M>>(mut self, i: I) -> Self {
        let inc = i.into_ir();
        for spec in inc.specs() {
            self.includes.push(spec.clone());
        }
        self
    }

    /// Apply a typed `SelectInput`.
    pub fn with_select_input<S: crate::inputs::SelectInput<Model = M>>(mut self, s: S) -> Self {
        self.select = s.into_ir();
        self
    }

    #[doc(hidden)]
    pub fn filter_for_test(&self) -> &Filter {
        &self.filter
    }
```

- [ ] **Step 5: Extend `CreateOperation`**

Open `prax-query/src/operations/create.rs`. Append:

```rust
    /// Apply a typed `CreateInput`. Replaces the current create payload.
    pub fn with_data_input<C: crate::inputs::CreateInput<Model = M, Data = M::Data>>(mut self, c: C) -> Self
    where
        M: crate::traits::CreateData,
    {
        self.data = c.into_ir();
        self
    }

    /// Apply a typed `IncludeInput`.
    pub fn with_include_input<I: crate::inputs::IncludeInput<Model = M>>(mut self, i: I) -> Self {
        let inc = i.into_ir();
        for spec in inc.specs() {
            self.includes.push(spec.clone());
        }
        self
    }

    /// Apply a typed `SelectInput`.
    pub fn with_select_input<S: crate::inputs::SelectInput<Model = M>>(mut self, s: S) -> Self {
        self.select = s.into_ir();
        self
    }
```

Same caveat: if `CreateOperation` lacks `includes`/`select`, skip those methods.

- [ ] **Step 6: Extend `UpsertOperation`**

Open `prax-query/src/operations/upsert.rs`. Append:

```rust
    /// Apply a typed `WhereUniqueInput`. Overwrites the existing filter.
    pub fn with_where_input<W: crate::inputs::WhereUniqueInput<Model = M>>(mut self, w: W) -> Self {
        self.filter = w.into_ir();
        self
    }

    /// Apply a typed `CreateInput` for the create branch.
    pub fn with_create_input<C: crate::inputs::CreateInput<Model = M, Data = <M as crate::traits::CreateData>::Data>>(mut self, c: C) -> Self
    where
        M: crate::traits::CreateData,
    {
        self.create_data = c.into_ir();
        self
    }

    /// Apply a typed `UpdateInput` for the update branch.
    pub fn with_update_input<U: crate::inputs::UpdateInput<Model = M, Data = <M as crate::traits::UpdateData>::Data>>(mut self, u: U) -> Self
    where
        M: crate::traits::UpdateData,
    {
        self.update_data = u.into_ir();
        self
    }

    /// Apply a typed `IncludeInput`.
    pub fn with_include_input<I: crate::inputs::IncludeInput<Model = M>>(mut self, i: I) -> Self {
        let inc = i.into_ir();
        for spec in inc.specs() {
            self.includes.push(spec.clone());
        }
        self
    }

    /// Apply a typed `SelectInput`.
    pub fn with_select_input<S: crate::inputs::SelectInput<Model = M>>(mut self, s: S) -> Self {
        self.select = s.into_ir();
        self
    }
```

Adjust field names (`create_data`/`update_data`) to match what's already in `UpsertOperation` (use `grep -n "pub create\|pub update\|create_data\|update_data" prax-query/src/operations/upsert.rs`).

- [ ] **Step 7: Run tests**

Run: `cargo test -p prax-query --test operation_ext_methods`
Expected: every test passes.

- [ ] **Step 8: Commit**

```bash
git add prax-query/src/operations/create.rs prax-query/src/operations/update.rs prax-query/src/operations/delete.rs prax-query/src/operations/upsert.rs prax-query/tests/operation_ext_methods.rs
git commit -m "feat(query): add with_*_input extension methods on write operations

CreateOperation, UpdateOperation, DeleteOperation, UpsertOperation
gain methods that consume the corresponding typed inputs. UpdateOp
AND-composes WHERE (allows side conditions); DeleteOp / UpsertOp
overwrite WHERE because the unique filter is the whole point of
those operations. Create/Update data inputs replace the runtime
payload through CreateInput::Data / UpdateInput::Data."
```

---

## Task 14: Extension methods on count + aggregate

**Files:**
- Modify: `prax-query/src/operations/count.rs`
- Modify: `prax-query/src/operations/aggregate.rs`
- Modify: `prax-query/tests/operation_ext_methods.rs`

- [ ] **Step 1: Append failing tests**

Append to `prax-query/tests/operation_ext_methods.rs`:

```rust
use prax_query::operations::CountOperation;

#[test]
fn count_op_with_where_input_ands() {
    let op = CountOperation::<NoopEngine, U>::new(NoopEngine)
        .r#where(Filter::Equals("active".into(), FilterValue::Bool(true)))
        .with_where_input(UWhereInput { email: Some(StringFilter::contains("@x.com")) });
    match op.filter_for_test() {
        Filter::And(parts) => assert_eq!(parts.len(), 2),
        other => panic!("expected Filter::And, got {:?}", other),
    }
}
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p prax-query --test operation_ext_methods`
Expected: compile error.

- [ ] **Step 3: Extend `CountOperation`**

Open `prax-query/src/operations/count.rs`. Append:

```rust
    /// Apply a typed `WhereInput`. AND-composes with any previously set filter.
    pub fn with_where_input<W: crate::inputs::WhereInput<Model = M>>(mut self, w: W) -> Self {
        let f = w.into_ir();
        self.filter = self.filter.and_then(f);
        self
    }

    /// Apply a typed `OrderByInput`.
    pub fn with_order_by_input<O: crate::inputs::OrderByInput<Model = M>>(mut self, o: O) -> Self {
        self.order_by = o.into_ir();
        self
    }

    #[doc(hidden)]
    pub fn filter_for_test(&self) -> &Filter {
        &self.filter
    }
```

If `CountOperation` doesn't have an `order_by` field, drop `with_order_by_input` here — phase 6 adds it alongside the aggregate macros.

- [ ] **Step 4: Extend `AggregateOperation`**

Open `prax-query/src/operations/aggregate.rs`. Append the same `with_where_input` method:

```rust
    /// Apply a typed `WhereInput`. AND-composes with any previously set filter.
    pub fn with_where_input<W: crate::inputs::WhereInput<Model = M>>(mut self, w: W) -> Self {
        let f = w.into_ir();
        self.filter = self.filter.and_then(f);
        self
    }
```

`with_aggregate_input` lives in phase 6 — leave it for now.

- [ ] **Step 5: Run tests**

Run: `cargo test -p prax-query --test operation_ext_methods`
Expected: every test passes.

- [ ] **Step 6: Commit**

```bash
git add prax-query/src/operations/count.rs prax-query/src/operations/aggregate.rs prax-query/tests/operation_ext_methods.rs
git commit -m "feat(query): add with_*_input on CountOperation and AggregateOperation

Count gets with_where_input + (where supported) with_order_by_input.
Aggregate gets with_where_input only; the aggregate-spec input lands
in phase 6 with the aggregate macros."
```

---

## Task 15: Re-export at the crate root

**Files:**
- Modify: `prax-query/src/lib.rs`

- [ ] **Step 1: Re-export the public surface**

In `prax-query/src/lib.rs`, locate the existing crate-root re-exports (search for `pub use`). After them, add:

```rust
// Typed input shapes (phase 1 of the typed-query-traits work).
pub use crate::capabilities::{
    SupportsArrayOps, SupportsCaseInsensitiveMode, SupportsCorrelatedSubquery,
    SupportsFullTextSearch, SupportsGeneratedColumns, SupportsJsonPath, SupportsNestedWrites,
    SupportsRelationFilter, SupportsScalarSubqueryInSelect,
};
pub use crate::inputs::{
    // Containers.
    AggregateArgs, CountArgs, CreateArgs, CreateManyArgs, DeleteArgs, DeleteManyArgs,
    FindFirstArgs, FindManyArgs, FindUniqueArgs, GroupByArgs, UpdateArgs, UpdateManyArgs,
    UpsertArgs,
    // Scalar filters.
    BigIntFilter, BigIntNullableFilter, BoolFilter, BoolNullableFilter, BytesFilter,
    BytesNullableFilter, DateFilter, DateNullableFilter, DateTimeFilter, DateTimeNullableFilter,
    DecimalFilter, DecimalNullableFilter, EnumFilter, EnumNullableFilter, FloatFilter,
    FloatNullableFilter, IntFilter, IntNullableFilter, JsonFilter, JsonNullableFilter,
    QueryMode, ScalarFilter, StringFilter, StringNullableFilter, TimeFilter,
    TimeNullableFilter, UuidFilter, UuidNullableFilter,
    // Update wrappers.
    BigIntFieldUpdate, BigIntNullableFieldUpdate, BoolFieldUpdate, BoolNullableFieldUpdate,
    BytesFieldUpdate, BytesNullableFieldUpdate, DateTimeFieldUpdate,
    DateTimeNullableFieldUpdate, DecimalFieldUpdate, DecimalNullableFieldUpdate,
    EnumFieldUpdate, EnumNullableFieldUpdate, FloatFieldUpdate, FloatNullableFieldUpdate,
    IntFieldUpdate, IntNullableFieldUpdate, JsonFieldUpdate, JsonNullableFieldUpdate,
    StringFieldUpdate, StringNullableFieldUpdate, UuidFieldUpdate, UuidNullableFieldUpdate,
    // Relation filters + meta.
    relation::{ListRelationFilter, LowerRelationFilter, RelationMeta, SingleRelationFilter},
    // Traits.
    AggregateInput, CountSelect, CreateInput, GroupByInput, IncludeInput, OrderByInput,
    PaginationInput, SelectInput, UpdateInput, WhereInput, WhereUniqueInput,
};
```

- [ ] **Step 2: Verify docs build**

Run: `cargo doc -p prax-query --no-deps --all-features`
Expected: clean — no broken intra-doc links.

- [ ] **Step 3: Run full prax-query test suite**

Run: `cargo test -p prax-query`
Expected: every test passes.

- [ ] **Step 4: Commit**

```bash
git add prax-query/src/lib.rs
git commit -m "feat(query): re-export typed input surface at crate root

Capability marker traits, scalar/relation filter wrappers, scalar
update wrappers, per-operation Args containers, and the 10 input
traits are now reachable via prax_query::*."
```

---

## Task 16: Final workspace verification

**Files:**
- None (verification only)

- [ ] **Step 1: Format check**

Run: `cargo fmt --all -- --check`
Expected: no diff. If diff, run `cargo fmt --all` and commit the result with `style: format`.

- [ ] **Step 2: Clippy across the workspace**

Run: `cargo clippy --workspace --all-targets --all-features -- -D warnings`
Expected: zero warnings or errors.

- [ ] **Step 3: Full workspace test suite (libs only)**

Run: `cargo test --workspace --lib`
Expected: every test passes.

- [ ] **Step 4: Run the new integration tests**

Run: `cargo test -p prax-query --test inputs_scalar --test inputs_relation --test inputs_update --test inputs_args --test operation_ext_methods`
Expected: every test passes.

- [ ] **Step 5: Confirm docs still build**

Run: `cargo doc --workspace --no-deps --all-features`
Expected: clean.

- [ ] **Step 6: Phase complete — no commit needed here**

The branch is ready for PR. Phase 2 (codegen of per-model input types in `prax-codegen`) opens its own worktree off `develop` once this branch merges.

---

## Acceptance criteria

- [ ] `Filter::ScalarSubquery` lands behind `#[non_exhaustive]` with `to_sql` lowering and tests.
- [ ] `QueryEngine::in_transaction` defaults to `false` with a test.
- [ ] `prax-query/src/capabilities.rs` exposes 9 marker traits with `#[diagnostic::on_unimplemented]` messages.
- [ ] `prax-query/src/inputs/` is structured into `traits.rs`, `scalar.rs`, `scalar_update.rs`, `relation.rs`, `args.rs`, `mod.rs`.
- [ ] Every scalar type listed in the spec has a non-nullable + nullable filter wrapper with `into_filter(column)` lowering.
- [ ] `ListRelationFilter` and `SingleRelationFilter` lower to `Filter::ScalarSubquery` EXISTS / NOT EXISTS fragments via `RelationMeta`.
- [ ] Every scalar type has a `*FieldUpdate` wrapper with `From<scalar>` shortcut.
- [ ] 13 `*Args` containers exist with the field shape promised by section 3 of the spec.
- [ ] Every read/write/count/aggregate operation has `with_*_input` extension methods that consume the typed inputs, AND-composing where appropriate.
- [ ] The full public surface re-exports at the `prax-query` crate root.
- [ ] `cargo fmt`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `cargo test --workspace --lib` all pass.

---

## Self-review notes

- **Spec coverage.** Phase 1 of section 8 in the spec maps to tasks 2 (Filter variant), 3 (in_transaction), 4 (capabilities), 5–11 (input types), 12–14 (ext methods), 15 (re-exports). Computed/virtual fields (spec §9), codegen (§3), macros (§4-5), and docs (§10) are out of scope for this plan and live in phase 2+.
- **Placeholders.** Every step has either runnable commands or complete Rust source. No "TBD", no "fill in", no "similar to Task N." The `_field` skips on operations where a field doesn't yet exist are explicit, not placeholders.
- **Type consistency.** `WhereInput::into_ir(self) -> Filter` is used uniformly across tasks 5, 6, 7, 8, 9, 12, 13, 14. `IncludeInput::into_ir(self) -> Include` is used consistently. `RelationMeta` constants are named identically across tasks 9, 10, 12. The `Args` field name `r#where` is consistent across tasks 11, 12, 13.
- **YAGNI.** No phase-2+ work is preemptively wired up. The `mode: QueryMode` field is parsed but ignored at the IR level (phase 2 dialect layer consumes it). Computed/virtual field SQL emission relies on `Filter::ScalarSubquery` but the variant has no producers in this plan.
- **TDD.** Every code-emitting task follows write-test → confirm-fail → implement → confirm-pass → commit.

