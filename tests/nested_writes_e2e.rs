//! Smoke tests for phase-5b nested-write runtime wiring.
//!
//! These tests exercise `CreateOperation::with(...)` against a
//! recording mock engine to verify that the parent INSERT, every
//! `NestedWriteOp::Create` child INSERT, and every
//! `NestedWriteOp::Connect` child UPDATE land in a single
//! transaction sequence.
//!
//! Schema source: derive-style models defined in this file, mirroring
//! `nested_write_postgres.rs` but pointed at a recording engine
//! instead of a live Postgres pool. The workspace `prax/schema.prax`
//! does not declare the `User <-> Post` relation because the
//! schema-path codegen's `relation_helpers` emits paths that don't
//! resolve in the workspace-root crate — a latent issue separate from
//! phase 5b. Derive-style accessors avoid that path entirely.
//!
//! `create!` macro coverage of nested writes is covered by the
//! lowering unit tests in
//! `prax-codegen/src/macros/lower/data_relation.rs`; the schema-aware
//! macro path will be exercised end-to-end once the schema-path
//! relation_helpers bug is fixed (deferred).

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use prax_orm::{Model, client};
use prax_query::capabilities::SupportsNestedWrites;
use prax_query::dialect::SqlDialect;
use prax_query::error::{QueryError, QueryResult};
use prax_query::filter::FilterValue;
use prax_query::nested::NestedWriteOp;
use prax_query::row::{FromRow, RowError, RowRef};
use prax_query::traits::{BoxFuture, Model as ModelTrait, ModelWithPk, QueryEngine};

/// Captured (sql, params) entries from the mock engine.
type StatementLog = Arc<Mutex<Vec<(String, Vec<FilterValue>)>>>;

/// Recording mock engine that also impls `SupportsNestedWrites` so
/// the `.with(...)` capability gate compiles.
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
        _sql: &str,
        _params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Vec<T>>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn query_one<T: ModelTrait + FromRow + Send + 'static>(
        &self,
        _sql: &str,
        _params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<T>> {
        Box::pin(async { Err(QueryError::not_found("test")) })
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
        _sql: &str,
        _params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Vec<T>>> {
        Box::pin(async { Ok(Vec::new()) })
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
        let sql_string = sql.to_string();
        // For batched-Connect UPDATEs of the form
        //   UPDATE t SET fk = $1 WHERE pk IN ($2, $3, ...)
        // the affected-rows check expects N rows = pks.len() - 1
        // (params[0] is the parent FK; the rest are child PKs).
        let affected = if sql.contains(" IN (") {
            (params.len() as u64).saturating_sub(1)
        } else {
            1
        };
        Box::pin(async move {
            recorded.lock().unwrap().push((sql_string, params));
            Ok(affected)
        })
    }
    fn count(&self, _sql: &str, _params: Vec<FilterValue>) -> BoxFuture<'_, QueryResult<u64>> {
        Box::pin(async { Ok(0) })
    }
}

impl SupportsNestedWrites for RecordingEngine {}

struct CannedRow;

impl RowRef for CannedRow {
    fn get_i32(&self, _column: &str) -> Result<i32, RowError> {
        Ok(7)
    }
    fn get_i32_opt(&self, _column: &str) -> Result<Option<i32>, RowError> {
        Ok(Some(7))
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

#[derive(Model, Debug, Clone)]
#[prax(table = "posts")]
pub struct Post {
    #[prax(id, auto)]
    pub id: i32,
    pub title: String,
    pub author_id: i32,
}

#[derive(Model, Debug, Clone)]
#[prax(table = "users")]
pub struct User {
    #[prax(id, auto)]
    pub id: i32,
    #[prax(unique)]
    pub email: String,
    #[prax(relation(target = "Post", foreign_key = "author_id"))]
    pub posts: Vec<Post>,
}

client!(User, Post);

#[tokio::test]
async fn nested_create_emits_parent_insert_then_child_inserts() {
    let engine = RecordingEngine::new();
    let c = prax_orm::PraxClient::new(engine.clone());

    let _u: User = c
        .user()
        .create()
        .set("email", "a@x.com")
        .with(user::posts::create(vec![
            vec![("title".into(), FilterValue::String("p1".into()))],
            vec![("title".into(), FilterValue::String("p2".into()))],
        ]))
        .exec()
        .await
        .expect("create + nested children");

    let stmts = engine.statements();
    // Multi-VALUES batching: parent INSERT + one multi-row child
    // INSERT carrying both posts. See NestedWriteOp::Create::execute.
    assert_eq!(
        stmts.len(),
        2,
        "parent + one batched child INSERT; got {stmts:#?}"
    );
    assert!(stmts[0].0.contains("INSERT INTO"));
    assert!(stmts[0].0.contains("users"));

    let (child_sql, child_params) = &stmts[1];
    assert!(child_sql.contains("INSERT INTO"));
    assert!(child_sql.contains("posts"));
    assert!(child_sql.contains("author_id"));
    // Two rows worth of placeholders + parent PK per row.
    assert!(
        child_sql.contains("),"),
        "expected multi-VALUES form; got {child_sql}"
    );
    assert_eq!(
        child_params.len(),
        4,
        "two rows x (title + FK) params; got {child_params:?}"
    );
}

#[tokio::test]
async fn nested_connect_emits_parent_insert_then_update() {
    let engine = RecordingEngine::new();
    let c = prax_orm::PraxClient::new(engine.clone());

    let _u: User = c
        .user()
        .create()
        .set("email", "a@x.com")
        .with(user::posts::connect(FilterValue::Int(42)))
        .exec()
        .await
        .expect("create + nested connect");

    let stmts = engine.statements();
    assert_eq!(
        stmts.len(),
        2,
        "parent insert + child update; got {stmts:#?}"
    );
    assert!(stmts[0].0.contains("INSERT INTO"));

    let (update_sql, update_params) = &stmts[1];
    assert!(update_sql.contains("UPDATE"), "got: {update_sql}");
    assert!(update_sql.contains("posts"), "got: {update_sql}");
    assert!(update_sql.contains("author_id"), "got: {update_sql}");
    assert!(update_sql.contains("WHERE"), "got: {update_sql}");
    assert!(
        update_params.contains(&FilterValue::Int(42)),
        "expected connect PK 42 in params, got {update_params:?}",
    );
}

#[tokio::test]
async fn mixed_create_and_connect_in_order() {
    let engine = RecordingEngine::new();
    let c = prax_orm::PraxClient::new(engine.clone());

    let _u: User = c
        .user()
        .create()
        .set("email", "a@x.com")
        .with(user::posts::create(vec![vec![(
            "title".into(),
            FilterValue::String("new".into()),
        )]]))
        .with(user::posts::connect(FilterValue::Int(99)))
        .exec()
        .await
        .expect("create + nested mixed");

    let stmts = engine.statements();
    assert_eq!(stmts.len(), 3, "got {stmts:#?}");
    assert!(stmts[0].0.contains("INSERT INTO"));
    assert!(
        stmts[1].0.contains("INSERT INTO"),
        "child create: {}",
        stmts[1].0
    );
    assert!(
        stmts[2].0.contains("UPDATE"),
        "child connect: {}",
        stmts[2].0
    );
}

#[tokio::test]
async fn nested_connect_single_passes_through_unchanged() {
    let engine = RecordingEngine::new();
    let c = prax_orm::PraxClient::new(engine.clone());

    let _u: User = c
        .user()
        .create()
        .set("email", "a@x.com")
        .with(user::posts::connect(FilterValue::Int(42)))
        .exec()
        .await
        .expect("create + single connect");

    let stmts = engine.statements();
    assert_eq!(stmts.len(), 2, "parent insert + single connect update");
    let (sql, params) = &stmts[1];
    assert!(sql.contains("UPDATE"), "got: {sql}");
    assert!(
        !sql.contains(" IN ("),
        "single Connect must not batch into IN-list: {sql}"
    );
    assert_eq!(params.len(), 2, "FK + single child PK");
}

#[tokio::test]
async fn nested_connect_pair_same_target_is_batched() {
    let engine = RecordingEngine::new();
    let c = prax_orm::PraxClient::new(engine.clone());

    let _u: User = c
        .user()
        .create()
        .set("email", "a@x.com")
        .with(user::posts::connect(FilterValue::Int(10)))
        .with(user::posts::connect(FilterValue::Int(11)))
        .exec()
        .await
        .expect("create + two connects same target");

    let stmts = engine.statements();
    assert_eq!(
        stmts.len(),
        2,
        "parent insert + one batched UPDATE; got {stmts:#?}"
    );
    let (sql, params) = &stmts[1];
    assert!(sql.contains("UPDATE"), "got: {sql}");
    assert!(
        sql.contains(" IN ("),
        "two Connects must batch into IN-list: {sql}"
    );
    assert_eq!(params.len(), 3, "FK + two child PKs");
    assert!(params.contains(&FilterValue::Int(10)));
    assert!(params.contains(&FilterValue::Int(11)));
}

#[tokio::test]
async fn nested_connect_then_create_then_connect_no_cross_batching() {
    let engine = RecordingEngine::new();
    let c = prax_orm::PraxClient::new(engine.clone());

    let _u: User = c
        .user()
        .create()
        .set("email", "a@x.com")
        .with(user::posts::connect(FilterValue::Int(10)))
        .with(user::posts::create(vec![vec![(
            "title".into(),
            FilterValue::String("p".into()),
        )]]))
        .with(user::posts::connect(FilterValue::Int(11)))
        .exec()
        .await
        .expect("connect, create, connect");

    let stmts = engine.statements();
    // parent INSERT + first UPDATE + child INSERT + second UPDATE
    assert_eq!(stmts.len(), 4, "got {stmts:#?}");
    assert!(stmts[1].0.contains("UPDATE"));
    assert!(
        !stmts[1].0.contains(" IN ("),
        "first connect must stay single: {}",
        stmts[1].0
    );
    assert!(stmts[2].0.contains("INSERT INTO"));
    assert!(stmts[3].0.contains("UPDATE"));
    assert!(
        !stmts[3].0.contains(" IN ("),
        "second connect must stay single: {}",
        stmts[3].0
    );
}

#[tokio::test]
async fn nested_connects_different_targets_are_not_batched() {
    let engine = RecordingEngine::new();
    let c = prax_orm::PraxClient::new(engine.clone());

    let _u: User = c
        .user()
        .create()
        .set("email", "a@x.com")
        .with(NestedWriteOp::Connect {
            relation: "posts",
            target_table: "posts",
            foreign_key: "author_id",
            target_pk: "id",
            pk: FilterValue::Int(10),
        })
        .with(NestedWriteOp::Connect {
            relation: "comments",
            target_table: "comments",
            foreign_key: "author_id",
            target_pk: "id",
            pk: FilterValue::Int(20),
        })
        .exec()
        .await
        .expect("two connects to different tables");

    let stmts = engine.statements();
    assert_eq!(
        stmts.len(),
        3,
        "parent insert + two separate UPDATEs; got {stmts:#?}"
    );
    assert!(stmts[1].0.contains("UPDATE"));
    assert!(stmts[1].0.contains("posts"), "first: {}", stmts[1].0);
    assert!(
        !stmts[1].0.contains(" IN ("),
        "must not batch across targets: {}",
        stmts[1].0
    );
    assert!(stmts[2].0.contains("UPDATE"));
    assert!(stmts[2].0.contains("comments"), "second: {}", stmts[2].0);
    assert!(
        !stmts[2].0.contains(" IN ("),
        "must not batch across targets: {}",
        stmts[2].0
    );
}

#[tokio::test]
async fn multi_connect_same_relation_batches_into_single_update() {
    let engine = RecordingEngine::new();
    let c = prax_orm::PraxClient::new(engine.clone());

    let _u: User = c
        .user()
        .create()
        .set("email", "owner@x.com")
        .with(user::posts::connect(FilterValue::Int(1)))
        .with(user::posts::connect(FilterValue::Int(2)))
        .with(user::posts::connect(FilterValue::Int(3)))
        .exec()
        .await
        .expect("create + three connects to same relation");

    let stmts = engine.statements();
    assert_eq!(
        stmts.len(),
        2,
        "parent insert + one batched UPDATE; got {stmts:#?}"
    );
    let (sql, params) = &stmts[1];
    assert!(sql.contains("UPDATE"), "got: {sql}");
    assert!(sql.contains("posts"), "got: {sql}");
    assert!(sql.contains("author_id"), "got: {sql}");
    assert!(sql.contains(" IN ("), "expected batched IN-list: {sql}");
    assert!(
        sql.contains("$2"),
        "expected three positional pk placeholders: {sql}"
    );
    assert!(
        sql.contains("$3"),
        "expected three positional pk placeholders: {sql}"
    );
    assert!(
        sql.contains("$4"),
        "expected three positional pk placeholders: {sql}"
    );
    assert_eq!(params.len(), 4, "FK + three child PKs");
    assert!(params.contains(&FilterValue::Int(1)));
    assert!(params.contains(&FilterValue::Int(2)));
    assert!(params.contains(&FilterValue::Int(3)));
}

/// Compile-only assertion: `NestedWriteOp::Connect` carries the
/// per-relation metadata so the executor can build its UPDATE without
/// a runtime lookup.
#[test]
fn connect_op_carries_relation_metadata() {
    let op = user::posts::connect(FilterValue::Int(1));
    match op {
        NestedWriteOp::Connect {
            target_table,
            foreign_key,
            target_pk,
            ..
        } => {
            assert_eq!(target_table, "posts");
            assert_eq!(foreign_key, "author_id");
            assert_eq!(target_pk, "id");
        }
        _ => panic!("expected Connect variant"),
    }
}

#[test]
fn model_with_pk_compiles_for_fixture() {
    // Ensures the derive-emitted ModelWithPk impl wires through —
    // CreateOperation::exec()'s slow path requires this bound.
    let p = Post {
        id: 5,
        title: "t".into(),
        author_id: 1,
    };
    assert_eq!(p.pk_value(), FilterValue::Int(5));
}
