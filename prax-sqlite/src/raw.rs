//! Legacy JSON-first SQLite query engine.
//!
//! This module preserves the original JSON-returning API for backward compatibility.
//! New code should use the typed `SqliteEngine` from `engine.rs` instead.

use std::collections::HashMap;

use rusqlite::types::Value;
use serde_json::Value as JsonValue;
use tracing::{debug, instrument};

use prax_query::filter::FilterValue;
use prax_query::types::SortOrder;

use crate::error::SqliteError;
use crate::pool::SqlitePool;
use crate::types::filter_value_to_sqlite;

/// Legacy SQLite query engine with JSON results.
#[derive(Clone)]
pub struct SqliteRawEngine {
    pool: SqlitePool,
}

/// Result of a query operation (JSON-based).
#[derive(Debug, Clone)]
pub struct SqliteJsonRow {
    /// The result data as JSON.
    pub data: JsonValue,
}

impl SqliteJsonRow {
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

impl SqliteRawEngine {
    /// Create a new SQLite engine with the given pool.
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Get a reference to the connection pool.
    pub fn pool(&self) -> &SqlitePool {
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
    ) -> (String, Vec<Value>) {
        let mut sql = String::new();
        let mut params: Vec<Value> = Vec::new();

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
                        params.push(filter_value_to_sqlite(value));
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
    ) -> (String, Vec<Value>) {
        let mut columns = Vec::new();
        let mut placeholders = Vec::new();
        let mut params: Vec<Value> = Vec::new();

        for (col, val) in data {
            columns.push(format!("\"{}\"", col));
            placeholders.push("?".to_string());
            params.push(filter_value_to_sqlite(val));
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
    ) -> (String, Vec<Value>) {
        let mut params: Vec<Value> = Vec::new();

        // SET clause
        let set_parts: Vec<String> = data
            .iter()
            .map(|(col, val)| {
                params.push(filter_value_to_sqlite(val));
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
                        params.push(filter_value_to_sqlite(value));
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
    ) -> (String, Vec<Value>) {
        let mut sql = format!("DELETE FROM \"{}\"", table);
        let mut params: Vec<Value> = Vec::new();

        if !filters.is_empty() {
            let mut conditions = Vec::new();
            for (field, value) in filters {
                match value {
                    FilterValue::Null => {
                        conditions.push(format!("\"{}\" IS NULL", field));
                    }
                    _ => {
                        conditions.push(format!("\"{}\" = ?", field));
                        params.push(filter_value_to_sqlite(value));
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
    ) -> Result<Vec<SqliteJsonRow>, SqliteError> {
        let (sql, params) = self.build_select(table, columns, filters, sort, limit, offset);
        debug!(sql = %sql, "Executing query_many");

        let conn = self.pool.get().await?;

        let results = conn.query_params(&sql, params).await?;

        Ok(results.into_iter().map(SqliteJsonRow::new).collect())
    }

    /// Execute a query and return a single result.
    #[instrument(skip(self, columns, filters), fields(table = %table))]
    pub async fn query_one(
        &self,
        table: &str,
        columns: &[String],
        filters: &HashMap<String, FilterValue>,
    ) -> Result<SqliteJsonRow, SqliteError> {
        let (sql, params) = self.build_select(table, columns, filters, &[], Some(1), None);
        debug!(sql = %sql, "Executing query_one");

        let conn = self.pool.get().await?;

        let results = conn.query_params(&sql, params).await?;

        results
            .into_iter()
            .next()
            .map(SqliteJsonRow::new)
            .ok_or_else(|| {
                SqliteError::query(format!(
                    "No row found in table '{}' with the given filters",
                    table
                ))
            })
    }

    /// Execute a query and return an optional result.
    #[instrument(skip(self, columns, filters), fields(table = %table))]
    pub async fn query_optional(
        &self,
        table: &str,
        columns: &[String],
        filters: &HashMap<String, FilterValue>,
    ) -> Result<Option<SqliteJsonRow>, SqliteError> {
        let (sql, params) = self.build_select(table, columns, filters, &[], Some(1), None);
        debug!(sql = %sql, "Executing query_optional");

        let conn = self.pool.get().await?;

        let results = conn.query_params(&sql, params).await?;

        Ok(results.into_iter().next().map(SqliteJsonRow::new))
    }

    /// Execute an INSERT and return the result.
    #[instrument(skip(self, data), fields(table = %table))]
    pub async fn execute_insert(
        &self,
        table: &str,
        data: &HashMap<String, FilterValue>,
    ) -> Result<SqliteJsonRow, SqliteError> {
        let (sql, params) = self.build_insert(table, data);
        debug!(sql = %sql, "Executing insert");

        let conn = self.pool.get().await?;

        let last_rowid = conn.execute_insert_params(&sql, params).await?;

        // Return the inserted row
        let mut result = data.clone();
        if !result.contains_key("id") {
            result.insert("id".to_string(), FilterValue::Int(last_rowid));
        }

        let json = result
            .into_iter()
            .map(|(k, v)| (k, filter_value_to_json(&v)))
            .collect::<serde_json::Map<_, _>>();

        Ok(SqliteJsonRow::new(JsonValue::Object(json)))
    }

    /// Execute an UPDATE and return the number of affected rows.
    #[instrument(skip(self, data, filters), fields(table = %table))]
    pub async fn execute_update(
        &self,
        table: &str,
        data: &HashMap<String, FilterValue>,
        filters: &HashMap<String, FilterValue>,
    ) -> Result<u64, SqliteError> {
        let (sql, params) = self.build_update(table, data, filters);
        debug!(sql = %sql, "Executing update");

        let conn = self.pool.get().await?;

        let affected = conn.execute_params(&sql, params).await?;

        Ok(affected as u64)
    }

    /// Execute a DELETE and return the number of affected rows.
    #[instrument(skip(self, filters), fields(table = %table))]
    pub async fn execute_delete(
        &self,
        table: &str,
        filters: &HashMap<String, FilterValue>,
    ) -> Result<u64, SqliteError> {
        let (sql, params) = self.build_delete(table, filters);
        debug!(sql = %sql, "Executing delete");

        let conn = self.pool.get().await?;

        let affected = conn.execute_params(&sql, params).await?;

        Ok(affected as u64)
    }

    /// Execute raw SQL and return results.
    #[instrument(skip(self, params), fields(sql = %sql))]
    pub async fn execute_raw(
        &self,
        sql: &str,
        params: &[FilterValue],
    ) -> Result<Vec<SqliteJsonRow>, SqliteError> {
        debug!("Executing raw SQL");

        let sqlite_params: Vec<Value> = params.iter().map(filter_value_to_sqlite).collect();

        let conn = self.pool.get().await?;

        let results = conn.query_params(sql, sqlite_params).await?;

        Ok(results.into_iter().map(SqliteJsonRow::new).collect())
    }

    // =========================================================================
    // Raw SQL Functions
    // =========================================================================

    /// Execute a raw SQL query using the `Sql` builder from prax-query.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use prax_query::raw::Sql;
    ///
    /// let sql = Sql::new("SELECT * FROM users WHERE age > ")
    ///     .bind(18)
    ///     .push(" AND active = ")
    ///     .bind(true);
    ///
    /// let results = engine.raw_sql(sql).await?;
    /// ```
    #[instrument(skip(self, sql))]
    pub async fn raw_sql(
        &self,
        sql: prax_query::raw::Sql,
    ) -> Result<Vec<SqliteJsonRow>, SqliteError> {
        let (query_string, params) = sql.build();
        debug!(sql = %query_string, "Executing raw SQL from builder");
        self.raw_sql_query(&query_string, &params).await
    }

    /// Execute a raw SQL query string with parameters and return results.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let results = engine.raw_sql_query(
    ///     "SELECT * FROM users WHERE age > ? AND active = ?",
    ///     &[FilterValue::Int(18), FilterValue::Bool(true)]
    /// ).await?;
    /// ```
    #[instrument(skip(self, params), fields(sql = %sql))]
    pub async fn raw_sql_query(
        &self,
        sql: &str,
        params: &[FilterValue],
    ) -> Result<Vec<SqliteJsonRow>, SqliteError> {
        debug!("Executing raw SQL query");

        let sqlite_params: Vec<Value> = params.iter().map(filter_value_to_sqlite).collect();

        let conn = self.pool.get().await?;

        let results = conn.query_params(sql, sqlite_params).await?;

        Ok(results.into_iter().map(SqliteJsonRow::new).collect())
    }

    /// Execute a raw SQL statement and return the number of affected rows.
    ///
    /// Use this for INSERT, UPDATE, DELETE, or other statements that don't return rows.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let affected = engine.raw_sql_execute(
    ///     "UPDATE users SET last_login = datetime('now') WHERE id = ?",
    ///     &[FilterValue::Int(user_id)]
    /// ).await?;
    /// println!("Updated {} rows", affected);
    /// ```
    #[instrument(skip(self, params), fields(sql = %sql))]
    pub async fn raw_sql_execute(
        &self,
        sql: &str,
        params: &[FilterValue],
    ) -> Result<u64, SqliteError> {
        debug!("Executing raw SQL statement");

        let sqlite_params: Vec<Value> = params.iter().map(filter_value_to_sqlite).collect();

        let conn = self.pool.get().await?;

        let affected = conn.execute_params(sql, sqlite_params).await?;

        Ok(affected as u64)
    }

    /// Execute a raw SQL query and return the first result.
    ///
    /// Returns an error if no rows are returned.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let user = engine.raw_sql_first(
    ///     "SELECT * FROM users WHERE id = ?",
    ///     &[FilterValue::Int(user_id)]
    /// ).await?;
    /// ```
    #[instrument(skip(self, params), fields(sql = %sql))]
    pub async fn raw_sql_first(
        &self,
        sql: &str,
        params: &[FilterValue],
    ) -> Result<SqliteJsonRow, SqliteError> {
        debug!("Executing raw SQL first");

        let sqlite_params: Vec<Value> = params.iter().map(filter_value_to_sqlite).collect();

        let conn = self.pool.get().await?;

        let results = conn.query_params(sql, sqlite_params).await?;

        results
            .into_iter()
            .next()
            .map(SqliteJsonRow::new)
            .ok_or_else(|| SqliteError::query("raw_sql_first returned no rows"))
    }

    /// Execute a raw SQL query and return the first result, or None if no rows.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let user = engine.raw_sql_optional(
    ///     "SELECT * FROM users WHERE email = ?",
    ///     &[FilterValue::String("test@example.com".into())]
    /// ).await?;
    /// ```
    #[instrument(skip(self, params), fields(sql = %sql))]
    pub async fn raw_sql_optional(
        &self,
        sql: &str,
        params: &[FilterValue],
    ) -> Result<Option<SqliteJsonRow>, SqliteError> {
        debug!("Executing raw SQL optional");

        let sqlite_params: Vec<Value> = params.iter().map(filter_value_to_sqlite).collect();

        let conn = self.pool.get().await?;

        let results = conn.query_params(sql, sqlite_params).await?;

        Ok(results.into_iter().next().map(SqliteJsonRow::new))
    }

    /// Execute a raw SQL query and return a single scalar value.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let count: i64 = engine.raw_sql_scalar(
    ///     "SELECT COUNT(*) FROM users WHERE active = ?",
    ///     &[FilterValue::Bool(true)]
    /// ).await?;
    /// ```
    #[instrument(skip(self, params), fields(sql = %sql))]
    pub async fn raw_sql_scalar<T>(
        &self,
        sql: &str,
        params: &[FilterValue],
    ) -> Result<T, SqliteError>
    where
        T: for<'a> serde::Deserialize<'a>,
    {
        debug!("Executing raw SQL scalar");

        let sqlite_params: Vec<Value> = params.iter().map(filter_value_to_sqlite).collect();

        let conn = self.pool.get().await?;

        let results = conn.query_params(sql, sqlite_params).await?;

        let row = results
            .into_iter()
            .next()
            .ok_or_else(|| SqliteError::query("raw_sql_scalar returned no rows"))?;

        // Get the first column value
        let value = row
            .as_object()
            .and_then(|obj| obj.values().next())
            .ok_or_else(|| SqliteError::query("raw_sql_scalar returned empty row"))?;

        serde_json::from_value(value.clone()).map_err(|e| {
            SqliteError::deserialization(format!("failed to deserialize scalar: {}", e))
        })
    }

    /// Execute multiple raw SQL statements in a batch.
    ///
    /// This is useful for running schema migrations or multiple DDL statements.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// engine.raw_sql_batch(r#"
    ///     CREATE TABLE IF NOT EXISTS users (id INTEGER PRIMARY KEY);
    ///     CREATE TABLE IF NOT EXISTS posts (id INTEGER PRIMARY KEY);
    /// "#).await?;
    /// ```
    #[instrument(skip(self), fields(sql_len = %sql.len()))]
    pub async fn raw_sql_batch(&self, sql: &str) -> Result<(), SqliteError> {
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
    ) -> Result<u64, SqliteError> {
        let mut sql = format!("SELECT COUNT(*) as count FROM \"{}\"", table);
        let mut params: Vec<Value> = Vec::new();

        if !filters.is_empty() {
            let mut conditions = Vec::new();
            for (field, value) in filters {
                match value {
                    FilterValue::Null => {
                        conditions.push(format!("\"{}\" IS NULL", field));
                    }
                    _ => {
                        conditions.push(format!("\"{}\" = ?", field));
                        params.push(filter_value_to_sqlite(value));
                    }
                }
            }
            sql.push_str(" WHERE ");
            sql.push_str(&conditions.join(" AND "));
        }

        debug!(sql = %sql, "Executing count");

        let conn = self.pool.get().await?;

        let results = conn.query_params(&sql, params).await?;

        // Extract count from first row
        let count = results
            .first()
            .and_then(|row| row.get("count"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);

        Ok(count as u64)
    }
}

/// Convert a FilterValue to JSON.
fn filter_value_to_json(value: &FilterValue) -> JsonValue {
    match value {
        FilterValue::Null => JsonValue::Null,
        FilterValue::Bool(b) => JsonValue::Bool(*b),
        FilterValue::Int(i) => JsonValue::Number((*i).into()),
        FilterValue::Float(f) => serde_json::Number::from_f64(*f)
            .map(JsonValue::Number)
            .unwrap_or(JsonValue::Null),
        FilterValue::String(s) => JsonValue::String(s.clone()),
        FilterValue::Json(j) => j.clone(),
        FilterValue::List(list) => {
            JsonValue::Array(list.iter().map(filter_value_to_json).collect())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_value_to_json() {
        assert_eq!(filter_value_to_json(&FilterValue::Null), JsonValue::Null);
        assert_eq!(
            filter_value_to_json(&FilterValue::Bool(true)),
            JsonValue::Bool(true)
        );
        assert_eq!(
            filter_value_to_json(&FilterValue::Int(42)),
            JsonValue::Number(42.into())
        );
        assert_eq!(
            filter_value_to_json(&FilterValue::String("test".to_string())),
            JsonValue::String("test".to_string())
        );
    }

    #[test]
    fn test_build_select_simple() {
        let sql = "SELECT * FROM \"users\"";
        assert!(sql.contains("SELECT"));
        assert!(sql.contains("users"));
    }

    #[test]
    fn test_query_result() {
        let result = SqliteJsonRow::new(JsonValue::Object(serde_json::Map::new()));
        assert!(result.json().is_object());
    }

    #[test]
    fn test_query_result_into_json() {
        let json = JsonValue::Object(serde_json::Map::new());
        let result = SqliteJsonRow::new(json.clone());
        assert_eq!(result.into_json(), json);
    }

    #[test]
    fn test_sql_builder_integration() {
        use prax_query::raw::Sql;

        let sql = Sql::new("SELECT * FROM users WHERE age > ")
            .bind(18)
            .push(" AND active = ")
            .bind(true);

        let (query, params) = sql.build();
        assert!(query.contains("SELECT"));
        assert!(query.contains("users"));
        assert_eq!(params.len(), 2);
    }
}
