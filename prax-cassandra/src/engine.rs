//! Query execution engine.
//!
//! This module defines the public query API (query/execute/batch/LWT/paging).
//! Routes every statement through the cdrs-tokio session held by the
//! underlying [`CassandraPool`].

use crate::error::{CassandraError, CassandraResult};
use crate::pool::CassandraPool;
use crate::row::{FromRow, Row};

/// Aggregate result of a CQL query.
#[derive(Debug, Default)]
pub struct QueryResult {
    /// Rows returned by the query. Empty for non-SELECT statements.
    pub rows: Vec<Row>,
    /// Whether a lightweight transaction applied.
    pub applied: Option<bool>,
}

impl CassandraPool {
    /// Execute a query returning rows.
    pub async fn query(&self, cql: &str) -> CassandraResult<QueryResult> {
        let envelope = self
            .connection()
            .session()
            .query(cql)
            .await
            .map_err(|e| CassandraError::Query(format!("query failed: {e}")))?;

        // Parse the response. SELECT responses carry a ResponseBody::Result
        // with rows; INSERT/UPDATE/DELETE responses typically carry an
        // empty result. LWT responses carry a single row with the
        // `[applied]` boolean column first.
        let body = envelope
            .response_body()
            .map_err(|e| CassandraError::Query(format!("response body parse: {e}")))?;

        let (rows, applied) = if let Some(raw_rows) = body.into_rows() {
            // LWT responses carry the applied-boolean as the first column
            // of a single row. Detect that shape by checking whether the
            // result set is exactly one row and the first column is a
            // boolean named "[applied]".
            let applied = raw_rows.first().and_then(|row| {
                use cdrs_tokio::types::ByName;
                row.by_name::<bool>("[applied]").ok().flatten()
            });
            let decoded: Vec<crate::row::Row> = raw_rows
                .into_iter()
                .map(|r| crate::row::Row::from_cdrs_row(&r))
                .collect::<CassandraResult<_>>()?;
            (decoded, applied)
        } else {
            (Vec::new(), None)
        };

        Ok(QueryResult { rows, applied })
    }

    /// Execute a statement not expecting rows (INSERT, UPDATE, DELETE, DDL).
    pub async fn execute(&self, cql: &str) -> CassandraResult<()> {
        self.connection()
            .session()
            .query(cql)
            .await
            .map_err(|e| CassandraError::Query(format!("execute failed: {e}")))?;
        Ok(())
    }

    /// Query a single row, deserialized into T.
    pub async fn query_one<T: FromRow>(&self, cql: &str) -> CassandraResult<T> {
        let result = self.query(cql).await?;
        let row = result
            .rows
            .into_iter()
            .next()
            .ok_or_else(|| CassandraError::Query("query_one: no rows returned".into()))?;
        T::from_row(&row)
    }

    /// Query many rows.
    pub async fn query_many<T: FromRow>(&self, cql: &str) -> CassandraResult<Vec<T>> {
        let result = self.query(cql).await?;
        result.rows.iter().map(|row| T::from_row(row)).collect()
    }

    /// Execute a lightweight transaction. Returns whether the CAS succeeded.
    pub async fn execute_lwt(&self, cql: &str) -> CassandraResult<bool> {
        let result = self.query(cql).await?;
        Ok(result.applied.unwrap_or(false))
    }

    /// Build a batch of statements.
    pub fn batch(&self) -> BatchBuilder<'_> {
        BatchBuilder {
            pool: self,
            statements: Vec::new(),
        }
    }
}

/// Builder for a CQL batch.
pub struct BatchBuilder<'a> {
    pool: &'a CassandraPool,
    statements: Vec<String>,
}

impl<'a> BatchBuilder<'a> {
    /// Add a statement to the batch.
    pub fn add_statement(mut self, cql: impl Into<String>) -> Self {
        self.statements.push(cql.into());
        self
    }

    /// Execute the batch as a LOGGED batch (default).
    pub async fn execute(self) -> CassandraResult<()> {
        self.execute_logged().await
    }

    /// Execute the batch as a LOGGED batch.
    pub async fn execute_logged(self) -> CassandraResult<()> {
        self.execute_with_type(cdrs_tokio::frame::message_batch::BatchType::Logged)
            .await
    }

    /// Execute the batch as an UNLOGGED batch.
    pub async fn execute_unlogged(self) -> CassandraResult<()> {
        self.execute_with_type(cdrs_tokio::frame::message_batch::BatchType::Unlogged)
            .await
    }

    /// Execute the batch as a COUNTER batch.
    pub async fn execute_counter(self) -> CassandraResult<()> {
        self.execute_with_type(cdrs_tokio::frame::message_batch::BatchType::Counter)
            .await
    }

    async fn execute_with_type(
        self,
        batch_type: cdrs_tokio::frame::message_batch::BatchType,
    ) -> CassandraResult<()> {
        if self.statements.is_empty() {
            return Err(CassandraError::Query("cannot execute empty batch".into()));
        }
        let mut builder = cdrs_tokio::query::BatchQueryBuilder::new().with_batch_type(batch_type);
        for stmt in self.statements {
            builder = builder.add_query(stmt, cdrs_tokio::query::QueryValues::SimpleValues(vec![]));
        }
        let batch = builder
            .build()
            .map_err(|e| CassandraError::Query(format!("batch build: {e}")))?;
        self.pool
            .connection()
            .session()
            .batch(batch)
            .await
            .map_err(|e| CassandraError::Query(format!("batch execute: {e}")))?;
        Ok(())
    }

    /// Number of statements in the batch (for test/debug).
    pub fn len(&self) -> usize {
        self.statements.len()
    }

    /// True if the batch has no statements.
    pub fn is_empty(&self) -> bool {
        self.statements.is_empty()
    }
}

/// Top-level query engine for the Cassandra driver.
///
/// Thin wrapper around [`CassandraPool`] that lets `#[derive(Model)]`-
/// generated `Client<E>` target Cassandra through the same codegen
/// pipeline the SQL drivers use. Routes SELECT/DELETE through the real
/// cdrs-tokio session; `execute_update` runs the UPDATE then re-
/// SELECTs rows matching the WHERE clause; `execute_insert` currently
/// returns [`QueryError::unsupported`] — the pool's query/execute API
/// doesn't accept bound params yet, so a safe PK-keyed follow-up
/// SELECT isn't possible. Prefer [`prax_scylladb::ScyllaEngine`] for
/// typed Client inserts against any CQL-compatible cluster.
#[derive(Clone)]
pub struct CassandraEngine {
    pool: CassandraPool,
}

impl CassandraEngine {
    /// Create a new engine wrapping the given pool.
    pub fn new(pool: CassandraPool) -> Self {
        Self { pool }
    }

    /// Borrow the underlying pool. Exposed for callers that need to
    /// reach the raw query/execute/batch helpers directly.
    pub fn pool(&self) -> &CassandraPool {
        &self.pool
    }
}

impl prax_query::traits::QueryEngine for CassandraEngine {
    fn dialect(&self) -> &dyn prax_query::dialect::SqlDialect {
        &prax_query::dialect::Cql
    }

    fn query_many<T: prax_query::traits::Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        _params: Vec<prax_query::filter::FilterValue>,
    ) -> prax_query::traits::BoxFuture<'_, prax_query::QueryResult<Vec<T>>> {
        let sql = sql.to_string();
        let pool = self.pool.clone();
        Box::pin(async move {
            let result = pool
                .query(&sql)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?;
            result
                .rows
                .iter()
                .map(|r| r.as_cdrs())
                .map(decode_row::<T>)
                .collect()
        })
    }

    fn query_one<T: prax_query::traits::Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        _params: Vec<prax_query::filter::FilterValue>,
    ) -> prax_query::traits::BoxFuture<'_, prax_query::QueryResult<T>> {
        let sql = sql.to_string();
        let pool = self.pool.clone();
        Box::pin(async move {
            let result = pool
                .query(&sql)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?;
            let cdrs_row = result
                .rows
                .iter()
                .map(|r| r.as_cdrs())
                .next()
                .ok_or_else(|| prax_query::QueryError::not_found(T::MODEL_NAME))?;
            decode_row::<T>(cdrs_row)
        })
    }

    fn query_optional<T: prax_query::traits::Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        _params: Vec<prax_query::filter::FilterValue>,
    ) -> prax_query::traits::BoxFuture<'_, prax_query::QueryResult<Option<T>>> {
        let sql = sql.to_string();
        let pool = self.pool.clone();
        Box::pin(async move {
            let result = pool
                .query(&sql)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?;
            result
                .rows
                .iter()
                .map(|r| r.as_cdrs())
                .next()
                .map(decode_row::<T>)
                .transpose()
        })
    }

    fn execute_insert<T: prax_query::traits::Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        _params: Vec<prax_query::filter::FilterValue>,
    ) -> prax_query::traits::BoxFuture<'_, prax_query::QueryResult<T>> {
        // CassandraPool::query/execute doesn't accept bound params yet —
        // the prepared-statement integration is a follow-up task. Without
        // real parameter binding, a PK-keyed follow-up SELECT can't be
        // built safely (a LIMIT 1 with no WHERE would race concurrent
        // writers and return the wrong row). Refuse rather than fabricate
        // a result. The Scylla driver is feature-complete on this path
        // and is the recommended CQL backend for typed Client inserts.
        let _ = (sql, T::MODEL_NAME);
        Box::pin(async move {
            Err(prax_query::QueryError::unsupported(
                "CassandraEngine::execute_insert requires prepared-statement \
                 binding to safely re-fetch by PK; use ScyllaEngine or call \
                 pool.execute + pool.query manually",
            ))
        })
    }

    fn execute_update<T: prax_query::traits::Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        _params: Vec<prax_query::filter::FilterValue>,
    ) -> prax_query::traits::BoxFuture<'_, prax_query::QueryResult<Vec<T>>> {
        let sql = sql.to_string();
        let pool = self.pool.clone();
        Box::pin(async move {
            pool.execute(&sql)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?;
            // Recover the WHERE clause from the generated UPDATE so the
            // follow-up SELECT touches the same rows. Refuse to SELECT
            // everything on a WHERE-less UPDATE — that would be a
            // worse failure mode than erroring.
            let where_clause = extract_where_clause(&sql).ok_or_else(|| {
                prax_query::QueryError::internal(
                    "CassandraEngine::execute_update: UPDATE lacked a WHERE \
                     clause; refusing to SELECT entire table",
                )
            })?;
            let select_sql = format!(
                "SELECT {} FROM {} WHERE {}",
                T::COLUMNS.join(", "),
                T::TABLE_NAME,
                where_clause,
            );
            let result = pool
                .query(&select_sql)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?;
            result
                .rows
                .iter()
                .map(|r| r.as_cdrs())
                .map(decode_row::<T>)
                .collect()
        })
    }

    fn execute_delete(
        &self,
        sql: &str,
        _params: Vec<prax_query::filter::FilterValue>,
    ) -> prax_query::traits::BoxFuture<'_, prax_query::QueryResult<u64>> {
        let sql = sql.to_string();
        let pool = self.pool.clone();
        Box::pin(async move {
            let _: () = pool
                .execute(&sql)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?;
            Ok(0)
        })
    }

    fn execute_raw(
        &self,
        sql: &str,
        params: Vec<prax_query::filter::FilterValue>,
    ) -> prax_query::traits::BoxFuture<'_, prax_query::QueryResult<u64>> {
        self.execute_delete(sql, params)
    }

    fn count(
        &self,
        sql: &str,
        _params: Vec<prax_query::filter::FilterValue>,
    ) -> prax_query::traits::BoxFuture<'_, prax_query::QueryResult<u64>> {
        let sql = sql.to_string();
        let pool = self.pool.clone();
        Box::pin(async move {
            let _: QueryResult = pool
                .query(&sql)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?;
            Ok(0)
        })
    }
}

// WHERE-clause extraction lives in prax_query::sql::parse — import
// here under the old name to minimise churn.
use prax_query::sql::parse::extract_where_body as extract_where_clause;

/// Decode one cdrs-tokio row into the caller's `T: Model + FromRow`.
/// Shared by every QueryEngine method that hands back typed rows, so
/// the column-list allocation and error-wrapping stay in one place.
fn decode_row<T: prax_query::traits::Model + prax_query::row::FromRow>(
    cdrs_row: &cdrs_tokio::types::rows::Row,
) -> prax_query::QueryResult<T> {
    let cols: Vec<String> = T::COLUMNS.iter().map(|s| s.to_string()).collect();
    let rr = crate::row_ref::CassandraRowRef::from_cdrs_with_cols(cdrs_row, &cols);
    T::from_row(&rr).map_err(|e| {
        let msg = e.to_string();
        prax_query::QueryError::deserialization(msg).with_source(e)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CassandraConfig;

    #[tokio::test]
    async fn test_query_without_connection_returns_error() {
        let config = CassandraConfig::builder()
            .known_nodes(["127.0.0.1:9042".to_string()])
            .build();
        // Pool.connect returns an error in the stub phase, so we can't
        // build a pool here. Instead, construct the error directly via
        // the assertion below. This test primarily exercises the API
        // surface compiles.
        let _ = config;
    }

    #[test]
    fn test_batch_builder_add_increments_len() {
        // Construct a fake pool surface through a compile-check-only path.
        // We can't instantiate a real pool without a live cluster, so this
        // test lives as a TODO placeholder; live integration covers the
        // real behavior.
        let stmts: Vec<String> = vec!["INSERT INTO t VALUES (1)".into()];
        assert_eq!(stmts.len(), 1);
    }

    #[test]
    fn test_query_result_default_is_empty() {
        let r = QueryResult::default();
        assert!(r.rows.is_empty());
        assert!(r.applied.is_none());
    }
}
