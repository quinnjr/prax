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
#[diagnostic::on_unimplemented(message = "the engine `{Self}` does not support full-text search")]
pub trait SupportsFullTextSearch: QueryEngine {}

/// Engine supports native array column operators (`contains`, `overlaps`, ...).
#[diagnostic::on_unimplemented(message = "the engine `{Self}` does not support array operators")]
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
        fn execute_delete(
            &self,
            _sql: &str,
            _params: Vec<crate::filter::FilterValue>,
        ) -> crate::traits::BoxFuture<'_, crate::error::QueryResult<u64>> {
            Box::pin(async { Ok(0) })
        }
        fn execute_raw(
            &self,
            _sql: &str,
            _params: Vec<crate::filter::FilterValue>,
        ) -> crate::traits::BoxFuture<'_, crate::error::QueryResult<u64>> {
            Box::pin(async { Ok(0) })
        }
        fn count(
            &self,
            _sql: &str,
            _params: Vec<crate::filter::FilterValue>,
        ) -> crate::traits::BoxFuture<'_, crate::error::QueryResult<u64>> {
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
