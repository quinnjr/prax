//! PostgreSQL query engine implementation.

use std::marker::PhantomData;
use std::sync::Arc;

use prax_query::QueryResult;
use prax_query::filter::FilterValue;
use prax_query::traits::{BoxFuture, Model, QueryEngine};
use tracing::trace;

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
                tx.query(&sql, &param_refs)
                    .await
                    .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?
            } else {
                let conn = self.pool.get().await.map_err(|e| {
                    prax_query::QueryError::connection(e.to_string()).with_source(e)
                })?;
                conn.query(&sql, &param_refs)
                    .await
                    .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?
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

            // Shared `no rows` → `NotFound` translation, factored out
            // so both dispatch arms convert the same error text the
            // same way.
            let map_err = |e: String| -> prax_query::QueryError {
                if e.contains("no rows") {
                    prax_query::QueryError::not_found(T::MODEL_NAME)
                } else {
                    prax_query::QueryError::database(e)
                }
            };

            let row = if let Some(tx) = &self.tx_conn {
                tx.query_one(&sql, &param_refs)
                    .await
                    .map_err(|e| map_err(e.to_string()).with_source(e))?
            } else {
                let conn = self.pool.get().await.map_err(|e| {
                    prax_query::QueryError::connection(e.to_string()).with_source(e)
                })?;
                conn.query_one(&sql, &param_refs)
                    .await
                    .map_err(|e| map_err(e.to_string()).with_source(e))?
            };

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
                    .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?
            } else {
                let conn = self.pool.get().await.map_err(|e| {
                    prax_query::QueryError::connection(e.to_string()).with_source(e)
                })?;
                conn.query_opt(&sql, &param_refs)
                    .await
                    .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?
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
                    .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?
            } else {
                let conn = self.pool.get().await.map_err(|e| {
                    prax_query::QueryError::connection(e.to_string()).with_source(e)
                })?;
                conn.query_one(&sql, &param_refs)
                    .await
                    .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?
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
                tx.query(&sql, &param_refs)
                    .await
                    .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?
            } else {
                let conn = self.pool.get().await.map_err(|e| {
                    prax_query::QueryError::connection(e.to_string()).with_source(e)
                })?;
                conn.query(&sql, &param_refs)
                    .await
                    .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?
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
                tx.execute(&sql, &param_refs)
                    .await
                    .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))
            } else {
                let conn = self.pool.get().await.map_err(|e| {
                    prax_query::QueryError::connection(e.to_string()).with_source(e)
                })?;
                conn.execute(&sql, &param_refs)
                    .await
                    .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))
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
                tx.execute(&sql, &param_refs)
                    .await
                    .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))
            } else {
                let conn = self.pool.get().await.map_err(|e| {
                    prax_query::QueryError::connection(e.to_string()).with_source(e)
                })?;
                conn.execute(&sql, &param_refs)
                    .await
                    .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))
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
                    .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?
            } else {
                let conn = self.pool.get().await.map_err(|e| {
                    prax_query::QueryError::connection(e.to_string()).with_source(e)
                })?;
                conn.query_one(&sql, &param_refs)
                    .await
                    .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?
            };

            let count: i64 = row.get(0);
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
                tx.query(&sql, &param_refs)
                    .await
                    .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?
            } else {
                let conn = self.pool.get().await.map_err(|e| {
                    prax_query::QueryError::connection(e.to_string()).with_source(e)
                })?;
                conn.query(&sql, &param_refs)
                    .await
                    .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?
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
            conn.batch_execute("BEGIN")
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?;

            let tx_conn = Arc::new(conn);
            let tx_engine = PgEngine {
                pool: self.pool.clone(),
                tx_conn: Some(tx_conn.clone()),
            };

            // Run the caller's closure on the tx-bound engine clone.
            // When the future resolves the closure's engine clone has
            // dropped, so `tx_conn` is the only remaining `Arc` (plus
            // the clone we handed to the engine itself).
            let result = f(tx_engine).await;

            // Finalise: COMMIT on success, best-effort ROLLBACK on
            // failure. Preserve the caller's error if rollback fails —
            // the connection drops in a moment either way and the
            // server aborts the transaction on session close.
            match result {
                Ok(v) => {
                    tx_conn.batch_execute("COMMIT").await.map_err(|e| {
                        prax_query::QueryError::database(e.to_string()).with_source(e)
                    })?;
                    Ok(v)
                }
                Err(e) => {
                    let _ = tx_conn.batch_execute("ROLLBACK").await;
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
/// NULL maps to `FilterValue::Null`. NUMERIC is returned as
/// `FilterValue::String` because the workspace's tokio-postgres
/// feature set doesn't enable `with-rust_decimal-*`; the aggregate
/// result folder's numeric parser reads the text form back into a
/// float for sum/avg accessors.
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
        Type::TEXT | Type::VARCHAR | Type::CHAR | Type::NAME | Type::BPCHAR | Type::NUMERIC => row
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
    // Integration tests would require a real PostgreSQL database
}
