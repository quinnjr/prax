//! PostgreSQL query engine implementation.

use std::marker::PhantomData;
use std::sync::Arc;

use prax_query::QueryResult;
use prax_query::filter::FilterValue;
use prax_query::traits::{BoxFuture, Model, QueryEngine};
use tracing::{error, trace};

use crate::pool::PgPool;
use crate::types::filter_value_to_sql;

/// PostgreSQL query engine that implements the Prax `QueryEngine`
/// trait.
///
/// Two modes, controlled by the `tx_conn` field:
///
/// - **Pool mode** (`tx_conn == None`, the default): each query
///   acquires a fresh connection from [`PgPool`] and drops it after
///   the call.
/// - **Transaction mode** (`tx_conn == Some(conn)`): each query routes
///   through the single pinned [`deadpool_postgres::Object`]. The
///   tx-bound engine is built by [`PgEngine::transaction`], which
///   issues a raw `BEGIN`; the outer future then runs `COMMIT` or
///   `ROLLBACK` on the same connection based on the closure's
///   `Ok` / `Err` result.
///
/// We lean on raw `BEGIN` / `COMMIT` / `ROLLBACK` strings instead of
/// `tokio_postgres::Transaction<'_>` because `Transaction<'_>` borrows
/// from its owning `Client`, and bundling both into a heap cell
/// requires `mem::transmute` gymnastics to launder the lifetime to
/// `'static`. Since `Object` implements `Deref<Target = Client>` and
/// `Client::query` / `execute` take `&self`, an `Arc<Object>` is all
/// we need — every engine clone can share it freely, and the last
/// clone drops the `Arc`, which drops the `Object` back to the pool.
/// This path is explicitly sanctioned by the task plan's "fall back"
/// guardrail.
#[derive(Clone)]
pub struct PgEngine {
    pool: PgPool,
    /// Present when this engine is bound to an in-flight transaction.
    /// `None` in the normal pool-backed case.
    tx_conn: Option<Arc<deadpool_postgres::Object>>,
}

impl PgEngine {
    /// Create a new PostgreSQL engine with the given connection pool.
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            tx_conn: None,
        }
    }

    /// Get a reference to the connection pool.
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Convert filter values to PostgreSQL parameters.
    #[allow(clippy::result_large_err)]
    fn to_params(
        values: &[FilterValue],
    ) -> Result<Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>>, prax_query::QueryError>
    {
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
}

/// Map a `tokio_postgres` driver error onto the [`prax_query::QueryError`]
/// taxonomy, categorizing by SQLSTATE when the server supplied one.
///
/// Route a driver-layer error through the SQLSTATE categorization in
/// [`crate::error`] (`From<PgError> for QueryError`): unique (23505) and
/// foreign-key (23503) violations become `constraint_violation`, not-null
/// violations (23502) become `invalid_input`, and anything else stays a
/// generic database error. The driver error is preserved as the source.
/// Accepts both `PgError` (pool-mode path) and `tokio_postgres::Error`
/// (tx-mode path, converted via the `#[from]` variant).
fn map_driver_err<E: Into<crate::error::PgError>>(e: E) -> prax_query::QueryError {
    prax_query::QueryError::from(e.into())
}

/// Panic/cancellation guard for the transaction path.
///
/// Armed right after `BEGIN` and disarmed once the closure's future has
/// resolved and the explicit `COMMIT`/`ROLLBACK` step takes over. If the
/// guard is dropped while still armed — the closure panicked and the
/// transaction future is unwinding, or the outer task was cancelled —
/// the pinned connection would otherwise return to the pool with the
/// transaction still open: dropping the last `Arc` merely recycles the
/// `Object`, and no `PgRecycling` method rolls back an open transaction
/// (`Clean` resets session state but runs inside it), so the next checkout
/// would silently inherit the live transaction. The armed guard therefore retires the connection
/// instead: when it holds the last `Arc` handle it takes the `Object`
/// out of the pool so the session closes and the server aborts the
/// transaction; when the caller stashed a tx-engine clone the handle
/// cannot be reclaimed and it can only log.
struct TxPanicGuard {
    tx_conn: Option<Arc<deadpool_postgres::Object>>,
}

impl TxPanicGuard {
    fn new(tx_conn: Arc<deadpool_postgres::Object>) -> Self {
        Self {
            tx_conn: Some(tx_conn),
        }
    }

    /// Disarm the guard and hand the connection back for explicit
    /// COMMIT/ROLLBACK handling. Call only after the closure's future
    /// has resolved normally.
    fn disarm(mut self) -> Arc<deadpool_postgres::Object> {
        self.tx_conn
            .take()
            .expect("TxPanicGuard always holds a connection until disarmed")
    }
}

impl Drop for TxPanicGuard {
    fn drop(&mut self) {
        let Some(tx_conn) = self.tx_conn.take() else {
            // Disarmed: the transaction was settled explicitly.
            return;
        };
        match Arc::try_unwrap(tx_conn) {
            Ok(conn) => {
                // Last handle: dropping the bare `Client` closes the
                // session (the server then aborts the open tx) and
                // shrinks the pool by one instead of recycling a
                // connection with a live transaction.
                error!(
                    "transaction closure panicked (or was cancelled) with BEGIN still \
                     open; retiring the pinned connection from the pool so the open \
                     transaction aborts with the session"
                );
                let _client = deadpool_postgres::Object::take(conn);
            }
            Err(_) => {
                error!(
                    "transaction closure panicked (or was cancelled) with BEGIN still \
                     open, and a cloned tx engine may still reference the pinned \
                     connection; it cannot be retired, so the open transaction may \
                     leak back into the pool"
                );
            }
        }
    }
}

impl QueryEngine for PgEngine {
    fn dialect(&self) -> &dyn prax_query::dialect::SqlDialect {
        &prax_query::dialect::Postgres
    }

    fn query_many<T: Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Vec<T>>> {
        let sql = sql.to_string();
        Box::pin(async move {
            trace!(sql = %sql, "Executing query_many");

            let pg_params = Self::to_params(&params)?;
            let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
                pg_params.iter().map(|p| p.as_ref() as _).collect();

            let rows = if let Some(tx) = &self.tx_conn {
                // Tx mode: drive the pinned connection directly so the
                // query lands inside the same BEGIN…COMMIT block as
                // every sibling call.
                tx.query(&sql, &param_refs).await.map_err(map_driver_err)?
            } else {
                let conn = self.pool.get().await.map_err(|e| {
                    prax_query::QueryError::connection(e.to_string()).with_source(e)
                })?;
                conn.query(&sql, &param_refs)
                    .await
                    .map_err(map_driver_err)?
            };

            crate::deserialize::rows_into::<T>(rows)
        })
    }

    fn query_one<T: Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<T>> {
        let sql = sql.to_string();
        Box::pin(async move {
            trace!(sql = %sql, "Executing query_one");

            let pg_params = Self::to_params(&params)?;
            let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
                pg_params.iter().map(|p| p.as_ref() as _).collect();

            // Use `query_opt` and detect the zero-row case structurally:
            // tokio-postgres reports `query_one`'s zero-row outcome as a
            // `Kind::RowCount` error ("query returned an unexpected number
            // of rows"), which error-text matching cannot reliably
            // distinguish from a genuine driver failure.
            let row = if let Some(tx) = &self.tx_conn {
                tx.query_opt(&sql, &param_refs)
                    .await
                    .map_err(map_driver_err)?
            } else {
                let conn = self.pool.get().await.map_err(|e| {
                    prax_query::QueryError::connection(e.to_string()).with_source(e)
                })?;
                conn.query_opt(&sql, &param_refs)
                    .await
                    .map_err(map_driver_err)?
            };

            let row = row.ok_or_else(|| prax_query::QueryError::not_found(T::MODEL_NAME))?;
            crate::deserialize::row_into::<T>(row)
        })
    }

    fn query_optional<T: Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Option<T>>> {
        let sql = sql.to_string();
        Box::pin(async move {
            trace!(sql = %sql, "Executing query_optional");

            let pg_params = Self::to_params(&params)?;
            let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
                pg_params.iter().map(|p| p.as_ref() as _).collect();

            let row = if let Some(tx) = &self.tx_conn {
                tx.query_opt(&sql, &param_refs)
                    .await
                    .map_err(map_driver_err)?
            } else {
                let conn = self.pool.get().await.map_err(|e| {
                    prax_query::QueryError::connection(e.to_string()).with_source(e)
                })?;
                conn.query_opt(&sql, &param_refs)
                    .await
                    .map_err(map_driver_err)?
            };

            row.map(crate::deserialize::row_into::<T>).transpose()
        })
    }

    fn execute_insert<T: Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<T>> {
        let sql = sql.to_string();
        Box::pin(async move {
            trace!(sql = %sql, "Executing insert");

            let pg_params = Self::to_params(&params)?;
            let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
                pg_params.iter().map(|p| p.as_ref() as _).collect();

            let row = if let Some(tx) = &self.tx_conn {
                tx.query_one(&sql, &param_refs)
                    .await
                    .map_err(map_driver_err)?
            } else {
                let conn = self.pool.get().await.map_err(|e| {
                    prax_query::QueryError::connection(e.to_string()).with_source(e)
                })?;
                conn.query_one(&sql, &param_refs)
                    .await
                    .map_err(map_driver_err)?
            };

            crate::deserialize::row_into::<T>(row)
        })
    }

    fn execute_update<T: Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Vec<T>>> {
        let sql = sql.to_string();
        Box::pin(async move {
            trace!(sql = %sql, "Executing update");

            let pg_params = Self::to_params(&params)?;
            let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
                pg_params.iter().map(|p| p.as_ref() as _).collect();

            let rows = if let Some(tx) = &self.tx_conn {
                tx.query(&sql, &param_refs).await.map_err(map_driver_err)?
            } else {
                let conn = self.pool.get().await.map_err(|e| {
                    prax_query::QueryError::connection(e.to_string()).with_source(e)
                })?;
                conn.query(&sql, &param_refs)
                    .await
                    .map_err(map_driver_err)?
            };

            crate::deserialize::rows_into::<T>(rows)
        })
    }

    fn execute_delete(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<u64>> {
        let sql = sql.to_string();
        Box::pin(async move {
            trace!(sql = %sql, "Executing delete");

            let pg_params = Self::to_params(&params)?;
            let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
                pg_params.iter().map(|p| p.as_ref() as _).collect();

            if let Some(tx) = &self.tx_conn {
                tx.execute(&sql, &param_refs).await.map_err(map_driver_err)
            } else {
                let conn = self.pool.get().await.map_err(|e| {
                    prax_query::QueryError::connection(e.to_string()).with_source(e)
                })?;
                conn.execute(&sql, &param_refs)
                    .await
                    .map_err(map_driver_err)
            }
        })
    }

    fn execute_raw(&self, sql: &str, params: Vec<FilterValue>) -> BoxFuture<'_, QueryResult<u64>> {
        let sql = sql.to_string();
        Box::pin(async move {
            trace!(sql = %sql, "Executing raw SQL");

            let pg_params = Self::to_params(&params)?;
            let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
                pg_params.iter().map(|p| p.as_ref() as _).collect();

            if let Some(tx) = &self.tx_conn {
                tx.execute(&sql, &param_refs).await.map_err(map_driver_err)
            } else {
                let conn = self.pool.get().await.map_err(|e| {
                    prax_query::QueryError::connection(e.to_string()).with_source(e)
                })?;
                conn.execute(&sql, &param_refs)
                    .await
                    .map_err(map_driver_err)
            }
        })
    }

    fn count(&self, sql: &str, params: Vec<FilterValue>) -> BoxFuture<'_, QueryResult<u64>> {
        let sql = sql.to_string();
        Box::pin(async move {
            trace!(sql = %sql, "Executing count");

            let pg_params = Self::to_params(&params)?;
            let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
                pg_params.iter().map(|p| p.as_ref() as _).collect();

            let row = if let Some(tx) = &self.tx_conn {
                tx.query_one(&sql, &param_refs)
                    .await
                    .map_err(map_driver_err)?
            } else {
                let conn = self.pool.get().await.map_err(|e| {
                    prax_query::QueryError::connection(e.to_string()).with_source(e)
                })?;
                conn.query_one(&sql, &param_refs)
                    .await
                    .map_err(map_driver_err)?
            };

            let count: i64 = row.try_get::<_, i64>(0).map_err(map_driver_err)?;
            Ok(count as u64)
        })
    }

    fn aggregate_query(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Vec<std::collections::HashMap<String, FilterValue>>>> {
        let sql = sql.to_string();
        Box::pin(async move {
            trace!(sql = %sql, "Executing aggregate_query");

            let pg_params = Self::to_params(&params)?;
            let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
                pg_params.iter().map(|p| p.as_ref() as _).collect();

            let rows = if let Some(tx) = &self.tx_conn {
                tx.query(&sql, &param_refs).await.map_err(map_driver_err)?
            } else {
                let conn = self.pool.get().await.map_err(|e| {
                    prax_query::QueryError::connection(e.to_string()).with_source(e)
                })?;
                conn.query(&sql, &param_refs)
                    .await
                    .map_err(map_driver_err)?
            };

            Ok(rows
                .into_iter()
                .map(|row| {
                    let mut map = std::collections::HashMap::new();
                    for (i, col) in row.columns().iter().enumerate() {
                        let name = col.name().to_string();
                        let value = decode_aggregate_cell(&row, i, col.type_());
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
            // support lands. Users can still run SAVEPOINT / RELEASE
            // manually via `execute_raw` if they need it.
            if self.tx_conn.is_some() {
                return Err(prax_query::QueryError::internal(
                    "nested transactions not yet implemented \
                     (call .transaction() on the outer engine only, or \
                     issue SAVEPOINT via execute_raw)",
                ));
            }

            // Acquire a dedicated raw `deadpool_postgres::Object`.
            // Going through `PgPool::inner()` keeps the connection
            // pinned to this future — every query the closure emits
            // will run on the same physical connection.
            let conn =
                self.pool.inner().get().await.map_err(|e| {
                    prax_query::QueryError::connection(e.to_string()).with_source(e)
                })?;

            // Issue `BEGIN` directly as a batch_execute on the raw
            // connection. Using `tokio_postgres::Transaction<'_>`
            // would bundle a borrow back into `conn`; instead we rely
            // on the connection's session state (postgres tracks the
            // BEGIN/COMMIT/ROLLBACK on the connection itself, so every
            // subsequent query on the same `Object` sees the same
            // transaction). This is the approach sanctioned by the
            // task plan's fallback guardrail.
            conn.batch_execute("BEGIN").await.map_err(map_driver_err)?;

            let tx_conn = Arc::new(conn);
            let tx_engine = PgEngine {
                pool: self.pool.clone(),
                tx_conn: Some(tx_conn.clone()),
            };

            // Arm the panic guard while the closure runs: if the
            // closure's future panics (or this future is cancelled at
            // the await) the `BEGIN` is still open, and the guard
            // retires the pinned connection instead of letting it
            // recycle into the pool with a live transaction.
            let guard = TxPanicGuard::new(tx_conn);

            // Run the caller's closure on the tx-bound engine clone.
            // When the future resolves the closure's engine clone has
            // dropped, so the guard's `Arc` is the only remaining handle
            // (unless the caller stashed a clone of the tx engine).
            let result = f(tx_engine).await;

            // The closure returned normally: take the connection back
            // from the guard and settle the transaction explicitly.
            let tx_conn = guard.disarm();

            // Finalise: COMMIT on success, best-effort ROLLBACK on
            // failure. Preserve the caller's error if rollback fails —
            // but note the pinned `Object` does *not* close on drop:
            // dropping the last `Arc` merely returns it to the pool, and
            // no `PgRecycling` method rolls back an open transaction
            // (`Clean` resets session state but runs inside it). A failed ROLLBACK on a still-live connection
            // would therefore leak a still-open transaction back into
            // the pool, contaminating whatever checks it out next. To
            // avoid that, take the connection out of the pool
            // permanently when rollback fails — possible only when this
            // is the last `Arc` handle (if the caller stashed a clone of
            // the tx engine somewhere, the connection recycles as
            // before, which is the best we can do).
            match result {
                Ok(v) => {
                    tx_conn
                        .batch_execute("COMMIT")
                        .await
                        .map_err(map_driver_err)?;
                    Ok(v)
                }
                Err(e) => {
                    if tx_conn.batch_execute("ROLLBACK").await.is_err()
                        && let Ok(conn) = Arc::try_unwrap(tx_conn)
                    {
                        // Dropping the bare `Client` closes the session
                        // (server then aborts the tx) and shrinks the
                        // pool by one instead of recycling a dirty conn.
                        let _client = deadpool_postgres::Object::take(conn);
                    }
                    Err(e)
                }
            }
        })
    }
}

/// A typed query builder that uses the PostgreSQL engine.
pub struct PgQueryBuilder<T: Model> {
    engine: PgEngine,
    _marker: PhantomData<T>,
}

impl<T: Model> PgQueryBuilder<T> {
    /// Create a new query builder.
    pub fn new(engine: PgEngine) -> Self {
        Self {
            engine,
            _marker: PhantomData,
        }
    }

    /// Get the underlying engine.
    pub fn engine(&self) -> &PgEngine {
        &self.engine
    }
}

/// Decode a single aggregate result cell by its Postgres column type.
///
/// Aggregate result sets don't have a fixed schema — SUM over an
/// INT4 column comes back as BIGINT, AVG returns NUMERIC, MIN/MAX
/// preserves the source column's type, and COUNT is always BIGINT.
/// Rather than route these through the `FromRow` machinery (which
/// needs a model whose columns are known at compile time), we
/// type-dispatch at runtime on `Column::type_()` and project into a
/// [`FilterValue`].
///
/// NULL maps to `FilterValue::Null`. NUMERIC (what AVG returns) is
/// decoded through `rust_decimal::Decimal` — tokio-postgres has no
/// `FromSql for String` impl for NUMERIC, so the `db-tokio-postgres`
/// feature of `rust_decimal` supplies the decoder — and then rendered
/// to its text form as `FilterValue::String`; the aggregate result
/// folder's numeric parser reads that text back into a float for the
/// sum/avg accessors.
///
/// Unknown types fall through to `try_get::<String>` so a novel
/// column type doesn't silently drop. Decoding failures record
/// `FilterValue::Null` rather than aborting the whole query.
fn decode_aggregate_cell(
    row: &tokio_postgres::Row,
    idx: usize,
    ty: &tokio_postgres::types::Type,
) -> FilterValue {
    use tokio_postgres::types::Type;
    match *ty {
        Type::BOOL => row
            .try_get::<_, Option<bool>>(idx)
            .ok()
            .flatten()
            .map(FilterValue::Bool)
            .unwrap_or(FilterValue::Null),
        Type::INT2 => row
            .try_get::<_, Option<i16>>(idx)
            .ok()
            .flatten()
            .map(|n| FilterValue::Int(n as i64))
            .unwrap_or(FilterValue::Null),
        Type::INT4 => row
            .try_get::<_, Option<i32>>(idx)
            .ok()
            .flatten()
            .map(|n| FilterValue::Int(n as i64))
            .unwrap_or(FilterValue::Null),
        Type::INT8 => row
            .try_get::<_, Option<i64>>(idx)
            .ok()
            .flatten()
            .map(FilterValue::Int)
            .unwrap_or(FilterValue::Null),
        Type::FLOAT4 => row
            .try_get::<_, Option<f32>>(idx)
            .ok()
            .flatten()
            .map(|f| FilterValue::Float(f as f64))
            .unwrap_or(FilterValue::Null),
        Type::FLOAT8 => row
            .try_get::<_, Option<f64>>(idx)
            .ok()
            .flatten()
            .map(FilterValue::Float)
            .unwrap_or(FilterValue::Null),
        // NUMERIC has no `FromSql for String` impl in tokio-postgres, so it
        // must be decoded through `rust_decimal::Decimal` (enabled via the
        // crate's `db-tokio-postgres` feature) and then rendered to its text
        // form. The aggregate result folder parses this back into an f64 for
        // the sum/avg accessors. This is the type AVG() returns, so getting it
        // wrong silently drops every average.
        Type::NUMERIC => row
            .try_get::<_, Option<rust_decimal::Decimal>>(idx)
            .ok()
            .flatten()
            .map(|d| FilterValue::String(d.to_string()))
            .unwrap_or(FilterValue::Null),
        Type::TEXT | Type::VARCHAR | Type::CHAR | Type::NAME | Type::BPCHAR => row
            .try_get::<_, Option<String>>(idx)
            .ok()
            .flatten()
            .map(FilterValue::String)
            .unwrap_or(FilterValue::Null),
        Type::JSON | Type::JSONB => row
            .try_get::<_, Option<serde_json::Value>>(idx)
            .ok()
            .flatten()
            .map(FilterValue::Json)
            .unwrap_or(FilterValue::Null),
        _ => row
            .try_get::<_, Option<String>>(idx)
            .ok()
            .flatten()
            .map(FilterValue::String)
            .unwrap_or(FilterValue::Null),
    }
}

#[cfg(test)]
mod tests {
    // Integration tests would require a real PostgreSQL database.
    // `TxPanicGuard`'s armed-drop path manipulates real pool objects
    // (`deadpool_postgres::Object` cannot be fabricated without a live
    // connection), so it is covered only by live integration testing,
    // not unit tests.
}
