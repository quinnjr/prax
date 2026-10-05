//! Row-lock (`FOR UPDATE` / `FOR SHARE`) rendering and validation on the
//! find operations.

use prax_query::advanced::RowLock;
use prax_query::capabilities::SupportsRowLocks;
use prax_query::dialect::{Mssql, Mysql, Postgres, SqlDialect, Sqlite};
use prax_query::error::{QueryError, QueryResult};
use prax_query::filter::FilterValue;
use prax_query::operations::{FindFirstOperation, FindManyOperation, FindUniqueOperation};
use prax_query::relations::IncludeSpec;
use prax_query::traits::{BoxFuture, Model, ModelRelationLoader, QueryEngine};
use prax_query::types::OrderByField;

macro_rules! mock_engine {
    ($name:ident, $dialect:expr) => {
        #[derive(Clone)]
        struct $name;

        impl QueryEngine for $name {
            fn dialect(&self) -> &dyn SqlDialect {
                &$dialect
            }
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
                Box::pin(async { Err(QueryError::not_found("t")) })
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
                Box::pin(async { Err(QueryError::not_found("t")) })
            }
            fn execute_update<T: Model + prax_query::row::FromRow + Send + 'static>(
                &self,
                _: &str,
                _: Vec<FilterValue>,
            ) -> BoxFuture<'_, QueryResult<Vec<T>>> {
                Box::pin(async { Ok(Vec::new()) })
            }
            fn execute_delete(
                &self,
                _: &str,
                _: Vec<FilterValue>,
            ) -> BoxFuture<'_, QueryResult<u64>> {
                Box::pin(async { Ok(0) })
            }
            fn execute_raw(&self, _: &str, _: Vec<FilterValue>) -> BoxFuture<'_, QueryResult<u64>> {
                Box::pin(async { Ok(0) })
            }
            fn count(&self, _: &str, _: Vec<FilterValue>) -> BoxFuture<'_, QueryResult<u64>> {
                Box::pin(async { Ok(0) })
            }
        }

        impl SupportsRowLocks for $name {}
    };
}

mock_engine!(MockEngine, Postgres);
mock_engine!(SqliteEngine, Sqlite);

#[derive(Debug)]
struct Job;
impl Model for Job {
    const MODEL_NAME: &'static str = "Job";
    const TABLE_NAME: &'static str = "jobs";
    const PRIMARY_KEY: &'static [&'static str] = &["id"];
    const COLUMNS: &'static [&'static str] = &["id"];
}
impl prax_query::row::FromRow for Job {
    fn from_row(_: &impl prax_query::row::RowRef) -> Result<Self, prax_query::row::RowError> {
        Ok(Job)
    }
}
impl<E: QueryEngine> ModelRelationLoader<E> for Job {
    fn load_relation<'a>(
        _: &'a E,
        _: &'a mut [Self],
        _: &'a IncludeSpec,
    ) -> BoxFuture<'a, QueryResult<()>> {
        Box::pin(async { Ok(()) })
    }
}

fn find_many() -> FindManyOperation<MockEngine, Job> {
    FindManyOperation::<MockEngine, Job>::new(MockEngine)
}

#[test]
fn postgres_skip_locked_after_limit_offset() {
    let (sql, _) = find_many()
        .order_by(OrderByField::asc("id"))
        .skip(5)
        .take(10)
        .lock(RowLock::for_update().skip_locked())
        .build_sql(&Postgres);
    assert!(
        sql.ends_with("LIMIT 10 OFFSET 5 FOR UPDATE SKIP LOCKED"),
        "{sql}"
    );
}

#[test]
fn postgres_of_tables_are_quoted() {
    let (sql, _) = find_many()
        .lock(RowLock::for_share().of(["jobs"]).nowait())
        .build_sql(&Postgres);
    assert!(sql.ends_with("FOR SHARE OF \"jobs\" NOWAIT"), "{sql}");
}

#[test]
fn postgres_key_strengths() {
    let (sql, _) = find_many()
        .lock(RowLock::for_no_key_update())
        .build_sql(&Postgres);
    assert!(sql.ends_with("FOR NO KEY UPDATE"), "{sql}");
    let (sql, _) = find_many()
        .lock(RowLock::for_key_share())
        .build_sql(&Postgres);
    assert!(sql.ends_with("FOR KEY SHARE"), "{sql}");
}

#[test]
fn mysql_renders_and_rejects() {
    let ok = Mysql
        .row_lock_sql(&RowLock::for_update().nowait().build())
        .unwrap();
    assert_eq!(ok.suffix.as_deref(), Some("FOR UPDATE NOWAIT"));
    assert_eq!(ok.table_hint, None);
    assert!(
        Mysql
            .row_lock_sql(&RowLock::for_no_key_update().build())
            .is_err()
    );
    assert!(
        Mysql
            .row_lock_sql(&RowLock::for_key_share().build())
            .is_err()
    );
    assert!(
        Mysql
            .row_lock_sql(&RowLock::for_update().of(["jobs"]).build())
            .is_err()
    );
}

#[test]
fn mssql_table_hint_after_table() {
    let (sql, _) = find_many()
        .lock(RowLock::for_update().skip_locked())
        .build_sql(&Mssql);
    assert!(
        sql.contains("FROM jobs WITH (UPDLOCK, ROWLOCK, READPAST)"),
        "{sql}"
    );
    assert!(
        Mssql
            .row_lock_sql(&RowLock::for_share().skip_locked().build())
            .is_err()
    );
    assert!(
        Mssql
            .row_lock_sql(&RowLock::for_key_share().build())
            .is_err()
    );
    assert!(
        Mssql
            .row_lock_sql(&RowLock::for_update().of(["jobs"]).build())
            .is_err()
    );
    let share = Mssql
        .row_lock_sql(&RowLock::for_share().nowait().build())
        .unwrap();
    assert_eq!(
        share.table_hint.as_deref(),
        Some("WITH (HOLDLOCK, ROWLOCK, NOWAIT)")
    );
}

#[test]
fn sqlite_lock_is_unsupported_at_exec() {
    let op = FindManyOperation::<SqliteEngine, Job>::new(SqliteEngine).for_update();
    let err = futures::executor::block_on(op.exec()).unwrap_err();
    assert!(err.to_string().contains("row locks"), "{err}");
}

#[test]
fn lock_with_distinct_is_rejected() {
    let err =
        futures::executor::block_on(find_many().distinct(["id"]).for_update().exec()).unwrap_err();
    assert!(err.to_string().contains("distinct"), "{err}");
}

#[test]
fn find_first_and_unique_render_lock() {
    let (sql, _) = FindFirstOperation::<MockEngine, Job>::new(MockEngine)
        .for_update()
        .build_sql(&Postgres);
    assert!(sql.ends_with("LIMIT 1 FOR UPDATE"), "{sql}");
    let (sql, _) = FindUniqueOperation::<MockEngine, Job>::new(MockEngine)
        .for_update()
        .build_sql(&Postgres);
    assert!(sql.ends_with("LIMIT 1 FOR UPDATE"), "{sql}");
    let (sql, _) = FindUniqueOperation::<MockEngine, Job>::new(MockEngine)
        .for_share()
        .build_sql(&Postgres);
    assert!(sql.ends_with("LIMIT 1 FOR SHARE"), "{sql}");
}

#[test]
fn no_lock_leaves_sql_unchanged() {
    let (sql, _) = find_many().build_sql(&Postgres);
    assert_eq!(sql, "SELECT * FROM jobs");
}
