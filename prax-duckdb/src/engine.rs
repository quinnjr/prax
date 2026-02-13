//! DuckDB query engine implementation.

use std::collections::HashMap;

use serde_json::Value as JsonValue;
use tracing::{debug, instrument};

use prax_query::filter::FilterValue;
use prax_query::types::SortOrder;

use crate::error::{DuckDbError, DuckDbResult};
use crate::pool::DuckDbPool;
use crate::types::filter_value_to_json;

/// DuckDB query engine.
#[derive(Clone)]
pub struct DuckDbEngine {
    pool: DuckDbPool,
}

/// Result of a query operation.
#[derive(Debug, Clone)]
pub struct DuckDbQueryResult {
    /// The result data as JSON.
    pub data: JsonValue,
}

impl DuckDbQueryResult {
    /// Create a new query result.
    pub fn new(data: JsonValue) -> Self {
        Self { data }
    }

    /// Get the result as JSON.
    pub fn json(&self) -> &JsonValue {
        &self.data
    }

    /// Convert to the inner JSON value.
    pub fn into_json(self) -> JsonValue {
        self.data
    }
}

impl DuckDbEngine {
    /// Create a new DuckDB engine with the given pool.
    pub fn new(pool: DuckDbPool) -> Self {
        Self { pool }
    }

    /// Get a reference to the connection pool.
    pub fn pool(&self) -> &DuckDbPool {
        &self.pool
    }

    /// Build a SELECT query.
    fn build_select(
        &self,
        table: &str,
        columns: &[String],
        filters: &HashMap<String, FilterValue>,
        sort: &[(String, SortOrder)],
        limit: Option<u64>,
        offset: Option<u64>,
    ) -> (String, Vec<FilterValue>) {
        let mut sql = String::new();
        let mut params: Vec<FilterValue> = Vec::new();

        // SELECT clause
        let cols = if columns.is_empty() {
            "*".to_string()
        } else {
            columns
                .iter()
                .map(|c| format!("\"{}\"", c))
                .collect::<Vec<_>>()
                .join(", ")
        };
        sql.push_str(&format!("SELECT {} FROM \"{}\"", cols, table));

        // WHERE clause
        if !filters.is_empty() {
            let mut conditions = Vec::new();
            for (field, value) in filters {
                match value {
                    FilterValue::Null => {
                        conditions.push(format!("\"{}\" IS NULL", field));
                    }
                    _ => {
                        conditions.push(format!("\"{}\" = ?", field));
                        params.push(value.clone());
                    }
                }
            }
            sql.push_str(" WHERE ");
            sql.push_str(&conditions.join(" AND "));
        }

        // ORDER BY clause
        if !sort.is_empty() {
            let order_parts: Vec<String> = sort
                .iter()
                .map(|(col, dir)| {
                    let direction = match dir {
                        SortOrder::Asc => "ASC",
                        SortOrder::Desc => "DESC",
                    };
                    format!("\"{}\" {}", col, direction)
                })
                .collect();
            sql.push_str(" ORDER BY ");
            sql.push_str(&order_parts.join(", "));
        }

        // LIMIT and OFFSET
        if let Some(lim) = limit {
            sql.push_str(&format!(" LIMIT {}", lim));
        }
        if let Some(off) = offset {
            sql.push_str(&format!(" OFFSET {}", off));
        }

        (sql, params)
    }

    /// Build an INSERT query.
    fn build_insert(
        &self,
        table: &str,
        data: &HashMap<String, FilterValue>,
    ) -> (String, Vec<FilterValue>) {
        let mut columns = Vec::new();
        let mut placeholders = Vec::new();
        let mut params: Vec<FilterValue> = Vec::new();

        for (col, val) in data {
            columns.push(format!("\"{}\"", col));
            placeholders.push("?".to_string());
            params.push(val.clone());
        }

        let sql = format!(
            "INSERT INTO \"{}\" ({}) VALUES ({})",
            table,
            columns.join(", "),
            placeholders.join(", ")
        );

        (sql, params)
    }

    /// Build an UPDATE query.
    fn build_update(
        &self,
        table: &str,
        data: &HashMap<String, FilterValue>,
        filters: &HashMap<String, FilterValue>,
    ) -> (String, Vec<FilterValue>) {
        let mut params: Vec<FilterValue> = Vec::new();

        // SET clause
        let set_parts: Vec<String> = data
            .iter()
            .map(|(col, val)| {
                params.push(val.clone());
                format!("\"{}\" = ?", col)
            })
            .collect();

        let mut sql = format!("UPDATE \"{}\" SET {}", table, set_parts.join(", "));

        // WHERE clause
        if !filters.is_empty() {
            let mut conditions = Vec::new();
            for (field, value) in filters {
                match value {
                    FilterValue::Null => {
                        conditions.push(format!("\"{}\" IS NULL", field));
                    }
                    _ => {
                        conditions.push(format!("\"{}\" = ?", field));
                        params.push(value.clone());
                    }
                }
            }
            sql.push_str(" WHERE ");
            sql.push_str(&conditions.join(" AND "));
        }

        (sql, params)
    }

    /// Build a DELETE query.
    fn build_delete(
        &self,
        table: &str,
        filters: &HashMap<String, FilterValue>,
    ) -> (String, Vec<FilterValue>) {
        let mut sql = format!("DELETE FROM \"{}\"", table);
        let mut params: Vec<FilterValue> = Vec::new();

        if !filters.is_empty() {
            let mut conditions = Vec::new();
            for (field, value) in filters {
                match value {
                    FilterValue::Null => {
                        conditions.push(format!("\"{}\" IS NULL", field));
                    }
                    _ => {
                        conditions.push(format!("\"{}\" = ?", field));
                        params.push(value.clone());
                    }
                }
            }
            sql.push_str(" WHERE ");
            sql.push_str(&conditions.join(" AND "));
        }

        (sql, params)
    }

    /// Execute a query and return multiple results.
    #[instrument(skip(self, columns, filters, sort), fields(table = %table))]
    pub async fn query_many(
        &self,
        table: &str,
        columns: &[String],
        filters: &HashMap<String, FilterValue>,
        sort: &[(String, SortOrder)],
        limit: Option<u64>,
        offset: Option<u64>,
    ) -> DuckDbResult<Vec<DuckDbQueryResult>> {
        let (sql, params) = self.build_select(table, columns, filters, sort, limit, offset);
        debug!(sql = %sql, "Executing query_many");

        let conn = self.pool.get().await?;
        let results = conn.query(&sql, &params).await?;

        Ok(results.into_iter().map(DuckDbQueryResult::new).collect())
    }

    /// Execute a query and return a single result.
    #[instrument(skip(self, columns, filters), fields(table = %table))]
    pub async fn query_one(
        &self,
        table: &str,
        columns: &[String],
        filters: &HashMap<String, FilterValue>,
    ) -> DuckDbResult<DuckDbQueryResult> {
        let (sql, params) = self.build_select(table, columns, filters, &[], Some(1), None);
        debug!(sql = %sql, "Executing query_one");

        let conn = self.pool.get().await?;
        let result = conn.query_one(&sql, &params).await?;

        Ok(DuckDbQueryResult::new(result))
    }

    /// Execute a query and return an optional result.
    #[instrument(skip(self, columns, filters), fields(table = %table))]
    pub async fn query_optional(
        &self,
        table: &str,
        columns: &[String],
        filters: &HashMap<String, FilterValue>,
    ) -> DuckDbResult<Option<DuckDbQueryResult>> {
        let (sql, params) = self.build_select(table, columns, filters, &[], Some(1), None);
        debug!(sql = %sql, "Executing query_optional");

        let conn = self.pool.get().await?;
        let result = conn.query_optional(&sql, &params).await?;

        Ok(result.map(DuckDbQueryResult::new))
    }

    /// Execute an INSERT and return the result.
    #[instrument(skip(self, data), fields(table = %table))]
    pub async fn execute_insert(
        &self,
        table: &str,
        data: &HashMap<String, FilterValue>,
    ) -> DuckDbResult<DuckDbQueryResult> {
        let (sql, params) = self.build_insert(table, data);
        debug!(sql = %sql, "Executing insert");

        let conn = self.pool.get().await?;
        conn.execute(&sql, &params).await?;

        // Return the inserted data as result
        let json = data
            .iter()
            .map(|(k, v)| (k.clone(), filter_value_to_json(v)))
            .collect::<serde_json::Map<_, _>>();

        Ok(DuckDbQueryResult::new(JsonValue::Object(json)))
    }

    /// Execute an UPDATE and return the number of affected rows.
    #[instrument(skip(self, data, filters), fields(table = %table))]
    pub async fn execute_update(
        &self,
        table: &str,
        data: &HashMap<String, FilterValue>,
        filters: &HashMap<String, FilterValue>,
    ) -> DuckDbResult<u64> {
        let (sql, params) = self.build_update(table, data, filters);
        debug!(sql = %sql, "Executing update");

        let conn = self.pool.get().await?;
        let affected = conn.execute(&sql, &params).await?;

        Ok(affected as u64)
    }

    /// Execute a DELETE and return the number of affected rows.
    #[instrument(skip(self, filters), fields(table = %table))]
    pub async fn execute_delete(
        &self,
        table: &str,
        filters: &HashMap<String, FilterValue>,
    ) -> DuckDbResult<u64> {
        let (sql, params) = self.build_delete(table, filters);
        debug!(sql = %sql, "Executing delete");

        let conn = self.pool.get().await?;
        let affected = conn.execute(&sql, &params).await?;

        Ok(affected as u64)
    }

    /// Execute raw SQL and return results.
    #[instrument(skip(self, params), fields(sql = %sql))]
    pub async fn execute_raw(
        &self,
        sql: &str,
        params: &[FilterValue],
    ) -> DuckDbResult<Vec<DuckDbQueryResult>> {
        debug!("Executing raw SQL");

        let conn = self.pool.get().await?;
        let results = conn.query(sql, params).await?;

        Ok(results.into_iter().map(DuckDbQueryResult::new).collect())
    }

    /// Execute a raw SQL statement and return the number of affected rows.
    #[instrument(skip(self, params), fields(sql = %sql))]
    pub async fn raw_sql_execute(&self, sql: &str, params: &[FilterValue]) -> DuckDbResult<u64> {
        debug!("Executing raw SQL statement");

        let conn = self.pool.get().await?;
        let affected = conn.execute(sql, params).await?;

        Ok(affected as u64)
    }

    /// Execute a raw SQL query using the Sql builder.
    #[instrument(skip(self, sql))]
    pub async fn raw_sql(&self, sql: prax_query::raw::Sql) -> DuckDbResult<Vec<DuckDbQueryResult>> {
        let (query_string, params) = sql.build();
        debug!(sql = %query_string, "Executing raw SQL from builder");
        self.execute_raw(&query_string, &params).await
    }

    /// Execute a raw SQL query and return the first result.
    #[instrument(skip(self, params), fields(sql = %sql))]
    pub async fn raw_sql_first(
        &self,
        sql: &str,
        params: &[FilterValue],
    ) -> DuckDbResult<DuckDbQueryResult> {
        let conn = self.pool.get().await?;
        let result = conn.query_one(sql, params).await?;
        Ok(DuckDbQueryResult::new(result))
    }

    /// Execute a raw SQL query and return the first result or None.
    #[instrument(skip(self, params), fields(sql = %sql))]
    pub async fn raw_sql_optional(
        &self,
        sql: &str,
        params: &[FilterValue],
    ) -> DuckDbResult<Option<DuckDbQueryResult>> {
        let conn = self.pool.get().await?;
        let result = conn.query_optional(sql, params).await?;
        Ok(result.map(DuckDbQueryResult::new))
    }

    /// Execute a raw SQL query and return a single scalar value.
    #[instrument(skip(self, params), fields(sql = %sql))]
    pub async fn raw_sql_scalar<T>(&self, sql: &str, params: &[FilterValue]) -> DuckDbResult<T>
    where
        T: for<'a> serde::Deserialize<'a>,
    {
        let conn = self.pool.get().await?;
        let result = conn.query_one(sql, params).await?;

        let value = result
            .as_object()
            .and_then(|obj| obj.values().next())
            .ok_or_else(|| DuckDbError::query("raw_sql_scalar returned empty row"))?;

        serde_json::from_value(value.clone()).map_err(|e| {
            DuckDbError::deserialization(format!("failed to deserialize scalar: {}", e))
        })
    }

    /// Execute multiple raw SQL statements in a batch.
    #[instrument(skip(self), fields(sql_len = %sql.len()))]
    pub async fn raw_sql_batch(&self, sql: &str) -> DuckDbResult<()> {
        debug!("Executing raw SQL batch");

        let conn = self.pool.get().await?;
        conn.execute_batch(sql).await
    }

    /// Count rows matching the filter.
    #[instrument(skip(self, filters), fields(table = %table))]
    pub async fn count(
        &self,
        table: &str,
        filters: &HashMap<String, FilterValue>,
    ) -> DuckDbResult<u64> {
        let mut sql = format!("SELECT COUNT(*) as count FROM \"{}\"", table);
        let mut params: Vec<FilterValue> = Vec::new();

        if !filters.is_empty() {
            let mut conditions = Vec::new();
            for (field, value) in filters {
                match value {
                    FilterValue::Null => {
                        conditions.push(format!("\"{}\" IS NULL", field));
                    }
                    _ => {
                        conditions.push(format!("\"{}\" = ?", field));
                        params.push(value.clone());
                    }
                }
            }
            sql.push_str(" WHERE ");
            sql.push_str(&conditions.join(" AND "));
        }

        debug!(sql = %sql, "Executing count");

        let conn = self.pool.get().await?;
        let results = conn.query(&sql, &params).await?;

        let count = results
            .first()
            .and_then(|row| row.get("count"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);

        Ok(count as u64)
    }

    // =========================================================================
    // DuckDB-specific analytical operations
    // =========================================================================

    /// Copy query results to a Parquet file.
    #[instrument(skip(self), fields(query_len = %query.len()))]
    pub async fn copy_to_parquet(&self, query: &str, path: &str) -> DuckDbResult<()> {
        let conn = self.pool.get().await?;
        conn.copy_to_parquet(query, path).await
    }

    /// Copy query results to a CSV file.
    #[instrument(skip(self), fields(query_len = %query.len()))]
    pub async fn copy_to_csv(&self, query: &str, path: &str, header: bool) -> DuckDbResult<()> {
        let conn = self.pool.get().await?;
        conn.copy_to_csv(query, path, header).await
    }

    /// Query a Parquet file.
    pub async fn query_parquet(&self, path: &str) -> DuckDbResult<Vec<DuckDbQueryResult>> {
        let conn = self.pool.get().await?;
        let results = conn.query_parquet(path).await?;
        Ok(results.into_iter().map(DuckDbQueryResult::new).collect())
    }

    /// Query a CSV file.
    pub async fn query_csv(
        &self,
        path: &str,
        header: bool,
    ) -> DuckDbResult<Vec<DuckDbQueryResult>> {
        let conn = self.pool.get().await?;
        let results = conn.query_csv(path, header).await?;
        Ok(results.into_iter().map(DuckDbQueryResult::new).collect())
    }

    /// Query a JSON file.
    pub async fn query_json(&self, path: &str) -> DuckDbResult<Vec<DuckDbQueryResult>> {
        let conn = self.pool.get().await?;
        let results = conn.query_json(path).await?;
        Ok(results.into_iter().map(DuckDbQueryResult::new).collect())
    }

    /// Get DuckDB version.
    pub async fn version(&self) -> DuckDbResult<String> {
        let result = self.raw_sql_first("SELECT version()", &[]).await?;
        result
            .data
            .as_object()
            .and_then(|obj| obj.values().next())
            .and_then(|v| v.as_str())
            .map(String::from)
            .ok_or_else(|| DuckDbError::query("Failed to get version"))
    }

    /// Explain a query plan.
    pub async fn explain(&self, query: &str) -> DuckDbResult<String> {
        let sql = format!("EXPLAIN {}", query);
        let results = self.execute_raw(&sql, &[]).await?;

        let mut plan = String::new();
        for result in results {
            if let Some(obj) = result.data.as_object() {
                for value in obj.values() {
                    if let Some(s) = value.as_str() {
                        plan.push_str(s);
                        plan.push('\n');
                    }
                }
            }
        }

        Ok(plan)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DuckDbConfig;

    #[tokio::test]
    async fn test_engine_creation() {
        let pool = DuckDbPool::new(DuckDbConfig::in_memory()).await.unwrap();
        let engine = DuckDbEngine::new(pool);

        let version = engine.version().await.unwrap();
        assert!(!version.is_empty());
    }

    #[tokio::test]
    async fn test_query_many() {
        let pool = DuckDbPool::new(DuckDbConfig::in_memory()).await.unwrap();
        let engine = DuckDbEngine::new(pool);

        engine
            .raw_sql_batch(
                "CREATE TABLE test (id INTEGER, name VARCHAR);
                 INSERT INTO test VALUES (1, 'Alice'), (2, 'Bob');",
            )
            .await
            .unwrap();

        let results = engine
            .query_many("test", &[], &HashMap::new(), &[], None, None)
            .await
            .unwrap();
        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn test_count() {
        let pool = DuckDbPool::new(DuckDbConfig::in_memory()).await.unwrap();
        let engine = DuckDbEngine::new(pool);

        engine
            .raw_sql_batch(
                "CREATE TABLE test (id INTEGER);
                 INSERT INTO test VALUES (1), (2), (3);",
            )
            .await
            .unwrap();

        let count = engine.count("test", &HashMap::new()).await.unwrap();
        assert_eq!(count, 3);
    }
}
