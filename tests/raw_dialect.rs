//! `PraxClient::query_raw` / `execute_raw` (and the raw operations) render
//! a [`Sql`]'s placeholders for the executing engine's dialect, unless the
//! query chose a database type with `Sql::with_db_type`.

use std::sync::{Arc, Mutex};

use prax_orm::{Model, PraxClient};
use prax_query::dialect::{Mssql, Mysql, Postgres, SqlDialect};
use prax_query::error::{QueryError, QueryResult};
use prax_query::filter::FilterValue;
use prax_query::raw::{RawExecuteOperation, RawQueryOperation, Sql};
use prax_query::row::FromRow;
use prax_query::sql::DatabaseType;
use prax_query::traits::{BoxFuture, Model as ModelTrait, QueryEngine};

#[derive(Debug, Model)]
#[prax(table = "raw_dialect_users")]
struct User {
    #[prax(id, auto)]
    id: i32,
    email: String,
}

/// Records the SQL each call receives; returns no rows.
#[derive(Clone)]
struct RecordingEngine {
    dialect: &'static (dyn SqlDialect + 'static),
    seen: Arc<Mutex<Vec<String>>>,
}

impl RecordingEngine {
    fn new(dialect: &'static (dyn SqlDialect + 'static)) -> Self {
        Self {
            dialect,
            seen: Arc::default(),
        }
    }

    fn record(&self, sql: &str) {
        self.seen.lock().unwrap().push(sql.to_owned());
    }

    fn seen(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }
}

impl QueryEngine for RecordingEngine {
    fn dialect(&self) -> &dyn SqlDialect {
        self.dialect
    }
    fn query_many<T: ModelTrait + FromRow + Send + 'static>(
        &self,
        sql: &str,
        _params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Vec<T>>> {
        self.record(sql);
        Box::pin(async { Ok(Vec::new()) })
    }
    fn query_one<T: ModelTrait + FromRow + Send + 'static>(
        &self,
        sql: &str,
        _params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<T>> {
        self.record(sql);
        Box::pin(async { Err(QueryError::not_found("recording engine")) })
    }
    fn query_optional<T: ModelTrait + FromRow + Send + 'static>(
        &self,
        sql: &str,
        _params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Option<T>>> {
        self.record(sql);
        Box::pin(async { Ok(None) })
    }
    fn execute_insert<T: ModelTrait + FromRow + Send + 'static>(
        &self,
        _sql: &str,
        _params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<T>> {
        Box::pin(async { Err(QueryError::not_found("recording engine")) })
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
    fn execute_raw(&self, sql: &str, _params: Vec<FilterValue>) -> BoxFuture<'_, QueryResult<u64>> {
        self.record(sql);
        Box::pin(async { Ok(0) })
    }
    fn count(&self, _sql: &str, _params: Vec<FilterValue>) -> BoxFuture<'_, QueryResult<u64>> {
        Box::pin(async { Ok(0) })
    }
}

fn select_by_email() -> Sql {
    Sql::new("SELECT id, email FROM raw_dialect_users WHERE email = ")
        .bind("a@example.com")
        .push(" AND id > ")
        .bind(1)
}

fn delete_by_email() -> Sql {
    Sql::new("DELETE FROM raw_dialect_users WHERE email = ").bind("a@example.com")
}

#[tokio::test]
async fn client_renders_placeholders_for_the_engine_dialect() {
    let cases: [(&'static (dyn SqlDialect + 'static), &str, &str); 3] = [
        (&Postgres, "email = $1 AND id > $2", "email = $1"),
        (&Mysql, "email = ? AND id > ?", "email = ?"),
        (&Mssql, "email = @P1 AND id > @P2", "email = @P1"),
    ];
    for (dialect, select_tail, delete_tail) in cases {
        let engine = RecordingEngine::new(dialect);
        let client = PraxClient::new(engine.clone());
        client
            .query_raw::<User>(select_by_email())
            .await
            .expect("query_raw");
        client
            .execute_raw(delete_by_email())
            .await
            .expect("execute_raw");
        let seen = engine.seen();
        assert!(seen[0].ends_with(select_tail), "{seen:?}");
        assert!(seen[1].ends_with(delete_tail), "{seen:?}");
    }
}

#[tokio::test]
async fn explicit_db_type_wins_over_the_engine_dialect() {
    let engine = RecordingEngine::new(&Mysql);
    let client = PraxClient::new(engine.clone());
    client
        .execute_raw(delete_by_email().with_db_type(DatabaseType::PostgreSQL))
        .await
        .expect("execute_raw");
    assert_eq!(
        engine.seen(),
        ["DELETE FROM raw_dialect_users WHERE email = $1"]
    );
}

#[tokio::test]
async fn raw_operations_render_for_the_engine_dialect() {
    let engine = RecordingEngine::new(&Mysql);
    RawQueryOperation::<User, _>::new(engine.clone(), select_by_email())
        .exec()
        .await
        .expect("exec");
    RawQueryOperation::<User, _>::new(engine.clone(), select_by_email())
        .exec_optional()
        .await
        .expect("exec_optional");
    RawExecuteOperation::new(engine.clone(), delete_by_email())
        .exec()
        .await
        .expect("exec");
    let seen = engine.seen();
    assert_eq!(seen.len(), 3, "{seen:?}");
    assert!(seen.iter().all(|sql| !sql.contains('$')), "{seen:?}");
    assert!(seen[0].ends_with("email = ? AND id > ?"), "{seen:?}");
}
