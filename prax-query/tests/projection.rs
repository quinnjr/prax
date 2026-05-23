//! ScalarProjection SQL emission tests.

use prax_query::dialect::Postgres;
use prax_query::filter::{Filter, FilterValue};
use prax_query::operations::FindManyOperation;
use prax_query::projection::ScalarProjection;
use prax_query::traits::{BoxFuture, Model, QueryEngine};

// ---------------------------------------------------------------------------
// Minimal mock infrastructure (mirrors operation_ext_methods.rs)
// ---------------------------------------------------------------------------

struct Users;
impl Model for Users {
    const MODEL_NAME: &'static str = "Users";
    const TABLE_NAME: &'static str = "users";
    const PRIMARY_KEY: &'static [&'static str] = &["id"];
    const COLUMNS: &'static [&'static str] = &["id", "name"];
}
impl prax_query::row::FromRow for Users {
    fn from_row(_row: &impl prax_query::row::RowRef) -> Result<Self, prax_query::row::RowError> {
        Ok(Users)
    }
}

#[derive(Clone)]
struct NoopEngine;
impl QueryEngine for NoopEngine {
    fn query_many<T: Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        _: &str,
        _: Vec<FilterValue>,
    ) -> BoxFuture<'_, prax_query::error::QueryResult<Vec<T>>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn query_one<T: Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        _: &str,
        _: Vec<FilterValue>,
    ) -> BoxFuture<'_, prax_query::error::QueryResult<T>> {
        Box::pin(async { Err(prax_query::error::QueryError::not_found("t")) })
    }
    fn query_optional<T: Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        _: &str,
        _: Vec<FilterValue>,
    ) -> BoxFuture<'_, prax_query::error::QueryResult<Option<T>>> {
        Box::pin(async { Ok(None) })
    }
    fn execute_insert<T: Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        _: &str,
        _: Vec<FilterValue>,
    ) -> BoxFuture<'_, prax_query::error::QueryResult<T>> {
        Box::pin(async { Err(prax_query::error::QueryError::not_found("t")) })
    }
    fn execute_update<T: Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        _: &str,
        _: Vec<FilterValue>,
    ) -> BoxFuture<'_, prax_query::error::QueryResult<Vec<T>>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn execute_delete(
        &self,
        _: &str,
        _: Vec<FilterValue>,
    ) -> BoxFuture<'_, prax_query::error::QueryResult<u64>> {
        Box::pin(async { Ok(0) })
    }
    fn execute_raw(
        &self,
        _: &str,
        _: Vec<FilterValue>,
    ) -> BoxFuture<'_, prax_query::error::QueryResult<u64>> {
        Box::pin(async { Ok(0) })
    }
    fn count(
        &self,
        _: &str,
        _: Vec<FilterValue>,
    ) -> BoxFuture<'_, prax_query::error::QueryResult<u64>> {
        Box::pin(async { Ok(0) })
    }
}

// ---------------------------------------------------------------------------
// Helper: build a SELECT for `Users` with `extra_projections` only (no WHERE)
// ---------------------------------------------------------------------------
fn build_test_select_postgres(projections: Vec<ScalarProjection>) -> String {
    let mut op = FindManyOperation::<NoopEngine, Users>::new(NoopEngine);
    op.extra_projections = projections;
    let (sql, _params) = op.build_sql(&Postgres);
    sql
}

fn build_test_select_with_where(
    proj: ScalarProjection,
    where_val: FilterValue,
) -> (String, Vec<FilterValue>) {
    let mut op = FindManyOperation::<NoopEngine, Users>::new(NoopEngine)
        .r#where(Filter::Equals("id".into(), where_val));
    op.extra_projections = vec![proj];
    op.build_sql(&Postgres)
}

fn build_test_select_postgres_with_params(
    projections: Vec<ScalarProjection>,
) -> (String, Vec<FilterValue>) {
    let mut op = FindManyOperation::<NoopEngine, Users>::new(NoopEngine);
    op.extra_projections = projections;
    op.build_sql(&Postgres)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn single_projection_appends_alias_to_select() {
    // SELECT *, (SELECT COUNT(*) FROM "posts" WHERE ...) AS "_count_posts" FROM "users"
    let proj = ScalarProjection::new(
        "SELECT COUNT(*) FROM \"posts\" WHERE \"posts\".\"author_id\" = \"users\".\"id\"",
        vec![],
        "_count_posts",
    );
    let sql = build_test_select_postgres(vec![proj]);
    assert!(
        sql.contains(
            ", (SELECT COUNT(*) FROM \"posts\" WHERE \"posts\".\"author_id\" = \"users\".\"id\") AS \"_count_posts\""
        ),
        "got: {sql}"
    );
}

#[test]
fn placeholder_renumbers_against_running_counter() {
    // The WHERE clause has id = $1 (one param). The projection comes first
    // in the SELECT and its param should be $1, pushing the WHERE param to $2.
    // Wait — projection params come first: proj gets $1, WHERE gets $2.
    let proj = ScalarProjection::new(
        "SELECT COUNT(*) FROM \"posts\" WHERE \"posts\".\"views\" > {0}",
        vec![FilterValue::Int(100)],
        "_high_view_count",
    );
    let (sql, params) = build_test_select_with_where(proj, FilterValue::Int(7));
    // projection param is emitted first → $1; WHERE param is $2
    assert!(sql.contains("> $1)"), "projection placeholder wrong: {sql}");
    assert!(
        sql.contains("WHERE") && sql.contains("$2"),
        "where placeholder wrong: {sql}"
    );
    assert_eq!(params, vec![FilterValue::Int(100), FilterValue::Int(7)]);
}

#[test]
fn multiple_projections_emit_in_order_with_correct_placeholders() {
    let p1 = ScalarProjection::new(
        "SELECT COUNT(*) FROM \"posts\" WHERE \"posts\".\"views\" > {0}",
        vec![FilterValue::Int(50)],
        "_views_above_50",
    );
    let p2 = ScalarProjection::new(
        "SELECT COUNT(*) FROM \"comments\" WHERE \"comments\".\"author_id\" = \"users\".\"id\" AND \"comments\".\"score\" > {0}",
        vec![FilterValue::Int(10)],
        "_high_score_comment_count",
    );
    let (sql, params) = build_test_select_postgres_with_params(vec![p1, p2]);
    assert!(
        sql.contains("> $1)"),
        "first projection placeholder wrong: {sql}"
    );
    assert!(
        sql.contains("> $2)"),
        "second projection placeholder wrong: {sql}"
    );
    assert!(
        sql.find("_views_above_50").unwrap() < sql.find("_high_score_comment_count").unwrap(),
        "projections out of order: {sql}"
    );
    assert_eq!(params, vec![FilterValue::Int(50), FilterValue::Int(10)]);
}

#[test]
fn no_extra_projections_emits_plain_select() {
    let op = FindManyOperation::<NoopEngine, Users>::new(NoopEngine);
    let (sql, params) = op.build_sql(&Postgres);
    assert_eq!(sql, "SELECT * FROM users");
    assert!(params.is_empty());
}
