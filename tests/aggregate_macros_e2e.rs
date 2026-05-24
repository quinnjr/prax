//! End-to-end coverage for phase-6 aggregate macros:
//!
//!   - `count!` select: per-column `COUNT(col)` / `COUNT(*)` emission.
//!   - `aggregate!` all five aggregate functions (SUM/AVG/MIN/MAX/COUNT).
//!   - `aggregate!` WHERE-clause filtering.
//!   - `group_by!` GROUP BY clause emission.
//!   - `group_by!` HAVING clause emission.
//!   - `aggregate!` omits unspecified aggregate blocks.
//!
//! # Design — why we use the runtime API here
//!
//! The `aggregate!` / `group_by!` / `count!` macro lowering for aggregate
//! fields is driven by schema metadata in a `LowerCtx` which requires the
//! schema to be resolved via `prax_schema!("prax/schema.prax")`.  The
//! schema-path codegen's `relation_helpers` emit `super::<Model>` paths that
//! don't resolve in the workspace-root test crate context — a latent issue
//! documented in `tests/nested_writes_e2e.rs` and `tests/computed_fields_e2e.rs`.
//!
//! Derive-style models (using `#[derive(Model)]`) avoid the relation-helper
//! path issue when no cross-model relations are declared.  However the
//! `aggregate!`, `group_by!`, and `count!` macros with select still require
//! `AggregateArgs` / `GroupByArgs` structs that are only emitted by
//! schema-path codegen, not by the derive macro.  Because of this we use the
//! runtime builder API (`AggregateOperation` / `GroupByOperation`) directly,
//! mirroring the approach taken in `computed_fields_e2e.rs`.
//!
//! All tests call `build_sql(&Postgres)` (synchronous) to materialise the
//! operation into SQL — this is the right level for "DSL → SQL emission chain"
//! tests.  No live engine is needed.
//!
//! # TODO (cleanup)
//!
//! When the schema-path `relation_helpers` path-resolution bug is fixed AND
//! the derive macro emits `AggregateArgs` structs, the macro-DSL variants of
//! these tests can be re-enabled.

#![allow(dead_code)]
#![allow(unused_imports)]

use std::borrow::Cow;
use std::sync::{Arc, Mutex};

use prax_orm::{Model, client};
use prax_query::capabilities::{SupportsNestedWrites, SupportsScalarSubqueryInSelect};
use prax_query::dialect::SqlDialect;
use prax_query::error::{QueryError, QueryResult};
use prax_query::filter::{Filter, FilterValue};
use prax_query::row::{FromRow, RowError, RowRef};
use prax_query::traits::{BoxFuture, Model as ModelTrait, QueryEngine};
use prax_query::types::{OrderBy, OrderByField, SortOrder};
use prax_query::{
    AggregateField, AggregateOperation, GroupByOperation, HavingCondition, HavingOp, having,
};

// ── RecordingEngine ───────────────────────────────────────────────────────────
//
// Verbatim copy from tests/nested_writes_e2e.rs.

type StatementLog = Arc<Mutex<Vec<(String, Vec<FilterValue>)>>>;

/// Recording mock engine for e2e tests.
#[derive(Clone)]
struct RecordingEngine {
    recorded: StatementLog,
}

impl RecordingEngine {
    fn new() -> Self {
        Self {
            recorded: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn statements(&self) -> Vec<(String, Vec<FilterValue>)> {
        self.recorded.lock().unwrap().clone()
    }
}

impl QueryEngine for RecordingEngine {
    fn dialect(&self) -> &dyn SqlDialect {
        &prax_query::dialect::Postgres
    }
    fn query_many<T: ModelTrait + FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Vec<T>>> {
        let recorded = self.recorded.clone();
        let sql = sql.to_string();
        Box::pin(async move {
            recorded.lock().unwrap().push((sql, params));
            Ok(Vec::new())
        })
    }
    fn query_one<T: ModelTrait + FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<T>> {
        let recorded = self.recorded.clone();
        let sql = sql.to_string();
        Box::pin(async move {
            recorded.lock().unwrap().push((sql, params));
            T::from_row(&CannedRow).map_err(|e| QueryError::internal(e.to_string()))
        })
    }
    fn query_optional<T: ModelTrait + FromRow + Send + 'static>(
        &self,
        _sql: &str,
        _params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Option<T>>> {
        Box::pin(async { Ok(None) })
    }
    fn execute_insert<T: ModelTrait + FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<T>> {
        let recorded = self.recorded.clone();
        let sql = sql.to_string();
        Box::pin(async move {
            recorded.lock().unwrap().push((sql, params));
            T::from_row(&CannedRow).map_err(|e| QueryError::internal(e.to_string()))
        })
    }
    fn execute_update<T: ModelTrait + FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Vec<T>>> {
        let recorded = self.recorded.clone();
        let sql = sql.to_string();
        Box::pin(async move {
            recorded.lock().unwrap().push((sql, params));
            Ok(Vec::new())
        })
    }
    fn execute_delete(
        &self,
        _sql: &str,
        _params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<u64>> {
        Box::pin(async { Ok(0) })
    }
    fn execute_raw(&self, sql: &str, params: Vec<FilterValue>) -> BoxFuture<'_, QueryResult<u64>> {
        let recorded = self.recorded.clone();
        let sql = sql.to_string();
        Box::pin(async move {
            recorded.lock().unwrap().push((sql, params));
            Ok(1)
        })
    }
    fn count(&self, _sql: &str, _params: Vec<FilterValue>) -> BoxFuture<'_, QueryResult<u64>> {
        Box::pin(async { Ok(0) })
    }
}

impl SupportsNestedWrites for RecordingEngine {}
impl SupportsScalarSubqueryInSelect for RecordingEngine {}

// Canned row — returns plausible defaults for all scalar types so that
// `T::from_row(&CannedRow)` succeeds in `execute_insert` / `query_one`.
struct CannedRow;

impl RowRef for CannedRow {
    fn get_i32(&self, _column: &str) -> Result<i32, RowError> {
        Ok(1)
    }
    fn get_i32_opt(&self, _column: &str) -> Result<Option<i32>, RowError> {
        Ok(Some(1))
    }
    fn get_i64(&self, _column: &str) -> Result<i64, RowError> {
        Ok(0)
    }
    fn get_i64_opt(&self, _column: &str) -> Result<Option<i64>, RowError> {
        Ok(None)
    }
    fn get_f64(&self, _column: &str) -> Result<f64, RowError> {
        Ok(0.0)
    }
    fn get_f64_opt(&self, _column: &str) -> Result<Option<f64>, RowError> {
        Ok(None)
    }
    fn get_bool(&self, _column: &str) -> Result<bool, RowError> {
        Ok(false)
    }
    fn get_bool_opt(&self, _column: &str) -> Result<Option<bool>, RowError> {
        Ok(None)
    }
    fn get_str(&self, _column: &str) -> Result<&str, RowError> {
        Ok("canned")
    }
    fn get_str_opt(&self, _column: &str) -> Result<Option<&str>, RowError> {
        Ok(Some("canned"))
    }
    fn get_bytes(&self, _column: &str) -> Result<&[u8], RowError> {
        Ok(b"")
    }
    fn get_bytes_opt(&self, _column: &str) -> Result<Option<&[u8]>, RowError> {
        Ok(None)
    }
}

// ── Models ────────────────────────────────────────────────────────────────────

/// Derive-style model used by all aggregate e2e tests.
///
/// No cross-model relations are declared, so the `relation_helpers`
/// schema-path bug does not fire.
#[derive(Model, Debug, Clone, Default)]
#[prax(table = "users")]
pub struct User {
    #[prax(id, auto)]
    pub id: i32,
    #[prax(unique)]
    pub email: String,
    pub team_id: i32,
    pub region: String,
    pub active: bool,
    pub views: i32,
    pub score: i32,
}

client!(User);

// ── Test 1: count! select — per-column COUNT emission ─────────────────────────

/// `count! select: { _all: true, email: true }` (runtime equiv):
/// emits `COUNT(*)` and `COUNT(email)` in SELECT.
///
/// Uses the runtime `count_column` / `count` builder methods because the
/// `count!` select macro requires codegen-emitted `UserCountSelect` structs.
#[test]
fn count_select_emits_per_column_counts() {
    let op: AggregateOperation<User, RecordingEngine> = AggregateOperation::new()
        .count() // _all: true  → COUNT(*)
        .count_column("email"); // email: true → COUNT(email)

    let (sql, params) = op.build_sql(&prax_query::dialect::Postgres);

    assert!(sql.contains("COUNT(*)"), "missing COUNT(*); got: {sql}");
    assert!(
        sql.contains("COUNT(email)"),
        "missing COUNT(email); got: {sql}"
    );
    assert!(params.is_empty(), "no params expected; got: {params:?}");
}

// ── Test 2: aggregate! — all five functions ───────────────────────────────────

/// `aggregate! { _sum: { views }, _avg: { score }, _min: { views },
///               _max: { views }, _count: { _all } }` (runtime equiv):
/// emits SUM, AVG, MIN, MAX, COUNT(*) in a single SELECT.
#[test]
fn aggregate_emits_all_five_functions() {
    let op: AggregateOperation<User, RecordingEngine> = AggregateOperation::new()
        .sum("views")
        .avg("score")
        .min("views")
        .max("views")
        .count();

    let (sql, params) = op.build_sql(&prax_query::dialect::Postgres);

    assert!(sql.contains("SUM(views)"), "missing SUM(views); got: {sql}");
    assert!(sql.contains("AVG(score)"), "missing AVG(score); got: {sql}");
    assert!(sql.contains("MIN(views)"), "missing MIN(views); got: {sql}");
    assert!(sql.contains("MAX(views)"), "missing MAX(views); got: {sql}");
    assert!(sql.contains("COUNT(*)"), "missing COUNT(*); got: {sql}");
    assert!(params.is_empty(), "no params expected; got: {params:?}");
}

// ── Test 3: aggregate! where — WHERE clause filtering ────────────────────────

/// `aggregate! { where: { active: true }, _count: { _all } }` (runtime equiv):
/// emits a WHERE clause containing the `active` column and records the param.
#[test]
fn aggregate_where_filters_underlying_select() {
    let op: AggregateOperation<User, RecordingEngine> = AggregateOperation::new()
        .count()
        .r#where(Filter::Equals("active".into(), FilterValue::Bool(true)));

    let (sql, params) = op.build_sql(&prax_query::dialect::Postgres);

    assert!(sql.contains("WHERE"), "missing WHERE clause; got: {sql}");
    assert!(
        sql.contains("active"),
        "missing `active` column in WHERE; got: {sql}"
    );
    assert_eq!(params.len(), 1, "expected exactly 1 param; got: {params:?}");
    assert_eq!(
        params[0],
        FilterValue::Bool(true),
        "param should be Bool(true); got: {:?}",
        params[0]
    );
}

// ── Test 4: group_by! — GROUP BY clause ──────────────────────────────────────

/// `group_by!(c.user, { by: [team_id, region], _count: { _all } })` (runtime equiv):
/// emits `GROUP BY team_id, region` and `COUNT(*)` in SELECT.
#[test]
fn group_by_emits_group_by_clause() {
    let op: GroupByOperation<User, RecordingEngine> =
        GroupByOperation::new(vec!["team_id".into(), "region".into()]).count();

    let (sql, params) = op.build_sql(&prax_query::dialect::Postgres);

    assert!(
        sql.contains("GROUP BY team_id, region"),
        "missing GROUP BY clause; got: {sql}"
    );
    assert!(sql.contains("COUNT(*)"), "missing COUNT(*); got: {sql}");
    assert!(params.is_empty(), "no params expected; got: {params:?}");
}

// ── Test 5: group_by! having — HAVING clause ─────────────────────────────────

/// `group_by!(c.user, { by: [team_id], _count: { _all },
///             having: { _count: { _all: { gt: 5 } } } })` (runtime equiv):
/// emits `HAVING COUNT(*) > 5`.
///
/// Note: HAVING thresholds are inlined as float literals, not parameterized.
#[test]
fn group_by_having_emits_having_clause() {
    let op: GroupByOperation<User, RecordingEngine> = GroupByOperation::new(vec!["team_id".into()])
        .count()
        .having(having::count_gt(5.0));

    let (sql, _params) = op.build_sql(&prax_query::dialect::Postgres);

    assert!(sql.contains("HAVING"), "missing HAVING clause; got: {sql}");
    assert!(
        sql.contains("COUNT(*) > 5"),
        "missing `COUNT(*) > 5` in HAVING; got: {sql}"
    );
}

// ── Test 6: aggregate! — omits unspecified blocks ────────────────────────────

/// `aggregate! { _sum: { views } }` (runtime equiv):
/// emits `SUM(views)` but NOT `AVG`, `MIN`, `MAX`, or `COUNT`.
#[test]
fn aggregate_omits_unspecified_blocks() {
    let op: AggregateOperation<User, RecordingEngine> = AggregateOperation::new().sum("views");

    let (sql, params) = op.build_sql(&prax_query::dialect::Postgres);

    assert!(sql.contains("SUM(views)"), "missing SUM(views); got: {sql}");
    assert!(!sql.contains("AVG"), "unexpected AVG in SQL; got: {sql}");
    assert!(!sql.contains("MIN"), "unexpected MIN in SQL; got: {sql}");
    assert!(!sql.contains("MAX"), "unexpected MAX in SQL; got: {sql}");
    assert!(
        !sql.contains("COUNT"),
        "unexpected COUNT in SQL; got: {sql}"
    );
    assert!(params.is_empty(), "no params expected; got: {params:?}");
}
