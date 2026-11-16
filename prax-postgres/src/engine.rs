//! PostgreSQL query engine implementation.

use std::marker::PhantomData;
use std::sync::Arc;

use prax_query::QueryResult;
use prax_query::filter::FilterValue;
use prax_query::traits::{BoxFuture, Model, QueryEngine};
use tracing::{error, trace};

use crate::error::checkout_error;
use crate::notify::{validate_channel, validate_payload};
use crate::pool::PgPool;
use crate::types::filter_value_to_sql;

/// Statement behind [`PgEngine::notify`].
const NOTIFY_SQL: &str = "SELECT pg_notify($1, $2)";

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

    /// Send a notification on `channel` (`SELECT pg_notify($1, $2)`).
    ///
    /// On the transaction-bound engine that
    /// `PraxClient::<PgEngine>::transaction` hands its closure (reach it with
    /// `tx.engine()`), the notification is part of that transaction:
    /// delivered when it commits, discarded if it rolls back. Otherwise it
    /// runs on a pooled connection and is delivered at once.
    ///
    /// The channel name is matched verbatim (case preserved) by a
    /// [`PgListener`](crate::listener::PgListener) subscribed to the same
    /// string. It must be non-empty, at most 63 bytes and free of NUL, and
    /// the payload shorter than 8000 bytes; both are checked before
    /// anything is sent (an invalid-input error otherwise).
    pub async fn notify(&self, channel: &str, payload: &str) -> QueryResult<()> {
        validate_channel(channel)
            .and_then(|()| validate_payload(payload))
            .map_err(|e| prax_query::QueryError::invalid_input("notify", e.to_string()))?;
        let params: [&(dyn tokio_postgres::types::ToSql + Sync); 2] = [&channel, &payload];
        if let Some(tx) = &self.tx_conn {
            tx.execute(NOTIFY_SQL, &params)
                .await
                .map_err(map_driver_err)?;
        } else {
            let conn = self.pool.get().await.map_err(checkout_error)?;
            conn.execute(NOTIFY_SQL, &params)
                .await
                .map_err(map_driver_err)?;
        }
        Ok(())
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

/// A pinned transaction connection that can be taken out of its pool.
///
/// Abstracted so [`TxGuard`]'s state machine can be unit-tested without a
/// live `deadpool_postgres::Object`.
trait RetireConn {
    /// Take the connection out of its pool for good; dropping it closes
    /// the session, so the server aborts any open transaction.
    fn retire(self);
}

impl RetireConn for deadpool_postgres::Object {
    fn retire(self) {
        // Dropping the bare `Client` closes the session (the server then
        // aborts the open tx) and shrinks the pool by one instead of
        // recycling a connection with a live transaction.
        let _client = deadpool_postgres::Object::take(self);
    }
}

/// Retire `conn` if `tx_conn` is its last handle. Returns whether it was
/// retired; when the caller stashed a clone of the tx engine the handle
/// cannot be reclaimed.
fn retire_last_handle<C: RetireConn>(tx_conn: Arc<C>) -> bool {
    match Arc::try_unwrap(tx_conn) {
        Ok(conn) => {
            conn.retire();
            true
        }
        Err(_) => false,
    }
}

/// Panic/cancellation guard for the transaction path.
///
/// Armed as soon as the connection is checked out — *before* `BEGIN` is
/// sent — and disarmed only once the transaction is known to be settled
/// (`COMMIT`/`ROLLBACK` has completed). Every await in between (`BEGIN`,
/// the caller's closure, `COMMIT`, `ROLLBACK`) is a cancellation point:
/// if the transaction future is dropped there (a panic unwinding, or a
/// `tokio::time::timeout` firing), a request may already be queued on the
/// connection, so the session can be inside an open transaction with no
/// way to tell. Returning it to the pool would let the next borrower's
/// autocommit statements run inside that transaction — no `PgRecycling`
/// method detects or rolls back an open transaction (`Verified` runs an
/// empty query, `Clean` resets session state inside it). The armed guard
/// therefore retires the connection instead, which is always safe: the
/// session closes and the server aborts whatever was open. When the
/// caller stashed a tx-engine clone the handle cannot be reclaimed and it
/// can only log.
struct TxGuard<C: RetireConn = deadpool_postgres::Object> {
    tx_conn: Option<Arc<C>>,
}

impl<C: RetireConn> TxGuard<C> {
    fn new(conn: C) -> Self {
        Self {
            tx_conn: Some(Arc::new(conn)),
        }
    }

    /// The pinned connection. The guard stays armed while it is used.
    fn conn(&self) -> &Arc<C> {
        self.tx_conn
            .as_ref()
            .expect("TxGuard always holds a connection until consumed")
    }

    /// Disarm the guard and hand the connection back; dropping the
    /// returned handle recycles it into the pool. Call only once the
    /// session is known to be outside a transaction.
    fn disarm(mut self) -> Arc<C> {
        self.tx_conn
            .take()
            .expect("TxGuard always holds a connection until consumed")
    }

    /// Retire the connection now (the session's transaction state is
    /// unknown). Returns whether it was retired.
    fn retire(mut self) -> bool {
        let tx_conn = self
            .tx_conn
            .take()
            .expect("TxGuard always holds a connection until consumed");
        retire_last_handle(tx_conn)
    }
}

impl<C: RetireConn> Drop for TxGuard<C> {
    fn drop(&mut self) {
        let Some(tx_conn) = self.tx_conn.take() else {
            // Consumed: the transaction was settled explicitly.
            return;
        };
        if retire_last_handle(tx_conn) {
            error!(
                "transaction future was dropped (panic or cancellation) before the \
                 transaction was settled; retiring the pinned connection from the pool \
                 so any open transaction aborts with the session"
            );
        } else {
            error!(
                "transaction future was dropped (panic or cancellation) before the \
                 transaction was settled, and a cloned tx engine may still reference \
                 the pinned connection; it cannot be retired, so an open transaction \
                 may leak back into the pool"
            );
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
                let conn = self.pool.get().await.map_err(checkout_error)?;
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
                let conn = self.pool.get().await.map_err(checkout_error)?;
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
                let conn = self.pool.get().await.map_err(checkout_error)?;
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
                let conn = self.pool.get().await.map_err(checkout_error)?;
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
                let conn = self.pool.get().await.map_err(checkout_error)?;
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
                let conn = self.pool.get().await.map_err(checkout_error)?;
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
                let conn = self.pool.get().await.map_err(checkout_error)?;
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
                let conn = self.pool.get().await.map_err(checkout_error)?;
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
                let conn = self.pool.get().await.map_err(checkout_error)?;
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
            let conn = self.pool.inner().get().await.map_err(checkout_error)?;

            // Arm the guard *before* sending `BEGIN`: once the request is
            // queued, a cancellation at any later await (BEGIN itself, the
            // closure, COMMIT, ROLLBACK) could leave the session inside an
            // open transaction, so from here on a dropped future retires
            // the connection instead of recycling it.
            let guard = TxGuard::new(conn);

            // Issue `BEGIN` directly as a batch_execute on the raw
            // connection. Using `tokio_postgres::Transaction<'_>`
            // would bundle a borrow back into `conn`; instead we rely
            // on the connection's session state (postgres tracks the
            // BEGIN/COMMIT/ROLLBACK on the connection itself, so every
            // subsequent query on the same `Object` sees the same
            // transaction). This is the approach sanctioned by the
            // task plan's fallback guardrail.
            if let Err(e) = guard.conn().batch_execute("BEGIN").await {
                // The session's state is unknown (most likely the
                // connection is broken); retiring is always safe.
                guard.retire();
                return Err(map_driver_err(e));
            }

            let tx_engine = PgEngine {
                pool: self.pool.clone(),
                tx_conn: Some(guard.conn().clone()),
            };

            // Run the caller's closure on the tx-bound engine clone.
            // When the future resolves the closure's engine clone has
            // dropped, so the guard's `Arc` is the only remaining handle
            // (unless the caller stashed a clone of the tx engine).
            let result = f(tx_engine).await;

            // Finalise: COMMIT on success, best-effort ROLLBACK on
            // failure, with the guard still armed so a cancellation
            // while either is in flight retires the connection. The
            // pinned `Object` does *not* close on drop: dropping the last
            // `Arc` merely returns it to the pool, and no `PgRecycling`
            // method rolls back an open transaction. So the guard is
            // disarmed (the connection recycles) only when the session
            // is known to be outside a transaction; otherwise the
            // connection is retired — possible only when this is the
            // last `Arc` handle (if the caller stashed a clone of the tx
            // engine somewhere, the connection recycles as before, which
            // is the best we can do).
            match result {
                Ok(v) => match guard.conn().batch_execute("COMMIT").await {
                    Ok(()) => {
                        drop(guard.disarm());
                        Ok(v)
                    }
                    Err(e) => {
                        if e.as_db_error().is_some() {
                            // A server error on COMMIT (serialization
                            // failure, deferred constraint) always ends
                            // the transaction: the connection is clean.
                            drop(guard.disarm());
                        } else {
                            // Transport failure: state unknown.
                            guard.retire();
                        }
                        Err(map_driver_err(e))
                    }
                },
                Err(e) => {
                    if guard.conn().batch_execute("ROLLBACK").await.is_ok() {
                        drop(guard.disarm());
                    } else {
                        // Preserve the caller's error; the transaction
                        // may still be open, so never recycle it.
                        guard.retire();
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
    // `TxGuard`'s state machine is exercised below with a fake
    // connection; its use in `transaction()` against real pool objects
    // is covered by the live e2e tests (`tests/pool_recycling.rs`).

    use super::*;

    #[tokio::test]
    async fn notify_validates_before_connecting() {
        // deadpool connects lazily, and nothing listens on port 1: only
        // validation can answer these.
        let pool = crate::pool::PgPoolBuilder::new()
            .url("postgres://prax@127.0.0.1:1/prax")
            .min_connections(0)
            .build()
            .await
            .unwrap();
        let engine = PgEngine::new(pool);
        let too_long = "a".repeat(64);
        let oversized = "x".repeat(8000);
        for (channel, payload) in [
            ("", "x"),
            (too_long.as_str(), "x"),
            ("a\0b", "x"),
            ("ok", oversized.as_str()),
        ] {
            let err = engine.notify(channel, payload).await.unwrap_err();
            assert_eq!(
                err.code,
                prax_query::ErrorCode::InvalidParameter,
                "{channel:?}: {err}"
            );
        }
    }

    /// Fake pinned connection that counts retirements.
    struct FakeConn(Arc<std::sync::atomic::AtomicUsize>);

    impl RetireConn for FakeConn {
        fn retire(self) {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    fn fake() -> (FakeConn, Arc<std::sync::atomic::AtomicUsize>) {
        let n = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        (FakeConn(n.clone()), n)
    }

    fn retired(n: &std::sync::atomic::AtomicUsize) -> usize {
        n.load(std::sync::atomic::Ordering::SeqCst)
    }

    #[test]
    fn armed_guard_retires_on_drop() {
        let (conn, n) = fake();
        let guard = TxGuard::new(conn);
        drop(guard);
        assert_eq!(retired(&n), 1);
    }

    #[test]
    fn disarmed_guard_recycles() {
        let (conn, n) = fake();
        let guard = TxGuard::new(conn);
        let handle = guard.disarm();
        drop(handle);
        assert_eq!(retired(&n), 0, "a settled transaction must recycle");
    }

    #[test]
    fn explicit_retire_retires_once() {
        let (conn, n) = fake();
        let guard = TxGuard::new(conn);
        assert!(guard.retire());
        assert_eq!(retired(&n), 1, "retire must not also fire from Drop");
    }

    #[test]
    fn stashed_clone_blocks_retirement() {
        let (conn, n) = fake();
        let guard = TxGuard::new(conn);
        let stash = guard.conn().clone();
        drop(guard);
        assert_eq!(retired(&n), 0);
        drop(stash);
        assert_eq!(retired(&n), 0);

        let (conn, n) = fake();
        let guard = TxGuard::new(conn);
        let stash = guard.conn().clone();
        assert!(!guard.retire());
        drop(stash);
        assert_eq!(retired(&n), 0);
    }

    /// The guard is armed before the first await, so a future cancelled
    /// at that await (as `BEGIN` would be) retires the connection.
    #[tokio::test]
    async fn cancellation_at_first_await_retires() {
        let (conn, n) = fake();
        let fut = async move {
            let guard = TxGuard::new(conn);
            // Stands in for `BEGIN` awaiting its reply.
            std::future::pending::<()>().await;
            drop(guard.disarm());
        };
        let res = tokio::time::timeout(std::time::Duration::from_millis(10), fut).await;
        assert!(res.is_err(), "the stand-in BEGIN never completes");
        assert_eq!(retired(&n), 1);
    }
}
