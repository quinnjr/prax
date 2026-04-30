//! SQLite query engine implementing `prax_query::QueryEngine`.

use std::sync::Arc;

use prax_query::error::{QueryError, QueryResult};
use prax_query::filter::FilterValue;
use prax_query::row::FromRow;
use prax_query::traits::{BoxFuture, Model, QueryEngine};
use rusqlite::types::Value as SqlValue;
use tokio_rusqlite::Connection as RusqliteConnection;
use tracing::trace;

use crate::connection::SqliteConnection;
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
///
/// # Transaction mode
///
/// `SqliteEngine` has two modes, controlled by `tx_conn`:
///
/// - **Pool mode** (`tx_conn == None`, default): each query acquires
///   a fresh [`SqliteConnection`] from the pool and drops it after
///   the call.
/// - **Transaction mode** (`tx_conn == Some(..)`): each query routes
///   through the same [`tokio_rusqlite::Connection`] handle so the
///   whole BEGIN…COMMIT block serialises onto the connection's
///   background thread in order. The `Arc<SqliteConnection>` keeps
///   the pool permit alive for the transaction's lifetime; dropping
///   the last clone returns the connection to the idle pool.
#[derive(Clone)]
pub struct SqliteEngine {
    pool: SqlitePool,
    /// Present when this engine is bound to an in-flight transaction.
    /// `None` in the normal pool-backed case.
    tx_conn: Option<Arc<SqliteConnection>>,
}

impl SqliteEngine {
    /// Create a new engine with the given pool.
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            pool,
            tx_conn: None,
        }
    }

    /// Get a reference to the connection pool.
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Resolve which raw [`tokio_rusqlite::Connection`] to run the
    /// query against, along with an optional owned
    /// [`SqliteConnection`] guard whose `Drop` returns the
    /// connection to the idle pool once we're done with it.
    ///
    /// In tx mode the guard is `None` because the pinned connection
    /// is already kept alive by the `Arc<SqliteConnection>` on
    /// `self.tx_conn`. In pool mode we hand back a fresh guard so
    /// the caller can clone the inner handle, drive one `call(..)`
    /// through it, and drop the guard on the way out.
    async fn resolve_conn(&self) -> QueryResult<(RusqliteConnection, Option<SqliteConnection>)> {
        if let Some(tx) = &self.tx_conn {
            Ok((tx.inner().clone(), None))
        } else {
            let conn = self
                .pool
                .get()
                .await
                .map_err(|e| QueryError::connection(e.to_string()).with_source(e))?;
            let handle = conn.inner().clone();
            Ok((handle, Some(conn)))
        }
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
        let (handle, _guard) = self.resolve_conn().await?;
        let bound = Self::bind(&params);
        let snapshots: Vec<SqliteRowRef> = handle
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
        let (handle, _guard) = self.resolve_conn().await?;
        let bound = Self::bind(&params);
        let snapshot: Option<SqliteRowRef> = handle
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
        let (handle, _guard) = self.resolve_conn().await?;
        let bound = Self::bind(&params);
        let n = handle
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
        let (handle, _guard) = self.resolve_conn().await?;
        let bound = Self::bind(&params);
        let n = handle
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

    fn aggregate_query(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Vec<std::collections::HashMap<String, FilterValue>>>> {
        let sql = sql.to_string();
        Box::pin(async move {
            trace!(sql = %sql, "sqlite aggregate_query");
            let (handle, _guard) = self.resolve_conn().await?;
            let bound = Self::bind(&params);
            let rows: Vec<std::collections::HashMap<String, FilterValue>> = handle
                .call(move |c| {
                    let mut stmt = c.prepare(&sql)?;
                    let column_names: Vec<String> =
                        stmt.column_names().iter().map(|s| s.to_string()).collect();
                    let refs: Vec<&dyn rusqlite::ToSql> =
                        bound.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
                    let mut rows = stmt.query(refs.as_slice())?;
                    let mut out = Vec::new();
                    while let Some(row) = rows.next()? {
                        let mut map = std::collections::HashMap::new();
                        for (i, name) in column_names.iter().enumerate() {
                            // SQLite storage classes are dynamic per-row:
                            // INTEGER, REAL, TEXT, BLOB, NULL. Pull each
                            // cell as an untyped `Value` and project into
                            // the closest `FilterValue` variant. BLOB
                            // becomes `Null` — aggregate results never
                            // return BLOB in practice, and surfacing raw
                            // bytes through FilterValue doesn't buy
                            // anything for the caller.
                            let v: SqlValue = row.get(i).unwrap_or(SqlValue::Null);
                            let fv = match v {
                                SqlValue::Null => FilterValue::Null,
                                SqlValue::Integer(n) => FilterValue::Int(n),
                                SqlValue::Real(f) => FilterValue::Float(f),
                                SqlValue::Text(s) => FilterValue::String(s),
                                SqlValue::Blob(_) => FilterValue::Null,
                            };
                            map.insert(name.clone(), fv);
                        }
                        out.push(map);
                    }
                    Ok(out)
                })
                .await
                .map_err(|e| QueryError::database(e.to_string()).with_source(e))?;
            Ok(rows)
        })
    }

    fn transaction<'a, R, Fut, F>(&'a self, f: F) -> BoxFuture<'a, QueryResult<R>>
    where
        F: FnOnce(Self) -> Fut + Send + 'a,
        Fut: std::future::Future<Output = QueryResult<R>> + Send + 'a,
        R: Send + 'a,
        Self: Clone,
    {
        Box::pin(async move {
            // Refuse nested transactions until savepoint support
            // lands. Callers can still drive SAVEPOINT / RELEASE
            // manually via `execute_raw` if they need it.
            if self.tx_conn.is_some() {
                return Err(QueryError::internal(
                    "nested transactions not yet implemented \
                     (call .transaction() on the outer engine only, or \
                     issue SAVEPOINT via execute_raw)",
                ));
            }

            // Pin a single pooled connection. Wrapping it in `Arc`
            // keeps the pool permit alive for the whole transaction
            // and makes every engine clone share the same
            // `tokio_rusqlite::Connection` handle — every query
            // dispatched through the closure's engine therefore
            // serialises onto the same background thread as our
            // initial `BEGIN`.
            let conn = self
                .pool
                .get()
                .await
                .map_err(|e| QueryError::connection(e.to_string()).with_source(e))?;
            let handle = conn.inner().clone();
            handle
                .call(|c| {
                    c.execute_batch("BEGIN")?;
                    Ok(())
                })
                .await
                .map_err(|e| QueryError::database(e.to_string()).with_source(e))?;

            let tx_conn = Arc::new(conn);
            let tx_engine = SqliteEngine {
                pool: self.pool.clone(),
                tx_conn: Some(tx_conn.clone()),
            };

            let result = f(tx_engine).await;

            // Finalise: COMMIT on success, best-effort ROLLBACK on
            // failure. Preserve the caller's error if ROLLBACK fails —
            // SQLite's autocommit resumes on the next statement
            // regardless, and dropping the connection releases any
            // lingering tx state.
            match result {
                Ok(v) => {
                    handle
                        .call(|c| {
                            c.execute_batch("COMMIT")?;
                            Ok(())
                        })
                        .await
                        .map_err(|e| QueryError::database(e.to_string()).with_source(e))?;
                    Ok(v)
                }
                Err(e) => {
                    let _ = handle
                        .call(|c| {
                            c.execute_batch("ROLLBACK")?;
                            Ok(())
                        })
                        .await;
                    Err(e)
                }
            }
        })
    }
}
