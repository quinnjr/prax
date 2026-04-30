//! Raw SQL query execution with type-safe parameter interpolation.
//!
//! This module provides a safe way to execute raw SQL queries while still
//! benefiting from parameterized queries to prevent SQL injection.
//!
//! # Creating SQL Queries
//!
//! ```rust
//! use prax_query::Sql;
//!
//! // Simple query
//! let sql = Sql::new("SELECT * FROM users");
//! assert_eq!(sql.sql(), "SELECT * FROM users");
//!
//! // Query with parameters (binding appends placeholder)
//! let sql = Sql::new("SELECT * FROM users WHERE id = ")
//!     .bind(42);
//! assert_eq!(sql.params().len(), 1);
//! ```
//!
//! # Using the raw_query! Macro
//!
//! ```rust
//! use prax_query::raw_query;
//!
//! // Simple query
//! let sql = raw_query!("SELECT 1");
//!
//! // Query with one parameter - {} is replaced with $N placeholder
//! let id = 42;
//! let sql = raw_query!("SELECT * FROM users WHERE id = {}", id);
//! assert_eq!(sql.params().len(), 1);
//! assert!(sql.sql().contains("$1"));
//!
//! // Query with multiple parameters
//! let name = "John";
//! let age = 25;
//! let sql = raw_query!("SELECT * FROM users WHERE name = {} AND age > {}", name, age);
//! assert_eq!(sql.params().len(), 2);
//! ```
//!
//! # Building Queries Incrementally
//!
//! ```rust
//! use prax_query::Sql;
//!
//! // Join multiple conditions
//! let conditions = vec!["active = true", "verified = true"];
//! let sql = Sql::new("SELECT * FROM users WHERE ")
//!     .push(conditions.join(" AND "));
//!
//! assert!(sql.sql().contains("active = true AND verified = true"));
//! ```
//!
//! # Safety
//!
//! All values passed via `raw_query!` are parameterized and never interpolated
//! directly into the SQL string, preventing SQL injection attacks.
//!
//! ```rust
//! use prax_query::raw_query;
//!
//! // This malicious input will NOT cause SQL injection
//! let malicious = "'; DROP TABLE users; --";
//! let sql = raw_query!("SELECT * FROM users WHERE name = {}", malicious);
//!
//! // The malicious string is safely bound as a parameter
//! assert_eq!(sql.params().len(), 1);
//! // The SQL itself doesn't contain the malicious text
//! assert!(!sql.sql().contains("DROP TABLE"));
//! ```

use std::marker::PhantomData;
use tracing::debug;

use crate::error::QueryResult;
use crate::filter::FilterValue;
use crate::sql::DatabaseType;
use crate::traits::{Model, QueryEngine};

/// A raw SQL query with parameterized values.
#[derive(Debug, Clone)]
pub struct Sql {
    /// The SQL string parts (between parameters).
    parts: Vec<String>,
    /// The parameter values.
    params: Vec<FilterValue>,
    /// The database type for parameter formatting.
    db_type: DatabaseType,
}

impl Sql {
    /// Create a new raw SQL query.
    pub fn new(sql: impl Into<String>) -> Self {
        Self {
            parts: vec![sql.into()],
            params: Vec::new(),
            db_type: DatabaseType::PostgreSQL,
        }
    }

    /// Create an empty SQL query.
    pub fn empty() -> Self {
        Self {
            parts: Vec::new(),
            params: Vec::new(),
            db_type: DatabaseType::PostgreSQL,
        }
    }

    /// Set the database type for parameter formatting.
    pub fn with_db_type(mut self, db_type: DatabaseType) -> Self {
        self.db_type = db_type;
        self
    }

    /// Append a literal SQL string.
    pub fn push(mut self, sql: impl Into<String>) -> Self {
        if let Some(last) = self.parts.last_mut() {
            last.push_str(&sql.into());
        } else {
            self.parts.push(sql.into());
        }
        self
    }

    /// Bind a parameter value.
    pub fn bind(mut self, value: impl Into<FilterValue>) -> Self {
        let index = self.params.len() + 1;
        let placeholder = self.db_type.placeholder(index);

        if let Some(last) = self.parts.last_mut() {
            // push_str accepts &str, which Cow<str> derefs to
            last.push_str(&placeholder);
        } else {
            // Convert to owned string for storage
            self.parts.push(placeholder.into_owned());
        }

        self.params.push(value.into());
        self
    }

    /// Bind multiple parameter values at once.
    pub fn bind_many(mut self, values: impl IntoIterator<Item = FilterValue>) -> Self {
        for value in values {
            self = self.bind(value);
        }
        self
    }

    /// Append a conditional clause.
    pub fn push_if(self, condition: bool, sql: impl Into<String>) -> Self {
        if condition { self.push(sql) } else { self }
    }

    /// Bind a parameter conditionally.
    pub fn bind_if(self, condition: bool, value: impl Into<FilterValue>) -> Self {
        if condition { self.bind(value) } else { self }
    }

    /// Push SQL and bind a value together.
    pub fn push_bind(self, sql: impl Into<String>, value: impl Into<FilterValue>) -> Self {
        self.push(sql).bind(value)
    }

    /// Push SQL and bind a value conditionally.
    pub fn push_bind_if(
        self,
        condition: bool,
        sql: impl Into<String>,
        value: impl Into<FilterValue>,
    ) -> Self {
        if condition {
            self.push(sql).bind(value)
        } else {
            self
        }
    }

    /// Add a separator between parts if there are previous parts.
    pub fn separated(self, separator: &str) -> SeparatedSql {
        SeparatedSql {
            sql: self,
            separator: separator.to_string(),
            first: true,
        }
    }

    /// Build the final SQL string and parameters.
    pub fn build(self) -> (String, Vec<FilterValue>) {
        let sql = self.parts.join("");
        debug!(sql_len = sql.len(), param_count = self.params.len(), db_type = ?self.db_type, "Sql::build()");
        (sql, self.params)
    }

    /// Get the SQL string (without consuming).
    pub fn sql(&self) -> String {
        self.parts.join("")
    }

    /// Get the parameters (without consuming).
    pub fn params(&self) -> &[FilterValue] {
        &self.params
    }

    /// Get the number of bound parameters.
    pub fn param_count(&self) -> usize {
        self.params.len()
    }

    /// Check if the query is empty.
    pub fn is_empty(&self) -> bool {
        self.parts.is_empty() || self.parts.iter().all(|p| p.is_empty())
    }
}

impl Default for Sql {
    fn default() -> Self {
        Self::empty()
    }
}

impl std::fmt::Display for Sql {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.parts.join(""))
    }
}

/// A helper for building SQL with separators between items.
#[derive(Debug, Clone)]
pub struct SeparatedSql {
    sql: Sql,
    separator: String,
    first: bool,
}

impl SeparatedSql {
    /// Push a literal SQL string with separator.
    pub fn push(mut self, sql: impl Into<String>) -> Self {
        if !self.first {
            self.sql = self.sql.push(&self.separator);
        }
        self.sql = self.sql.push(sql);
        self.first = false;
        self
    }

    /// Push SQL and bind a value with separator.
    pub fn push_bind(mut self, sql: impl Into<String>, value: impl Into<FilterValue>) -> Self {
        if !self.first {
            self.sql = self.sql.push(&self.separator);
        }
        self.sql = self.sql.push(sql).bind(value);
        self.first = false;
        self
    }

    /// Push SQL and bind conditionally with separator.
    pub fn push_bind_if(
        mut self,
        condition: bool,
        sql: impl Into<String>,
        value: impl Into<FilterValue>,
    ) -> Self {
        if condition {
            if !self.first {
                self.sql = self.sql.push(&self.separator);
            }
            self.sql = self.sql.push(sql).bind(value);
            self.first = false;
        }
        self
    }

    /// Finish and get the underlying Sql.
    pub fn finish(self) -> Sql {
        self.sql
    }

    /// Build the final SQL string and parameters.
    pub fn build(self) -> (String, Vec<FilterValue>) {
        self.sql.build()
    }
}

/// Raw query operation for executing typed queries.
#[derive(Debug)]
pub struct RawQueryOperation<M, E>
where
    M: Model + Send + 'static,
    E: QueryEngine,
{
    _model: PhantomData<M>,
    engine: E,
    sql: Sql,
}

impl<M, E> RawQueryOperation<M, E>
where
    M: Model + crate::row::FromRow + Send + 'static,
    E: QueryEngine,
{
    /// Create a new raw query operation.
    pub fn new(engine: E, sql: Sql) -> Self {
        Self {
            _model: PhantomData,
            engine,
            sql,
        }
    }

    /// Execute the query and return all matching records.
    pub async fn exec(self) -> QueryResult<Vec<M>> {
        let (sql, params) = self.sql.build();
        self.engine.query_many(&sql, params).await
    }

    /// Execute the query and return a single record.
    pub async fn exec_one(self) -> QueryResult<M> {
        let (sql, params) = self.sql.build();
        self.engine.query_one(&sql, params).await
    }

    /// Execute the query and return an optional record.
    pub async fn exec_optional(self) -> QueryResult<Option<M>> {
        let (sql, params) = self.sql.build();
        self.engine.query_optional(&sql, params).await
    }
}

/// Raw execute operation for mutations.
#[derive(Debug)]
pub struct RawExecuteOperation<E>
where
    E: QueryEngine,
{
    engine: E,
    sql: Sql,
}

impl<E> RawExecuteOperation<E>
where
    E: QueryEngine,
{
    /// Create a new raw execute operation.
    pub fn new(engine: E, sql: Sql) -> Self {
        Self { engine, sql }
    }

    /// Execute the mutation and return the number of affected rows.
    pub async fn exec(self) -> QueryResult<u64> {
        let (sql, params) = self.sql.build();
        self.engine.execute_raw(&sql, params).await
    }
}

/// Helper function to create a raw SQL query from a string.
pub fn sql(query: impl Into<String>) -> Sql {
    Sql::new(query)
}

/// Helper function to create a raw SQL query from parts.
///
/// This is typically used with the `raw_query!` macro.
pub fn sql_with_params(sql_str: impl Into<String>, params: Vec<FilterValue>) -> Sql {
    let mut sql = Sql::new(sql_str);
    sql.params = params;
    sql
}

/// A macro for creating raw SQL queries with inline parameter binding.
///
/// # Example
///
/// ```rust,ignore
/// let sql = raw_query!("SELECT * FROM users WHERE id = {} AND active = {}", user_id, true);
/// ```
///
/// The `{}` placeholders are replaced with database-specific parameter markers ($1, $2, etc.
/// for PostgreSQL, ? for MySQL/SQLite) and the values are bound as parameters.
#[macro_export]
macro_rules! raw_query {
    // Base case: just a string, no parameters
    ($sql:expr) => {
        $crate::raw::Sql::new($sql)
    };

    // With parameters
    ($sql:expr, $($params:expr),+ $(,)?) => {{
        let parts: Vec<&str> = $sql.split("{}").collect();
        let param_values: Vec<$crate::filter::FilterValue> = vec![
            $($params.into()),+
        ];

        let mut sql = $crate::raw::Sql::empty();
        let mut param_iter = param_values.into_iter();

        // Interleave parts and parameters
        for (i, part) in parts.iter().enumerate() {
            if !part.is_empty() {
                sql = sql.push(*part);
            }
            if i < parts.len() - 1 {
                if let Some(param) = param_iter.next() {
                    sql = sql.bind(param);
                }
            }
        }

        sql
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sql_new() {
        let sql = Sql::new("SELECT * FROM users");
        assert_eq!(sql.sql(), "SELECT * FROM users");
        assert!(sql.params().is_empty());
    }

    #[test]
    fn test_sql_push() {
        let sql = Sql::new("SELECT * FROM users").push(" WHERE id = 1");
        assert_eq!(sql.sql(), "SELECT * FROM users WHERE id = 1");
    }

    #[test]
    fn test_sql_bind() {
        let sql = Sql::new("SELECT * FROM users WHERE id = ").bind(42i32);
        let (query, params) = sql.build();
        assert_eq!(query, "SELECT * FROM users WHERE id = $1");
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_sql_multiple_binds() {
        let sql = Sql::new("SELECT * FROM users WHERE id = ")
            .bind(42i32)
            .push(" AND name = ")
            .bind("John".to_string());
        let (query, params) = sql.build();
        assert_eq!(query, "SELECT * FROM users WHERE id = $1 AND name = $2");
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn test_sql_push_bind() {
        let sql = Sql::new("SELECT * FROM users WHERE").push_bind(" id = ", 42i32);
        let (query, params) = sql.build();
        assert_eq!(query, "SELECT * FROM users WHERE id = $1");
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_sql_push_if() {
        let include_active = true;
        let include_deleted = false;

        let sql = Sql::new("SELECT * FROM users")
            .push_if(include_active, " WHERE active = true")
            .push_if(include_deleted, " AND deleted = false");

        assert_eq!(sql.sql(), "SELECT * FROM users WHERE active = true");
    }

    #[test]
    #[allow(clippy::unnecessary_literal_unwrap)]
    fn test_sql_bind_if() {
        let filter_id = Some(42i32);
        let filter_name: Option<String> = None;

        let sql = Sql::new("SELECT * FROM users WHERE 1=1")
            .push_bind_if(filter_id.is_some(), " AND id = ", filter_id.unwrap_or(0))
            .push_bind_if(filter_name.is_some(), " AND name = ", String::new());

        let (query, params) = sql.build();
        assert_eq!(query, "SELECT * FROM users WHERE 1=1 AND id = $1");
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_sql_separated() {
        let columns = vec!["id", "name", "email"];

        let mut sep = Sql::new("SELECT ").separated(", ");

        for col in columns {
            sep = sep.push(col);
        }

        let sql = sep.finish().push(" FROM users");
        assert_eq!(sql.sql(), "SELECT id, name, email FROM users");
    }

    #[test]
    fn test_sql_separated_with_binds() {
        let filters = vec![("id", 1i32), ("active", 1i32)];

        let mut sep = Sql::new("SELECT * FROM users WHERE ").separated(" AND ");

        for (col, val) in filters {
            sep = sep.push_bind(format!("{} = ", col), val);
        }

        let (query, params) = sep.build();
        assert_eq!(query, "SELECT * FROM users WHERE id = $1 AND active = $2");
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn test_sql_mysql() {
        let sql = Sql::new("SELECT * FROM users WHERE id = ")
            .with_db_type(DatabaseType::MySQL)
            .bind(42i32);
        let (query, params) = sql.build();
        assert_eq!(query, "SELECT * FROM users WHERE id = ?");
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_sql_sqlite() {
        let sql = Sql::new("SELECT * FROM users WHERE id = ")
            .with_db_type(DatabaseType::SQLite)
            .bind(42i32);
        let (query, params) = sql.build();
        assert_eq!(query, "SELECT * FROM users WHERE id = ?");
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_sql_is_empty() {
        assert!(Sql::empty().is_empty());
        assert!(!Sql::new("SELECT 1").is_empty());
    }

    #[test]
    fn test_sql_display() {
        let sql = Sql::new("SELECT * FROM users WHERE id = ").bind(42i32);
        assert_eq!(format!("{}", sql), "SELECT * FROM users WHERE id = $1");
    }

    #[test]
    fn test_raw_query_macro_no_params() {
        let sql = raw_query!("SELECT * FROM users");
        assert_eq!(sql.sql(), "SELECT * FROM users");
        assert!(sql.params().is_empty());
    }

    #[test]
    fn test_raw_query_macro_with_params() {
        let sql = raw_query!(
            "SELECT * FROM users WHERE id = {} AND active = {}",
            42i32,
            true
        );
        let (query, params) = sql.build();
        assert_eq!(query, "SELECT * FROM users WHERE id = $1 AND active = $2");
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn test_raw_query_macro_string_params() {
        let name = "John".to_string();
        let sql = raw_query!("SELECT * FROM users WHERE name = {}", name);
        let (query, params) = sql.build();
        assert_eq!(query, "SELECT * FROM users WHERE name = $1");
        assert_eq!(params.len(), 1);
    }

    #[test]
    fn test_bind_many() {
        let values: Vec<FilterValue> = vec![
            FilterValue::Int(1),
            FilterValue::Int(2),
            FilterValue::Int(3),
        ];

        let sql = Sql::new("SELECT * FROM users WHERE id IN (")
            .bind_many(values)
            .push(")");

        let (query, params) = sql.build();
        assert_eq!(query, "SELECT * FROM users WHERE id IN ($1$2$3)");
        assert_eq!(params.len(), 3);
    }

    #[test]
    fn test_build_in_clause() {
        let ids = vec![1, 2, 3];

        let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("${}", i)).collect();

        let sql = Sql::new(format!(
            "SELECT * FROM users WHERE id IN ({})",
            placeholders.join(", ")
        ));

        let params: Vec<FilterValue> = ids.into_iter().map(FilterValue::Int).collect();
        let sql = sql_with_params(sql.sql(), params);

        let (query, params) = sql.build();
        assert_eq!(query, "SELECT * FROM users WHERE id IN ($1, $2, $3)");
        assert_eq!(params.len(), 3);
    }
}
