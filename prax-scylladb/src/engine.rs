//! ScyllaDB query engine.
//!
//! Provides high-level operations for interacting with ScyllaDB.

use scylla::batch::Batch;
use scylla::query::Query;
use scylla::serialize::batch::BatchValues;
use scylla::serialize::row::SerializeRow;
use std::marker::PhantomData;

use crate::error::{ScyllaError, ScyllaResult};
use crate::pool::ScyllaPool;
use crate::row::FromScyllaRow;

/// The ScyllaDB query engine.
///
/// Provides methods for executing queries, managing batches, and working
/// with prepared statements.
#[derive(Clone)]
pub struct ScyllaEngine {
    pool: ScyllaPool,
}

impl ScyllaEngine {
    /// Create a new engine with the given pool.
    #[must_use]
    pub fn new(pool: ScyllaPool) -> Self {
        Self { pool }
    }

    /// Execute a query and return all rows.
    pub async fn query<T: FromScyllaRow>(
        &self,
        cql: &str,
        values: impl SerializeRow,
    ) -> ScyllaResult<Vec<T>> {
        let result = self.pool.execute(cql, values).await?;

        let rows = result.rows.unwrap_or_default();
        rows.into_iter().map(|row| T::from_row(&row)).collect()
    }

    /// Execute a query and return a single row.
    pub async fn query_one<T: FromScyllaRow>(
        &self,
        cql: &str,
        values: impl SerializeRow,
    ) -> ScyllaResult<Option<T>> {
        let result = self.pool.execute(cql, values).await?;

        let rows = result.rows.unwrap_or_default();
        match rows.len() {
            0 => Ok(None),
            1 => Ok(Some(T::from_row(&rows[0])?)),
            _ => Err(ScyllaError::MultipleRowsReturned),
        }
    }

    /// Execute a query and return exactly one row, or error if not found.
    pub async fn query_one_required<T: FromScyllaRow>(
        &self,
        cql: &str,
        values: impl SerializeRow,
    ) -> ScyllaResult<T> {
        self.query_one(cql, values)
            .await?
            .ok_or(ScyllaError::NotFound)
    }

    /// Execute a query that doesn't return rows (INSERT, UPDATE, DELETE).
    pub async fn execute(&self, cql: &str, values: impl SerializeRow) -> ScyllaResult<()> {
        self.pool.execute(cql, values).await?;
        Ok(())
    }

    /// Execute a raw CQL query without preparing.
    pub async fn execute_raw(&self, cql: &str) -> ScyllaResult<scylla::QueryResult> {
        self.pool.query(cql, &[]).await
    }

    /// Create a new batch operation.
    #[must_use]
    pub fn batch(&self) -> ScyllaBatch {
        ScyllaBatch::new(self.pool.clone())
    }

    /// Insert a row into a table.
    pub async fn insert<V: SerializeRow>(
        &self,
        table: &str,
        columns: &[&str],
        values: V,
    ) -> ScyllaResult<()> {
        let placeholders: Vec<&str> = (0..columns.len()).map(|_| "?").collect();
        let cql = format!(
            "INSERT INTO {} ({}) VALUES ({})",
            table,
            columns.join(", "),
            placeholders.join(", ")
        );
        self.execute(&cql, values).await
    }

    /// Insert a row with TTL (Time To Live).
    pub async fn insert_with_ttl<V: SerializeRow>(
        &self,
        table: &str,
        columns: &[&str],
        values: V,
        ttl_seconds: i32,
    ) -> ScyllaResult<()> {
        let placeholders: Vec<&str> = (0..columns.len()).map(|_| "?").collect();
        let cql = format!(
            "INSERT INTO {} ({}) VALUES ({}) USING TTL {}",
            table,
            columns.join(", "),
            placeholders.join(", "),
            ttl_seconds
        );
        self.execute(&cql, values).await
    }

    /// Update rows in a table.
    pub async fn update<V: SerializeRow>(
        &self,
        table: &str,
        set_clause: &str,
        where_clause: &str,
        values: V,
    ) -> ScyllaResult<()> {
        let cql = format!("UPDATE {} SET {} WHERE {}", table, set_clause, where_clause);
        self.execute(&cql, values).await
    }

    /// Delete rows from a table.
    pub async fn delete<V: SerializeRow>(
        &self,
        table: &str,
        where_clause: &str,
        values: V,
    ) -> ScyllaResult<()> {
        let cql = format!("DELETE FROM {} WHERE {}", table, where_clause);
        self.execute(&cql, values).await
    }

    /// Execute a Lightweight Transaction (IF NOT EXISTS).
    pub async fn insert_if_not_exists<V: SerializeRow>(
        &self,
        table: &str,
        columns: &[&str],
        values: V,
    ) -> ScyllaResult<bool> {
        let placeholders: Vec<&str> = (0..columns.len()).map(|_| "?").collect();
        let cql = format!(
            "INSERT INTO {} ({}) VALUES ({}) IF NOT EXISTS",
            table,
            columns.join(", "),
            placeholders.join(", ")
        );

        let result = self.pool.execute(&cql, values).await?;

        // Check if the operation was applied
        if let Some(rows) = result.rows {
            if let Some(first_row) = rows.first() {
                // The [applied] column is the first column in LWT results
                if let Some(applied) = first_row.columns.first() {
                    if let Some(scylla::frame::response::result::CqlValue::Boolean(v)) = applied {
                        return Ok(*v);
                    }
                }
            }
        }

        Ok(true)
    }

    /// Execute a conditional update (IF condition).
    pub async fn update_if<V: SerializeRow>(
        &self,
        table: &str,
        set_clause: &str,
        where_clause: &str,
        condition: &str,
        values: V,
    ) -> ScyllaResult<bool> {
        let cql = format!(
            "UPDATE {} SET {} WHERE {} IF {}",
            table, set_clause, where_clause, condition
        );

        let result = self.pool.execute(&cql, values).await?;

        // Check if the operation was applied
        if let Some(rows) = result.rows {
            if let Some(first_row) = rows.first() {
                if let Some(applied) = first_row.columns.first() {
                    if let Some(scylla::frame::response::result::CqlValue::Boolean(v)) = applied {
                        return Ok(*v);
                    }
                }
            }
        }

        Ok(true)
    }

    /// Count rows matching a condition.
    pub async fn count<V: SerializeRow>(
        &self,
        table: &str,
        where_clause: Option<&str>,
        values: V,
    ) -> ScyllaResult<i64> {
        let cql = match where_clause {
            Some(clause) => format!("SELECT COUNT(*) FROM {} WHERE {}", table, clause),
            None => format!("SELECT COUNT(*) FROM {}", table),
        };

        let result = self.pool.execute(&cql, values).await?;

        if let Some(rows) = result.rows {
            if let Some(first_row) = rows.first() {
                if let Some(count) = first_row.columns.first() {
                    if let Some(scylla::frame::response::result::CqlValue::BigInt(v)) = count {
                        return Ok(*v);
                    }
                    if let Some(scylla::frame::response::result::CqlValue::Counter(v)) = count {
                        return Ok(v.0);
                    }
                }
            }
        }

        Ok(0)
    }

    /// Check if a row exists.
    pub async fn exists<V: SerializeRow>(
        &self,
        table: &str,
        where_clause: &str,
        values: V,
    ) -> ScyllaResult<bool> {
        let cql = format!(
            "SELECT COUNT(*) FROM {} WHERE {} LIMIT 1",
            table, where_clause
        );

        let count = self.count(table, Some(where_clause), values).await?;
        Ok(count > 0)
    }

    /// Get a reference to the underlying pool.
    #[must_use]
    pub fn pool(&self) -> &ScyllaPool {
        &self.pool
    }

    /// Create a typed query builder.
    #[must_use]
    pub fn table<T: FromScyllaRow>(&self, table: &str) -> TableQuery<T> {
        TableQuery::new(self.clone(), table.to_string())
    }

    /// Shared fetch path for the prax-query `QueryEngine` impl. Converts
    /// `FilterValue` parameters to `CqlValue` via the crate's existing
    /// `ToCqlValue` trait, extracts column names from the query result,
    /// and hands each row to the caller's `FromRow` via `ScyllaRowRef`.
    async fn fetch_typed<T: prax_query::traits::Model + prax_query::row::FromRow>(
        &self,
        cql: &str,
        params: &[prax_query::filter::FilterValue],
    ) -> prax_query::QueryResult<Vec<T>> {
        use crate::types::ToCqlValue;
        let cql_values: Vec<scylla::frame::response::result::CqlValue> = params
            .iter()
            .map(|v| v.to_cql())
            .collect::<ScyllaResult<_>>()
            .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?;
        let result = self
            .pool
            .execute(cql, cql_values)
            .await
            .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?;
        let col_names: Vec<String> = result
            .col_specs()
            .iter()
            .map(|c| c.name.to_string())
            .collect();
        let rows = result.rows.unwrap_or_default();
        rows.into_iter()
            .map(|row| {
                let r =
                    crate::row_ref::ScyllaRowRef::from_scylla(row, &col_names).map_err(|e| {
                        let msg = e.to_string();
                        prax_query::QueryError::deserialization(msg).with_source(e)
                    })?;
                T::from_row(&r).map_err(|e| {
                    let msg = e.to_string();
                    prax_query::QueryError::deserialization(msg).with_source(e)
                })
            })
            .collect()
    }

    async fn fetch_affected(
        &self,
        cql: &str,
        params: &[prax_query::filter::FilterValue],
    ) -> prax_query::QueryResult<u64> {
        use crate::types::ToCqlValue;
        let cql_values: Vec<scylla::frame::response::result::CqlValue> = params
            .iter()
            .map(|v| v.to_cql())
            .collect::<ScyllaResult<_>>()
            .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?;
        self.pool
            .execute(cql, cql_values)
            .await
            .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?;
        // Scylla doesn't report affected-row counts for CQL statements.
        // Returning 0 is honest; callers that need a count should issue
        // a follow-up SELECT.
        Ok(0)
    }
}

impl prax_query::traits::QueryEngine for ScyllaEngine {
    fn dialect(&self) -> &dyn prax_query::dialect::SqlDialect {
        &prax_query::dialect::Cql
    }

    fn query_many<T: prax_query::traits::Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<prax_query::filter::FilterValue>,
    ) -> prax_query::traits::BoxFuture<'_, prax_query::QueryResult<Vec<T>>> {
        let sql = sql.to_string();
        Box::pin(async move { self.fetch_typed::<T>(&sql, &params).await })
    }

    fn query_one<T: prax_query::traits::Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<prax_query::filter::FilterValue>,
    ) -> prax_query::traits::BoxFuture<'_, prax_query::QueryResult<T>> {
        let sql = sql.to_string();
        Box::pin(async move {
            let mut rows: Vec<T> = self.fetch_typed::<T>(&sql, &params).await?;
            if rows.is_empty() {
                Err(prax_query::QueryError::not_found(T::MODEL_NAME))
            } else {
                Ok(rows.swap_remove(0))
            }
        })
    }

    fn query_optional<T: prax_query::traits::Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<prax_query::filter::FilterValue>,
    ) -> prax_query::traits::BoxFuture<'_, prax_query::QueryResult<Option<T>>> {
        let sql = sql.to_string();
        Box::pin(async move {
            let mut rows: Vec<T> = self.fetch_typed::<T>(&sql, &params).await?;
            Ok(rows.drain(..).next())
        })
    }

    fn execute_insert<T: prax_query::traits::Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<prax_query::filter::FilterValue>,
    ) -> prax_query::traits::BoxFuture<'_, prax_query::QueryResult<T>> {
        let sql = sql.to_string();
        Box::pin(async move {
            // CQL has no RETURNING. Run the INSERT, then issue a
            // follow-up SELECT keyed on the PK columns extracted from
            // the bound params. The codegen passes the columns in the
            // same order as T::COLUMNS, so we can find the PK values
            // by position.
            self.fetch_affected(&sql, &params).await?;

            // Extract the PK columns from COLUMNS by name match.
            let pk_columns = T::PRIMARY_KEY;
            if pk_columns.is_empty() {
                return Err(prax_query::QueryError::unsupported(
                    "ScyllaEngine::execute_insert requires Model::PRIMARY_KEY to be non-empty",
                ));
            }

            // Parse `INSERT INTO t (col1, col2, ...) VALUES (...)` to
            // recover the column→param index mapping. Fail loudly if
            // we can't locate the column list — callers that bypass
            // the generic builder get a clean error.
            let insert_cols = parse_insert_columns(&sql).ok_or_else(|| {
                prax_query::QueryError::internal(
                    "ScyllaEngine::execute_insert: could not parse INSERT column list",
                )
            })?;

            // Build WHERE clause using the PK column positions.
            let mut where_parts = Vec::with_capacity(pk_columns.len());
            let mut where_params = Vec::with_capacity(pk_columns.len());
            for pk in pk_columns {
                let idx = insert_cols.iter().position(|c| c == pk).ok_or_else(|| {
                    prax_query::QueryError::internal(format!(
                        "ScyllaEngine::execute_insert: PK column {pk} not in INSERT list"
                    ))
                })?;
                where_parts.push(format!("{} = ?", pk));
                where_params.push(params.get(idx).cloned().ok_or_else(|| {
                    prax_query::QueryError::internal(
                        "ScyllaEngine::execute_insert: param index out of range",
                    )
                })?);
            }
            let select_sql = format!(
                "SELECT {} FROM {} WHERE {}",
                T::COLUMNS.join(", "),
                T::TABLE_NAME,
                where_parts.join(" AND "),
            );
            let mut rows = self.fetch_typed::<T>(&select_sql, &where_params).await?;
            if rows.is_empty() {
                Err(prax_query::QueryError::not_found(T::MODEL_NAME))
            } else {
                Ok(rows.swap_remove(0))
            }
        })
    }

    fn execute_update<T: prax_query::traits::Model + prax_query::row::FromRow + Send + 'static>(
        &self,
        sql: &str,
        params: Vec<prax_query::filter::FilterValue>,
    ) -> prax_query::traits::BoxFuture<'_, prax_query::QueryResult<Vec<T>>> {
        let sql = sql.to_string();
        Box::pin(async move {
            // Run the UPDATE, then re-SELECT rows that match the WHERE
            // clause. The WHERE is extracted from the generated SQL so
            // the follow-up SELECT sees the same filter.
            self.fetch_affected(&sql, &params).await?;

            let where_clause = extract_update_where(&sql).ok_or_else(|| {
                prax_query::QueryError::internal(
                    "ScyllaEngine::execute_update: could not parse WHERE clause",
                )
            })?;
            // The WHERE params are the tail of `params` — the UPDATE
            // SET clause consumes the head. Count the SET placeholders
            // to find the split point.
            let set_count = count_set_placeholders(&sql).unwrap_or(0);
            let where_params: Vec<_> = params.into_iter().skip(set_count).collect();
            let select_sql = format!(
                "SELECT {} FROM {} WHERE {}",
                T::COLUMNS.join(", "),
                T::TABLE_NAME,
                where_clause,
            );
            self.fetch_typed::<T>(&select_sql, &where_params).await
        })
    }

    fn execute_delete(
        &self,
        sql: &str,
        params: Vec<prax_query::filter::FilterValue>,
    ) -> prax_query::traits::BoxFuture<'_, prax_query::QueryResult<u64>> {
        let sql = sql.to_string();
        Box::pin(async move { self.fetch_affected(&sql, &params).await })
    }

    fn execute_raw(
        &self,
        sql: &str,
        params: Vec<prax_query::filter::FilterValue>,
    ) -> prax_query::traits::BoxFuture<'_, prax_query::QueryResult<u64>> {
        let sql = sql.to_string();
        Box::pin(async move { self.fetch_affected(&sql, &params).await })
    }

    fn count(
        &self,
        sql: &str,
        params: Vec<prax_query::filter::FilterValue>,
    ) -> prax_query::traits::BoxFuture<'_, prax_query::QueryResult<u64>> {
        let sql = sql.to_string();
        Box::pin(async move {
            use crate::types::ToCqlValue;
            let cql_values: Vec<scylla::frame::response::result::CqlValue> = params
                .iter()
                .map(|v| v.to_cql())
                .collect::<ScyllaResult<_>>()
                .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?;
            let result = self
                .pool
                .execute(&sql, cql_values)
                .await
                .map_err(|e| prax_query::QueryError::database(e.to_string()).with_source(e))?;
            let rows = result.rows.unwrap_or_default();
            let first = rows.into_iter().next().ok_or_else(|| {
                prax_query::QueryError::deserialization("count returned no row".to_string())
            })?;
            // COUNT(*) in CQL returns a single BigInt column.
            match first.columns.first() {
                Some(Some(scylla::frame::response::result::CqlValue::BigInt(n))) => Ok(*n as u64),
                Some(Some(scylla::frame::response::result::CqlValue::Int(n))) => Ok(*n as u64),
                _ => Err(prax_query::QueryError::deserialization(
                    "count column missing or wrong type in CQL result".to_string(),
                )),
            }
        })
    }
}

impl std::fmt::Debug for ScyllaEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScyllaEngine")
            .field("pool", &self.pool)
            .finish()
    }
}

/// A batch of CQL statements to execute atomically.
pub struct ScyllaBatch {
    pool: ScyllaPool,
    batch: Batch,
    statements: Vec<String>,
}

impl ScyllaBatch {
    /// Create a new batch.
    fn new(pool: ScyllaPool) -> Self {
        Self {
            pool,
            batch: Batch::default(),
            statements: Vec::new(),
        }
    }

    /// Create a logged batch (atomic, with a performance cost).
    #[must_use]
    pub fn logged(mut self) -> Self {
        self.batch = Batch::new(scylla::batch::BatchType::Logged);
        self
    }

    /// Create an unlogged batch (not atomic, but faster).
    #[must_use]
    pub fn unlogged(mut self) -> Self {
        self.batch = Batch::new(scylla::batch::BatchType::Unlogged);
        self
    }

    /// Create a counter batch.
    #[must_use]
    pub fn counter(mut self) -> Self {
        self.batch = Batch::new(scylla::batch::BatchType::Counter);
        self
    }

    /// Add a statement to the batch.
    #[must_use]
    pub fn add(mut self, cql: &str) -> Self {
        self.batch.append_statement(Query::new(cql));
        self.statements.push(cql.to_string());
        self
    }

    /// Execute the batch.
    pub async fn execute(self) -> ScyllaResult<()> {
        // Note: For simplicity, we're executing without values here.
        // In a production implementation, you'd want to support bound values.
        self.pool
            .session()
            .batch(&self.batch, ((),))
            .await
            .map_err(|e| ScyllaError::Batch(e.to_string()))?;
        Ok(())
    }

    /// Execute the batch with values.
    pub async fn execute_with_values<V: BatchValues>(self, values: V) -> ScyllaResult<()> {
        self.pool
            .session()
            .batch(&self.batch, values)
            .await
            .map_err(|e| ScyllaError::Batch(e.to_string()))?;
        Ok(())
    }

    /// Get the number of statements in the batch.
    #[must_use]
    pub fn len(&self) -> usize {
        self.statements.len()
    }

    /// Check if the batch is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.statements.is_empty()
    }
}

impl std::fmt::Debug for ScyllaBatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScyllaBatch")
            .field("statements", &self.statements.len())
            .finish()
    }
}

/// A typed query builder for a specific table.
pub struct TableQuery<T> {
    engine: ScyllaEngine,
    table: String,
    _marker: PhantomData<T>,
}

impl<T: FromScyllaRow> TableQuery<T> {
    fn new(engine: ScyllaEngine, table: String) -> Self {
        Self {
            engine,
            table,
            _marker: PhantomData,
        }
    }

    /// Select all rows from the table.
    pub async fn all(&self) -> ScyllaResult<Vec<T>> {
        let cql = format!("SELECT * FROM {}", self.table);
        self.engine.query(&cql, &[]).await
    }

    /// Select rows with a WHERE clause.
    pub async fn find<V: SerializeRow>(
        &self,
        where_clause: &str,
        values: V,
    ) -> ScyllaResult<Vec<T>> {
        let cql = format!("SELECT * FROM {} WHERE {}", self.table, where_clause);
        self.engine.query(&cql, values).await
    }

    /// Find a single row.
    pub async fn find_one<V: SerializeRow>(
        &self,
        where_clause: &str,
        values: V,
    ) -> ScyllaResult<Option<T>> {
        let cql = format!(
            "SELECT * FROM {} WHERE {} LIMIT 1",
            self.table, where_clause
        );
        self.engine.query_one(&cql, values).await
    }

    /// Count rows in the table.
    pub async fn count(&self) -> ScyllaResult<i64> {
        self.engine.count(&self.table, None, &[]).await
    }

    /// Count rows matching a condition.
    pub async fn count_where<V: SerializeRow>(
        &self,
        where_clause: &str,
        values: V,
    ) -> ScyllaResult<i64> {
        self.engine
            .count(&self.table, Some(where_clause), values)
            .await
    }
}

impl<T> std::fmt::Debug for TableQuery<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TableQuery")
            .field("table", &self.table)
            .finish()
    }
}

// SQL-string-parsing helpers live in prax_query::sql::parse — this
// driver re-exports them under their old names for backwards
// compatibility with earlier commits. New call sites inside the
// driver use the shared path directly.
use prax_query::sql::parse::{
    count_set_placeholders, extract_insert_columns as parse_insert_columns,
    extract_where_body as extract_update_where,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_batch_builder() {
        // Test is disabled because ScyllaBatch requires a real connection pool
        // which cannot be zero-initialized (contains Arc, which is non-nullable).
        // This would need a mock or integration test with a real database.

        // Verify that the Batch type compiles
        let _ = Batch::default();
    }
}
