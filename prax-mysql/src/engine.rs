//! MySQL query engine implementing `prax_query::QueryEngine`.

use std::sync::Arc;

use mysql_async::prelude::*;
use mysql_async::{Params, Row as MyRow, Value as MyValue};
use prax_query::error::{QueryError, QueryResult};
use prax_query::filter::FilterValue;
use prax_query::row::FromRow;
use prax_query::traits::{BoxFuture, Model, QueryEngine};
use tokio::sync::Mutex;
use tracing::trace;

use crate::connection::MysqlConnection;
use crate::pool::MysqlPool;
use crate::row_ref::MysqlRowRef;
use crate::types::filter_value_to_mysql;

/// MySQL query engine backed by `mysql_async`.
///
/// # Breaking changes (0.7)
///
/// `MysqlEngine` no longer has inherent `query` / `query_one` / `query_opt`
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
///   [`crate::row_ref::MysqlRowRef::from_row`] inside a custom `FromRow`
///   impl.
/// - For the legacy JSON-blob API, use [`crate::raw::MysqlRawEngine`] +
///   [`crate::raw::MysqlJsonRow`].
/// - To run side-effecting SQL that returns no rows, call
///   [`prax_query::traits::QueryEngine::execute_raw`].
///
/// See `CHANGELOG.md` for the full migration guide.
///
/// # Transaction mode
///
/// `MysqlEngine` has two modes, controlled by `tx_conn`:
///
/// - **Pool mode** (`tx_conn == None`, default): each query acquires
///   a fresh connection from the pool and drops it after the call.
/// - **Transaction mode** (`tx_conn == Some(..)`): each query pins
///   the same [`MysqlConnection`] through an `Arc<Mutex<_>>` so
///   `BEGIN`, every closure-emitted query, and `COMMIT`/`ROLLBACK`
///   all land on the same physical connection. The mutex is
///   `tokio::sync::Mutex` because `mysql_async` calls are async and
///   the lock has to span `.await`.
#[derive(Clone)]
pub struct MysqlEngine {
    pool: MysqlPool,
    /// Present when this engine is bound to an in-flight transaction.
    /// `None` in the normal pool-backed case.
    tx_conn: Option<Arc<Mutex<MysqlConnection>>>,
}

impl MysqlEngine {
    /// Create a new engine with the given pool.
    pub fn new(pool: MysqlPool) -> Self {
        Self {
            pool,
            tx_conn: None,
        }
    }

    /// Get a reference to the connection pool.
    pub fn pool(&self) -> &MysqlPool {
        &self.pool
    }

    fn bind(params: &[FilterValue]) -> Vec<MyValue> {
        params.iter().map(filter_value_to_mysql).collect()
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
        trace!(sql = %sql, "mysql query_rows");
        let bound = Self::bind(&params);
        let rows: Vec<MyRow> = if let Some(tx) = &self.tx_conn {
            // Tx mode: drive the pinned connection so the query lands
            // inside the same BEGIN…COMMIT block as every sibling call.
            let mut guard = tx.lock().await;
            guard
                .inner_mut()
                .exec(sql.as_str(), Params::Positional(bound))
                .await
                .map_err(|e| QueryError::database(e.to_string()).with_source(e))?
        } else {
            let mut conn = self
                .pool
                .get()
                .await
                .map_err(|e| QueryError::connection(e.to_string()).with_source(e))?;
            conn.inner_mut()
                .exec(sql.as_str(), Params::Positional(bound))
                .await
                .map_err(|e| QueryError::database(e.to_string()).with_source(e))?
        };

        rows.into_iter()
            .map(|row| {
                let rr = MysqlRowRef::from_row(row).map_err(|e| {
                    let msg = e.to_string();
                    QueryError::deserialization(msg).with_source(e)
                })?;
                T::from_row(&rr).map_err(|e| {
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
        trace!(sql = %sql, "mysql query_first_row");
        let bound = Self::bind(&params);
        let row: Option<MyRow> = if let Some(tx) = &self.tx_conn {
            let mut guard = tx.lock().await;
            guard
                .inner_mut()
                .exec_first(sql.as_str(), Params::Positional(bound))
                .await
                .map_err(|e| QueryError::database(e.to_string()).with_source(e))?
        } else {
            let mut conn = self
                .pool
                .get()
                .await
                .map_err(|e| QueryError::connection(e.to_string()).with_source(e))?;
            conn.inner_mut()
                .exec_first(sql.as_str(), Params::Positional(bound))
                .await
                .map_err(|e| QueryError::database(e.to_string()).with_source(e))?
        };

        match row {
            Some(r) => {
                let rr = MysqlRowRef::from_row(r).map_err(|e| {
                    let msg = e.to_string();
                    QueryError::deserialization(msg).with_source(e)
                })?;
                let t = T::from_row(&rr).map_err(|e| {
                    let msg = e.to_string();
                    QueryError::deserialization(msg).with_source(e)
                })?;
                Ok(Some(t))
            }
            None => Ok(None),
        }
    }

    async fn exec_raw(&self, sql: String, params: Vec<FilterValue>) -> QueryResult<u64> {
        let bound = Self::bind(&params);
        if let Some(tx) = &self.tx_conn {
            let mut guard = tx.lock().await;
            guard
                .inner_mut()
                .exec_drop(sql.as_str(), Params::Positional(bound))
                .await
                .map_err(|e| QueryError::database(e.to_string()).with_source(e))?;
            Ok(guard.inner().affected_rows())
        } else {
            let mut conn = self
                .pool
                .get()
                .await
                .map_err(|e| QueryError::connection(e.to_string()).with_source(e))?;
            conn.inner_mut()
                .exec_drop(sql.as_str(), Params::Positional(bound))
                .await
                .map_err(|e| QueryError::database(e.to_string()).with_source(e))?;
            Ok(conn.inner().affected_rows())
        }
    }

    async fn count_rows(&self, sql: String, params: Vec<FilterValue>) -> QueryResult<u64> {
        let bound = Self::bind(&params);
        let count: Option<(i64,)> = if let Some(tx) = &self.tx_conn {
            let mut guard = tx.lock().await;
            guard
                .inner_mut()
                .exec_first(sql.as_str(), Params::Positional(bound))
                .await
                .map_err(|e| QueryError::database(e.to_string()).with_source(e))?
        } else {
            let mut conn = self
                .pool
                .get()
                .await
                .map_err(|e| QueryError::connection(e.to_string()).with_source(e))?;
            conn.inner_mut()
                .exec_first(sql.as_str(), Params::Positional(bound))
                .await
                .map_err(|e| QueryError::database(e.to_string()).with_source(e))?
        };
        count.map(|(n,)| n as u64).ok_or_else(|| {
            prax_query::QueryError::deserialization("count query returned no rows".to_string())
        })
    }
}

impl QueryEngine for MysqlEngine {
    fn dialect(&self) -> &dyn prax_query::dialect::SqlDialect {
        &prax_query::dialect::Mysql
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
        // MySQL 8.0 does NOT support `INSERT ... RETURNING` (that's a
        // MariaDB 10.5+ extension), so `Mysql::returning_clause` emits
        // nothing and we reconstruct the inserted row by running the
        // INSERT, grabbing `LAST_INSERT_ID()` off the same connection,
        // and issuing a follow-up SELECT on the primary key.
        //
        // The SELECT-back has to run on the same connection as the
        // INSERT — `last_insert_id` is a per-session value, and the
        // pool could hand a different connection to a later call. We
        // therefore borrow the connection once and issue both
        // statements on it.
        let sql = sql.to_string();
        Box::pin(async move {
            if T::PRIMARY_KEY.len() != 1 {
                return Err(QueryError::database(format!(
                    "MySQL execute_insert requires a single-column primary \
                     key on {} to look up the inserted row via \
                     LAST_INSERT_ID(); got {} PK columns",
                    T::MODEL_NAME,
                    T::PRIMARY_KEY.len(),
                )));
            }
            let pk = T::PRIMARY_KEY[0];

            let bound = Self::bind(&params);
            let select_sql = format!(
                "SELECT {cols} FROM {table} WHERE {pk} = ?",
                cols = T::COLUMNS.join(", "),
                table = T::TABLE_NAME,
                pk = pk,
            );

            trace!(sql = %sql, "mysql execute_insert");

            // In tx mode, drive the pinned connection so INSERT,
            // LAST_INSERT_ID and the SELECT-back all land on the same
            // session. In pool mode, borrow a single connection for
            // the same reason — `last_insert_id` is per-session and
            // the pool could otherwise hand a different connection to
            // the SELECT.
            let row: Option<MyRow> = if let Some(tx) = &self.tx_conn {
                let mut guard = tx.lock().await;
                guard
                    .inner_mut()
                    .exec_drop(sql.as_str(), Params::Positional(bound))
                    .await
                    .map_err(|e| QueryError::database(e.to_string()).with_source(e))?;
                let last_id = guard.inner().last_insert_id().ok_or_else(|| {
                    QueryError::database(format!(
                        "MySQL execute_insert: no LAST_INSERT_ID after inserting \
                         into {} — does the table have an AUTO_INCREMENT column?",
                        T::TABLE_NAME,
                    ))
                })?;
                trace!(sql = %select_sql, id = last_id, "mysql execute_insert select-back");
                guard
                    .inner_mut()
                    .exec_first(
                        select_sql.as_str(),
                        Params::Positional(vec![MyValue::UInt(last_id)]),
                    )
                    .await
                    .map_err(|e| QueryError::database(e.to_string()).with_source(e))?
            } else {
                let mut conn = self
                    .pool
                    .get()
                    .await
                    .map_err(|e| QueryError::connection(e.to_string()).with_source(e))?;
                conn.inner_mut()
                    .exec_drop(sql.as_str(), Params::Positional(bound))
                    .await
                    .map_err(|e| QueryError::database(e.to_string()).with_source(e))?;
                let last_id = conn.inner().last_insert_id().ok_or_else(|| {
                    QueryError::database(format!(
                        "MySQL execute_insert: no LAST_INSERT_ID after inserting \
                         into {} — does the table have an AUTO_INCREMENT column?",
                        T::TABLE_NAME,
                    ))
                })?;
                trace!(sql = %select_sql, id = last_id, "mysql execute_insert select-back");
                conn.inner_mut()
                    .exec_first(
                        select_sql.as_str(),
                        Params::Positional(vec![MyValue::UInt(last_id)]),
                    )
                    .await
                    .map_err(|e| QueryError::database(e.to_string()).with_source(e))?
            };
            let row = row.ok_or_else(|| QueryError::not_found(T::MODEL_NAME))?;
            let rr = MysqlRowRef::from_row(row).map_err(|e| {
                let msg = e.to_string();
                QueryError::deserialization(msg).with_source(e)
            })?;
            T::from_row(&rr).map_err(|e| {
                let msg = e.to_string();
                QueryError::deserialization(msg).with_source(e)
            })
        })
    }

    fn execute_update<T: Model + FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Vec<T>>> {
        // MySQL 8.0 has no `UPDATE ... RETURNING`, so the update builder
        // emits a plain UPDATE (Mysql::returning_clause returns "").
        // We run it, then re-run the WHERE against a SELECT on the same
        // connection to pull back the updated rows.
        //
        // `sql` already has the form `UPDATE <table> SET <...> WHERE
        // <filter>`. We reuse the `WHERE <filter>` portion by splitting
        // on " WHERE " — every generated UPDATE always includes a WHERE
        // clause (UpdateOperation requires `.r#where(...)`), and the
        // WHERE parameters are bound positionally after the SET
        // parameters so we need to extract just the WHERE-phase params
        // to rebind on the SELECT.
        let sql = sql.to_string();
        Box::pin(async move {
            let bound = Self::bind(&params);

            trace!(sql = %sql, "mysql execute_update");

            // Extract the `WHERE ...` tail so we can re-SELECT with it.
            // UpdateOperation::build_sql always produces `... WHERE <filter>`.
            let where_idx = sql.rfind(" WHERE ").ok_or_else(|| {
                QueryError::database(
                    "MySQL execute_update expected a WHERE clause in the \
                     generated SQL; got none. Cannot fetch updated rows."
                        .to_string(),
                )
            })?;
            let where_clause = &sql[where_idx..];

            // The WHERE params follow the SET params in `params`. Count
            // the `?` placeholders in the UPDATE body (before `WHERE`)
            // to know how many to skip.
            let set_body = &sql[..where_idx];
            let set_param_count = set_body.matches('?').count();
            let where_params: Vec<FilterValue> = params.into_iter().skip(set_param_count).collect();

            let select_sql = format!(
                "SELECT {cols} FROM {table}{where_clause}",
                cols = T::COLUMNS.join(", "),
                table = T::TABLE_NAME,
            );
            trace!(sql = %select_sql, "mysql execute_update select-back");
            let where_bound = Self::bind(&where_params);

            // Tx mode pins the connection so UPDATE + SELECT see the
            // same snapshot. Pool mode borrows one connection for the
            // same reason — otherwise MySQL's REPEATABLE READ default
            // could let the SELECT land on a pre-UPDATE snapshot.
            let rows: Vec<MyRow> = if let Some(tx) = &self.tx_conn {
                let mut guard = tx.lock().await;
                guard
                    .inner_mut()
                    .exec_drop(sql.as_str(), Params::Positional(bound))
                    .await
                    .map_err(|e| QueryError::database(e.to_string()).with_source(e))?;
                guard
                    .inner_mut()
                    .exec(select_sql.as_str(), Params::Positional(where_bound))
                    .await
                    .map_err(|e| QueryError::database(e.to_string()).with_source(e))?
            } else {
                let mut conn = self
                    .pool
                    .get()
                    .await
                    .map_err(|e| QueryError::connection(e.to_string()).with_source(e))?;
                conn.inner_mut()
                    .exec_drop(sql.as_str(), Params::Positional(bound))
                    .await
                    .map_err(|e| QueryError::database(e.to_string()).with_source(e))?;
                conn.inner_mut()
                    .exec(select_sql.as_str(), Params::Positional(where_bound))
                    .await
                    .map_err(|e| QueryError::database(e.to_string()).with_source(e))?
            };
            rows.into_iter()
                .map(|row| {
                    let rr = MysqlRowRef::from_row(row).map_err(|e| {
                        let msg = e.to_string();
                        QueryError::deserialization(msg).with_source(e)
                    })?;
                    T::from_row(&rr).map_err(|e| {
                        let msg = e.to_string();
                        QueryError::deserialization(msg).with_source(e)
                    })
                })
                .collect()
        })
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
            trace!(sql = %sql, "mysql aggregate_query");
            let bound = Self::bind(&params);
            let rows: Vec<MyRow> = if let Some(tx) = &self.tx_conn {
                let mut guard = tx.lock().await;
                guard
                    .inner_mut()
                    .exec(sql.as_str(), Params::Positional(bound))
                    .await
                    .map_err(|e| QueryError::database(e.to_string()).with_source(e))?
            } else {
                let mut conn = self
                    .pool
                    .get()
                    .await
                    .map_err(|e| QueryError::connection(e.to_string()).with_source(e))?;
                conn.inner_mut()
                    .exec(sql.as_str(), Params::Positional(bound))
                    .await
                    .map_err(|e| QueryError::database(e.to_string()).with_source(e))?
            };

            Ok(rows
                .into_iter()
                .map(|row| {
                    let mut map = std::collections::HashMap::new();
                    // Snapshot the column list once per row — `columns_ref`
                    // returns an Arc<[Column]> slice which we clone out to
                    // avoid borrowing `row` across per-column takes.
                    let cols: Vec<(String, usize)> = row
                        .columns_ref()
                        .iter()
                        .enumerate()
                        .map(|(i, c)| (c.name_str().to_string(), i))
                        .collect();
                    for (name, idx) in cols {
                        let value = decode_mysql_aggregate_cell(&row, idx);
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
            // Refuse nested transactions until dialect-aware SAVEPOINT
            // support lands. Callers can still drive SAVEPOINT / RELEASE
            // manually via `execute_raw` if they need it.
            if self.tx_conn.is_some() {
                return Err(QueryError::internal(
                    "nested transactions not yet implemented \
                     (call .transaction() on the outer engine only, or \
                     issue SAVEPOINT via execute_raw)",
                ));
            }

            // Pin a single connection for the duration of the tx.
            // mysql_async's `Transaction<'_>` type borrows from its
            // `Conn`, which would force a `mem::transmute` to bundle
            // both into a heap cell. We follow the `PgEngine` fallback
            // instead: issue raw `START TRANSACTION` and let the
            // connection's own session state carry the transaction.
            let mut conn = self
                .pool
                .get()
                .await
                .map_err(|e| QueryError::connection(e.to_string()).with_source(e))?;
            conn.inner_mut()
                .query_drop("START TRANSACTION")
                .await
                .map_err(|e| QueryError::database(e.to_string()).with_source(e))?;

            let tx_conn = Arc::new(Mutex::new(conn));
            let tx_engine = MysqlEngine {
                pool: self.pool.clone(),
                tx_conn: Some(tx_conn.clone()),
            };

            // Run the caller's closure on the tx-bound engine clone.
            let result = f(tx_engine).await;

            // Finalise: COMMIT on success, best-effort ROLLBACK on
            // failure. Preserve the caller's error if ROLLBACK fails —
            // the connection drops in a moment either way and the
            // server aborts the transaction on session close.
            let mut guard = tx_conn.lock().await;
            match result {
                Ok(v) => {
                    guard
                        .inner_mut()
                        .query_drop("COMMIT")
                        .await
                        .map_err(|e| QueryError::database(e.to_string()).with_source(e))?;
                    Ok(v)
                }
                Err(e) => {
                    let _ = guard.inner_mut().query_drop("ROLLBACK").await;
                    Err(e)
                }
            }
        })
    }
}

/// Decode a MySQL aggregate result cell into a [`FilterValue`].
///
/// MySQL returns aggregates with dialect-specific widths: COUNT is
/// BIGINT, SUM over an INT column returns DECIMAL, AVG returns
/// DECIMAL, MIN/MAX preserves the source column's width. We can't
/// know those in advance for arbitrary aggregate queries, so we
/// introspect the raw `mysql_async::Value` tag and project to the
/// closest `FilterValue`. DECIMAL arrives as `Value::Bytes` (text
/// encoding) — we parse to f64 so the `AggregateResult` folder's
/// `sum`/`avg` HashMap picks up a usable number.
///
/// `Value::NULL` maps to `FilterValue::Null`. Unknown variants fall
/// back to their debug-text representation so a novel column type
/// doesn't silently drop.
fn decode_mysql_aggregate_cell(row: &MyRow, idx: usize) -> FilterValue {
    use mysql_async::from_value_opt;
    let raw = match row.as_ref(idx) {
        Some(v) => v.clone(),
        None => return FilterValue::Null,
    };
    match raw {
        MyValue::NULL => FilterValue::Null,
        MyValue::Int(n) => FilterValue::Int(n),
        MyValue::UInt(n) => {
            // MySQL BIGINT UNSIGNED can exceed i64::MAX; clamp to keep
            // `FilterValue::Int` monotonic. Values in that range are
            // rare in aggregate result sets (they come from
            // BIGINT UNSIGNED columns only).
            FilterValue::Int(n.min(i64::MAX as u64) as i64)
        }
        MyValue::Float(f) => FilterValue::Float(f as f64),
        MyValue::Double(f) => FilterValue::Float(f),
        MyValue::Bytes(ref bytes) => {
            // Text-encoded values (VARCHAR, CHAR, DECIMAL, DATE, ...).
            // DECIMAL arrives here; parse to f64 if it looks numeric so
            // sum/avg accessors round-trip.
            if let Ok(s) = std::str::from_utf8(bytes) {
                if let Ok(n) = s.parse::<i64>() {
                    FilterValue::Int(n)
                } else if let Ok(f) = s.parse::<f64>() {
                    FilterValue::Float(f)
                } else {
                    FilterValue::String(s.to_string())
                }
            } else {
                // Non-UTF8 bytes (BINARY / BLOB). Fall back to lossy.
                FilterValue::String(String::from_utf8_lossy(bytes).into_owned())
            }
        }
        other => {
            // Date/Time variants — re-encode as String via
            // from_value_opt(String), which mysql_async implements for
            // every primitive. If that fails, surface Null rather than
            // aborting the whole aggregate.
            from_value_opt::<String>(other)
                .map(FilterValue::String)
                .unwrap_or(FilterValue::Null)
        }
    }
}
