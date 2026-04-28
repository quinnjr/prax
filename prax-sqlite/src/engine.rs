//! SQLite query engine implementing `prax_query::QueryEngine`.

use prax_query::error::{QueryError, QueryResult};
use prax_query::filter::FilterValue;
use prax_query::row::FromRow;
use prax_query::traits::{BoxFuture, Model, QueryEngine};
use rusqlite::types::Value as SqlValue;
use tracing::trace;

use crate::pool::SqlitePool;
use crate::row_ref::SqliteRowRef;
use crate::types::filter_value_to_sqlite;

/// SQLite query engine backed by `tokio_rusqlite`.
///
/// # Breaking changes (0.7)
///
/// `SqliteEngine` no longer has inherent `query` / `query_one` / `query_opt`
/// methods that returned untyped `RowData` / `serde_json::Value`. It now
/// implements [`prax_query::traits::QueryEngine`], whose row-returning
/// methods are generic over `T: Model + FromRow` and return typed models.
///
/// Migration:
/// - Replace `engine.query(sql, params)` with
///   `engine.query_many::<YourType>(sql, params).await?`, where `YourType`
///   carries `#[derive(prax_orm::Model)]` (which emits both `Model` and
///   `FromRow`) or hand-written `impl Model + impl FromRow`.
/// - For ad-hoc typed queries without a full `Model`, bridge through
///   [`crate::row_ref::SqliteRowRef::from_rusqlite`] inside a custom
///   `FromRow` impl.
/// - For the legacy JSON-blob API, use [`crate::raw::SqliteRawEngine`] +
///   [`crate::raw::SqliteJsonRow`].
/// - To run side-effecting SQL that returns no rows, call
///   [`prax_query::traits::QueryEngine::execute_raw`].
///
/// See `CHANGELOG.md` for the full migration guide.
#[derive(Clone)]
pub struct SqliteEngine {
    pool: SqlitePool,
}

impl SqliteEngine {
    /// Create a new engine with the given pool.
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Get a reference to the connection pool.
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    fn bind(params: &[FilterValue]) -> Vec<SqlValue> {
        params.iter().map(filter_value_to_sqlite).collect()
    }

    /// Decode multiple rows into typed models.
    ///
    /// # Short-circuit on decode error
    ///
    /// Uses `Result<Vec<T>, _>::collect`, which returns the first decode
    /// error and discards every successfully-decoded row before it. A
    /// row-level type mismatch therefore aborts the whole batch rather
    /// than returning partial results. Callers that want per-row
    /// recovery should manually iterate rows and handle each result.
    async fn query_rows<T: Model + FromRow>(
        &self,
        sql: String,
        params: Vec<FilterValue>,
    ) -> QueryResult<Vec<T>> {
        trace!(sql = %sql, "sqlite query_rows");
        let conn = self
            .pool
            .get()
            .await
            .map_err(|e| QueryError::connection(e.to_string()).with_source(e))?;
        let bound = Self::bind(&params);
        let snapshots: Vec<SqliteRowRef> = conn
            .inner()
            .call(move |c| {
                let mut stmt = c.prepare(&sql)?;
                let refs: Vec<&dyn rusqlite::ToSql> =
                    bound.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
                let mut rows = stmt.query(refs.as_slice())?;
                let mut out = Vec::new();
                while let Some(row) = rows.next()? {
                    out.push(SqliteRowRef::from_rusqlite(row).map_err(|e| {
                        rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::other(
                            e.to_string(),
                        )))
                    })?);
                }
                Ok(out)
            })
            .await
            .map_err(|e| QueryError::database(e.to_string()).with_source(e))?;

        snapshots
            .into_iter()
            .map(|r| {
                T::from_row(&r).map_err(|e| {
                    let msg = e.to_string();
                    QueryError::deserialization(msg).with_source(e)
                })
            })
            .collect()
    }

    /// Stop after the first row so callers that want a single row do not pay
    /// for materializing the tail. Naively routing `query_one`/`query_optional`
    /// through `query_rows` + `.pop()` would decode every matching row and
    /// throw away all but one; a caller who accidentally asked for a single
    /// row from a million-row table would allocate a million typed models.
    async fn query_first_row<T: Model + FromRow>(
        &self,
        sql: String,
        params: Vec<FilterValue>,
    ) -> QueryResult<Option<T>> {
        trace!(sql = %sql, "sqlite query_first_row");
        let conn = self
            .pool
            .get()
            .await
            .map_err(|e| QueryError::connection(e.to_string()).with_source(e))?;
        let bound = Self::bind(&params);
        let snapshot: Option<SqliteRowRef> = conn
            .inner()
            .call(move |c| {
                let mut stmt = c.prepare(&sql)?;
                let refs: Vec<&dyn rusqlite::ToSql> =
                    bound.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
                let mut rows = stmt.query(refs.as_slice())?;
                match rows.next()? {
                    Some(row) => Ok(Some(SqliteRowRef::from_rusqlite(row).map_err(|e| {
                        rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::other(
                            e.to_string(),
                        )))
                    })?)),
                    None => Ok(None),
                }
            })
            .await
            .map_err(|e| QueryError::database(e.to_string()).with_source(e))?;

        snapshot
            .map(|r| {
                T::from_row(&r).map_err(|e| {
                    let msg = e.to_string();
                    QueryError::deserialization(msg).with_source(e)
                })
            })
            .transpose()
    }

    async fn exec_raw(&self, sql: String, params: Vec<FilterValue>) -> QueryResult<u64> {
        let conn = self
            .pool
            .get()
            .await
            .map_err(|e| QueryError::connection(e.to_string()).with_source(e))?;
        let bound = Self::bind(&params);
        let n = conn
            .inner()
            .call(move |c| {
                let refs: Vec<&dyn rusqlite::ToSql> =
                    bound.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
                Ok(c.execute(&sql, refs.as_slice())?)
            })
            .await
            .map_err(|e| QueryError::database(e.to_string()).with_source(e))?;
        Ok(n as u64)
    }

    async fn count_rows(&self, sql: String, params: Vec<FilterValue>) -> QueryResult<u64> {
        let conn = self
            .pool
            .get()
            .await
            .map_err(|e| QueryError::connection(e.to_string()).with_source(e))?;
        let bound = Self::bind(&params);
        let n = conn
            .inner()
            .call(move |c| {
                let refs: Vec<&dyn rusqlite::ToSql> =
                    bound.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
                let mut stmt = c.prepare(&sql)?;
                let n: i64 = stmt.query_row(refs.as_slice(), |r| r.get(0))?;
                Ok(n)
            })
            .await
            .map_err(|e| QueryError::database(e.to_string()).with_source(e))?;
        Ok(n as u64)
    }
}

impl QueryEngine for SqliteEngine {
    fn dialect(&self) -> &dyn prax_query::dialect::SqlDialect {
        &prax_query::dialect::Sqlite
    }

    fn query_many<T: Model + FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Vec<T>>> {
        let sql = sql.to_string();
        Box::pin(self.query_rows::<T>(sql, params))
    }

    fn query_one<T: Model + FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<T>> {
        let sql = sql.to_string();
        Box::pin(async move {
            self.query_first_row::<T>(sql, params)
                .await?
                .ok_or_else(|| QueryError::not_found(T::MODEL_NAME))
        })
    }

    fn query_optional<T: Model + FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Option<T>>> {
        let sql = sql.to_string();
        Box::pin(self.query_first_row::<T>(sql, params))
    }

    fn execute_insert<T: Model + FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<T>> {
        // SQLite 3.35+ supports INSERT ... RETURNING. INSERT RETURNING yields
        // at most one row per inserted tuple; query_first_row avoids ever
        // materializing a tail if the caller's SQL yields many (which would
        // be a misuse, but the engine shouldn't punish it with unbounded
        // allocation).
        let sql = sql.to_string();
        Box::pin(async move {
            self.query_first_row::<T>(sql, params)
                .await?
                .ok_or_else(|| QueryError::not_found(T::MODEL_NAME))
        })
    }

    fn execute_update<T: Model + FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Vec<T>>> {
        let sql = sql.to_string();
        Box::pin(self.query_rows::<T>(sql, params))
    }

    fn execute_delete(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<u64>> {
        let sql = sql.to_string();
        Box::pin(self.exec_raw(sql, params))
    }

    fn execute_raw(&self, sql: &str, params: Vec<FilterValue>) -> BoxFuture<'_, QueryResult<u64>> {
        let sql = sql.to_string();
        Box::pin(self.exec_raw(sql, params))
    }

    fn count(&self, sql: &str, params: Vec<FilterValue>) -> BoxFuture<'_, QueryResult<u64>> {
        let sql = sql.to_string();
        Box::pin(self.count_rows(sql, params))
    }
}
