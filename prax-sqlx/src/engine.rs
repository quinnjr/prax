//! SQLx query engine implementation.

use crate::config::{DatabaseBackend, SqlxConfig};
use crate::error::{SqlxError, SqlxResult};
use crate::pool::SqlxPool;
use crate::row::SqlxRow;
use crate::types::quote_identifier;
use prax_query::QueryResult;
use prax_query::filter::FilterValue;
use prax_query::traits::{BoxFuture, Model, QueryEngine};
use sqlx::{Column, Row};
use std::sync::Arc;
use tracing::debug;

/// SQLx-based query engine for Prax.
///
/// This engine provides compile-time checked queries through SQLx,
/// supporting PostgreSQL, MySQL, and SQLite.
///
/// Two modes, controlled by the `tx` field:
///
/// - **Pool mode** (`tx == None`, the default): each query acquires a fresh
///   connection from the pool and drops it after the call.
/// - **Transaction mode** (`tx == Some(handle)`): every query routes through
///   the transaction's pinned connection. The tx-bound engine is built by
///   [`QueryEngine::transaction`], which issues `COMMIT` or `ROLLBACK` on the
///   same transaction when the closure resolves.
///
/// # Example
///
/// ```rust,ignore
/// use prax_sqlx::{SqlxEngine, SqlxConfig};
///
/// let config = SqlxConfig::from_url("postgres://localhost/mydb")?;
/// let engine = SqlxEngine::new(config).await?;
///
/// // Execute queries
/// let count = engine.count_table("users", None).await?;
/// ```
#[derive(Clone)]
pub struct SqlxEngine {
    pool: Arc<SqlxPool>,
    backend: DatabaseBackend,
    /// Present when this engine is bound to an in-flight transaction.
    /// `None` in the normal pool-backed case.
    tx: Option<TxHandle>,
}

/// A transaction in flight on one of the supported backends.
///
/// `Pool::begin()` yields a `sqlx::Transaction<'static>` that owns its
/// pooled connection, so it can be shared between the finalizer in
/// `QueryEngine::transaction` and every clone of the tx-bound engine. It is
/// neither `Clone` nor usable as an `Executor` through a shared reference,
/// so it lives behind an `Arc<futures::lock::Mutex<Option<_>>>`: the async
/// mutex serializes query access and stays `Send` across `.await`, and the
/// `Option` lets the finalizer `take()` the transaction to consume it via
/// `commit` / `rollback` — after which any engine clones the closure stashed
/// fail loudly instead of running queries against a dead transaction.
#[derive(Clone)]
enum TxHandle {
    #[cfg(feature = "postgres")]
    Postgres(TxSlot<sqlx::Postgres>),
    #[cfg(feature = "mysql")]
    MySql(TxSlot<sqlx::MySql>),
    #[cfg(feature = "sqlite")]
    Sqlite(TxSlot<sqlx::Sqlite>),
}

/// Shared, lockable slot holding an open backend transaction.
type TxSlot<DB> = Arc<futures::lock::Mutex<Option<sqlx::Transaction<'static, DB>>>>;

/// Guard on a [`TxSlot`], held for the duration of a single query.
type TxGuard<'m, DB> = futures::lock::MutexGuard<'m, Option<sqlx::Transaction<'static, DB>>>;

/// Error returned when a query is attempted through a transaction that has
/// already been committed or rolled back.
const TX_FINALIZED: &str = "transaction has already been committed or rolled back";

impl SqlxEngine {
    /// Create a new SQLx engine from configuration.
    pub async fn new(config: SqlxConfig) -> SqlxResult<Self> {
        let backend = config.backend;
        let pool = SqlxPool::connect(&config).await?;
        Ok(Self {
            pool: Arc::new(pool),
            backend,
            tx: None,
        })
    }

    /// Create a new engine from an existing pool.
    pub fn from_pool(pool: SqlxPool) -> Self {
        let backend = pool.backend();
        Self {
            pool: Arc::new(pool),
            backend,
            tx: None,
        }
    }

    /// Get the database backend type.
    pub fn backend(&self) -> DatabaseBackend {
        self.backend
    }

    /// Get the connection pool.
    pub fn pool(&self) -> &SqlxPool {
        &self.pool
    }

    /// Close the engine and all connections.
    pub async fn close(&self) {
        self.pool.close().await;
    }

    // ==================== Low-Level Query Methods ====================

    /// Execute a raw SQL query and return multiple rows.
    pub async fn raw_query_many(
        &self,
        sql: &str,
        params: &[FilterValue],
    ) -> SqlxResult<Vec<SqlxRow>> {
        debug!(sql = %sql, "Executing raw_query_many");

        match &*self.pool {
            #[cfg(feature = "postgres")]
            SqlxPool::Postgres(pool) => {
                let mut query = sqlx::query(sql);
                for param in params {
                    query = bind_pg_param(query, param)?;
                }
                let rows = match self.pg_tx_guard().await? {
                    Some(mut guard) => {
                        // Tx mode: run on the transaction's pinned connection
                        // so the query lands inside the same BEGIN…COMMIT
                        // block as every sibling call.
                        let tx = guard
                            .as_mut()
                            .ok_or_else(|| SqlxError::Internal(TX_FINALIZED.into()))?;
                        query.fetch_all(&mut **tx).await?
                    }
                    None => query.fetch_all(pool).await?,
                };
                Ok(rows.into_iter().map(SqlxRow::Postgres).collect())
            }
            #[cfg(feature = "mysql")]
            SqlxPool::MySql(pool) => {
                let mut query = sqlx::query(sql);
                for param in params {
                    query = bind_mysql_param(query, param)?;
                }
                let rows = match self.mysql_tx_guard().await? {
                    Some(mut guard) => {
                        let tx = guard
                            .as_mut()
                            .ok_or_else(|| SqlxError::Internal(TX_FINALIZED.into()))?;
                        query.fetch_all(&mut **tx).await?
                    }
                    None => query.fetch_all(pool).await?,
                };
                Ok(rows.into_iter().map(SqlxRow::MySql).collect())
            }
            #[cfg(feature = "sqlite")]
            SqlxPool::Sqlite(pool) => {
                let mut query = sqlx::query(sql);
                for param in params {
                    query = bind_sqlite_param(query, param)?;
                }
                let rows = match self.sqlite_tx_guard().await? {
                    Some(mut guard) => {
                        let tx = guard
                            .as_mut()
                            .ok_or_else(|| SqlxError::Internal(TX_FINALIZED.into()))?;
                        query.fetch_all(&mut **tx).await?
                    }
                    None => query.fetch_all(pool).await?,
                };
                Ok(rows.into_iter().map(SqlxRow::Sqlite).collect())
            }
        }
    }

    /// Execute a raw SQL query and return a single row.
    pub async fn raw_query_one(&self, sql: &str, params: &[FilterValue]) -> SqlxResult<SqlxRow> {
        debug!(sql = %sql, "Executing raw_query_one");

        match &*self.pool {
            #[cfg(feature = "postgres")]
            SqlxPool::Postgres(pool) => {
                let mut query = sqlx::query(sql);
                for param in params {
                    query = bind_pg_param(query, param)?;
                }
                let row = match self.pg_tx_guard().await? {
                    Some(mut guard) => {
                        let tx = guard
                            .as_mut()
                            .ok_or_else(|| SqlxError::Internal(TX_FINALIZED.into()))?;
                        query.fetch_one(&mut **tx).await?
                    }
                    None => query.fetch_one(pool).await?,
                };
                Ok(SqlxRow::Postgres(row))
            }
            #[cfg(feature = "mysql")]
            SqlxPool::MySql(pool) => {
                let mut query = sqlx::query(sql);
                for param in params {
                    query = bind_mysql_param(query, param)?;
                }
                let row = match self.mysql_tx_guard().await? {
                    Some(mut guard) => {
                        let tx = guard
                            .as_mut()
                            .ok_or_else(|| SqlxError::Internal(TX_FINALIZED.into()))?;
                        query.fetch_one(&mut **tx).await?
                    }
                    None => query.fetch_one(pool).await?,
                };
                Ok(SqlxRow::MySql(row))
            }
            #[cfg(feature = "sqlite")]
            SqlxPool::Sqlite(pool) => {
                let mut query = sqlx::query(sql);
                for param in params {
                    query = bind_sqlite_param(query, param)?;
                }
                let row = match self.sqlite_tx_guard().await? {
                    Some(mut guard) => {
                        let tx = guard
                            .as_mut()
                            .ok_or_else(|| SqlxError::Internal(TX_FINALIZED.into()))?;
                        query.fetch_one(&mut **tx).await?
                    }
                    None => query.fetch_one(pool).await?,
                };
                Ok(SqlxRow::Sqlite(row))
            }
        }
    }

    /// Execute a raw SQL query and return an optional row.
    pub async fn raw_query_optional(
        &self,
        sql: &str,
        params: &[FilterValue],
    ) -> SqlxResult<Option<SqlxRow>> {
        debug!(sql = %sql, "Executing raw_query_optional");

        match &*self.pool {
            #[cfg(feature = "postgres")]
            SqlxPool::Postgres(pool) => {
                let mut query = sqlx::query(sql);
                for param in params {
                    query = bind_pg_param(query, param)?;
                }
                let row = match self.pg_tx_guard().await? {
                    Some(mut guard) => {
                        let tx = guard
                            .as_mut()
                            .ok_or_else(|| SqlxError::Internal(TX_FINALIZED.into()))?;
                        query.fetch_optional(&mut **tx).await?
                    }
                    None => query.fetch_optional(pool).await?,
                };
                Ok(row.map(SqlxRow::Postgres))
            }
            #[cfg(feature = "mysql")]
            SqlxPool::MySql(pool) => {
                let mut query = sqlx::query(sql);
                for param in params {
                    query = bind_mysql_param(query, param)?;
                }
                let row = match self.mysql_tx_guard().await? {
                    Some(mut guard) => {
                        let tx = guard
                            .as_mut()
                            .ok_or_else(|| SqlxError::Internal(TX_FINALIZED.into()))?;
                        query.fetch_optional(&mut **tx).await?
                    }
                    None => query.fetch_optional(pool).await?,
                };
                Ok(row.map(SqlxRow::MySql))
            }
            #[cfg(feature = "sqlite")]
            SqlxPool::Sqlite(pool) => {
                let mut query = sqlx::query(sql);
                for param in params {
                    query = bind_sqlite_param(query, param)?;
                }
                let row = match self.sqlite_tx_guard().await? {
                    Some(mut guard) => {
                        let tx = guard
                            .as_mut()
                            .ok_or_else(|| SqlxError::Internal(TX_FINALIZED.into()))?;
                        query.fetch_optional(&mut **tx).await?
                    }
                    None => query.fetch_optional(pool).await?,
                };
                Ok(row.map(SqlxRow::Sqlite))
            }
        }
    }

    /// Execute a SQL statement (INSERT, UPDATE, DELETE) and return affected rows.
    pub async fn raw_execute(&self, sql: &str, params: &[FilterValue]) -> SqlxResult<u64> {
        debug!(sql = %sql, "Executing raw_execute");

        match &*self.pool {
            #[cfg(feature = "postgres")]
            SqlxPool::Postgres(pool) => {
                let mut query = sqlx::query(sql);
                for param in params {
                    query = bind_pg_param(query, param)?;
                }
                let result = match self.pg_tx_guard().await? {
                    Some(mut guard) => {
                        let tx = guard
                            .as_mut()
                            .ok_or_else(|| SqlxError::Internal(TX_FINALIZED.into()))?;
                        query.execute(&mut **tx).await?
                    }
                    None => query.execute(pool).await?,
                };
                Ok(result.rows_affected())
            }
            #[cfg(feature = "mysql")]
            SqlxPool::MySql(pool) => {
                let mut query = sqlx::query(sql);
                for param in params {
                    query = bind_mysql_param(query, param)?;
                }
                let result = match self.mysql_tx_guard().await? {
                    Some(mut guard) => {
                        let tx = guard
                            .as_mut()
                            .ok_or_else(|| SqlxError::Internal(TX_FINALIZED.into()))?;
                        query.execute(&mut **tx).await?
                    }
                    None => query.execute(pool).await?,
                };
                Ok(result.rows_affected())
            }
            #[cfg(feature = "sqlite")]
            SqlxPool::Sqlite(pool) => {
                let mut query = sqlx::query(sql);
                for param in params {
                    query = bind_sqlite_param(query, param)?;
                }
                let result = match self.sqlite_tx_guard().await? {
                    Some(mut guard) => {
                        let tx = guard
                            .as_mut()
                            .ok_or_else(|| SqlxError::Internal(TX_FINALIZED.into()))?;
                        query.execute(&mut **tx).await?
                    }
                    None => query.execute(pool).await?,
                };
                Ok(result.rows_affected())
            }
        }
    }

    /// Count rows in a table with optional filter.
    pub async fn count_table(&self, table: &str, filter: Option<&str>) -> SqlxResult<u64> {
        let table = quote_identifier(self.backend, table);
        let sql = match filter {
            Some(f) => format!("SELECT COUNT(*) as count FROM {} WHERE {}", table, f),
            None => format!("SELECT COUNT(*) as count FROM {}", table),
        };

        let row = self.raw_query_one(&sql, &[]).await?;
        match row {
            #[cfg(feature = "postgres")]
            SqlxRow::Postgres(r) => Ok(r.try_get::<i64, _>("count")? as u64),
            #[cfg(feature = "mysql")]
            SqlxRow::MySql(r) => Ok(r.try_get::<i64, _>("count")? as u64),
            #[cfg(feature = "sqlite")]
            SqlxRow::Sqlite(r) => Ok(r.try_get::<i64, _>("count")? as u64),
        }
    }

    // ==================== Transaction Helpers ====================

    /// Lock the in-flight Postgres transaction, if this engine is tx-bound.
    ///
    /// Returns `Ok(None)` in pool mode. A backend mismatch between the
    /// transaction handle and the pool is an internal invariant violation
    /// and errors rather than silently running outside the transaction.
    #[cfg(feature = "postgres")]
    async fn pg_tx_guard(&self) -> SqlxResult<Option<TxGuard<'_, sqlx::Postgres>>> {
        match self.tx.as_ref() {
            None => Ok(None),
            Some(TxHandle::Postgres(slot)) => Ok(Some(slot.lock().await)),
            // unreachable with a single backend feature enabled; keep the arm for multi-feature builds
            #[allow(unreachable_patterns)]
            Some(_) => Err(SqlxError::Internal(
                "transaction handle backend does not match engine pool".into(),
            )),
        }
    }

    /// Lock the in-flight MySQL transaction, if this engine is tx-bound.
    #[cfg(feature = "mysql")]
    async fn mysql_tx_guard(&self) -> SqlxResult<Option<TxGuard<'_, sqlx::MySql>>> {
        match self.tx.as_ref() {
            None => Ok(None),
            Some(TxHandle::MySql(slot)) => Ok(Some(slot.lock().await)),
            // unreachable with a single backend feature enabled; keep the arm for multi-feature builds
            #[allow(unreachable_patterns)]
            Some(_) => Err(SqlxError::Internal(
                "transaction handle backend does not match engine pool".into(),
            )),
        }
    }

    /// Lock the in-flight SQLite transaction, if this engine is tx-bound.
    #[cfg(feature = "sqlite")]
    async fn sqlite_tx_guard(&self) -> SqlxResult<Option<TxGuard<'_, sqlx::Sqlite>>> {
        match self.tx.as_ref() {
            None => Ok(None),
            Some(TxHandle::Sqlite(slot)) => Ok(Some(slot.lock().await)),
            // unreachable with a single backend feature enabled; keep the arm for multi-feature builds
            #[allow(unreachable_patterns)]
            Some(_) => Err(SqlxError::Internal(
                "transaction handle backend does not match engine pool".into(),
            )),
        }
    }
}

// ==================== Parameter Binding Helpers ====================

/// Untyped NULL binding for Postgres.
///
/// `FilterValue::Null` must bind successfully against a column of any type
/// (nullable INT4, UUID, TIMESTAMPTZ, custom ENUM, ...), but sqlx derives the
/// parameter's declared type from the Rust type of the bind — so the old
/// `Option::<String>::None` sent a TEXT-typed NULL that Postgres rejects on
/// assignment to non-text columns ("column is of type integer but expression
/// is of type text"). Declaring the parameter as the `unknown` pseudo-type
/// makes the server infer the parameter type from context, which is always
/// valid for a NULL. sqlx resolves the name to OID 705 once per connection
/// and caches it. This mirrors `PgNull` in prax-postgres/src/types.rs.
#[cfg(feature = "postgres")]
#[derive(Debug, Clone, Copy)]
struct UntypedNull;

#[cfg(feature = "postgres")]
impl sqlx::Type<sqlx::Postgres> for UntypedNull {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        sqlx::postgres::PgTypeInfo::with_name("unknown")
    }
}

#[cfg(feature = "postgres")]
impl<'q> sqlx::Encode<'q, sqlx::Postgres> for UntypedNull {
    fn encode_by_ref(
        &self,
        _buf: &mut sqlx::postgres::PgArgumentBuffer,
    ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
        Ok(sqlx::encode::IsNull::Yes)
    }
}

#[cfg(feature = "postgres")]
fn bind_pg_param<'q>(
    query: sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>,
    value: &'q FilterValue,
) -> SqlxResult<sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>> {
    Ok(match value {
        FilterValue::String(s) => query.bind(s.as_str()),
        FilterValue::Int(i) => query.bind(*i),
        FilterValue::Float(f) => query.bind(*f),
        FilterValue::Bool(b) => query.bind(*b),
        FilterValue::Null => query.bind(UntypedNull),
        FilterValue::Json(j) => query.bind(j.clone()),
        FilterValue::List(_) => {
            // Lists are deliberately unsupported in raw binds. Typed IN
            // filters expand to scalar placeholders upstream
            // (prax-query/src/filter.rs), so this arm only fires for
            // hand-built raw queries.
            return Err(SqlxError::type_conversion(
                "list values not supported in raw binds; use typed IN filters",
            ));
        }
        FilterValue::Bytes(b) => query.bind(b.as_slice()),
        FilterValue::Array(items) => match pg_array_kind(items)? {
            PgArrayKind::Int => query.bind(pg_array_elems(items, |v| match v {
                FilterValue::Int(i) => Some(*i),
                _ => None,
            })),
            PgArrayKind::Float => query.bind(pg_array_elems(items, |v| match v {
                FilterValue::Float(f) => Some(*f),
                _ => None,
            })),
            PgArrayKind::Bool => query.bind(pg_array_elems(items, |v| match v {
                FilterValue::Bool(b) => Some(*b),
                _ => None,
            })),
            PgArrayKind::Text => query.bind(pg_array_elems(items, |v| match v {
                FilterValue::String(s) => Some(s.clone()),
                _ => None,
            })),
            PgArrayKind::Json => query.bind(pg_array_elems(items, |v| match v {
                FilterValue::Json(j) => Some(j.clone()),
                _ => None,
            })),
            PgArrayKind::Bytes => query.bind(pg_array_elems(items, |v| match v {
                FilterValue::Bytes(b) => Some(b.clone()),
                _ => None,
            })),
        },
        _ => {
            return Err(SqlxError::type_conversion(
                "unsupported FilterValue variant",
            ));
        }
    })
}

/// Element type chosen for a Postgres array bind.
#[cfg(feature = "postgres")]
#[derive(Debug, PartialEq, Eq)]
enum PgArrayKind {
    Int,
    Float,
    Bool,
    Text,
    Json,
    Bytes,
}

/// Choose the array element kind from the first non-`Null` element.
///
/// Empty and all-`Null` arrays fall back to `Text` (they bind as `text[]`,
/// a documented limitation: the server cannot infer a better element type).
/// String arrays bind as `text[]` too, so a comparison with a non-text
/// column needs a cast in the SQL (`uuid_col = ANY($1::uuid[])`).
/// Mixed kinds and nested `List`/`Array` elements are rejected.
#[cfg(feature = "postgres")]
fn pg_array_kind(items: &[FilterValue]) -> SqlxResult<PgArrayKind> {
    let mismatch =
        || SqlxError::type_conversion("FilterValue::Array elements must share one scalar kind");
    let mut kind: Option<PgArrayKind> = None;
    for item in items {
        let k = match item {
            FilterValue::Null => continue,
            FilterValue::Int(_) => PgArrayKind::Int,
            FilterValue::Float(_) => PgArrayKind::Float,
            FilterValue::Bool(_) => PgArrayKind::Bool,
            FilterValue::String(_) => PgArrayKind::Text,
            FilterValue::Json(_) => PgArrayKind::Json,
            FilterValue::Bytes(_) => PgArrayKind::Bytes,
            _ => return Err(mismatch()),
        };
        match &kind {
            None => kind = Some(k),
            Some(existing) if *existing == k => {}
            Some(_) => return Err(mismatch()),
        }
    }
    Ok(kind.unwrap_or(PgArrayKind::Text))
}

/// Map array items to `Option<T>` (`Null` -> `None`). `pg_array_kind` has
/// already validated that every non-`Null` item matches `pick`.
#[cfg(feature = "postgres")]
fn pg_array_elems<T>(
    items: &[FilterValue],
    pick: impl Fn(&FilterValue) -> Option<T>,
) -> Vec<Option<T>> {
    items.iter().map(pick).collect()
}

#[cfg(feature = "mysql")]
fn bind_mysql_param<'q>(
    query: sqlx::query::Query<'q, sqlx::MySql, sqlx::mysql::MySqlArguments>,
    value: &'q FilterValue,
) -> SqlxResult<sqlx::query::Query<'q, sqlx::MySql, sqlx::mysql::MySqlArguments>> {
    Ok(match value {
        FilterValue::String(s) => query.bind(s.as_str()),
        FilterValue::Int(i) => query.bind(*i),
        FilterValue::Float(f) => query.bind(*f),
        FilterValue::Bool(b) => query.bind(*b),
        FilterValue::Null => query.bind(Option::<String>::None),
        FilterValue::Json(j) => query.bind(j.to_string()),
        FilterValue::List(_) => {
            return Err(SqlxError::type_conversion(
                "list values not supported in raw binds; use typed IN filters",
            ));
        }
        FilterValue::Bytes(b) => query.bind(b.as_slice()),
        FilterValue::Array(_) => query.bind(value.to_json_value().to_string()),
        _ => {
            return Err(SqlxError::type_conversion(
                "unsupported FilterValue variant",
            ));
        }
    })
}

#[cfg(feature = "sqlite")]
fn bind_sqlite_param<'q>(
    query: sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>>,
    value: &'q FilterValue,
) -> SqlxResult<sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>>> {
    Ok(match value {
        FilterValue::String(s) => query.bind(s.as_str()),
        FilterValue::Int(i) => query.bind(*i),
        FilterValue::Float(f) => query.bind(*f),
        FilterValue::Bool(b) => query.bind(*b),
        FilterValue::Null => query.bind(Option::<String>::None),
        FilterValue::Json(j) => query.bind(j.to_string()),
        FilterValue::List(_) => {
            return Err(SqlxError::type_conversion(
                "list values not supported in raw binds; use typed IN filters",
            ));
        }
        FilterValue::Bytes(b) => query.bind(b.as_slice()),
        FilterValue::Array(_) => query.bind(value.to_json_value().to_string()),
        _ => {
            return Err(SqlxError::type_conversion(
                "unsupported FilterValue variant",
            ));
        }
    })
}

// ==================== Transaction Finalization ====================

/// Commit or roll back a transaction after the closure has resolved.
///
/// The transaction is `take()`n out of the shared slot so any engine clones
/// the closure stashed past the end of `transaction` fail loudly on their
/// next query instead of silently running outside the transaction. On
/// failure the rollback is best-effort: the caller's error is preserved, and
/// dropping the connection aborts the transaction server-side anyway.
async fn finalize_tx<R, DB>(slot: &TxSlot<DB>, result: QueryResult<R>) -> QueryResult<R>
where
    DB: sqlx::Database,
{
    let tx = slot.lock().await.take();
    match (result, tx) {
        (Ok(value), Some(tx)) => {
            tx.commit()
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;
            Ok(value)
        }
        (Err(err), Some(tx)) => {
            if let Err(rb) = tx.rollback().await {
                tracing::warn!(error = %rb, "transaction rollback failed; connection drop will abort server-side");
            }
            Err(err)
        }
        (Ok(_), None) => Err(prax_query::QueryError::internal(
            "transaction was finalized before commit could be issued",
        )),
        (Err(err), None) => Err(err),
    }
}

// ==================== Aggregate Result Decoding ====================

/// Decode one aggregate result row into a column-name → [`FilterValue`] map.
///
/// Aggregate and group-by result sets have no fixed schema: `COUNT` comes
/// back as a backend-specific integer width, `AVG` as NUMERIC/NEWDECIMAL/
/// REAL, and group-by queries include the grouped columns themselves. Each
/// cell is probed in specificity order; NULL and unrecognized types decode
/// to `FilterValue::Null` rather than aborting the whole query (matching the
/// reference `decode_aggregate_cell` in prax-postgres).
fn decode_aggregate_row(row: &SqlxRow) -> std::collections::HashMap<String, FilterValue> {
    let mut map = std::collections::HashMap::new();
    match row {
        #[cfg(feature = "postgres")]
        SqlxRow::Postgres(r) => {
            for (i, col) in r.columns().iter().enumerate() {
                map.insert(col.name().to_string(), decode_pg_aggregate_cell(r, i));
            }
        }
        #[cfg(feature = "mysql")]
        SqlxRow::MySql(r) => {
            for (i, col) in r.columns().iter().enumerate() {
                map.insert(col.name().to_string(), decode_mysql_aggregate_cell(r, i));
            }
        }
        #[cfg(feature = "sqlite")]
        SqlxRow::Sqlite(r) => {
            for (i, col) in r.columns().iter().enumerate() {
                map.insert(col.name().to_string(), decode_sqlite_aggregate_cell(r, i));
            }
        }
    }
    map
}

/// Probe a Postgres cell in width order. sqlx type-checks strictly per OID,
/// so each probe only matches its own column type.
#[cfg(feature = "postgres")]
fn decode_pg_aggregate_cell(r: &sqlx::postgres::PgRow, i: usize) -> FilterValue {
    if let Ok(Some(b)) = r.try_get::<Option<bool>, _>(i) {
        return FilterValue::Bool(b);
    }
    if let Ok(Some(n)) = r.try_get::<Option<i64>, _>(i) {
        return FilterValue::Int(n);
    }
    if let Ok(Some(n)) = r.try_get::<Option<i32>, _>(i) {
        return FilterValue::Int(n as i64);
    }
    if let Ok(Some(n)) = r.try_get::<Option<i16>, _>(i) {
        return FilterValue::Int(n as i64);
    }
    if let Ok(Some(f)) = r.try_get::<Option<f64>, _>(i) {
        return FilterValue::Float(f);
    }
    if let Ok(Some(f)) = r.try_get::<Option<f32>, _>(i) {
        return FilterValue::Float(f as f64);
    }
    // NUMERIC (what AVG and SUM over numeric/int8 return) has no direct
    // string/float decode in sqlx; go through rust_decimal and render as
    // text. The aggregate result folder parses the text back into f64 for
    // the sum/avg accessors.
    if let Ok(Some(d)) = r.try_get::<Option<rust_decimal::Decimal>, _>(i) {
        return FilterValue::String(d.to_string());
    }
    if let Ok(Some(s)) = r.try_get::<Option<String>, _>(i) {
        return FilterValue::String(s);
    }
    if let Ok(Some(j)) = r.try_get::<Option<serde_json::Value>, _>(i) {
        return FilterValue::Json(j);
    }
    FilterValue::Null
}

/// Probe a MySQL cell. NOTE: integers must be probed before anything else —
/// MySQL `bool::compatible` accepts every integer column type, so probing
/// bool first would misclassify `COUNT(*)` (LONGLONG) as `Bool(true)`.
#[cfg(feature = "mysql")]
fn decode_mysql_aggregate_cell(r: &sqlx::mysql::MySqlRow, i: usize) -> FilterValue {
    if let Ok(Some(n)) = r.try_get::<Option<i64>, _>(i) {
        return FilterValue::Int(n);
    }
    if let Ok(Some(n)) = r.try_get::<Option<u64>, _>(i) {
        // Unsigned columns; values beyond i64::MAX saturate rather than wrap.
        return FilterValue::Int(i64::try_from(n).unwrap_or(i64::MAX));
    }
    if let Ok(Some(f)) = r.try_get::<Option<f64>, _>(i) {
        return FilterValue::Float(f);
    }
    if let Ok(Some(f)) = r.try_get::<Option<f32>, _>(i) {
        return FilterValue::Float(f as f64);
    }
    // NEWDECIMAL (SUM/AVG) — same text-render rationale as the pg NUMERIC arm.
    if let Ok(Some(d)) = r.try_get::<Option<rust_decimal::Decimal>, _>(i) {
        return FilterValue::String(d.to_string());
    }
    if let Ok(Some(s)) = r.try_get::<Option<String>, _>(i) {
        return FilterValue::String(s);
    }
    if let Ok(Some(j)) = r.try_get::<Option<serde_json::Value>, _>(i) {
        return FilterValue::Json(j);
    }
    FilterValue::Null
}

/// Probe a SQLite cell. NOTE: no bool probe — SQLite `bool::compatible`
/// accepts every INTEGER value, which would misclassify `COUNT(*)` as Bool.
#[cfg(feature = "sqlite")]
fn decode_sqlite_aggregate_cell(r: &sqlx::sqlite::SqliteRow, i: usize) -> FilterValue {
    if let Ok(Some(n)) = r.try_get::<Option<i64>, _>(i) {
        return FilterValue::Int(n);
    }
    if let Ok(Some(f)) = r.try_get::<Option<f64>, _>(i) {
        return FilterValue::Float(f);
    }
    if let Ok(Some(s)) = r.try_get::<Option<String>, _>(i) {
        return FilterValue::String(s);
    }
    FilterValue::Null
}

// ==================== QueryEngine Trait Implementation ====================

impl QueryEngine for SqlxEngine {
    fn dialect(&self) -> &dyn prax_query::dialect::SqlDialect {
        match self.backend {
            DatabaseBackend::Postgres => &prax_query::dialect::Postgres,
            DatabaseBackend::MySql => &prax_query::dialect::Mysql,
            DatabaseBackend::Sqlite => &prax_query::dialect::Sqlite,
        }
    }

    fn query_many<T: Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Vec<T>>> {
        let sql = sql.to_string();
        Box::pin(async move {
            debug!(sql = %sql, "Executing query_many via QueryEngine");

            let rows = self
                .raw_query_many(&sql, &params)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;

            rows.iter()
                .map(|r| {
                    let rr = crate::row_ref::SqlxRowRef::from_sqlx(r).map_err(|e| {
                        let msg = e.to_string();
                        prax_query::QueryError::deserialization(msg).with_source(e)
                    })?;
                    T::from_row(&rr).map_err(|e| {
                        let msg = e.to_string();
                        prax_query::QueryError::deserialization(msg).with_source(e)
                    })
                })
                .collect()
        })
    }

    fn query_one<T: Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<T>> {
        let sql = sql.to_string();
        Box::pin(async move {
            debug!(sql = %sql, "Executing query_one via QueryEngine");

            let row = self.raw_query_one(&sql, &params).await.map_err(|e| {
                let msg = e.to_string();
                if msg.contains("no rows") {
                    prax_query::QueryError::not_found(T::MODEL_NAME)
                } else {
                    prax_query::QueryError::database(msg)
                }
            })?;

            let rr = crate::row_ref::SqlxRowRef::from_sqlx(&row).map_err(|e| {
                let msg = e.to_string();
                prax_query::QueryError::deserialization(msg).with_source(e)
            })?;
            T::from_row(&rr).map_err(|e| {
                let msg = e.to_string();
                prax_query::QueryError::deserialization(msg).with_source(e)
            })
        })
    }

    fn query_optional<T: Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Option<T>>> {
        let sql = sql.to_string();
        Box::pin(async move {
            debug!(sql = %sql, "Executing query_optional via QueryEngine");

            let row = self
                .raw_query_optional(&sql, &params)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;

            match row {
                Some(r) => {
                    let rr = crate::row_ref::SqlxRowRef::from_sqlx(&r).map_err(|e| {
                        let msg = e.to_string();
                        prax_query::QueryError::deserialization(msg).with_source(e)
                    })?;
                    T::from_row(&rr).map(Some).map_err(|e| {
                        let msg = e.to_string();
                        prax_query::QueryError::deserialization(msg).with_source(e)
                    })
                }
                None => Ok(None),
            }
        })
    }

    fn execute_insert<T: Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<T>> {
        let sql = sql.to_string();
        Box::pin(async move {
            debug!(sql = %sql, "Executing execute_insert via QueryEngine");

            let row = self
                .raw_query_one(&sql, &params)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;

            let rr = crate::row_ref::SqlxRowRef::from_sqlx(&row).map_err(|e| {
                let msg = e.to_string();
                prax_query::QueryError::deserialization(msg).with_source(e)
            })?;
            T::from_row(&rr).map_err(|e| {
                let msg = e.to_string();
                prax_query::QueryError::deserialization(msg).with_source(e)
            })
        })
    }

    fn execute_update<T: Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Vec<T>>> {
        let sql = sql.to_string();
        Box::pin(async move {
            debug!(sql = %sql, "Executing execute_update via QueryEngine");

            let rows = self
                .raw_query_many(&sql, &params)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;

            rows.iter()
                .map(|r| {
                    let rr = crate::row_ref::SqlxRowRef::from_sqlx(r).map_err(|e| {
                        let msg = e.to_string();
                        prax_query::QueryError::deserialization(msg).with_source(e)
                    })?;
                    T::from_row(&rr).map_err(|e| {
                        let msg = e.to_string();
                        prax_query::QueryError::deserialization(msg).with_source(e)
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
        Box::pin(async move {
            debug!(sql = %sql, "Executing execute_delete via QueryEngine");

            let affected = self
                .raw_execute(&sql, &params)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;

            Ok(affected)
        })
    }

    fn execute_raw(&self, sql: &str, params: Vec<FilterValue>) -> BoxFuture<'_, QueryResult<u64>> {
        let sql = sql.to_string();
        Box::pin(async move {
            debug!(sql = %sql, "Executing execute_raw via QueryEngine");

            let affected = self
                .raw_execute(&sql, &params)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;

            Ok(affected)
        })
    }

    fn count(&self, sql: &str, params: Vec<FilterValue>) -> BoxFuture<'_, QueryResult<u64>> {
        let sql = sql.to_string();
        Box::pin(async move {
            debug!(sql = %sql, "Executing count via QueryEngine");

            let row = self
                .raw_query_one(&sql, &params)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;

            let count = match row {
                #[cfg(feature = "postgres")]
                SqlxRow::Postgres(r) => r
                    .try_get::<i64, _>(0)
                    .map_err(|e| prax_query::QueryError::database(e.to_string()))?
                    as u64,
                #[cfg(feature = "mysql")]
                SqlxRow::MySql(r) => r
                    .try_get::<i64, _>(0)
                    .map_err(|e| prax_query::QueryError::database(e.to_string()))?
                    as u64,
                #[cfg(feature = "sqlite")]
                SqlxRow::Sqlite(r) => r
                    .try_get::<i64, _>(0)
                    .map_err(|e| prax_query::QueryError::database(e.to_string()))?
                    as u64,
            };

            Ok(count)
        })
    }

    fn aggregate_query(
        &self,
        sql: &str,
        params: Vec<FilterValue>,
    ) -> BoxFuture<'_, QueryResult<Vec<std::collections::HashMap<String, FilterValue>>>> {
        let sql = sql.to_string();
        Box::pin(async move {
            debug!(sql = %sql, "Executing aggregate_query via QueryEngine");

            let rows = self
                .raw_query_many(&sql, &params)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()))?;

            Ok(rows.iter().map(decode_aggregate_row).collect())
        })
    }

    fn in_transaction(&self) -> bool {
        self.tx.is_some()
    }

    fn transaction<'a, R, Fut, F>(&'a self, f: F) -> BoxFuture<'a, QueryResult<R>>
    where
        F: FnOnce(Self) -> Fut + Send + 'a,
        Fut: std::future::Future<Output = QueryResult<R>> + Send + 'a,
        R: Send + 'a,
        Self: Clone,
    {
        Box::pin(async move {
            // Refuse nested transactions: a tx-bound engine can only hold one
            // open transaction at a time. Issue SAVEPOINT via execute_raw if
            // nesting is needed.
            if self.tx.is_some() {
                return Err(prax_query::QueryError::internal(
                    "nested transactions not supported on SqlxEngine \
                     (call .transaction() on the outer engine only, or \
                     issue SAVEPOINT via execute_raw)",
                ));
            }

            // Begin a backend-native transaction. `Pool::begin()` yields a
            // `Transaction<'static>` owning its pooled connection, so it can
            // be shared with the tx-bound engine clone behind an Arc.
            let tx_handle = match &*self.pool {
                #[cfg(feature = "postgres")]
                SqlxPool::Postgres(pool) => {
                    let tx = pool
                        .begin()
                        .await
                        .map_err(|e| prax_query::QueryError::database(e.to_string()))?;
                    TxHandle::Postgres(Arc::new(futures::lock::Mutex::new(Some(tx))))
                }
                #[cfg(feature = "mysql")]
                SqlxPool::MySql(pool) => {
                    let tx = pool
                        .begin()
                        .await
                        .map_err(|e| prax_query::QueryError::database(e.to_string()))?;
                    TxHandle::MySql(Arc::new(futures::lock::Mutex::new(Some(tx))))
                }
                #[cfg(feature = "sqlite")]
                SqlxPool::Sqlite(pool) => {
                    let tx = pool
                        .begin()
                        .await
                        .map_err(|e| prax_query::QueryError::database(e.to_string()))?;
                    TxHandle::Sqlite(Arc::new(futures::lock::Mutex::new(Some(tx))))
                }
            };

            let tx_engine = SqlxEngine {
                pool: self.pool.clone(),
                backend: self.backend,
                tx: Some(tx_handle.clone()),
            };

            // Run the caller's closure on the tx-bound engine clone. When the
            // future resolves, its engine clone is dropped; any clones the
            // closure stashed away keep pointing at the (soon to be emptied)
            // slot and will fail loudly on their next query.
            let result = f(tx_engine).await;

            // Finalize: COMMIT on success, best-effort ROLLBACK on failure.
            match &tx_handle {
                #[cfg(feature = "postgres")]
                TxHandle::Postgres(slot) => finalize_tx(slot, result).await,
                #[cfg(feature = "mysql")]
                TxHandle::MySql(slot) => finalize_tx(slot, result).await,
                #[cfg(feature = "sqlite")]
                TxHandle::Sqlite(slot) => finalize_tx(slot, result).await,
            }
        })
    }
}

/// SQLx routes exclusively to SQL backends (Postgres, MySQL, SQLite), all of
/// which support scalar subqueries in SELECT. Implementing this marker here
/// lets callers use [`FindManyOperation::with_scalar_projection`] and its
/// siblings on `SqlxEngine` just like the individual SQL engine crates.
impl prax_query::capabilities::SupportsScalarSubqueryInSelect for SqlxEngine {}
impl prax_query::capabilities::SupportsRowLocks for SqlxEngine {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::placeholder;

    #[test]
    fn test_placeholder_generation() {
        assert_eq!(placeholder(DatabaseBackend::Postgres, 1), "$1");
        assert_eq!(placeholder(DatabaseBackend::MySql, 1), "?");
        assert_eq!(placeholder(DatabaseBackend::Sqlite, 1), "?");
    }

    #[test]
    fn test_quote_identifier() {
        assert_eq!(
            quote_identifier(DatabaseBackend::Postgres, "users"),
            "\"users\""
        );
        assert_eq!(quote_identifier(DatabaseBackend::MySql, "users"), "`users`");
    }

    #[cfg(feature = "postgres")]
    #[test]
    fn test_untyped_null_binds_as_server_inferred_type() {
        use sqlx::TypeInfo as _;

        // The parameter is declared as the `unknown` pseudo-type with no
        // pre-resolved OID; sqlx resolves (and caches) OID 705 per connection,
        // and the server then infers the parameter type from context, so a
        // NULL binds cleanly against a column of any type.
        let info = <UntypedNull as sqlx::Type<sqlx::Postgres>>::type_info();
        assert_eq!(info.name(), "unknown");
        assert!(info.oid().is_none());

        let mut buf = sqlx::postgres::PgArgumentBuffer::default();
        let is_null = <UntypedNull as sqlx::Encode<'_, sqlx::Postgres>>::encode_by_ref(
            &UntypedNull,
            &mut buf,
        )
        .unwrap();
        assert!(is_null.is_null());
    }

    /// Build an engine backed by a single-connection in-memory SQLite pool.
    /// `max_connections(1)` keeps every statement on the same connection,
    /// so the in-memory database (and its schema) survives across calls.
    #[cfg(feature = "sqlite")]
    async fn sqlite_engine() -> SqlxEngine {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connect to in-memory sqlite");
        SqlxEngine::from_pool(SqlxPool::Sqlite(pool))
    }

    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn transaction_commits_on_success() {
        let engine = sqlite_engine().await;
        engine
            .raw_execute(
                "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
                &[],
            )
            .await
            .unwrap();

        let result: prax_query::QueryResult<()> = engine
            .transaction(|tx| async move {
                tx.raw_execute(
                    "INSERT INTO items (name) VALUES (?1)",
                    &[FilterValue::String("widget".into())],
                )
                .await?;
                Ok(())
            })
            .await;
        result.expect("transaction should commit");

        // The insert is visible outside the transaction: it committed.
        assert_eq!(engine.count_table("items", None).await.unwrap(), 1);
    }

    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn transaction_rolls_back_on_error() {
        let engine = sqlite_engine().await;
        engine
            .raw_execute(
                "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
                &[],
            )
            .await
            .unwrap();

        let result: prax_query::QueryResult<()> = engine
            .transaction(|tx| async move {
                tx.raw_execute(
                    "INSERT INTO items (name) VALUES (?1)",
                    &[FilterValue::String("widget".into())],
                )
                .await?;
                Err(prax_query::QueryError::internal("boom"))
            })
            .await;
        assert!(result.is_err());

        // The insert was rolled back: the table is still empty.
        assert_eq!(engine.count_table("items", None).await.unwrap(), 0);
    }

    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn stashed_engine_clone_fails_after_finalize() {
        let engine = sqlite_engine().await;

        // Stash the tx-bound engine by returning it from the closure; the
        // finalizer commits and empties the shared slot before handing it
        // back, so it must fail loudly on its next query.
        let stashed: SqlxEngine = engine
            .transaction(|tx| async move { Ok(tx) })
            .await
            .expect("transaction should commit");

        assert!(stashed.in_transaction());
        let err = stashed
            .raw_query_one("SELECT 1", &[])
            .await
            .err()
            .expect("queries on a finalized transaction must fail");
        assert!(
            matches!(
                err,
                SqlxError::Internal(ref msg) if msg.contains("committed or rolled back")
            ),
            "unexpected error: {err}"
        );
    }

    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn nested_transaction_is_rejected() {
        let engine = sqlite_engine().await;

        let result: prax_query::QueryResult<()> = engine
            .transaction(|tx| async move { tx.transaction(|_inner| async move { Ok(()) }).await })
            .await;
        let err = result.expect_err("nested transaction must be rejected");
        assert!(
            err.to_string().contains("nested transactions"),
            "unexpected error: {err}"
        );
    }

    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn aggregate_query_decodes_count_as_int_not_bool() {
        let engine = sqlite_engine().await;
        engine
            .raw_execute(
                "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
                &[],
            )
            .await
            .unwrap();
        for name in ["a", "b", "c"] {
            engine
                .raw_execute(
                    "INSERT INTO items (name) VALUES (?1)",
                    &[FilterValue::String(name.into())],
                )
                .await
                .unwrap();
        }

        let rows = engine
            .aggregate_query("SELECT COUNT(*) AS n, AVG(id) AS avg_id FROM items", vec![])
            .await
            .expect("aggregate query");
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        // Pins the probe order: SQLite `bool::compatible` accepts every
        // INTEGER value, so a bool probe would misclassify COUNT(*) as
        // Bool. Probing i64 first (with no bool probe at all) keeps it Int.
        assert_eq!(row.get("n"), Some(&FilterValue::Int(3)));
        assert!(!matches!(row.get("n"), Some(FilterValue::Bool(_))));
        assert_eq!(row.get("avg_id"), Some(&FilterValue::Float(2.0)));
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn bind_sqlite_param_rejects_list_values() {
        // Offline: sqlx::query only builds the statement, no connection is
        // needed to hit the List-rejection arm.
        let value = FilterValue::List(vec![FilterValue::Int(1)]);
        let query = sqlx::query::<sqlx::Sqlite>("SELECT ?1");
        let err = bind_sqlite_param(query, &value)
            .err()
            .expect("list bind must be rejected");
        assert!(
            matches!(err, SqlxError::TypeConversion(ref msg) if msg.contains("list values")),
            "unexpected error: {err}"
        );
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn bind_sqlite_param_accepts_bytes_and_array() {
        let bytes = FilterValue::Bytes(vec![0, 255]);
        let array = FilterValue::Array(vec![FilterValue::Int(1), FilterValue::String("a".into())]);
        assert!(bind_sqlite_param(sqlx::query::<sqlx::Sqlite>("SELECT ?1"), &bytes).is_ok());
        assert!(bind_sqlite_param(sqlx::query::<sqlx::Sqlite>("SELECT ?1"), &array).is_ok());
    }

    #[cfg(feature = "mysql")]
    #[test]
    fn bind_mysql_param_accepts_bytes_and_array() {
        let bytes = FilterValue::Bytes(vec![0, 255]);
        let array = FilterValue::Array(vec![FilterValue::Int(1), FilterValue::String("a".into())]);
        assert!(bind_mysql_param(sqlx::query::<sqlx::MySql>("SELECT ?"), &bytes).is_ok());
        assert!(bind_mysql_param(sqlx::query::<sqlx::MySql>("SELECT ?"), &array).is_ok());
    }

    #[cfg(feature = "postgres")]
    #[test]
    fn bind_pg_param_bytes_and_arrays_ok() {
        let bytes = FilterValue::Bytes(vec![0, 255]);
        assert!(bind_pg_param(sqlx::query::<sqlx::Postgres>("SELECT $1"), &bytes).is_ok());
        for array in [
            FilterValue::Array(vec![FilterValue::Int(1), FilterValue::Null]),
            FilterValue::Array(vec![FilterValue::Float(1.5)]),
            FilterValue::Array(vec![FilterValue::Bool(true)]),
            FilterValue::Array(vec![FilterValue::String("a".into())]),
            FilterValue::Array(vec![FilterValue::Json(serde_json::json!({"a": 1}))]),
            FilterValue::Array(vec![FilterValue::Bytes(vec![1, 2])]),
            FilterValue::Array(vec![]),
            FilterValue::Array(vec![FilterValue::Null]),
        ] {
            assert!(
                bind_pg_param(sqlx::query::<sqlx::Postgres>("SELECT $1"), &array).is_ok(),
                "{array:?}"
            );
        }
        let mixed = FilterValue::Array(vec![FilterValue::Int(1), FilterValue::String("a".into())]);
        let err = bind_pg_param(sqlx::query::<sqlx::Postgres>("SELECT $1"), &mixed)
            .err()
            .expect("mixed array must be rejected");
        assert!(
            matches!(err, SqlxError::TypeConversion(ref msg) if msg.contains("share one scalar kind")),
            "unexpected error: {err}"
        );
    }

    #[cfg(feature = "postgres")]
    #[test]
    fn pg_array_kind_picks_first_non_null() {
        use FilterValue::*;
        assert_eq!(pg_array_kind(&[Int(1), Null]).unwrap(), PgArrayKind::Int);
        assert_eq!(
            pg_array_kind(&[Null, Float(1.0)]).unwrap(),
            PgArrayKind::Float
        );
        assert_eq!(pg_array_kind(&[Bool(true)]).unwrap(), PgArrayKind::Bool);
        assert_eq!(
            pg_array_kind(&[String("a".into())]).unwrap(),
            PgArrayKind::Text
        );
        assert_eq!(
            pg_array_kind(&[Json(serde_json::json!(1))]).unwrap(),
            PgArrayKind::Json
        );
        assert_eq!(
            pg_array_kind(&[Bytes(vec![1])]).unwrap(),
            PgArrayKind::Bytes
        );
        assert_eq!(pg_array_kind(&[]).unwrap(), PgArrayKind::Text);
        assert_eq!(pg_array_kind(&[Null, Null]).unwrap(), PgArrayKind::Text);
    }

    #[cfg(feature = "postgres")]
    #[test]
    fn pg_array_kind_rejects_mixed_and_nested() {
        use FilterValue::*;
        assert!(pg_array_kind(&[Int(1), String("a".into())]).is_err());
        assert!(pg_array_kind(&[Int(1), Float(1.0)]).is_err());
        assert!(pg_array_kind(&[Array(vec![])]).is_err());
        assert!(pg_array_kind(&[List(vec![])]).is_err());
        assert!(pg_array_kind(&[Null, List(vec![])]).is_err());
    }
}
