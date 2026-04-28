//! Legacy JSON-returning MySQL engine.
//!
//! The primary `MysqlEngine` now implements `QueryEngine` and decodes rows
//! into typed models via `FromRow`. This module preserves the old JSON-blob
//! API as an escape hatch for callers that want untyped rows.

use std::collections::HashMap;

use mysql_async::prelude::*;
use mysql_async::{Params, Row, Value};
use serde_json::Value as JsonValue;
use tracing::{debug, instrument};

use prax_query::filter::FilterValue;
use prax_query::types::SortOrder;

use crate::error::MysqlError;
use crate::pool::MysqlPool;
use crate::types::{filter_value_to_mysql, from_mysql_value};

/// MySQL raw engine returning JSON-typed results.
pub struct MysqlRawEngine {
    pool: MysqlPool,
}

/// Result of a query operation returning JSON.
#[derive(Debug, Clone)]
pub struct MysqlJsonRow {
    /// The result data as JSON.
    pub data: JsonValue,
}

impl MysqlJsonRow {
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

impl MysqlRawEngine {
    /// Create a new MySQL raw engine with the given pool.
    pub fn new(pool: MysqlPool) -> Self {
        Self { pool }
    }

    /// Get a reference to the connection pool.
    pub fn pool(&self) -> &MysqlPool {
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
                .map(|c| format!("`{}`", c))
                .collect::<Vec<_>>()
                .join(", ")
        };
        sql.push_str(&format!("SELECT {} FROM `{}`", cols, table));

        // WHERE clause
        if !filters.is_empty() {
            let mut conditions = Vec::new();
            for (field, value) in filters {
                match value {
                    FilterValue::Null => {
                        conditions.push(format!("`{}` IS NULL", field));
                    }
                    _ => {
                        conditions.push(format!("`{}` = ?", field));
                        params.push(filter_value_to_mysql(value));
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
                    format!("`{}` {}", col, direction)
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
            columns.push(format!("`{}`", col));
            placeholders.push("?".to_string());
            params.push(filter_value_to_mysql(val));
        }

        let sql = format!(
            "INSERT INTO `{}` ({}) VALUES ({})",
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
                params.push(filter_value_to_mysql(val));
                format!("`{}` = ?", col)
            })
            .collect();

        let mut sql = format!("UPDATE `{}` SET {}", table, set_parts.join(", "));

        // WHERE clause
        if !filters.is_empty() {
            let mut conditions = Vec::new();
            for (field, value) in filters {
                match value {
                    FilterValue::Null => {
                        conditions.push(format!("`{}` IS NULL", field));
                    }
                    _ => {
                        conditions.push(format!("`{}` = ?", field));
                        params.push(filter_value_to_mysql(value));
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
        let mut sql = format!("DELETE FROM `{}`", table);
        let mut params: Vec<Value> = Vec::new();

        if !filters.is_empty() {
            let mut conditions = Vec::new();
            for (field, value) in filters {
                match value {
                    FilterValue::Null => {
                        conditions.push(format!("`{}` IS NULL", field));
                    }
                    _ => {
                        conditions.push(format!("`{}` = ?", field));
                        params.push(filter_value_to_mysql(value));
                    }
                }
            }
            sql.push_str(" WHERE ");
            sql.push_str(&conditions.join(" AND "));
        }

        (sql, params)
    }

    /// Convert a MySQL row to a JSON object.
    fn row_to_json(&self, row: &Row) -> JsonValue {
        let mut map = serde_json::Map::new();

        for (i, column) in row.columns_ref().iter().enumerate() {
            let name = column.name_str().to_string();
            let value: Option<Value> = row.get(i);
            map.insert(name, from_mysql_value(value.unwrap_or(Value::NULL)));
        }

        JsonValue::Object(map)
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
    ) -> Result<Vec<MysqlJsonRow>, MysqlError> {
        let (sql, params) = self.build_select(table, columns, filters, sort, limit, offset);
        debug!(sql = %sql, "Executing query_many");

        let mut conn = self.pool.get().await?;

        let rows: Vec<Row> = conn
            .inner_mut()
            .exec(&sql, Params::Positional(params))
            .await?;

        let results: Vec<MysqlJsonRow> = rows
            .iter()
            .map(|row| MysqlJsonRow::new(self.row_to_json(row)))
            .collect();

        Ok(results)
    }

    /// Execute a query and return a single result.
    #[instrument(skip(self, columns, filters), fields(table = %table))]
    pub async fn query_one(
        &self,
        table: &str,
        columns: &[String],
        filters: &HashMap<String, FilterValue>,
    ) -> Result<MysqlJsonRow, MysqlError> {
        let (sql, params) = self.build_select(table, columns, filters, &[], Some(1), None);
        debug!(sql = %sql, "Executing query_one");

        let mut conn = self.pool.get().await?;

        let row: Option<Row> = conn
            .inner_mut()
            .exec_first(&sql, Params::Positional(params))
            .await?;

        match row {
            Some(r) => Ok(MysqlJsonRow::new(self.row_to_json(&r))),
            None => Err(MysqlError::query(format!(
                "No row found in table '{}' with the given filters",
                table
            ))),
        }
    }

    /// Execute a query and return an optional result.
    #[instrument(skip(self, columns, filters), fields(table = %table))]
    pub async fn query_optional(
        &self,
        table: &str,
        columns: &[String],
        filters: &HashMap<String, FilterValue>,
    ) -> Result<Option<MysqlJsonRow>, MysqlError> {
        let (sql, params) = self.build_select(table, columns, filters, &[], Some(1), None);
        debug!(sql = %sql, "Executing query_optional");

        let mut conn = self.pool.get().await?;

        let row: Option<Row> = conn
            .inner_mut()
            .exec_first(&sql, Params::Positional(params))
            .await?;

        Ok(row.map(|r| MysqlJsonRow::new(self.row_to_json(&r))))
    }

    /// Execute an INSERT and return the result.
    #[instrument(skip(self, data), fields(table = %table))]
    pub async fn execute_insert(
        &self,
        table: &str,
        data: &HashMap<String, FilterValue>,
    ) -> Result<MysqlJsonRow, MysqlError> {
        let (sql, params) = self.build_insert(table, data);
        debug!(sql = %sql, "Executing insert");

        let mut conn = self.pool.get().await?;

        conn.inner_mut()
            .exec_drop(&sql, Params::Positional(params))
            .await?;

        let last_insert_id = conn.inner().last_insert_id().unwrap_or(0);

        // Return the inserted row
        let mut result = data.clone();
        if !result.contains_key("id") {
            result.insert("id".to_string(), FilterValue::Int(last_insert_id as i64));
        }

        let json = result
            .into_iter()
            .map(|(k, v)| (k, filter_value_to_json(&v)))
            .collect::<serde_json::Map<_, _>>();

        Ok(MysqlJsonRow::new(JsonValue::Object(json)))
    }

    /// Execute an UPDATE and return the number of affected rows.
    #[instrument(skip(self, data, filters), fields(table = %table))]
    pub async fn execute_update(
        &self,
        table: &str,
        data: &HashMap<String, FilterValue>,
        filters: &HashMap<String, FilterValue>,
    ) -> Result<u64, MysqlError> {
        let (sql, params) = self.build_update(table, data, filters);
        debug!(sql = %sql, "Executing update");

        let mut conn = self.pool.get().await?;

        conn.inner_mut()
            .exec_drop(&sql, Params::Positional(params))
            .await?;

        Ok(conn.inner().affected_rows())
    }

    /// Execute a DELETE and return the number of affected rows.
    #[instrument(skip(self, filters), fields(table = %table))]
    pub async fn execute_delete(
        &self,
        table: &str,
        filters: &HashMap<String, FilterValue>,
    ) -> Result<u64, MysqlError> {
        let (sql, params) = self.build_delete(table, filters);
        debug!(sql = %sql, "Executing delete");

        let mut conn = self.pool.get().await?;

        conn.inner_mut()
            .exec_drop(&sql, Params::Positional(params))
            .await?;

        Ok(conn.inner().affected_rows())
    }

    /// Execute raw SQL and return results.
    #[instrument(skip(self, params), fields(sql = %sql))]
    pub async fn execute_raw(
        &self,
        sql: &str,
        params: &[FilterValue],
    ) -> Result<Vec<MysqlJsonRow>, MysqlError> {
        debug!("Executing raw SQL");

        let mysql_params: Vec<Value> = params.iter().map(filter_value_to_mysql).collect();

        let mut conn = self.pool.get().await?;

        let rows: Vec<Row> = conn
            .inner_mut()
            .exec(sql, Params::Positional(mysql_params))
            .await?;

        let results: Vec<MysqlJsonRow> = rows
            .iter()
            .map(|row| MysqlJsonRow::new(self.row_to_json(row)))
            .collect();

        Ok(results)
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
    ) -> Result<Vec<MysqlJsonRow>, MysqlError> {
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
    ) -> Result<Vec<MysqlJsonRow>, MysqlError> {
        debug!("Executing raw SQL query");

        let mysql_params: Vec<Value> = params.iter().map(filter_value_to_mysql).collect();

        let mut conn = self.pool.get().await?;

        let rows: Vec<Row> = conn
            .inner_mut()
            .exec(sql, Params::Positional(mysql_params))
            .await?;

        let results: Vec<MysqlJsonRow> = rows
            .iter()
            .map(|row| MysqlJsonRow::new(self.row_to_json(row)))
            .collect();

        Ok(results)
    }

    /// Execute a raw SQL statement and return the number of affected rows.
    ///
    /// Use this for INSERT, UPDATE, DELETE, or other statements that don't return rows.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let affected = engine.raw_sql_execute(
    ///     "UPDATE users SET last_login = NOW() WHERE id = ?",
    ///     &[FilterValue::Int(user_id)]
    /// ).await?;
    /// println!("Updated {} rows", affected);
    /// ```
    #[instrument(skip(self, params), fields(sql = %sql))]
    pub async fn raw_sql_execute(
        &self,
        sql: &str,
        params: &[FilterValue],
    ) -> Result<u64, MysqlError> {
        debug!("Executing raw SQL statement");

        let mysql_params: Vec<Value> = params.iter().map(filter_value_to_mysql).collect();

        let mut conn = self.pool.get().await?;

        conn.inner_mut()
            .exec_drop(sql, Params::Positional(mysql_params))
            .await?;

        Ok(conn.inner().affected_rows())
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
    ) -> Result<MysqlJsonRow, MysqlError> {
        debug!("Executing raw SQL first");

        let mysql_params: Vec<Value> = params.iter().map(filter_value_to_mysql).collect();

        let mut conn = self.pool.get().await?;

        let row: Option<Row> = conn
            .inner_mut()
            .exec_first(sql, Params::Positional(mysql_params))
            .await?;

        match row {
            Some(r) => Ok(MysqlJsonRow::new(self.row_to_json(&r))),
            None => Err(MysqlError::query("raw_sql_first returned no rows")),
        }
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
    ) -> Result<Option<MysqlJsonRow>, MysqlError> {
        debug!("Executing raw SQL optional");

        let mysql_params: Vec<Value> = params.iter().map(filter_value_to_mysql).collect();

        let mut conn = self.pool.get().await?;

        let row: Option<Row> = conn
            .inner_mut()
            .exec_first(sql, Params::Positional(mysql_params))
            .await?;

        Ok(row.map(|r| MysqlJsonRow::new(self.row_to_json(&r))))
    }

    /// Execute a raw SQL query and return a single scalar value as JSON.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let result = engine.raw_sql_scalar(
    ///     "SELECT COUNT(*) as count FROM users WHERE active = ?",
    ///     &[FilterValue::Bool(true)]
    /// ).await?;
    /// let count = result.as_i64().unwrap_or(0);
    /// ```
    #[instrument(skip(self, params), fields(sql = %sql))]
    pub async fn raw_sql_scalar(
        &self,
        sql: &str,
        params: &[FilterValue],
    ) -> Result<JsonValue, MysqlError> {
        debug!("Executing raw SQL scalar");

        let mysql_params: Vec<Value> = params.iter().map(filter_value_to_mysql).collect();

        let mut conn = self.pool.get().await?;

        let row: Option<Row> = conn
            .inner_mut()
            .exec_first(sql, Params::Positional(mysql_params))
            .await?;

        match row {
            Some(r) => {
                // Get the first column value
                if r.columns_ref().is_empty() {
                    return Err(MysqlError::query("raw_sql_scalar returned empty row"));
                }
                let value: Option<Value> = r.get(0);
                Ok(from_mysql_value(value.unwrap_or(Value::NULL)))
            }
            None => Err(MysqlError::query("raw_sql_scalar returned no rows")),
        }
    }

    /// Execute multiple raw SQL statements in a batch (without parameters).
    ///
    /// Note: This does not support parameterized queries. For parameterized
    /// queries, execute each statement individually.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// engine.raw_sql_batch(r#"
    ///     CREATE TABLE IF NOT EXISTS users (id INT PRIMARY KEY);
    ///     CREATE TABLE IF NOT EXISTS posts (id INT PRIMARY KEY);
    /// "#).await?;
    /// ```
    #[instrument(skip(self), fields(sql_len = %sql.len()))]
    pub async fn raw_sql_batch(&self, sql: &str) -> Result<(), MysqlError> {
        debug!("Executing raw SQL batch");

        let mut conn = self.pool.get().await?;

        // MySQL doesn't support multi-statement queries by default,
        // so we split and execute each statement individually
        for statement in sql.split(';').filter(|s| !s.trim().is_empty()) {
            conn.inner_mut().query_drop(statement.trim()).await?;
        }

        Ok(())
    }

    /// Count rows matching the filter.
    #[instrument(skip(self, filters), fields(table = %table))]
    pub async fn count(
        &self,
        table: &str,
        filters: &HashMap<String, FilterValue>,
    ) -> Result<u64, MysqlError> {
        let mut sql = format!("SELECT COUNT(*) as count FROM `{}`", table);
        let mut params: Vec<Value> = Vec::new();

        if !filters.is_empty() {
            let mut conditions = Vec::new();
            for (field, value) in filters {
                match value {
                    FilterValue::Null => {
                        conditions.push(format!("`{}` IS NULL", field));
                    }
                    _ => {
                        conditions.push(format!("`{}` = ?", field));
                        params.push(filter_value_to_mysql(value));
                    }
                }
            }
            sql.push_str(" WHERE ");
            sql.push_str(&conditions.join(" AND "));
        }

        debug!(sql = %sql, "Executing count");

        let mut conn = self.pool.get().await?;

        let count: Option<u64> = conn
            .inner_mut()
            .exec_first(&sql, Params::Positional(params))
            .await?;

        Ok(count.unwrap_or(0))
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
    fn test_build_select_simple() {
        let sql = "SELECT * FROM `users`";
        assert!(sql.contains("SELECT"));
        assert!(sql.contains("users"));
    }

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
    fn test_query_result() {
        let result = MysqlJsonRow::new(JsonValue::Object(serde_json::Map::new()));
        assert!(result.json().is_object());
    }

    #[test]
    fn test_query_result_into_json() {
        let json = JsonValue::Object(serde_json::Map::new());
        let result = MysqlJsonRow::new(json.clone());
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
