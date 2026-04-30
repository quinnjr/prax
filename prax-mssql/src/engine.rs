//! Microsoft SQL Server query engine implementation.

use std::marker::PhantomData;
use std::sync::Arc;

use bb8::PooledConnection;
use bb8_tiberius::ConnectionManager;
use prax_query::QueryResult;
use prax_query::filter::FilterValue;
use prax_query::row::FromRow;
use prax_query::traits::{BoxFuture, Model, QueryEngine};
use tokio::sync::Mutex;
use tracing::trace;

use crate::pool::MssqlPool;
use crate::row_ref::MssqlRowRef;
use crate::types::filter_value_to_sql;

/// `PooledConnection` with the `'static` lifetime emitted by
/// [`bb8::Pool::get_owned`]. Needed because in-flight transactions
/// must outlive any stack frame — we hand the tx-bound engine back
/// to the closure, which can pass it into spawned tasks.
type OwnedTxClient = PooledConnection<'static, ConnectionManager>;

/// Microsoft SQL Server query engine that implements the Prax QueryEngine trait.
///
/// # Transaction mode
///
/// `MssqlEngine` has two modes, controlled by `tx_conn`:
///
/// - **Pool mode** (`tx_conn == None`, default): each query acquires
///   a fresh connection via the short-lived `MssqlPool::get()` and
///   drops it on the way out.
/// - **Transaction mode** (`tx_conn == Some(..)`): each query locks
///   the pinned [`OwnedTxClient`] so every `BEGIN`, closure-emitted
///   statement, and `COMMIT`/`ROLLBACK` runs on the same physical
///   connection. The mutex is `tokio::sync::Mutex` because tiberius
///   calls are async and the lock has to span `.await`.
#[derive(Clone)]
pub struct MssqlEngine {
    pool: MssqlPool,
    /// Present when this engine is bound to an in-flight transaction.
    /// `None` in the normal pool-backed case.
    tx_conn: Option<Arc<Mutex<OwnedTxClient>>>,
}

impl MssqlEngine {
    /// Create a new MSSQL engine with the given connection pool.
    pub fn new(pool: MssqlPool) -> Self {
        Self {
            pool,
            tx_conn: None,
        }
    }

    /// Get a reference to the connection pool.
    pub fn pool(&self) -> &MssqlPool {
        &self.pool
    }

    /// Convert filter values to MSSQL parameters.
    fn to_params(
        values: &[FilterValue],
    ) -> Result<Vec<Box<dyn tiberius::ToSql>>, prax_query::QueryError> {
        values
            .iter()
            .map(|v| {
                filter_value_to_sql(v).map_err(|e| {
                    let msg = e.to_string();
                    prax_query::QueryError::database(msg).with_source(e)
                })
            })
            .collect()
    }

    /// Convert PostgreSQL-style parameter placeholders ($1, $2) to MSSQL-style (@P1, @P2).
    fn convert_params(sql: &str) -> String {
        let mut result = sql.to_string();
        let mut i = 1;

        while result.contains(&format!("${}", i)) {
            result = result.replace(&format!("${}", i), &format!("@P{}", i));
            i += 1;
        }

        result
    }

    /// T-SQL places `OUTPUT INSERTED.*` between the column list and
    /// `VALUES ...`, not at the end of the statement. The cross-dialect
    /// `build_sql` in prax-query appends the dialect's returning clause
    /// after `VALUES (...)`, which works for Postgres / SQLite / (future)
    /// MySQL RETURNING but yields a 102 "Incorrect syntax near 'OUTPUT'"
    /// error against SQL Server. We rearrange the statement here so the
    /// generic builder doesn't need to know about T-SQL clause ordering.
    ///
    /// Input:  `INSERT INTO t (c1,c2) VALUES (@P1,@P2) OUTPUT INSERTED.*`
    /// Output: `INSERT INTO t (c1,c2) OUTPUT INSERTED.* VALUES (@P1,@P2)`
    ///
    /// Leaves the SQL untouched if no ` OUTPUT ` clause is present (e.g.
    /// raw SQL from `QueryEngine::execute_raw`) or if the clause is
    /// already correctly positioned before `VALUES`.
    fn rearrange_output_for_insert(sql: &str) -> String {
        let Some(output_idx) = sql.rfind(" OUTPUT ") else {
            return sql.to_string();
        };
        let Some(values_idx) = sql.find(" VALUES ") else {
            return sql.to_string();
        };
        if output_idx < values_idx {
            // already in T-SQL order
            return sql.to_string();
        }
        let prefix = &sql[..values_idx];
        let output_clause = &sql[output_idx..];
        let values_clause = &sql[values_idx..output_idx];
        format!("{prefix}{output_clause}{values_clause}")
    }

    /// For UPDATE, T-SQL places `OUTPUT INSERTED.*` between the SET
    /// clause and the WHERE clause. Mirrors `rearrange_output_for_insert`
    /// but anchors on ` WHERE ` instead of ` VALUES `. Update statements
    /// without a WHERE clause leave the trailing OUTPUT in place — that's
    /// already a T-SQL-legal form (`UPDATE t SET c=v OUTPUT INSERTED.*`).
    fn rearrange_output_for_update(sql: &str) -> String {
        let Some(output_idx) = sql.rfind(" OUTPUT ") else {
            return sql.to_string();
        };
        let Some(where_idx) = sql.find(" WHERE ") else {
            // OUTPUT at the end of a WHERE-less UPDATE is already legal.
            return sql.to_string();
        };
        if output_idx < where_idx {
            return sql.to_string();
        }
        let prefix = &sql[..where_idx];
        let output_clause = &sql[output_idx..];
        let where_clause = &sql[where_idx..output_idx];
        format!("{prefix}{output_clause}{where_clause}")
    }

    /// Decode a single row via the MssqlRowRef bridge.
    ///
    /// # Short-circuit on decode error
    ///
    /// When called via `.iter().map(Self::decode_row).collect()`, the
    /// iterator short-circuits on the first decode error and discards
    /// every successfully-decoded row before it. A row-level type
    /// mismatch therefore aborts the whole batch rather than returning
    /// partial results. Callers that want per-row recovery should
    /// manually iterate and handle each result.
    fn decode_row<T: FromRow>(row: &tiberius::Row) -> prax_query::QueryResult<T> {
        let row_ref = MssqlRowRef::from_row(row).map_err(|e| {
            let msg = e.to_string();
            prax_query::QueryError::deserialization(msg).with_source(e)
        })?;
        T::from_row(&row_ref).map_err(|e| {
            let msg = e.to_string();
            prax_query::QueryError::deserialization(msg).with_source(e)
        })
    }
}

impl QueryEngine for MssqlEngine {
    fn dialect(&self) -> &dyn prax_query::dialect::SqlDialect {
        &prax_query::dialect::Mssql
    }

    fn query_many<T: Model + FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Vec<T>>> {
        let sql = Self::convert_params(sql);
        Box::pin(async move {
            trace!(sql = %sql, "Executing query_many");

            let mssql_params = Self::to_params(&params)?;
            let param_refs: Vec<&dyn tiberius::ToSql> =
                mssql_params.iter().map(|p| p.as_ref()).collect();

            let rows =
                if let Some(tx) = &self.tx_conn {
                    // Tx mode: drive the pinned connection so the query
                    // lands inside the same BEGIN/COMMIT block as every
                    // sibling call. `PooledConnection` derefs to the
                    // inner `tiberius::Client`, so `.query()` here is the
                    // raw tiberius method (not the MssqlConnection wrapper).
                    let mut guard = tx.lock().await;
                    let stream = guard.query(&sql, &param_refs).await.map_err(|e| {
                        prax_query::QueryError::database(e.to_string()).with_source(e)
                    })?;
                    stream.into_first_result().await.map_err(|e| {
                        prax_query::QueryError::database(e.to_string()).with_source(e)
                    })?
                } else {
                    let mut conn = self.pool.get().await.map_err(|e| {
                        prax_query::QueryError::connection(e.to_string()).with_source(e)
                    })?;
                    conn.query(&sql, &param_refs).await.map_err(|e| {
                        prax_query::QueryError::database(e.to_string()).with_source(e)
                    })?
                };

            rows.iter().map(Self::decode_row).collect()
        })
    }

    fn query_one<T: Model + FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<T>> {
        let sql = Self::convert_params(sql);
        Box::pin(async move {
            trace!(sql = %sql, "Executing query_one");

            let mssql_params = Self::to_params(&params)?;
            let param_refs: Vec<&dyn tiberius::ToSql> =
                mssql_params.iter().map(|p| p.as_ref()).collect();

            // Shared `no rows` → `NotFound` translation, factored out
            // so both dispatch arms convert the same error text the
            // same way.
            let map_err = |e: tiberius::error::Error| -> prax_query::QueryError {
                let msg = e.to_string();
                if msg.contains("no rows") || msg.contains("returned no rows") {
                    prax_query::QueryError::not_found(T::MODEL_NAME)
                } else {
                    prax_query::QueryError::database(msg).with_source(e)
                }
            };

            let rows = if let Some(tx) = &self.tx_conn {
                let mut guard = tx.lock().await;
                let stream = guard.query(&sql, &param_refs).await.map_err(map_err)?;
                stream.into_first_result().await.map_err(map_err)?
            } else {
                let mut conn = self.pool.get().await.map_err(|e| {
                    prax_query::QueryError::connection(e.to_string()).with_source(e)
                })?;
                // Use the underlying client directly so we reuse the
                // same tiberius-error mapping as the tx arm. The
                // wrapper's `query_one` has its own "no rows" check —
                // we inline the equivalent via `rows.first()` below.
                conn.inner()
                    .query(&sql, &param_refs)
                    .await
                    .map_err(map_err)?
                    .into_first_result()
                    .await
                    .map_err(map_err)?
            };

            let row = rows
                .into_iter()
                .next()
                .ok_or_else(|| prax_query::QueryError::not_found(T::MODEL_NAME))?;
            Self::decode_row(&row)
        })
    }

    fn query_optional<T: Model + FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Option<T>>> {
        let sql = Self::convert_params(sql);
        Box::pin(async move {
            trace!(sql = %sql, "Executing query_optional");

            let mssql_params = Self::to_params(&params)?;
            let param_refs: Vec<&dyn tiberius::ToSql> =
                mssql_params.iter().map(|p| p.as_ref()).collect();

            let rows =
                if let Some(tx) = &self.tx_conn {
                    let mut guard = tx.lock().await;
                    let stream = guard.query(&sql, &param_refs).await.map_err(|e| {
                        prax_query::QueryError::database(e.to_string()).with_source(e)
                    })?;
                    stream.into_first_result().await.map_err(|e| {
                        prax_query::QueryError::database(e.to_string()).with_source(e)
                    })?
                } else {
                    let mut conn = self.pool.get().await.map_err(|e| {
                        prax_query::QueryError::connection(e.to_string()).with_source(e)
                    })?;
                    conn.query(&sql, &param_refs).await.map_err(|e| {
                        prax_query::QueryError::database(e.to_string()).with_source(e)
                    })?
                };

            match rows.into_iter().next() {
                Some(r) => Self::decode_row(&r).map(Some),
                None => Ok(None),
            }
        })
    }

    fn execute_insert<T: Model + FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<T>> {
        let sql = Self::rearrange_output_for_insert(&Self::convert_params(sql));
        Box::pin(async move {
            trace!(sql = %sql, "Executing insert");

            let mssql_params = Self::to_params(&params)?;
            let param_refs: Vec<&dyn tiberius::ToSql> =
                mssql_params.iter().map(|p| p.as_ref()).collect();

            // INSERT with OUTPUT returns rows through a result
            // stream. Drive either the pinned tx client or a fresh
            // pool connection, materialize the first row (exactly one
            // OUTPUT row per inserted tuple), then decode.
            let rows =
                if let Some(tx) = &self.tx_conn {
                    let mut guard = tx.lock().await;
                    let stream = guard.query(&sql, &param_refs).await.map_err(|e| {
                        prax_query::QueryError::database(e.to_string()).with_source(e)
                    })?;
                    stream.into_first_result().await.map_err(|e| {
                        prax_query::QueryError::database(e.to_string()).with_source(e)
                    })?
                } else {
                    let mut conn = self.pool.get().await.map_err(|e| {
                        prax_query::QueryError::connection(e.to_string()).with_source(e)
                    })?;
                    conn.query(&sql, &param_refs).await.map_err(|e| {
                        prax_query::QueryError::database(e.to_string()).with_source(e)
                    })?
                };

            let row = rows
                .into_iter()
                .next()
                .ok_or_else(|| prax_query::QueryError::not_found(T::MODEL_NAME))?;
            Self::decode_row(&row)
        })
    }

    fn execute_update<T: Model + FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Vec<T>>> {
        let sql = Self::rearrange_output_for_update(&Self::convert_params(sql));
        Box::pin(async move {
            trace!(sql = %sql, "Executing update");

            let mssql_params = Self::to_params(&params)?;
            let param_refs: Vec<&dyn tiberius::ToSql> =
                mssql_params.iter().map(|p| p.as_ref()).collect();

            let rows =
                if let Some(tx) = &self.tx_conn {
                    let mut guard = tx.lock().await;
                    let stream = guard.query(&sql, &param_refs).await.map_err(|e| {
                        prax_query::QueryError::database(e.to_string()).with_source(e)
                    })?;
                    stream.into_first_result().await.map_err(|e| {
                        prax_query::QueryError::database(e.to_string()).with_source(e)
                    })?
                } else {
                    let mut conn = self.pool.get().await.map_err(|e| {
                        prax_query::QueryError::connection(e.to_string()).with_source(e)
                    })?;
                    conn.query(&sql, &param_refs).await.map_err(|e| {
                        prax_query::QueryError::database(e.to_string()).with_source(e)
                    })?
                };

            rows.iter().map(Self::decode_row).collect()
        })
    }

    fn execute_delete(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<u64>> {
        let sql = Self::convert_params(sql);
        Box::pin(async move {
            trace!(sql = %sql, "Executing delete");

            let mssql_params = Self::to_params(&params)?;
            let param_refs: Vec<&dyn tiberius::ToSql> =
                mssql_params.iter().map(|p| p.as_ref()).collect();

            if let Some(tx) = &self.tx_conn {
                // `MssqlConnection::execute` already folds
                // `ExecuteResult::total()`; the raw tiberius
                // `execute` hit via the tx guard returns
                // `ExecuteResult` directly, so we call `.total()`
                // here to match.
                let mut guard = tx.lock().await;
                let result = guard
                    .execute(&sql, &param_refs)
                    .await
                    .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?;
                Ok(result.total())
            } else {
                let mut conn = self.pool.get().await.map_err(|e| {
                    prax_query::QueryError::connection(e.to_string()).with_source(e)
                })?;
                conn.execute(&sql, &param_refs)
                    .await
                    .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))
            }
        })
    }

    fn execute_raw(&self, sql: &str, params: Vec<FilterValue>) -> BoxFuture<'_, QueryResult<u64>> {
        let sql = Self::convert_params(sql);
        Box::pin(async move {
            trace!(sql = %sql, "Executing raw SQL");

            let mssql_params = Self::to_params(&params)?;
            let param_refs: Vec<&dyn tiberius::ToSql> =
                mssql_params.iter().map(|p| p.as_ref()).collect();

            if let Some(tx) = &self.tx_conn {
                let mut guard = tx.lock().await;
                let result = guard
                    .execute(&sql, &param_refs)
                    .await
                    .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?;
                Ok(result.total())
            } else {
                let mut conn = self.pool.get().await.map_err(|e| {
                    prax_query::QueryError::connection(e.to_string()).with_source(e)
                })?;
                conn.execute(&sql, &param_refs)
                    .await
                    .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))
            }
        })
    }

    fn count(&self, sql: &str, params: Vec<FilterValue>) -> BoxFuture<'_, QueryResult<u64>> {
        let sql = Self::convert_params(sql);
        Box::pin(async move {
            trace!(sql = %sql, "Executing count");

            let mssql_params = Self::to_params(&params)?;
            let param_refs: Vec<&dyn tiberius::ToSql> =
                mssql_params.iter().map(|p| p.as_ref()).collect();

            let rows =
                if let Some(tx) = &self.tx_conn {
                    let mut guard = tx.lock().await;
                    let stream = guard.query(&sql, &param_refs).await.map_err(|e| {
                        prax_query::QueryError::database(e.to_string()).with_source(e)
                    })?;
                    stream.into_first_result().await.map_err(|e| {
                        prax_query::QueryError::database(e.to_string()).with_source(e)
                    })?
                } else {
                    let mut conn = self.pool.get().await.map_err(|e| {
                        prax_query::QueryError::connection(e.to_string()).with_source(e)
                    })?;
                    conn.query(&sql, &param_refs).await.map_err(|e| {
                        prax_query::QueryError::database(e.to_string()).with_source(e)
                    })?
                };
            let row = rows.into_iter().next().ok_or_else(|| {
                prax_query::QueryError::deserialization("count query returned no rows".to_string())
            })?;

            // COUNT is always INT in SQL Server (COUNT_BIG is BIGINT). Probe i64
            // first (handles COUNT_BIG), fall back to i32 for COUNT. Use try_get so
            // a type mismatch surfaces cleanly rather than being conflated with a
            // NULL column.
            match row.try_get::<i64, _>(0) {
                Ok(Some(n)) => return Ok(n as u64),
                Ok(None) => {
                    return Err(prax_query::QueryError::deserialization(
                        "count query column 0 is NULL".to_string(),
                    ));
                }
                Err(_) => {} // wrong type, fall through to i32
            }
            match row.try_get::<i32, _>(0) {
                Ok(Some(n)) => Ok(n as u64),
                Ok(None) => Err(prax_query::QueryError::deserialization(
                    "count query column 0 is NULL".to_string(),
                )),
                Err(e) => {
                    let msg = format!("count query column 0 is not an integer: {e}");
                    Err(prax_query::QueryError::deserialization(msg).with_source(e))
                }
            }
        })
    }

    fn aggregate_query(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Vec<std::collections::HashMap<String, FilterValue>>>> {
        let sql = Self::convert_params(sql);
        Box::pin(async move {
            trace!(sql = %sql, "Executing aggregate_query");

            let mssql_params = Self::to_params(&params)?;
            let param_refs: Vec<&dyn tiberius::ToSql> =
                mssql_params.iter().map(|p| p.as_ref()).collect();

            let rows =
                if let Some(tx) = &self.tx_conn {
                    let mut guard = tx.lock().await;
                    let stream = guard.query(&sql, &param_refs).await.map_err(|e| {
                        prax_query::QueryError::database(e.to_string()).with_source(e)
                    })?;
                    stream.into_first_result().await.map_err(|e| {
                        prax_query::QueryError::database(e.to_string()).with_source(e)
                    })?
                } else {
                    let mut conn = self.pool.get().await.map_err(|e| {
                        prax_query::QueryError::connection(e.to_string()).with_source(e)
                    })?;
                    conn.query(&sql, &param_refs).await.map_err(|e| {
                        prax_query::QueryError::database(e.to_string()).with_source(e)
                    })?
                };

            Ok(rows
                .iter()
                .map(|row| {
                    let mut map = std::collections::HashMap::new();
                    for (i, col) in row.columns().iter().enumerate() {
                        let name = col.name().to_string();
                        let value = decode_mssql_aggregate_cell(row, i);
                        map.insert(name, value);
                    }
                    map
                })
                .collect())
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
            // Refuse nested transactions until dialect-aware
            // SAVEPOINT support lands. Callers can still drive
            // `SAVE TRANSACTION` / `ROLLBACK TRANSACTION name`
            // manually via `execute_raw` if they need it.
            if self.tx_conn.is_some() {
                return Err(prax_query::QueryError::internal(
                    "nested transactions not yet implemented \
                     (call .transaction() on the outer engine only, or \
                     issue SAVE TRANSACTION via execute_raw)",
                ));
            }

            // Borrow a `'static`-lifetime pooled client so the tx
            // can outlive this stack frame — bb8's default `get()`
            // yields a `'_`-borrowed handle that can't cross into
            // the closure's engine clone.
            let mut conn =
                self.pool.get_owned().await.map_err(|e| {
                    prax_query::QueryError::connection(e.to_string()).with_source(e)
                })?;

            // T-SQL `BEGIN TRANSACTION` drives the connection into a
            // user transaction. We go through `simple_query` because
            // the statement takes no parameters, and drain the
            // result stream so the server actually receives the BEGIN.
            conn.simple_query("BEGIN TRANSACTION")
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?
                .into_results()
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?;

            let tx_conn = Arc::new(Mutex::new(conn));
            let tx_engine = MssqlEngine {
                pool: self.pool.clone(),
                tx_conn: Some(tx_conn.clone()),
            };

            let result = f(tx_engine).await;

            // Finalise: COMMIT on success, best-effort ROLLBACK on
            // failure. Preserve the caller's error if ROLLBACK
            // fails — the connection drops in a moment either way
            // and the server aborts the transaction on session
            // close. Drain the `into_results` stream on both arms so
            // the server actually processes the batch.
            let mut guard = tx_conn.lock().await;
            match result {
                Ok(v) => {
                    guard
                        .simple_query("COMMIT TRANSACTION")
                        .await
                        .map_err(|e| {
                            prax_query::QueryError::database(e.to_string()).with_source(e)
                        })?
                        .into_results()
                        .await
                        .map_err(|e| {
                            prax_query::QueryError::database(e.to_string()).with_source(e)
                        })?;
                    Ok(v)
                }
                Err(e) => {
                    // Drain the ROLLBACK response so the server
                    // really processes it — dropping the future
                    // mid-stream leaves the tx open. Ignore the
                    // error text (we already have the caller's).
                    if let Ok(stream) = guard.simple_query("ROLLBACK TRANSACTION").await {
                        let _ = stream.into_results().await;
                    }
                    Err(e)
                }
            }
        })
    }
}

/// Decode a single aggregate result cell from tiberius into a
/// [`FilterValue`].
///
/// MSSQL aggregates return dialect-specific types: COUNT is INT,
/// SUM over an INT column returns the source width (BIGINT for
/// COUNT_BIG), AVG returns the numeric type of the column, and
/// MIN/MAX preserves it. Tiberius exposes columns through a
/// handful of numeric + text `try_get` specializations — we probe
/// them in order and project into a [`FilterValue`] that
/// [`prax_query::operations::AggregateResult::from_row`] can
/// interpret.
///
/// Unknown / unprobeable columns surface as `FilterValue::Null`
/// rather than aborting the whole aggregate query; the caller's
/// view shows the cell as "not decoded" instead of losing every
/// aggregate.
fn decode_mssql_aggregate_cell(row: &tiberius::Row, idx: usize) -> FilterValue {
    // i64 / i32 / i16 / f64 / f32 / bool / String — probe in that
    // order so COUNT(BIGINT) hits the i64 arm before degrading to a
    // float representation that would lose precision.
    if let Ok(Some(n)) = row.try_get::<i64, _>(idx) {
        return FilterValue::Int(n);
    }
    if let Ok(Some(n)) = row.try_get::<i32, _>(idx) {
        return FilterValue::Int(n as i64);
    }
    if let Ok(Some(n)) = row.try_get::<i16, _>(idx) {
        return FilterValue::Int(n as i64);
    }
    if let Ok(Some(f)) = row.try_get::<f64, _>(idx) {
        return FilterValue::Float(f);
    }
    if let Ok(Some(f)) = row.try_get::<f32, _>(idx) {
        return FilterValue::Float(f as f64);
    }
    if let Ok(Some(b)) = row.try_get::<bool, _>(idx) {
        return FilterValue::Bool(b);
    }
    if let Ok(Some(s)) = row.try_get::<&str, _>(idx) {
        return FilterValue::String(s.to_string());
    }
    FilterValue::Null
}

/// A typed query builder that uses the MSSQL engine.
pub struct MssqlQueryBuilder<T: Model> {
    engine: MssqlEngine,
    _marker: PhantomData<T>,
}

impl<T: Model> MssqlQueryBuilder<T> {
    /// Create a new query builder.
    pub fn new(engine: MssqlEngine) -> Self {
        Self {
            engine,
            _marker: PhantomData,
        }
    }

    /// Get the underlying engine.
    pub fn engine(&self) -> &MssqlEngine {
        &self.engine
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_params() {
        assert_eq!(
            MssqlEngine::convert_params("SELECT * FROM users WHERE id = $1"),
            "SELECT * FROM users WHERE id = @P1"
        );

        assert_eq!(
            MssqlEngine::convert_params("SELECT * FROM users WHERE id = $1 AND name = $2"),
            "SELECT * FROM users WHERE id = @P1 AND name = @P2"
        );

        assert_eq!(
            MssqlEngine::convert_params("SELECT * FROM users"),
            "SELECT * FROM users"
        );
    }
}
