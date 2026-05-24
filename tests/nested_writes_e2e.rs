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
use prax_query::filter::{Filter, FilterValue};
use prax_query::nested::NestedWriteOp;
use prax_query::row::{FromRow, RowError, RowRef};
use prax_query::traits::{BoxFuture, Model as ModelTrait, ModelWithPk, QueryEngine};

/// Captured (sql, params) entries from the mock engine.
type StatementLog = Arc<Mutex<Vec<(String, Vec<FilterValue>)>>>;

/// Which dialect the recording engine advertises. Drives the
/// upsert dispatch in `NestedWriteOp::Upsert::execute` (Postgres →
/// single-statement `ON CONFLICT`, MSSQL → two-statement fallback).
#[derive(Clone, Copy)]
enum DialectKind {
    Postgres,
    Mssql,
}

/// Recording mock engine that also impls `SupportsNestedWrites` so
/// the `.with(...)` capability gate compiles.
#[derive(Clone)]
struct RecordingEngine {
    recorded: StatementLog,
    /// Optional override sequence for execute_raw's affected-rows
    /// return value. Empty → fall back to the IN-list heuristic.
    /// Used by the upsert insert-path test to force the UPDATE phase
    /// to report 0 affected rows so the executor proceeds to INSERT.
    affected_override: Arc<Mutex<Vec<u64>>>,
    dialect_kind: DialectKind,
}

impl RecordingEngine {
    fn new() -> Self {
        Self {
            recorded: Arc::new(Mutex::new(Vec::new())),
            affected_override: Arc::new(Mutex::new(Vec::new())),
            dialect_kind: DialectKind::Postgres,
        }
    }

    /// Build an engine that returns each entry of `seq` from successive
    /// `execute_raw` calls, in order. Once exhausted, falls back to the
    /// default heuristic (IN-list → child PK count, else 1).
    fn with_affected(seq: Vec<u64>) -> Self {
        let mut rev = seq;
        rev.reverse();
        Self {
            recorded: Arc::new(Mutex::new(Vec::new())),
            affected_override: Arc::new(Mutex::new(rev)),
            dialect_kind: DialectKind::Postgres,
        }
    }

    /// Engine that reports the MSSQL dialect. Used to exercise the
    /// two-statement upsert fallback (MSSQL's `upsert_clause` returns
    /// empty).
    fn new_mssql() -> Self {
        let mut e = Self::new();
        e.dialect_kind = DialectKind::Mssql;
        e
    }

    fn with_affected_mssql(seq: Vec<u64>) -> Self {
        let mut e = Self::with_affected(seq);
        e.dialect_kind = DialectKind::Mssql;
        e
    }

    fn statements(&self) -> Vec<(String, Vec<FilterValue>)> {
        self.recorded.lock().unwrap().clone()
    }
}

impl QueryEngine for RecordingEngine {
    fn dialect(&self) -> &dyn SqlDialect {
        match self.dialect_kind {
            DialectKind::Postgres => &prax_query::dialect::Postgres,
            DialectKind::Mssql => &prax_query::dialect::Mssql,
        }
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
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<T>> {
        // Phase 5c's nested-upsert wiring rebuilds the affected row via
        // SELECT after the UPDATE branch fires. Record + synthesise so
        // the post-update path can complete in tests.
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
        // Phase 5c's `update!` nested-write wiring routes the parent
        // UPDATE through `execute_update`. Record so tests can assert
        // the full statement sequence, then return an empty vec (the
        // tests under test don't read rows back).
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
        let affected_override = self.affected_override.clone();
        let sql_string = sql.to_string();
        // For batched-Connect UPDATEs of the form
        //   UPDATE t SET fk = $1 WHERE pk IN ($2, $3, ...)
        // the affected-rows check expects N rows = pks.len() - 1
        // (params[0] is the parent FK; the rest are child PKs).
        let affected_default = if sql.contains(" IN (") {
            (params.len() as u64).saturating_sub(1)
        } else {
            1
        };
        Box::pin(async move {
            recorded.lock().unwrap().push((sql_string, params));
            let next = affected_override
                .lock()
                .unwrap()
                .pop()
                .unwrap_or(affected_default);
            Ok(next)
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

#[tokio::test]
async fn nested_disconnect_emits_parent_insert_then_update_set_null() {
    let engine = RecordingEngine::new();
    let c = prax_orm::PraxClient::new(engine.clone());

    let _u: User = c
        .user()
        .create()
        .set("email", "owner@x.com")
        .with(NestedWriteOp::Disconnect {
            relation: "posts",
            target_table: "posts",
            foreign_key: "author_id",
            target_pk: "id",
            pk: FilterValue::Int(42),
        })
        .exec()
        .await
        .expect("create + nested disconnect");

    let stmts = engine.statements();
    assert_eq!(stmts.len(), 2, "got {stmts:#?}");
    assert!(stmts[0].0.contains("INSERT INTO"));
    let (sql, params) = &stmts[1];
    assert!(sql.contains("UPDATE"), "got: {sql}");
    assert!(sql.contains("author_id"), "got: {sql}");
    assert!(sql.contains("NULL"), "got: {sql}");
    assert_eq!(params, &vec![FilterValue::Int(42)]);
}

#[tokio::test]
async fn nested_delete_emits_parent_insert_then_delete_where_pk() {
    let engine = RecordingEngine::new();
    let c = prax_orm::PraxClient::new(engine.clone());

    let _u: User = c
        .user()
        .create()
        .set("email", "owner@x.com")
        .with(NestedWriteOp::Delete {
            relation: "posts",
            target_table: "posts",
            target_pk: "id",
            pk: FilterValue::Int(7),
        })
        .exec()
        .await
        .expect("create + nested delete");

    let stmts = engine.statements();
    assert_eq!(stmts.len(), 2, "got {stmts:#?}");
    let (sql, params) = &stmts[1];
    assert!(sql.contains("DELETE FROM"), "got: {sql}");
    assert!(sql.contains("posts"), "got: {sql}");
    assert!(sql.contains("WHERE"), "got: {sql}");
    assert_eq!(params, &vec![FilterValue::Int(7)]);
}

#[tokio::test]
async fn nested_delete_many_with_filter_emits_fk_and_filter_clause() {
    use prax_query::filter::Filter;
    let engine = RecordingEngine::new();
    let c = prax_orm::PraxClient::new(engine.clone());

    let _u: User = c
        .user()
        .create()
        .set("email", "owner@x.com")
        .with(NestedWriteOp::DeleteMany {
            relation: "posts",
            target_table: "posts",
            foreign_key: "author_id",
            filter: Filter::Equals("published".into(), FilterValue::Bool(false)),
        })
        .exec()
        .await
        .expect("create + nested delete_many");

    let stmts = engine.statements();
    assert_eq!(stmts.len(), 2, "got {stmts:#?}");
    let (sql, params) = &stmts[1];
    assert!(sql.contains("DELETE FROM"), "got: {sql}");
    assert!(sql.contains("author_id"), "got: {sql}");
    assert!(sql.contains("AND"), "got: {sql}");
    assert!(sql.contains("published"), "got: {sql}");
    assert_eq!(params.len(), 2, "FK + filter param");
    assert!(matches!(params[1], FilterValue::Bool(false)));
}

#[tokio::test]
async fn nested_create_plus_disconnect_plus_delete_in_one_transaction() {
    let engine = RecordingEngine::new();
    let c = prax_orm::PraxClient::new(engine.clone());

    let _u: User = c
        .user()
        .create()
        .set("email", "owner@x.com")
        .with(user::posts::create(vec![vec![(
            "title".into(),
            FilterValue::String("new".into()),
        )]]))
        .with(NestedWriteOp::Disconnect {
            relation: "posts",
            target_table: "posts",
            foreign_key: "author_id",
            target_pk: "id",
            pk: FilterValue::Int(100),
        })
        .with(NestedWriteOp::Delete {
            relation: "posts",
            target_table: "posts",
            target_pk: "id",
            pk: FilterValue::Int(200),
        })
        .exec()
        .await
        .expect("create + create-child + disconnect + delete");

    let stmts = engine.statements();
    assert_eq!(
        stmts.len(),
        4,
        "parent + create child + disconnect + delete; got {stmts:#?}"
    );
    assert!(stmts[0].0.contains("INSERT INTO"));
    assert!(stmts[1].0.contains("INSERT INTO"));
    assert!(stmts[2].0.contains("UPDATE") && stmts[2].0.contains("NULL"));
    assert!(stmts[3].0.contains("DELETE FROM"));
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

#[tokio::test]
async fn nested_update_emits_parent_insert_then_update() {
    use prax_query::inputs::WriteOp;
    let engine = RecordingEngine::new();
    let c = prax_orm::PraxClient::new(engine.clone());

    let _u: User = c
        .user()
        .create()
        .set("email", "owner@x.com")
        .with(NestedWriteOp::Update {
            relation: "posts",
            target_table: "posts",
            target_pk: "id",
            pk: FilterValue::Int(42),
            payload: vec![(
                "title".to_string(),
                WriteOp::Set(FilterValue::String("renamed".into())),
            )],
        })
        .exec()
        .await
        .expect("create + nested update");

    let stmts = engine.statements();
    assert_eq!(stmts.len(), 2, "got {stmts:#?}");
    assert!(stmts[0].0.contains("INSERT INTO"));
    let (sql, params) = &stmts[1];
    assert!(sql.contains("UPDATE"), "got: {sql}");
    assert!(sql.contains("posts"), "got: {sql}");
    assert!(sql.contains("title"), "got: {sql}");
    assert!(sql.contains("SET"), "got: {sql}");
    assert!(sql.contains("WHERE"), "got: {sql}");
    assert_eq!(params.len(), 2);
    assert_eq!(params[1], FilterValue::Int(42));
}

#[tokio::test]
async fn nested_update_increment_emits_arithmetic_set_clause() {
    use prax_query::inputs::WriteOp;
    let engine = RecordingEngine::new();
    let c = prax_orm::PraxClient::new(engine.clone());

    let _u: User = c
        .user()
        .create()
        .set("email", "owner@x.com")
        .with(NestedWriteOp::Update {
            relation: "posts",
            target_table: "posts",
            target_pk: "id",
            pk: FilterValue::Int(42),
            payload: vec![("views".to_string(), WriteOp::Increment(FilterValue::Int(1)))],
        })
        .exec()
        .await
        .expect("create + nested update increment");

    let stmts = engine.statements();
    assert_eq!(stmts.len(), 2);
    let (sql, _) = &stmts[1];
    // Postgres dialect quotes idents — the fragment is `"views" = "views" + $1`.
    assert!(sql.contains("+"), "expected arithmetic SET clause: {sql}");
    assert!(sql.contains("views"), "got: {sql}");
}

#[tokio::test]
async fn nested_update_many_with_filter_emits_fk_and_filter() {
    use prax_query::filter::Filter;
    use prax_query::inputs::WriteOp;
    let engine = RecordingEngine::new();
    let c = prax_orm::PraxClient::new(engine.clone());

    let _u: User = c
        .user()
        .create()
        .set("email", "owner@x.com")
        .with(NestedWriteOp::UpdateMany {
            relation: "posts",
            target_table: "posts",
            foreign_key: "author_id",
            filter: Filter::Equals("published".into(), FilterValue::Bool(false)),
            payload: vec![("views".to_string(), WriteOp::Set(FilterValue::Int(0)))],
        })
        .exec()
        .await
        .expect("create + nested update_many");

    let stmts = engine.statements();
    assert_eq!(stmts.len(), 2, "got {stmts:#?}");
    let (sql, params) = &stmts[1];
    assert!(sql.contains("UPDATE"), "got: {sql}");
    assert!(sql.contains("author_id"), "got: {sql}");
    assert!(sql.contains("AND"), "got: {sql}");
    assert!(sql.contains("published"), "got: {sql}");
    // payload value + FK (parent_pk) + filter value = 3 params
    assert_eq!(params.len(), 3);
    assert_eq!(params[0], FilterValue::Int(0));
    assert_eq!(params[2], FilterValue::Bool(false));
}

#[tokio::test]
async fn nested_upsert_single_statement_on_postgres_dialect() {
    use prax_query::inputs::WriteOp;
    // Postgres dialect supports `ON CONFLICT (...) DO UPDATE SET ...`,
    // so the executor collapses the two-statement UPDATE-then-INSERT
    // form into a single `INSERT ... ON CONFLICT` statement.
    let engine = RecordingEngine::new();
    let c = prax_orm::PraxClient::new(engine.clone());

    let _u: User = c
        .user()
        .create()
        .set("email", "owner@x.com")
        .with(NestedWriteOp::Upsert {
            relation: "posts",
            target_table: "posts",
            foreign_key: "author_id",
            target_pk: "id",
            pk: FilterValue::Int(99),
            create_payload: vec![("title".to_string(), FilterValue::String("new".into()))],
            update_payload: vec![("views".to_string(), WriteOp::Increment(FilterValue::Int(1)))],
        })
        .exec()
        .await
        .expect("create + single-statement upsert");

    let stmts = engine.statements();
    assert_eq!(
        stmts.len(),
        2,
        "parent insert + single-statement upsert; got {stmts:#?}"
    );
    assert!(stmts[0].0.contains("INSERT INTO"));
    assert!(stmts[0].0.contains("users"));
    let (upsert_sql, upsert_params) = &stmts[1];
    assert!(
        upsert_sql.starts_with("INSERT INTO"),
        "expected single INSERT...ON CONFLICT, got: {upsert_sql}"
    );
    assert!(upsert_sql.contains("posts"), "got: {upsert_sql}");
    assert!(
        upsert_sql.contains("ON CONFLICT (\"id\")"),
        "anchored conflict target: {upsert_sql}"
    );
    assert!(upsert_sql.contains("DO UPDATE SET"), "got: {upsert_sql}");
    // INSERT supplies $1 (title), $2 (author_id=parent PK);
    // SET fragment uses $3 (views increment).
    assert!(
        upsert_sql.contains("VALUES ($1, $2)"),
        "INSERT VALUES placeholders: {upsert_sql}"
    );
    assert!(upsert_sql.contains("$3"), "got: {upsert_sql}");
    assert_eq!(upsert_params.len(), 3);
    assert_eq!(upsert_params[0], FilterValue::String("new".into()));
    assert_eq!(upsert_params[1], FilterValue::Int(7));
    assert_eq!(upsert_params[2], FilterValue::Int(1));
}

#[tokio::test]
async fn nested_upsert_two_statement_on_mssql_dialect() {
    use prax_query::inputs::WriteOp;
    // MSSQL's `upsert_clause` returns empty, so the executor must
    // fall back to the existing two-statement form:
    //   1st execute_raw (UPDATE phase) -> 0 (no row matched)
    //   2nd execute_raw (INSERT phase) -> 1
    let engine = RecordingEngine::with_affected_mssql(vec![0, 1]);
    let c = prax_orm::PraxClient::new(engine.clone());

    let _u: User = c
        .user()
        .create()
        .set("email", "owner@x.com")
        .with(NestedWriteOp::Upsert {
            relation: "posts",
            target_table: "posts",
            foreign_key: "author_id",
            target_pk: "id",
            pk: FilterValue::Int(99),
            create_payload: vec![("title".to_string(), FilterValue::String("new".into()))],
            update_payload: vec![("views".to_string(), WriteOp::Increment(FilterValue::Int(1)))],
        })
        .exec()
        .await
        .expect("create + two-statement upsert fallback");

    let stmts = engine.statements();
    assert_eq!(
        stmts.len(),
        3,
        "parent insert + UPDATE + INSERT (two-statement fallback); got {stmts:#?}"
    );
    let (update_sql, _) = &stmts[1];
    assert!(
        update_sql.starts_with("UPDATE"),
        "expected UPDATE first, got: {update_sql}"
    );
    assert!(!update_sql.contains("ON CONFLICT"), "got: {update_sql}");
    assert!(!update_sql.contains("ON DUPLICATE"), "got: {update_sql}");
    // MSSQL uses bracket-quoted idents.
    assert!(update_sql.contains("[posts]"), "got: {update_sql}");
    let (insert_sql, insert_params) = &stmts[2];
    assert!(insert_sql.starts_with("INSERT INTO"), "got: {insert_sql}");
    assert!(insert_sql.contains("[posts]"), "got: {insert_sql}");
    assert!(!insert_sql.contains("ON CONFLICT"), "got: {insert_sql}");
    assert!(insert_sql.contains("[author_id]"), "got: {insert_sql}");
    // create payload value + FK (parent PK) = 2 params
    assert_eq!(insert_params.len(), 2);
    assert_eq!(insert_params[0], FilterValue::String("new".into()));
}

#[tokio::test]
async fn nested_connect_or_create_connect_path() {
    // Affected-override sequence:
    //   1st execute_raw (connect_or_create UPDATE phase) -> 1 (row matched)
    // The executor must skip the INSERT phase.
    let engine = RecordingEngine::with_affected(vec![1]);
    let c = prax_orm::PraxClient::new(engine.clone());

    let _u: User = c
        .user()
        .create()
        .set("email", "owner@x.com")
        .with(NestedWriteOp::ConnectOrCreate {
            relation: "posts",
            target_table: "posts",
            foreign_key: "author_id",
            where_filter: Filter::Equals("id".into(), FilterValue::Int(42)),
            create_payload: vec![("title".to_string(), FilterValue::String("fallback".into()))],
        })
        .exec()
        .await
        .expect("create + connect_or_create connect path");

    let stmts = engine.statements();
    assert_eq!(
        stmts.len(),
        2,
        "parent insert + UPDATE only (no INSERT); got {stmts:#?}"
    );
    assert!(stmts[0].0.contains("INSERT INTO"));
    assert!(stmts[0].0.contains("users"));
    let (update_sql, update_params) = &stmts[1];
    assert!(update_sql.contains("UPDATE"), "got: {update_sql}");
    assert!(update_sql.contains("posts"), "got: {update_sql}");
    assert!(update_sql.contains("author_id"), "got: {update_sql}");
    // The child INSERT must not have run.
    assert!(
        !stmts.iter().skip(1).any(|(s, _)| s.contains("INSERT INTO")),
        "no child INSERT expected; got {stmts:#?}"
    );
    // UPDATE params: parent_pk ($1) + filter value ($2).
    assert_eq!(update_params.len(), 2);
    assert_eq!(update_params[1], FilterValue::Int(42));
}

#[tokio::test]
async fn nested_connect_or_create_create_path() {
    // Affected-override sequence:
    //   1st execute_raw (connect_or_create UPDATE phase) -> 0 (no match)
    //   2nd execute_raw (connect_or_create INSERT phase) -> 1
    let engine = RecordingEngine::with_affected(vec![0, 1]);
    let c = prax_orm::PraxClient::new(engine.clone());

    let _u: User = c
        .user()
        .create()
        .set("email", "owner@x.com")
        .with(NestedWriteOp::ConnectOrCreate {
            relation: "posts",
            target_table: "posts",
            foreign_key: "author_id",
            where_filter: Filter::Equals("id".into(), FilterValue::Int(42)),
            create_payload: vec![("title".to_string(), FilterValue::String("fallback".into()))],
        })
        .exec()
        .await
        .expect("create + connect_or_create create path");

    let stmts = engine.statements();
    assert_eq!(
        stmts.len(),
        3,
        "parent insert + connect_or_create UPDATE + child INSERT; got {stmts:#?}"
    );
    let (update_sql, _) = &stmts[1];
    assert!(update_sql.contains("UPDATE"), "got: {update_sql}");
    assert!(update_sql.contains("posts"), "got: {update_sql}");
    let (insert_sql, insert_params) = &stmts[2];
    assert!(insert_sql.contains("INSERT INTO"), "got: {insert_sql}");
    assert!(insert_sql.contains("posts"), "got: {insert_sql}");
    assert!(insert_sql.contains("title"), "got: {insert_sql}");
    assert!(
        insert_sql.contains("author_id"),
        "FK should be spliced in: {insert_sql}"
    );
    // create payload value + FK (parent PK) = 2 params; the parent PK
    // is appended last so it lines up with the FK column.
    assert_eq!(insert_params.len(), 2);
    assert_eq!(insert_params[0], FilterValue::String("fallback".into()));
}

#[tokio::test]
async fn nested_set_empty_list_emits_disconnect_all() {
    let engine = RecordingEngine::new();
    let c = prax_orm::PraxClient::new(engine.clone());

    let _u: User = c
        .user()
        .create()
        .set("email", "owner@x.com")
        .with(NestedWriteOp::Set {
            relation: "posts",
            target_table: "posts",
            foreign_key: "author_id",
            target_pk: "id",
            set_pks: vec![],
        })
        .exec()
        .await
        .expect("create + nested set empty");

    let stmts = engine.statements();
    assert_eq!(
        stmts.len(),
        2,
        "parent insert + single disconnect-all UPDATE; got {stmts:#?}"
    );
    let (sql, params) = &stmts[1];
    assert!(sql.contains("UPDATE"), "got: {sql}");
    assert!(sql.contains("posts"), "got: {sql}");
    assert!(sql.contains("NULL"), "got: {sql}");
    assert!(
        !sql.contains("NOT IN"),
        "empty list should not emit NOT IN: {sql}"
    );
    assert_eq!(params.len(), 1, "parent_pk only");
}

#[tokio::test]
async fn nested_set_with_pks_emits_disconnect_then_connect() {
    let engine = RecordingEngine::new();
    let c = prax_orm::PraxClient::new(engine.clone());

    let _u: User = c
        .user()
        .create()
        .set("email", "owner@x.com")
        .with(NestedWriteOp::Set {
            relation: "posts",
            target_table: "posts",
            foreign_key: "author_id",
            target_pk: "id",
            set_pks: vec![
                FilterValue::Int(1),
                FilterValue::Int(2),
                FilterValue::Int(3),
            ],
        })
        .exec()
        .await
        .expect("create + nested set with PKs");

    let stmts = engine.statements();
    assert_eq!(
        stmts.len(),
        3,
        "parent insert + disconnect UPDATE + connect UPDATE; got {stmts:#?}"
    );

    let (disconnect_sql, disconnect_params) = &stmts[1];
    assert!(disconnect_sql.contains("UPDATE"), "got: {disconnect_sql}");
    assert!(disconnect_sql.contains("NULL"), "got: {disconnect_sql}");
    assert!(disconnect_sql.contains("NOT IN"), "got: {disconnect_sql}");
    assert_eq!(disconnect_params.len(), 4, "parent_pk + 3 set_pks");

    let (connect_sql, connect_params) = &stmts[2];
    assert!(connect_sql.contains("UPDATE"), "got: {connect_sql}");
    assert!(connect_sql.contains("author_id"), "got: {connect_sql}");
    assert!(connect_sql.contains(" IN ("), "got: {connect_sql}");
    assert!(
        !connect_sql.contains("NOT IN"),
        "connect must not include NOT IN: {connect_sql}"
    );
    assert_eq!(connect_params.len(), 4, "parent_pk + 3 set_pks");
}

#[tokio::test]
async fn nested_set_combined_with_create_child_in_one_transaction() {
    let engine = RecordingEngine::new();
    let c = prax_orm::PraxClient::new(engine.clone());

    let _u: User = c
        .user()
        .create()
        .set("email", "owner@x.com")
        .with(user::posts::create(vec![vec![(
            "title".into(),
            FilterValue::String("new".into()),
        )]]))
        .with(NestedWriteOp::Set {
            relation: "posts",
            target_table: "posts",
            foreign_key: "author_id",
            target_pk: "id",
            set_pks: vec![FilterValue::Int(10), FilterValue::Int(20)],
        })
        .exec()
        .await
        .expect("create + create-child + set");

    let stmts = engine.statements();
    assert_eq!(
        stmts.len(),
        4,
        "parent + create-child + set-disconnect + set-connect; got {stmts:#?}"
    );
    assert!(stmts[0].0.contains("INSERT INTO"));
    assert!(
        stmts[1].0.contains("INSERT INTO"),
        "create child: {}",
        stmts[1].0
    );
    assert!(stmts[2].0.contains("UPDATE") && stmts[2].0.contains("NULL"));
    assert!(stmts[3].0.contains("UPDATE") && stmts[3].0.contains(" IN ("));
}

// ========== Phase 5c: nested writes inside update! / upsert! ==========

#[tokio::test]
async fn update_with_nested_create_emits_parent_update_then_child_insert() {
    let engine = RecordingEngine::new();
    let c = prax_orm::PraxClient::new(engine.clone());

    let _rows: Vec<User> = c
        .user()
        .update()
        .r#where(Filter::Equals("id".into(), FilterValue::Int(7)))
        .set("email", "renamed@x.com")
        .with(user::posts::create(vec![vec![(
            "title".into(),
            FilterValue::String("p1".into()),
        )]]))
        .exec()
        .await
        .expect("update + nested create");

    let stmts = engine.statements();
    assert_eq!(
        stmts.len(),
        2,
        "parent UPDATE + nested child INSERT; got {stmts:#?}"
    );
    assert!(
        stmts[0].0.starts_with("UPDATE"),
        "first stmt is UPDATE users: {}",
        stmts[0].0
    );
    assert!(stmts[0].0.contains("users"), "got: {}", stmts[0].0);
    assert!(
        stmts[1].0.contains("INSERT INTO"),
        "second stmt is child INSERT: {}",
        stmts[1].0
    );
    assert!(stmts[1].0.contains("posts"), "got: {}", stmts[1].0);
    assert!(stmts[1].0.contains("author_id"), "got: {}", stmts[1].0);
}

#[tokio::test]
async fn update_with_nested_disconnect_and_delete_runs_in_order() {
    let engine = RecordingEngine::new();
    let c = prax_orm::PraxClient::new(engine.clone());

    let _rows: Vec<User> = c
        .user()
        .update()
        .r#where(Filter::Equals("id".into(), FilterValue::Int(7)))
        .set("email", "owner@x.com")
        .with(NestedWriteOp::Disconnect {
            relation: "posts",
            target_table: "posts",
            foreign_key: "author_id",
            target_pk: "id",
            pk: FilterValue::Int(100),
        })
        .with(NestedWriteOp::Delete {
            relation: "posts",
            target_table: "posts",
            target_pk: "id",
            pk: FilterValue::Int(200),
        })
        .exec()
        .await
        .expect("update + disconnect + delete");

    let stmts = engine.statements();
    assert_eq!(
        stmts.len(),
        3,
        "parent UPDATE + disconnect UPDATE + DELETE; got {stmts:#?}"
    );
    assert!(stmts[0].0.starts_with("UPDATE"), "got: {}", stmts[0].0);
    assert!(stmts[0].0.contains("users"), "got: {}", stmts[0].0);
    assert!(
        stmts[1].0.contains("UPDATE") && stmts[1].0.contains("NULL"),
        "disconnect: {}",
        stmts[1].0
    );
    assert!(stmts[2].0.contains("DELETE FROM"), "delete: {}", stmts[2].0);
}

#[tokio::test]
async fn upsert_update_branch_runs_update_nested_only() {
    // The two-statement upsert UPDATE returns 1 affected — so the
    // executor takes the update branch and fires update_nested only.
    let engine = RecordingEngine::with_affected(vec![1]);
    let c = prax_orm::PraxClient::new(engine.clone());

    let _u: User = c
        .user()
        .upsert()
        .r#where(Filter::Equals("id".into(), FilterValue::Int(7)))
        .create_set("id", FilterValue::Int(7))
        .create_set("email", "new@x.com")
        .update_set("email", "renamed@x.com")
        .with_update_nested(NestedWriteOp::Disconnect {
            relation: "posts",
            target_table: "posts",
            foreign_key: "author_id",
            target_pk: "id",
            pk: FilterValue::Int(42),
        })
        .with_create_nested(user::posts::create(vec![vec![(
            "title".into(),
            FilterValue::String("from create branch".into()),
        )]]))
        .exec()
        .await
        .expect("upsert update branch");

    let stmts = engine.statements();
    // UPDATE + SELECT (re-fetch) + nested Disconnect
    assert_eq!(
        stmts.len(),
        3,
        "UPDATE + SELECT + nested disconnect; got {stmts:#?}"
    );
    assert!(stmts[0].0.starts_with("UPDATE"), "got: {}", stmts[0].0);
    assert!(stmts[0].0.contains("users"), "got: {}", stmts[0].0);
    assert!(stmts[1].0.starts_with("SELECT"), "got: {}", stmts[1].0);
    assert!(
        stmts[2].0.contains("UPDATE") && stmts[2].0.contains("NULL"),
        "nested Disconnect: {}",
        stmts[2].0
    );
    // No nested child INSERT on the create_nested side.
    assert!(
        !stmts
            .iter()
            .any(|(s, _)| s.contains("INSERT INTO") && s.contains("posts")),
        "no nested Create child INSERT on update branch: {stmts:#?}"
    );
}

#[tokio::test]
async fn upsert_create_branch_runs_create_nested_only() {
    // affected=0 on the UPDATE phase → executor falls into the INSERT
    // branch and fires create_nested.
    let engine = RecordingEngine::with_affected(vec![0]);
    let c = prax_orm::PraxClient::new(engine.clone());

    let _u: User = c
        .user()
        .upsert()
        .r#where(Filter::Equals("id".into(), FilterValue::Int(7)))
        .create_set("id", FilterValue::Int(7))
        .create_set("email", "new@x.com")
        .update_set("email", "renamed@x.com")
        .with_update_nested(NestedWriteOp::Disconnect {
            relation: "posts",
            target_table: "posts",
            foreign_key: "author_id",
            target_pk: "id",
            pk: FilterValue::Int(42),
        })
        .with_create_nested(user::posts::create(vec![vec![(
            "title".into(),
            FilterValue::String("from create branch".into()),
        )]]))
        .exec()
        .await
        .expect("upsert create branch");

    let stmts = engine.statements();
    // UPDATE (0 affected) + INSERT users + nested child INSERT posts
    assert_eq!(
        stmts.len(),
        3,
        "UPDATE + INSERT users + nested INSERT posts; got {stmts:#?}"
    );
    assert!(stmts[0].0.starts_with("UPDATE"), "got: {}", stmts[0].0);
    assert!(
        stmts[1].0.contains("INSERT INTO") && stmts[1].0.contains("users"),
        "create-branch INSERT users: {}",
        stmts[1].0
    );
    assert!(
        stmts[2].0.contains("INSERT INTO") && stmts[2].0.contains("posts"),
        "nested Create child INSERT: {}",
        stmts[2].0
    );
    // The update_nested Disconnect must NOT fire.
    assert!(
        !stmts.iter().any(|(s, _)| s.contains("NULL")),
        "no nested Disconnect on create branch: {stmts:#?}"
    );
}

#[tokio::test]
async fn upsert_with_nested_in_both_branches_only_one_fires() {
    // Branch dispatch sanity-check end-to-end: same op definition, two
    // engines configured with different affected counts.
    for (affected, expect_disconnect, expect_child_insert) in
        [(1u64, true, false), (0u64, false, true)]
    {
        let engine = RecordingEngine::with_affected(vec![affected]);
        let c = prax_orm::PraxClient::new(engine.clone());

        let _u: User = c
            .user()
            .upsert()
            .r#where(Filter::Equals("id".into(), FilterValue::Int(7)))
            .create_set("id", FilterValue::Int(7))
            .create_set("email", "new@x.com")
            .update_set("email", "renamed@x.com")
            .with_update_nested(NestedWriteOp::Disconnect {
                relation: "posts",
                target_table: "posts",
                foreign_key: "author_id",
                target_pk: "id",
                pk: FilterValue::Int(42),
            })
            .with_create_nested(user::posts::create(vec![vec![(
                "title".into(),
                FilterValue::String("p".into()),
            )]]))
            .exec()
            .await
            .expect("upsert with both branches populated");

        let stmts = engine.statements();
        let saw_disconnect = stmts.iter().any(|(s, _)| s.contains("NULL"));
        let saw_child_insert = stmts
            .iter()
            .any(|(s, _)| s.contains("INSERT INTO") && s.contains("posts"));
        assert_eq!(
            saw_disconnect, expect_disconnect,
            "affected={affected} stmts={stmts:#?}"
        );
        assert_eq!(
            saw_child_insert, expect_child_insert,
            "affected={affected} stmts={stmts:#?}"
        );
    }
}

#[tokio::test]
async fn nested_upsert_two_statement_on_mssql_dialect_update_path() {
    use prax_query::inputs::WriteOp;
    // MSSQL dialect, UPDATE returns 1 → INSERT does not fire.
    // This exercises the affected_rows > 0 branch on the two-statement fallback.
    let engine = RecordingEngine::with_affected_mssql(vec![1]);
    let c = prax_orm::PraxClient::new(engine.clone());

    let _u: User = c
        .user()
        .create()
        .set("email", "owner@x.com")
        .with(NestedWriteOp::Upsert {
            relation: "posts",
            target_table: "posts",
            foreign_key: "author_id",
            target_pk: "id",
            pk: FilterValue::Int(99),
            create_payload: vec![("title".to_string(), FilterValue::String("new".into()))],
            update_payload: vec![("views".to_string(), WriteOp::Increment(FilterValue::Int(1)))],
        })
        .exec()
        .await
        .expect("create + two-statement upsert update-only path");

    let stmts = engine.statements();
    assert_eq!(
        stmts.len(),
        2,
        "parent insert + UPDATE only (no second INSERT); got {stmts:#?}"
    );
    let (update_sql, _) = &stmts[1];
    assert!(update_sql.starts_with("UPDATE"), "got: {update_sql}");
    assert!(update_sql.contains("[posts]"), "got: {update_sql}");
    assert!(!update_sql.contains("ON CONFLICT"), "got: {update_sql}");
    // No INSERT into posts should have fired.
    assert!(
        !stmts
            .iter()
            .skip(1)
            .any(|(s, _)| s.starts_with("INSERT INTO") && s.contains("[posts]")),
        "no INSERT INTO posts expected; got {stmts:#?}"
    );
}
