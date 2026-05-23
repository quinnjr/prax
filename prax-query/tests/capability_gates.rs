//! Capability-gate tests for `SupportsScalarSubqueryInSelect`.
//!
//! Checks that `with_scalar_projection` is callable on an engine that impl's
//! the trait and that the projection appears in the emitted SQL.
//!
//! Negative assertions (MongoDB / ScyllaDB / Cassandra do *not* impl the
//! trait) are deferred to Task 15 trybuild diagnostics.

use prax_query::capabilities::SupportsScalarSubqueryInSelect;
use prax_query::dialect::Postgres;
use prax_query::error::QueryResult;
use prax_query::filter::FilterValue;
use prax_query::operations::{FindFirstOperation, FindManyOperation, FindUniqueOperation};
use prax_query::projection::ScalarProjection;
use prax_query::traits::{BoxFuture, Model, QueryEngine};

// ---------------------------------------------------------------------------
// Minimal capable engine stub
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct CapableEngine;

impl QueryEngine for CapableEngine {
    fn query_many<T: Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        _: &str,
        _: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Vec<T>>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn query_one<T: Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        _: &str,
        _: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<T>> {
        Box::pin(async { Err(prax_query::error::QueryError::not_found("t")) })
    }
    fn query_optional<T: Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        _: &str,
        _: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Option<T>>> {
        Box::pin(async { Ok(None) })
    }
    fn execute_insert<T: Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        _: &str,
        _: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<T>> {
        Box::pin(async { Err(prax_query::error::QueryError::not_found("t")) })
    }
    fn execute_update<T: Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        _: &str,
        _: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Vec<T>>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn execute_delete(&self, _: &str, _: Vec<FilterValue>) -> BoxFuture<'_, QueryResult<u64>> {
        Box::pin(async { Ok(0) })
    }
    fn execute_raw(&self, _: &str, _: Vec<FilterValue>) -> BoxFuture<'_, QueryResult<u64>> {
        Box::pin(async { Ok(0) })
    }
    fn count(&self, _: &str, _: Vec<FilterValue>) -> BoxFuture<'_, QueryResult<u64>> {
        Box::pin(async { Ok(0) })
    }
}

// Opt-in to scalar subquery support.
impl SupportsScalarSubqueryInSelect for CapableEngine {}

// ---------------------------------------------------------------------------
// Minimal model stub
// ---------------------------------------------------------------------------

struct Users;
impl Model for Users {
    const MODEL_NAME: &'static str = "Users";
    const TABLE_NAME: &'static str = "users";
    const PRIMARY_KEY: &'static [&'static str] = &["id"];
    const COLUMNS: &'static [&'static str] = &["id", "name"];
}
impl prax_query::row::FromRow for Users {
    fn from_row(_: &impl prax_query::row::RowRef) -> Result<Self, prax_query::row::RowError> {
        Ok(Users)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn find_many_with_scalar_projection_emits_subquery() {
    let proj = ScalarProjection::new(
        "SELECT COUNT(*) FROM posts WHERE posts.author_id = users.id",
        vec![],
        "_count_posts",
    );
    let op =
        FindManyOperation::<CapableEngine, Users>::new(CapableEngine).with_scalar_projection(proj);
    let (sql, _) = op.build_sql(&Postgres);
    assert!(
        sql.contains("_count_posts"),
        "expected projection alias in SQL, got: {sql}"
    );
    assert!(
        sql.contains("COUNT(*)"),
        "expected COUNT(*) subquery in SQL, got: {sql}"
    );
}

#[test]
fn find_first_with_scalar_projection_emits_subquery() {
    let proj = ScalarProjection::new(
        "SELECT COUNT(*) FROM posts WHERE posts.author_id = users.id",
        vec![],
        "_count_posts",
    );
    let op =
        FindFirstOperation::<CapableEngine, Users>::new(CapableEngine).with_scalar_projection(proj);
    let (sql, _) = op.build_sql(&Postgres);
    assert!(
        sql.contains("_count_posts"),
        "expected projection alias in SQL, got: {sql}"
    );
}

#[test]
fn find_unique_with_scalar_projection_emits_subquery() {
    let proj = ScalarProjection::new(
        "SELECT COUNT(*) FROM posts WHERE posts.author_id = users.id",
        vec![],
        "_count_posts",
    );
    let op = FindUniqueOperation::<CapableEngine, Users>::new(CapableEngine)
        .with_scalar_projection(proj);
    let (sql, _) = op.build_sql(&Postgres);
    assert!(
        sql.contains("_count_posts"),
        "expected projection alias in SQL, got: {sql}"
    );
}
